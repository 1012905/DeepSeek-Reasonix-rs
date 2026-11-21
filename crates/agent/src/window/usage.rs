//! Context fill accounting (Go: `usage_anchor.go`, `context_usage.go`,
//! `run_usage.go` `storeLatestRequestUsage`).
//!
//! The one reading of how full the context is, for the gauge and the fold
//! trigger alike: the provider's last reported size plus the growth since,
//! where there is an anchor, and the local estimate where there is none.
//! The provider's own count outranks the estimate, high or low; the hard
//! ceiling reads the estimate alone.

use reasonix_provider::Usage;

use super::estimate::{OutputEstimateState, RequestCalibrationShape, estimated_shape_tokens_with};
use super::{Window, estimate};

/// Ties one provider-reported prompt size to the visible view that request
/// carried (Go: `usageAnchor`). `prompt_tokens` stays zero until the usage
/// arrives, so a request whose stream ended without one leaves no anchor to
/// read.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UsageAnchor {
    /// The request's prompt tokens as the provider counted it, cached
    /// tokens included.
    pub prompt_tokens: i64,
    pub visible: RequestCalibrationShape,
    pub projection_version: u64,
}

/// The gauge's memoised prompt size, keyed on everything that can change
/// the answer: the transcript, projection, calibration, anchor, and
/// provider-visible tool schemas (Go: `contextUsage`).
#[derive(Debug, Clone)]
pub struct ContextUsageMemo {
    pub transcript_version: u64,
    pub projection_version: u64,
    pub calibration: Option<estimate::PromptTokenCalibration>,
    pub anchor: Option<UsageAnchor>,
    pub tool_schema_revision: u64,
    pub tokens: i64,
}

/// Records the shape the request about to be sent carries (Go: the
/// `activeReqShape` store in `prepareSamplingRequest`).
pub fn store_active_shape(window: &Window, shape: RequestCalibrationShape) {
    OutputEstimateState::lock(window.output_state()).active_shape = Some(shape);
}

/// Arms an anchor for the request about to be sent: shape and projection
/// recorded, prompt size left zero until the provider reports one (Go: the
/// `usageAnchor` store in `prepareSamplingRequest`).
pub fn store_anchor_shape(window: &Window, shape: RequestCalibrationShape, projection_version: u64) {
    OutputEstimateState::lock(window.output_state()).usage_anchor = Some(UsageAnchor {
        prompt_tokens: 0,
        visible: shape,
        projection_version,
    });
}

/// Completes the anchor of the request in flight. It trusts the same usage
/// as calibration does, minus the image rule: pixels the host cannot count
/// are exactly what the anchor is for (Go: `anchorProviderUsage`). A turn
/// the provider padded with its own tool results counts pages no fold can
/// remove, so it anchors nothing (`ServerToolRequests` guard noted in
/// `set_prompt_token_calibration_from_usage`).
pub fn anchor_provider_usage(window: &Window, usage: &Usage) {
    if usage.estimated {
        return;
    }
    let tokens = usage.prompt_tokens;
    let mut out = OutputEstimateState::lock(window.output_state());
    let sent = match &out.usage_anchor {
        Some(anchor) if anchor.visible.request_chars > 0 => anchor.clone(),
        _ => return,
    };
    if tokens <= 0 {
        return;
    }
    let mut anchored = sent;
    anchored.prompt_tokens = tokens;
    out.usage_anchor = Some(anchored);
}

/// The provider's last prompt size plus what the visible view has gained
/// since that request, both sides of the gain sized by the same local
/// estimate so a stale ratio cancels (Go: `providerReportedFloor`). It
/// answers None after any fold: the reported size describes a context that
/// no longer exists, and only the next request's usage anchors the new one.
pub fn provider_reported_floor(
    window: &Window,
    visible: &RequestCalibrationShape,
    projection_version: u64,
) -> Option<i64> {
    let out = OutputEstimateState::lock(window.output_state());
    let anchor = out.usage_anchor.as_ref()?;
    if anchor.prompt_tokens <= 0 || anchor.projection_version != projection_version {
        return None;
    }
    let calibration = out.prompt_calibration.as_ref();
    let gain = estimated_shape_tokens_with(calibration, visible)
        - estimated_shape_tokens_with(calibration, &anchor.visible);
    Some((anchor.prompt_tokens + gain).max(0))
}

/// The one reading of how full the context is (Go: `promptPressure`).
pub fn prompt_pressure(
    window: &Window,
    shape: &RequestCalibrationShape,
    projection_version: u64,
    estimate: i64,
) -> i64 {
    if let Some(floor) = provider_reported_floor(window, shape, projection_version) {
        return floor;
    }
    estimate
}

/// Records single-request usage, never a billable aggregate (Go:
/// `storeLatestRequestUsage`).
pub fn store_latest_request_usage(window: &Window, usage: &Usage) {
    // Skip request-only shells with no token shape.
    if usage.prompt_tokens <= 0 && usage.completion_tokens <= 0 && usage.total_tokens <= 0 {
        return;
    }
    OutputEstimateState::lock(window.output_state()).last_usage = Some(usage.clone());
    estimate::set_prompt_token_calibration_from_usage(window, usage);
    window.note_accepted_prompt_tokens(usage);
    anchor_provider_usage(window, usage);
}

impl Window {
    /// Records single-request usage, never a billable aggregate (Go:
    /// `storeLatestRequestUsage`).
    pub fn store_latest_request_usage(&self, usage: &Usage) {
        super::usage::store_latest_request_usage(self, usage);
    }

    /// The prompt size the next request sends, measured the way the fold
    /// trigger measures it: the provider's last reported size plus what the
    /// view gained since (Go: `ContextUsedTokens`). After a fold or on a
    /// rebound session there is no anchor, and the estimate stands.
    pub fn context_used_tokens(&self) -> i64 {
        let transcript_version = self.session().transcript_version();
        let projection_version = self.current_projection_version();
        let tool_schema_revision = self.registry().schema_revision();
        let visible = self.model_visible_messages();
        let shape = super::visible_request_shape(self, &visible);
        let mut out = OutputEstimateState::lock(self.output_state());
        if let Some(cached) = &out.context_usage
            && cached.transcript_version == transcript_version
            && cached.projection_version == projection_version
            && cached.calibration == out.prompt_calibration
            && cached.anchor == out.usage_anchor
            && cached.tool_schema_revision == tool_schema_revision
        {
            return cached.tokens;
        }
        let tokens = {
            let calibration = out.prompt_calibration.clone();
            let anchor = out.usage_anchor.clone();
            prompt_pressure_with(
                calibration.as_ref(),
                anchor.as_ref(),
                self,
                &shape,
                projection_version,
                estimated_shape_tokens_with(calibration.as_ref(), &shape),
            )
        };
        out.context_usage = Some(ContextUsageMemo {
            transcript_version,
            projection_version,
            calibration: out.prompt_calibration.clone(),
            anchor: out.usage_anchor.clone(),
            tool_schema_revision,
            tokens,
        });
        tokens
    }
}

/// The pressure read without a window borrow inside the lock (Go:
/// `promptPressure` composed of `providerReportedFloor`).
fn prompt_pressure_with(
    calibration: Option<&estimate::PromptTokenCalibration>,
    anchor: Option<&UsageAnchor>,
    window: &Window,
    shape: &RequestCalibrationShape,
    projection_version: u64,
    estimate_value: i64,
) -> i64 {
    if let Some(anchor) = anchor
        && anchor.prompt_tokens > 0
        && anchor.projection_version == projection_version
    {
        let gain = estimated_shape_tokens_with(calibration, shape)
            - estimated_shape_tokens_with(calibration, &anchor.visible);
        return (anchor.prompt_tokens + gain).max(0);
    }
    let _ = window;
    estimate_value
}

#[cfg(test)]
#[path = "usage_tests.rs"]
mod tests;

//! Overflow recovery (Go: `context_overflow.go` recovery half +
//! `contract/provider/context_overflow.go`'s classifier).
//!
//! A provider's "this input is too long" is answered by folding the context
//! and rebuilding the frozen request, so the round can be replayed instead
//! of failing the turn. The recovery is one-shot per request: a second
//! rejection means the fold did not reach far enough, not that another one
//! will.

use reasonix_provider::{ChatCompletionRequest, ProviderError};

use super::estimate::{self, output_budget_reserve_for};
use super::maintenance::ContextPreparePolicy;
use super::{Window, trigger_label};

/// The codes an error envelope sets when the request's input did not fit
/// the model's context (Go: `contextOverflowCodes`). The same rejection
/// also arrives as prose from gateways that flatten upstream errors, and
/// matching prose is how a classifier starts firing on messages that merely
/// mention a limit — so a code enters this set only when a provider
/// documents it.
const CONTEXT_OVERFLOW_CODES: [&str; 2] = ["context_length_exceeded", "context_window_exceeded"];

/// Go: `RetryableStatus` — 408 (request timeout), 429 (rate limit) and 5xx.
/// Other 4xx (400/401/402/422, …) are caller/config problems retrying
/// can't fix. Kept beside the classifier because it gates it.
fn retryable_status(status: u16) -> bool {
    status == 408 || status == 429 || (500..=599).contains(&status)
}

/// The machine-readable code from an OpenAI-shaped error envelope, nested
/// under `error` or at the top level (Go: `APIError.ErrorCode`). A
/// non-string code — gateways put the HTTP status there — is no code at
/// all.
fn error_code_from_body(body: &str) -> Option<String> {
    if body.is_empty() {
        return None;
    }
    let parsed: serde_json::Value = serde_json::from_str(body).ok()?;
    let nested = parsed
        .get("error")
        .and_then(|e| e.get("code"))
        .and_then(|c| c.as_str())
        .map(str::to_owned);
    nested.or_else(|| {
        parsed
            .get("code")
            .and_then(|c| c.as_str())
            .map(str::to_owned)
    })
}

/// The fingerprint of what a provider sees (Go:
/// `sessionstore.ProviderVisibleFingerprint(provider.ModelMessages(...))`).
fn fingerprint(msgs: &[reasonix_contract::provider::Message]) -> String {
    let wire = reasonix_contract::provider::model_messages(msgs);
    reasonix_state::sessionstore::compaction_state::provider_visible_fingerprint(&wire)
}

/// Reports whether err is a rejection for an input larger than the model's
/// context, and therefore recoverable by sending less (Go:
/// `provider.IsContextOverflow`). It reads the envelope's code and nothing
/// else, so an endpoint that reports no code is not classified, and the
/// turn fails as it did before. Divergence: this lives in the provider
/// crate in Go (`contract/provider`); the provider wave owns the move.
pub fn is_context_overflow(err: &ProviderError) -> bool {
    let ProviderError::Api {
        status, body, ..
    } = err
    else {
        return false;
    };
    // A retryable status is a busy or broken server whatever code rides
    // along; the backoff ladder already owns those.
    if retryable_status(*status) {
        return false;
    }
    error_code_from_body(body).is_some_and(|code| CONTEXT_OVERFLOW_CODES.contains(&code.as_str()))
}

/// What a fold-recovery attempt concluded (Go: the `overflowFolded` /
/// `overflowRecovery` pair on `samplingRequest`).
#[derive(Debug, Clone, PartialEq)]
pub enum OverflowRecovery {
    /// The error is not one this recovery answers; the caller's own
    /// handling stands.
    NotApplicable,
    /// The context folded and the frozen request was rebuilt smaller; the
    /// caller replays it.
    Recovered,
    /// The fold freed nothing (or could not run); the caller fails the
    /// attempt with `reason` folded into its own terminal (Go:
    /// `overflowRecovery`).
    Failed { reason: String },
}

impl Window {
    /// Answers a provider's "this input is too long" by folding the context
    /// and rebuilding the frozen request (Go: `recoverContextOverflow`).
    /// The `overflow_folded` flag is the caller's per-request latch, kept
    /// beside the frozen request rather than on the provider type.
    pub fn recover_context_overflow(
        &self,
        frozen: &mut ChatCompletionRequest,
        overflow_folded: &mut bool,
        err: &ProviderError,
    ) -> OverflowRecovery {
        if *overflow_folded
            || !(is_context_overflow(err)
                || self.refused_over_window(frozen, err))
        {
            return OverflowRecovery::NotApplicable;
        }
        *overflow_folded = true;
        self.probe
            .note_rejected(estimate::estimated_request_tokens(self, frozen));
        let refused = fingerprint(&frozen.messages);
        if let Err(prepare_err) = self.prepare(ContextPreparePolicy {
            trigger: trigger_label::OVERFLOW.to_owned(),
            ..ContextPreparePolicy::default()
        }) {
            return OverflowRecovery::Failed {
                reason: prepare_err.to_string(),
            };
        }
        // Go rebuilds through prepareSamplingRequest: the full path, Prepare
        // included, with the output clip re-applied. Blocked receipts keep
        // the second pressure pass from re-folding what just failed.
        let rebuilt = match self.prepare_request(self.composer_or_default()) {
            Ok(rebuilt) => rebuilt,
            Err(build_err) => {
                return OverflowRecovery::Failed {
                    reason: build_err.to_string(),
                };
            }
        };
        if fingerprint(&rebuilt.messages) == refused {
            return OverflowRecovery::Failed {
                reason: "folding freed nothing to send".to_owned(),
            };
        }
        *frozen = rebuilt;
        OverflowRecovery::Recovered
    }

    /// Classifies a codeless 400/413 by measurement: the estimate plus the
    /// configured (not the clipped) completion and its margin is past the
    /// window (Go: `refusedOverWindow`). The wording is not read; a wrong
    /// call costs one bounded fold.
    fn refused_over_window(
        &self,
        frozen: &ChatCompletionRequest,
        err: &ProviderError,
    ) -> bool {
        let ProviderError::Api {
            status, ..
        } = err
        else {
            return false;
        };
        if *status != 400 && *status != 413 {
            return false;
        }
        let window = self.effective_context_window();
        window > 0
            && estimate::estimated_request_tokens(self, frozen)
                + self.configured_output_budget(self.config().max_output_tokens)
                + output_budget_reserve_for(window)
                > window
    }
}

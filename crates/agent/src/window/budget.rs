//! Output budget and spend accounting (Go: `budget_tokens.go`,
//! `output_budget.go` clip half, `context_budget.go`).
//!
//! The output budget clips completion tokens at send time only; it never
//! moves `compact_ratio`. Exhausted windows fail locally before HTTP 400.

use reasonix_provider::{ChatCompletionRequest, Usage};

use crate::error::TurnError;

use super::estimate::{self, output_budget_reserve_for};
use super::Window;

/// A reported total and its input/output split overlap. Either can
/// establish a larger known count, but adding them would charge the same
/// tokens twice (Go: `budgetUsageTokens`).
pub fn budget_usage_tokens(usage: Option<&Usage>) -> u64 {
    let Some(usage) = usage else {
        return 0;
    };
    let split = usage.prompt_tokens.max(0) as u64 + usage.completion_tokens.max(0) as u64;
    split.max(usage.total_tokens.max(0) as u64)
}

/// Go: `addBudgetTokens` — saturating at u64::MAX.
pub fn add_budget_tokens(used: u64, delta: u64) -> u64 {
    match used.checked_add(delta) {
        Some(next) => next,
        None => u64::MAX,
    }
}

impl Window {
    /// The output budget in force: the request's explicit value wins, else
    /// the provider's declared default (Go: `configuredOutputBudget`).
    pub fn configured_output_budget(&self, explicit: i64) -> i64 {
        if explicit != 0 {
            return explicit;
        }
        estimate::OutputEstimateState::lock(self.output_state()).output_budget
    }

    /// Go: `sharesContextWindow` — read from the configured adapter policy
    /// (see [`super::SharedWindowPolicy`]).
    pub fn shares_context_window(&self) -> bool {
        self.shared_window().shares_context_window
    }

    /// Clips completion tokens at send time (Go: `effectiveOutputBudget`).
    /// `Ok(None)` leaves the request unchanged; `Ok(Some(available))` is the
    /// clipped budget; `Err` is the typed compaction-required failure.
    pub fn effective_output_budget(
        &self,
        req: &ChatCompletionRequest,
    ) -> Result<Option<i64>, TurnError> {
        let window = self.effective_context_window();
        if window <= 0 || !self.shares_context_window() {
            return Ok(None);
        }
        let budget = self.configured_output_budget(req.max_tokens);
        if budget <= 0 {
            return Ok(None);
        }
        let est = estimate::estimated_request_tokens(self, req);
        let available = window - est - output_budget_reserve_for(window);
        if available <= 0 {
            return Err(TurnError::CompactionRequired {
                reason: format!(
                    "estimated prompt {est} leaves no shared-window output budget"
                ),
            });
        }
        if budget <= available {
            return Ok(None);
        }
        Ok(Some(available))
    }

    /// The room left before the next automatic fold, measured with the
    /// estimate the compaction thresholds themselves compare against so
    /// what the model is told and what the host acts on cannot drift apart
    /// (Go: `contextBudget`).
    pub fn context_budget(&self) -> ContextBudget {
        let trigger = self.compact_trigger();
        let window = self.effective_context_window();
        if trigger <= 0 || window <= 0 {
            return ContextBudget::unmeasured(
                "the active provider declares no context window, so compaction is disabled",
            );
        }
        let used = self.context_used_tokens();
        ContextBudget {
            status: "ok".to_owned(),
            reason: String::new(),
            tokens_remaining: (trigger - used).max(0),
            tokens_used: used,
            compact_at: trigger,
            window,
        }
    }
}

/// Go: `tool.ContextBudget` — what a caller learns about the room left
/// before the next automatic fold.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContextBudget {
    pub status: String,
    pub reason: String,
    pub tokens_remaining: i64,
    pub tokens_used: i64,
    pub compact_at: i64,
    pub window: i64,
}

impl ContextBudget {
    fn unmeasured(reason: &str) -> ContextBudget {
        ContextBudget {
            status: "unmeasured".to_owned(),
            reason: reason.to_owned(),
            ..ContextBudget::default()
        }
    }

    /// Go: `Known` — an unmeasured budget carries a reason instead of
    /// numbers, and callers must check before reading them.
    pub fn known(&self) -> bool {
        self.status == "ok"
    }
}

#[cfg(test)]
mod tests {
    use reasonix_contract::provider::{Message, Role, ToolCall};

    use super::*;
    use crate::window::SharedWindowPolicy;
    use crate::window::testing::{assistant, config, user, window, window_over};

    fn shared_config(window_tokens: i64, budget: i64) -> crate::window::WindowConfig {
        let mut cfg = config(window_tokens, 0.0);
        cfg.shared_window = SharedWindowPolicy {
            shares_context_window: true,
            output_budget: budget,
            ..SharedWindowPolicy::default()
        };
        cfg
    }

    fn request_of(window: &Window, msgs: Vec<Message>) -> ChatCompletionRequest {
        use crate::seams::RequestComposer;
        let composer = crate::compose::DefaultComposer::default();
        let base = crate::seams::RequestBase::default();
        composer
            .compose("", &msgs, window.registry(), &base)
            .expect("compose")
    }

    fn calibrate(window: &Window, msgs: &[Message], tokens: i64) {
        let req = ChatCompletionRequest {
            messages: msgs.to_vec(),
            ..ChatCompletionRequest::default()
        };
        let shape = estimate::request_calibration_shape(&req, window.shared_window());
        estimate::set_prompt_token_calibration(window, tokens, &shape);
    }

    fn tool_call() -> ToolCall {
        ToolCall {
            id: "call_1".to_owned(),
            name: "bash".to_owned(),
            arguments: "{}".to_owned(),
            ..ToolCall::default()
        }
    }

    // Go: TestEffectiveOutputBudgetClipsSharedWindowRequest. The 950K
    // prompt fits, but not beside the provider's full 128K output default.
    #[test]
    fn a_near_window_request_clips_the_shared_output_budget() {
        let window = window_over(shared_config(1_048_576, 128 * 1024), vec![]);
        let msgs = vec![user(&"\u{5b57}".repeat(950_000))];
        calibrate(&window, &msgs, 950_000);
        let req = request_of(&window, msgs);

        let got = window
            .effective_output_budget(&req)
            .expect("resolves")
            .expect("near-window request kept the provider's full output budget");
        assert!(got > 0 && got < 128 * 1024);
        assert!(
            got + 950_000 <= 1_048_576 - estimate::OUTPUT_BUDGET_RESERVE,
            "input + output = {got}+950000 exceeds the reserved shared window"
        );
    }

    // Go: TestCalibratedOutputBudgetIncludesReplayedReasoning.
    #[test]
    fn a_replayed_reasoning_turn_clips_hard() {
        let window = window_over(shared_config(200_000, 128 * 1024), vec![]);
        let previous = vec![user(&"x".repeat(300_000))];
        calibrate(&window, &previous, 75_000);
        let mut reply = assistant("");
        reply.reasoning_content = "r".repeat(400_000);
        reply.tool_calls = vec![tool_call()];
        let mut current = previous.clone();
        current.push(reply);

        let before = estimate::estimated_prompt_tokens(&window, &previous);
        let after = estimate::estimated_prompt_tokens(&window, &current);
        assert!(
            after >= before + 99_000,
            "400K replayed reasoning was not calibrated: before={before} after={after}"
        );
        let req = request_of(&window, current);
        let budget = window.effective_output_budget(&req).expect("resolves");
        assert!(
            matches!(budget, Some(b) if b <= 20_000),
            "replayed reasoning budget = {budget:?}, want a clipped budget <= 20000"
        );
    }

    // Go: TestCalibratedOutputBudgetKeepsCJKConservativeFloor. The
    // unrepresented CJK runes are priced at the cold rate: 3 bytes each at
    // ~4 chars per token, i.e. 0.75 tokens per rune against a real ~0.6.
    #[test]
    fn mixed_script_requests_price_the_unrepresented_cjk_at_the_cold_rate() {
        let window = window_over(shared_config(1_048_576, 128 * 1024), vec![]);
        let previous = vec![user(&"x".repeat(300_000))];
        calibrate(&window, &previous, 75_000);
        let mut reply = assistant("");
        reply.reasoning_content = "\u{5b57}".repeat(1_200_000);
        reply.tool_calls = vec![tool_call()];
        let mut current = previous.clone();
        current.push(reply);

        let calibrated = estimate::estimated_prompt_tokens(&window, &current);
        let want_floor = 75_000 + 1_200_000 * 3 / 4;
        assert!(
            calibrated >= want_floor,
            "calibrated estimate {calibrated} fell below mixed-script safety floor {want_floor}"
        );

        let req = request_of(&window, current);
        let budget = window.effective_output_budget(&req).expect("resolves");
        assert!(
            matches!(budget, Some(b) if b > 0 && b < 128 * 1024),
            "mixed-script request budget = {budget:?}, want a clipped positive budget"
        );
    }

    // Go: TestEffectiveOutputBudgetRejectsExhaustedSharedWindow.
    #[test]
    fn an_exhausted_shared_window_fails_before_http_400() {
        let window = window_over(shared_config(1_048_576, 128 * 1024), vec![]);
        let msgs = vec![user(&"\u{5b57}".repeat(1_045_000))];
        calibrate(&window, &msgs, 1_045_000);
        let req = request_of(&window, msgs);
        let err = window
            .effective_output_budget(&req)
            .expect_err("want compaction-required");
        assert!(
            matches!(err, TurnError::CompactionRequired { .. }),
            "error = {err}, want the typed compaction-required identity"
        );
    }

    // Go: TestEffectiveOutputBudgetLeavesIndependentProviderUnchanged.
    #[test]
    fn an_independent_provider_is_never_clipped() {
        let window = window_over(config(1_048_576, 0.0), vec![]);
        let req = request_of(&window, vec![user(&"\u{5b57}".repeat(950_000))]);
        let got = window.effective_output_budget(&req).expect("resolves");
        assert_eq!(got, None);
    }

    // Go: TestEffectiveOutputBudgetHonorsExplicitOmit.
    #[test]
    fn an_explicit_output_omit_is_not_clipped() {
        let window = window_over(shared_config(1_048_576, 128 * 1024), vec![]);
        let mut req = request_of(&window, vec![user(&"\u{5b57}".repeat(950_000))]);
        req.max_tokens = -1;
        let got = window.effective_output_budget(&req).expect("resolves");
        assert_eq!(got, None);
    }

    // Go: TestOutputBudgetKeepsAWindowProportionalMarginOnALargeWindow.
    #[test]
    fn a_large_window_keeps_a_proportional_margin() {
        let window = window_over(shared_config(1_048_576, 65_536), vec![]);
        let msgs = vec![user(&"\u{5b57}".repeat(975_000))];
        calibrate(&window, &msgs, 975_000);
        let req = request_of(&window, msgs);
        let got = window
            .effective_output_budget(&req)
            .expect("resolves")
            .expect("want a clipped budget");
        // The real count was 1% above the estimate in the report this pins:
        // input and output must still fit the window with that much error.
        let actual = 975_000 + 975_000 / 100;
        assert!(
            actual + got <= 1_048_576,
            "a prompt 1% above its estimate plus the {got}-token budget is {}, past the window",
            actual + got
        );
    }

    // Go: TestTheFoldCeilingStopsWhereTheSendTimeOutputCheckStops. The
    // prompt size at which the send-time output check refuses a request is
    // the size at which a fold must free room.
    #[test]
    fn the_fold_ceiling_stops_where_the_send_time_output_check_stops() {
        let window = window_over(shared_config(128_000, 8 * 1024), vec![]);
        let ceiling = window.hard_input_ceiling();
        let at_ceiling = request_of(&window, vec![user("xxxx")]);
        assert!(
            window
                .effective_output_budget(&at_ceiling)
                .expect("resolves")
                .is_none(),
            "a tiny prompt is refused"
        );
        assert_eq!(
            ceiling,
            128_000 - estimate::output_budget_reserve_for(128_000),
            "the ceiling must be the window less the output reserve"
        );
        let independent = window_over(config(128_000, 0.0), vec![]);
        assert_eq!(
            independent.hard_input_ceiling(),
            128_000 - crate::window::trigger::PROTOCOL_RESERVE_TOKENS,
            "a provider whose reply does not share the window keeps the protocol reserve only"
        );
    }

    // Go: TestContextBudgetUnmeasuredWithoutWindow.
    #[test]
    fn an_unmeasured_window_leaves_the_budget_unmeasured() {
        let window = window(0, crate::window::trigger::DEFAULT_COMPACT_RATIO);
        let budget = window.context_budget();
        assert!(!budget.known(), "budget reported as known without a window");
        assert_eq!(budget.status, "unmeasured");
        assert!(!budget.reason.is_empty());
    }

    // Go: budget_tokens.go — the total and its split overlap.
    #[test]
    fn usage_totals_never_double_charge_the_split() {
        let usage = Usage {
            prompt_tokens: 100,
            completion_tokens: 40,
            total_tokens: 120,
            ..Usage::default()
        };
        assert_eq!(budget_usage_tokens(Some(&usage)), 140);
        assert_eq!(budget_usage_tokens(None), 0);
        let usage = Usage {
            prompt_tokens: -5,
            completion_tokens: 40,
            total_tokens: 30,
            ..Usage::default()
        };
        assert_eq!(budget_usage_tokens(Some(&usage)), 40);
        assert_eq!(add_budget_tokens(u64::MAX - 1, 5), u64::MAX);
        assert_eq!(add_budget_tokens(1, 2), 3);
    }

    #[allow(dead_code)]
    fn _roles_in_scope(role: Role) -> Role {
        role
    }
}

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

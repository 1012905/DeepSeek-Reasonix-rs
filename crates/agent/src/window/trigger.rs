//! The fold boundaries (Go: `compact.go` threshold half + `context_overflow.go`
//! probe half).
//!
//! `compact_ratio` is the sole automatic maintenance boundary; output budgets
//! are intentionally absent from it — they are clipped against the final
//! request at send time and must never make compaction happen earlier than
//! the user's configured ratio.

use std::sync::atomic::{AtomicI64, Ordering};

use reasonix_contract::provider::Message;

use super::estimate::{self, output_budget_reserve_for};
use super::Window;

/// Capacity share of the window before maintenance (Go:
/// `defaultCompactRatio`).
pub const DEFAULT_COMPACT_RATIO: f64 = 0.90;
/// Normal auto-checkpoint acceptance ceiling (Go:
/// `checkpointCeilingRatio`).
pub const CHECKPOINT_CEILING_RATIO: f64 = 0.50;
/// Recent verbatim tail as a fraction of the window (Go:
/// `refoldMinTokensRatio`).
pub const REFOLD_MIN_TOKENS_RATIO: f64 = 0.10;
pub const MIN_REFOLD_TOKENS: i64 = 32 * 1024;
pub const MAX_REFOLD_TOKENS: i64 = 96 * 1024;
/// When fixed prefix alone exceeds 50%, require at least this savings (Go:
/// `exceptionalMinSavingsRatio`).
pub const EXCEPTIONAL_MIN_SAVINGS_RATIO: f64 = 0.25;
/// Skip compaction below this many compactable messages (Go:
/// `minCompactMessages`).
pub const MIN_COMPACT_MESSAGES: usize = 2;
/// Provider framing and control fields not represented by message estimates
/// (Go: `protocolReserveTokens`).
pub const PROTOCOL_RESERVE_TOKENS: i64 = 256;
/// The smallest window a fold can work inside (Go: `minFoldSpanTokens`).
pub const MIN_FOLD_SPAN_TOKENS: i64 = 4000;

/// What an endpoint that declares no window still tells us about it: the
/// smallest request it refused as too long, and the largest it accepted.
/// Both are sizes of requests we actually sent, so together they bound the
/// real window instead of guessing at one (Go: `contextWindowProbe`).
#[derive(Debug, Default)]
pub struct ContextWindowProbe {
    rejected: AtomicI64,
    accepted: AtomicI64,
}

impl ContextWindowProbe {
    pub fn note_rejected(&self, tokens: i64) {
        if tokens <= 0 {
            return;
        }
        let mut current = self.rejected.load(Ordering::SeqCst);
        while current <= 0 || current > tokens {
            match self.rejected.compare_exchange_weak(
                current,
                tokens,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => return,
                Err(observed) => current = observed,
            }
        }
    }

    pub fn note_accepted(&self, tokens: i64) {
        if tokens <= 0 {
            return;
        }
        let mut current = self.accepted.load(Ordering::SeqCst);
        while current < tokens {
            match self.accepted.compare_exchange_weak(
                current,
                tokens,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => return,
                Err(observed) => current = observed,
            }
        }
    }

    /// The size to budget against, or zero while nothing has been rejected:
    /// a request that fit says nothing about where the ceiling is. Once one
    /// has, the largest accepted size is preferred — it is the only bound
    /// known to fit — and the refused size stands in until such a request
    /// exists (Go: `window()`).
    pub fn window(&self) -> i64 {
        let rejected = self.rejected.load(Ordering::SeqCst);
        if rejected <= 0 {
            return 0;
        }
        // A refusal too small for one fold to be worth summarizing is not a
        // window this agent can work inside: adopting it would put every
        // later turn under a ceiling no projection can meet.
        if rejected < MIN_FOLD_SPAN_TOKENS {
            return 0;
        }
        let accepted = self.accepted.load(Ordering::SeqCst);
        if accepted > 0 && accepted < rejected {
            return accepted;
        }
        rejected
    }
}

impl Window {
    /// The window every context budget is taken from: the configured one, or
    /// the bound probed from a rejection when no window was declared. Zero
    /// still means no window is known, which leaves compaction off (Go:
    /// `effectiveContextWindow`).
    pub fn effective_context_window(&self) -> i64 {
        if self.config.context_window > 0 {
            return self.config.context_window;
        }
        self.probe.window()
    }

    /// Records a provider-counted prompt size as a lower bound on the
    /// window. It applies the same trust rule as prompt calibration: an
    /// estimate is our own arithmetic (Go: `noteAcceptedPromptTokens`).
    pub fn note_accepted_prompt_tokens(&self, usage: &reasonix_provider::Usage) {
        if usage.estimated {
            return;
        }
        self.probe.note_accepted(usage.prompt_tokens.max(0));
    }

    /// How close a prompt is to not fitting (Go:
    /// `capacityCompactTrigger`). Divergence: the ablation set's forced 0.5
    /// ratio is a Go-side test/config knob the ablation wave owns; the
    /// configured ratio is the only input here.
    pub fn capacity_compact_trigger(&self) -> i64 {
        let window = self.effective_context_window();
        if window <= 0 {
            return 0;
        }
        let mut ratio = self.config.compact_ratio;
        if ratio <= 0.0 {
            ratio = DEFAULT_COMPACT_RATIO;
        }
        ((window as f64 * ratio) as i64).max(1)
    }

    /// The boundary in force, which is the number a frontend has to show
    /// (Go: `compactTrigger`). An unmeasured window leaves both unset rather
    /// than guessing at a size, and a ratio placed past the window is how
    /// maintenance is turned off — economics tightens a live boundary, never
    /// revives a retired one.
    pub fn compact_trigger(&self) -> i64 {
        self.capacity_compact_trigger()
    }

    /// Names the threshold that decides maintenance right now (Go:
    /// `compactBoundary`): the window share, or the absolute visible-input
    /// size that does not come from it.
    pub fn compact_boundary(&self) -> &'static str {
        let capacity = self.capacity_compact_trigger();
        if capacity <= 0 || self.config.compact_ratio > 1.0 {
            return "";
        }
        "capacity"
    }

    /// A physical input-safety boundary, not another user compaction
    /// threshold (Go: `hardInputCeiling`). On a provider whose reply shares
    /// the window it stops where the send-time output check stops, so a
    /// prompt the check would refuse is one the fold has already been told
    /// it must free.
    pub fn hard_input_ceiling(&self) -> i64 {
        let window = self.effective_context_window();
        if window <= 0 {
            return 0;
        }
        let mut reserve = PROTOCOL_RESERVE_TOKENS;
        if self.shared_window().shares_context_window && self.configured_output_budget(0) > 0 {
            reserve = reserve.max(output_budget_reserve_for(window));
        }
        (window - reserve).max(1)
    }

    /// How much new closed history a second fold waits for: every fold
    /// costs the whole prefix cache, so it must free more than it costs
    /// (Go: `refoldMinTokens`).
    pub fn refold_min_tokens(&self) -> i64 {
        let window = self.effective_context_window();
        if window <= 0 {
            return MIN_REFOLD_TOKENS;
        }
        let mut n = (window as f64 * REFOLD_MIN_TOKENS_RATIO) as i64;
        if window >= MIN_REFOLD_TOKENS * 2 && n < MIN_REFOLD_TOKENS {
            n = MIN_REFOLD_TOKENS;
        }
        if n > MAX_REFOLD_TOKENS {
            n = MAX_REFOLD_TOKENS;
        }
        let max = window / 2;
        if max > 0 && n > max {
            n = max;
        }
        // A tail reaching the trigger leaves nothing to fold: every message
        // is still held verbatim when the threshold arrives, so maintenance
        // folds nothing for the rest of the session. The user's threshold
        // wins.
        let trigger = self.compact_trigger();
        if trigger > 0 && n > trigger / 2 {
            n = trigger / 2;
        }
        n.max(1)
    }

    /// The auto-checkpoint acceptance upper bound: candidates below it are
    /// accepted without padding (Go: `checkpointCeiling`).
    pub fn checkpoint_ceiling(&self) -> i64 {
        let window = self.effective_context_window();
        if window <= 0 {
            return 0;
        }
        ((window as f64 * CHECKPOINT_CEILING_RATIO) as i64).max(1)
    }

    /// Required only when the fixed prefix alone already exceeds the 50%
    /// ceiling; otherwise ordinary candidates simply stay under 50% (Go:
    /// `exceptionalMinimumSavings`).
    pub fn exceptional_minimum_savings(&self) -> i64 {
        let window = self.effective_context_window();
        if window <= 0 {
            return 0;
        }
        ((window as f64 * EXCEPTIONAL_MIN_SAVINGS_RATIO) as i64).max(1)
    }

    /// Whether compacting the given region saves enough tokens to justify
    /// the summarization API call (Go: `foldEconomics`). It is false when
    /// the region is too small for the savings to outweigh the extra
    /// round-trip cost and latency of calling the summarizer.
    pub fn fold_economics(&self, region: &[Message]) -> bool {
        const MIN_FOLD_TOKENS: i64 = 100;
        estimate::estimated_prompt_tokens(self, region) >= MIN_FOLD_TOKENS
    }
}

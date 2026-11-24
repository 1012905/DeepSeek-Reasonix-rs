//! Token/cost accounting for the finishTurn `usage_update` and the status
//! snapshot's `usage` block (status.go:164-241; usage_update.go:5-28).
//!
//! Two tallies ride together: `turn` is this turn's increment, reset at
//! `beginTurn`; `cumulative` is the session-level running total. The real
//! `provider.Usage` source is runtime work; this module owns the addition, the
//! estimated OR, and the source resolution rules.

use serde::{Deserialize, Serialize};

/// Default source when no Usage event has named one (status.go:175-183).
pub const DEFAULT_USAGE_SOURCE: &str = "executor";

/// One token tally, wire-aligned with `ReasonixUsage`'s five token fields plus
/// the estimated flag and source. Pricing pointers live on the snapshot type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct UsageTotals {
    #[serde(rename = "promptTokens")]
    pub prompt_tokens: i64,
    #[serde(rename = "completionTokens")]
    pub completion_tokens: i64,
    #[serde(rename = "reasoningTokens")]
    pub reasoning_tokens: i64,
    #[serde(rename = "cacheHitTokens")]
    pub cache_hit_tokens: i64,
    #[serde(rename = "cacheMissTokens")]
    pub cache_miss_tokens: i64,
    pub estimated: bool,
    #[serde(rename = "usageSource")]
    pub usage_source: String,
}

impl UsageTotals {
    /// Adds another tally in place. `estimated` accumulates by OR; the source
    /// resolves to the first non-empty source, and two distinct sources report
    /// `mixed` (status.go:250, 253-255).
    pub fn add(&mut self, other: &UsageTotals) {
        self.prompt_tokens += other.prompt_tokens;
        self.completion_tokens += other.completion_tokens;
        self.reasoning_tokens += other.reasoning_tokens;
        self.cache_hit_tokens += other.cache_hit_tokens;
        self.cache_miss_tokens += other.cache_miss_tokens;
        self.estimated |= other.estimated;
        self.usage_source = resolve_source(&self.usage_source, &other.usage_source);
    }

    /// Zero tally with the default source.
    pub fn empty() -> Self {
        Self {
            usage_source: DEFAULT_USAGE_SOURCE.to_owned(),
            ..Default::default()
        }
    }
}

/// Source resolution: empty falls through to the other source; two distinct
/// non-empty sources report `mixed`; an empty pair defaults to `executor`.
pub fn resolve_source(a: &str, b: &str) -> String {
    match (a.trim(), b.trim()) {
        ("", "") => DEFAULT_USAGE_SOURCE.to_owned(),
        ("", other) => other.to_owned(),
        (other, "") => other.to_owned(),
        (x, y) if x == y => x.to_owned(),
        _ => "mixed".to_owned(),
    }
}

/// Turn + cumulative accumulator behind the status telemetry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct UsageAccumulator {
    pub turn: UsageTotals,
    pub cumulative: UsageTotals,
}

impl UsageAccumulator {
    /// Fresh accumulator with default sources.
    pub fn new() -> Self {
        Self {
            turn: UsageTotals::empty(),
            cumulative: UsageTotals::empty(),
        }
    }

    /// Adds one Usage event to both the turn increment and the cumulative total.
    pub fn add_event(&mut self, usage: &UsageTotals) {
        self.turn.add(usage);
        self.cumulative.add(usage);
    }

    /// `beginTurn`: the turn increment resets; the cumulative total survives.
    pub fn reset_turn(&mut self) {
        self.turn = UsageTotals::empty();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turn_resets_while_cumulative_accumulates() {
        let mut acc = UsageAccumulator::new();
        acc.add_event(&UsageTotals {
            prompt_tokens: 10,
            completion_tokens: 5,
            reasoning_tokens: 2,
            cache_hit_tokens: 0,
            cache_miss_tokens: 3,
            estimated: false,
            usage_source: "planner".to_owned(),
        });
        acc.add_event(&UsageTotals {
            prompt_tokens: 4,
            completion_tokens: 1,
            reasoning_tokens: 0,
            cache_hit_tokens: 1,
            cache_miss_tokens: 0,
            estimated: true,
            usage_source: "planner".to_owned(),
        });
        assert_eq!(acc.turn.prompt_tokens, 14);
        assert_eq!(acc.cumulative.prompt_tokens, 14);
        assert!(acc.cumulative.estimated);

        acc.reset_turn();
        assert_eq!(acc.turn.prompt_tokens, 0);
        assert_eq!(acc.cumulative.prompt_tokens, 14);
    }

    #[test]
    fn source_resolution_matches_reference_rules() {
        assert_eq!(resolve_source("", ""), "executor");
        assert_eq!(resolve_source("planner", ""), "planner");
        assert_eq!(resolve_source("", "executor"), "executor");
        assert_eq!(resolve_source("executor", "executor"), "executor");
        assert_eq!(resolve_source("planner", "executor"), "mixed");
    }

    #[test]
    fn estimated_flag_or_across_events() {
        let mut acc = UsageAccumulator::new();
        acc.add_event(&UsageTotals {
            estimated: false,
            usage_source: "executor".to_owned(),
            ..Default::default()
        });
        assert!(!acc.cumulative.estimated);
        acc.add_event(&UsageTotals {
            estimated: true,
            usage_source: "executor".to_owned(),
            ..Default::default()
        });
        assert!(acc.cumulative.estimated);
    }
}

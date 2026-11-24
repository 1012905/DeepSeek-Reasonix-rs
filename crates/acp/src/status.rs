//! `_reasonix.io/session/status` request and the `ReasonixSessionStatus` v1
//! recovery snapshot (status.go:20-145, 700-785).
//!
//! The snapshot deliberately omits reasoning text and unbounded terminal
//! output. Pricing ledger types (`originalTotals`, `costQuote`) ride as opaque
//! JSON values here; the pricing aggregation itself is runtime work.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Status schema version advertised in the vendor capability block.
pub const STATUS_SCHEMA_VERSION: i32 = 1;

// ---- Sandbox / runtime state -------------------------------------------------

/// Effective sandbox, after CLI hard overrides. Reports no credentials or
/// environment (status.go:40-47).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct SessionSandboxState {
    pub mode: String,
    pub engine: String,
    pub available: bool,
    #[serde(rename = "workspaceRoot")]
    pub workspace_root: String,
    #[serde(rename = "writeRoots")]
    pub write_roots: Vec<String>,
    #[serde(rename = "networkEnabled")]
    pub network_enabled: bool,
}

/// Planner/sandbox posture supplied by the composition root (status.go:51-54).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct SessionRuntimeState {
    #[serde(rename = "plannerMode")]
    pub planner_mode: String,
    pub sandbox: SessionSandboxState,
}

// ---- Goal --------------------------------------------------------------------

/// Goal runtime summary; absent for old hosts or when no goal is active
/// (status.go:70-82).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ReasonixGoalRuntime {
    #[serde(rename = "turnsUsed")]
    pub turns_used: i64,
    /// Deprecated: always zero; retained for protocol compatibility.
    #[serde(rename = "turnsLimit")]
    pub turns_limit: i64,
    #[serde(rename = "tokensUsed")]
    pub tokens_used: i64,
    #[serde(rename = "requestsUsed", skip_serializing_if = "Option::is_none")]
    pub requests_used: Option<i64>,
    #[serde(rename = "workDurationMs", skip_serializing_if = "Option::is_none")]
    pub work_duration_ms: Option<i64>,
    /// Deprecated: always zero.
    #[serde(rename = "tokensLimit")]
    pub tokens_limit: i64,
    #[serde(rename = "noProgressTurns")]
    pub no_progress_turns: i64,
    /// Deprecated: always zero.
    #[serde(rename = "noProgressLimit")]
    pub no_progress_limit: i64,
    #[serde(rename = "lastReason", skip_serializing_if = "Option::is_none")]
    pub last_reason: Option<String>,
    #[serde(rename = "stopCause", skip_serializing_if = "Option::is_none")]
    pub stop_cause: Option<String>,
    /// Deprecated: always zero.
    #[serde(rename = "budgetExtensions")]
    pub budget_extensions: i64,
}

/// Goal snapshot carried by the status tree (status.go:62-68).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ReasonixStatusGoal {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub objective: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime: Option<ReasonixGoalRuntime>,
}

// ---- Turn outcome / readiness ------------------------------------------------

/// Why the finished turn ended (status.go:84-87).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ReasonixTurnOutcome {
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Review readiness after a completed turn (status.go:89-93).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ReasonixFinalReadiness {
    #[serde(rename = "readyForReview")]
    pub ready_for_review: bool,
    pub summary: String,
    pub risks: Vec<String>,
}

// ---- Usage -------------------------------------------------------------------

/// One token/cost tally (status.go:95-112). Pricing ledger entries ride as
/// opaque values; the aggregation policy lives in the runtime.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ReasonixUsage {
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estimated: Option<bool>,
    #[serde(rename = "cacheHitRatio")]
    pub cache_hit_ratio: Option<f64>,
    #[serde(rename = "estimatedCost")]
    pub estimated_cost: Option<f64>,
    pub currency: Option<String>,
    #[serde(rename = "costComplete", skip_serializing_if = "Option::is_none")]
    pub cost_complete: Option<bool>,
    #[serde(rename = "displayComplete", skip_serializing_if = "Option::is_none")]
    pub display_complete: Option<bool>,
    #[serde(rename = "displayStatus", skip_serializing_if = "Option::is_none")]
    pub display_status: Option<String>,
    #[serde(rename = "aggregateMode", skip_serializing_if = "Option::is_none")]
    pub aggregate_mode: Option<String>,
    #[serde(rename = "originalTotals", skip_serializing_if = "Option::is_none")]
    pub original_totals: Option<Value>,
    #[serde(rename = "costQuote", skip_serializing_if = "Option::is_none")]
    pub cost_quote: Option<Value>,
    #[serde(rename = "usageSource")]
    pub usage_source: String,
}

/// Turn and cumulative tallies (status.go:114-117).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ReasonixStatusUsage {
    pub turn: ReasonixUsage,
    pub cumulative: ReasonixUsage,
}

// ---- Status tree -------------------------------------------------------------

/// The stable schemaVersion=1 recovery snapshot (status.go:121-137).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ReasonixSessionStatus {
    #[serde(rename = "schemaVersion")]
    pub schema_version: i32,
    pub sequence: u64,
    #[serde(rename = "sessionId")]
    pub session_id: String,
    pub state: String,
    pub model: String,
    pub effort: String,
    pub mode: String,
    #[serde(rename = "workMode")]
    pub work_mode: String,
    #[serde(rename = "plannerMode")]
    pub planner_mode: String,
    pub goal: ReasonixStatusGoal,
    pub phase: String,
    #[serde(rename = "turnOutcome")]
    pub turn_outcome: ReasonixTurnOutcome,
    #[serde(rename = "finalReadiness")]
    pub final_readiness: ReasonixFinalReadiness,
    pub sandbox: SessionSandboxState,
    pub usage: ReasonixStatusUsage,
}

/// Outbound status update envelope (status.go:139-145).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ReasonixStatusUpdate {
    #[serde(rename = "schemaVersion")]
    pub schema_version: i32,
    pub sequence: u64,
    #[serde(rename = "sessionId")]
    pub session_id: String,
    pub event: String,
    pub status: ReasonixSessionStatus,
}

/// `session/status` request params (status.go:34-36).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SessionStatusParams {
    #[serde(rename = "sessionId")]
    pub session_id: String,
}

// ---- Pure normalization tables ---------------------------------------------

/// Phase vocabulary the status snapshot accepts verbatim (status.go:604-615).
pub const PHASE_VOCABULARY: [&str; 15] = [
    "idle",
    "starting",
    "planning",
    "working",
    "implementing",
    "waiting_permission",
    "waiting_input",
    "checking_readiness",
    "cancelled",
    "review_ready",
    "completed",
    "paused",
    "readiness_paused",
    "recovery_paused",
    "error",
];

/// State coerce: anything but `running` reports `idle` (status.go:757-760).
pub fn normalize_state(raw: &str) -> &'static str {
    if raw == "running" { "running" } else { "idle" }
}

/// Effort: trimmed override, or `auto` when unset (status.go:554-559).
pub fn normalize_effort(value: Option<&str>) -> String {
    match value {
        Some(v) if !v.trim().is_empty() => v.trim().to_owned(),
        _ => "auto".to_owned(),
    }
}

/// Light runtime profiles force the planner off (status.go:641-648).
pub fn is_light_runtime_profile(profile: &str) -> bool {
    matches!(
        profile.trim().to_ascii_lowercase().as_str(),
        "economy" | "light" | "lite" | "eco"
    )
}

/// Work-mode label normalization; `light` dual-writes as `economy`, unknown
/// profiles fall back to `balanced` (status.go:734-742).
pub fn normalize_work_mode(raw: &str) -> String {
    match raw.trim().to_ascii_lowercase().as_str() {
        "economy" => "economy".to_owned(),
        "light" => "economy".to_owned(),
        "delivery" => "delivery".to_owned(),
        _ => "balanced".to_owned(),
    }
}

/// Goal status whitelist; anything else reports `none` (status.go:561-568).
pub fn normalize_goal_status(raw: &str) -> &'static str {
    match raw {
        "running" => "running",
        "complete" => "complete",
        "blocked" => "blocked",
        _ => "none",
    }
}

/// Persisted phase renormalization: a known phase passes through; an unknown
/// one falls back to the text heuristic (status.go:604-615).
pub fn normalize_persisted_phase(raw: &str) -> String {
    let trimmed = raw.trim();
    if PHASE_VOCABULARY.contains(&trimmed) {
        return trimmed.to_owned();
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.contains("planning") {
        "planning".to_owned()
    } else if lower.contains("executing") || lower.contains("implementing") {
        "implementing".to_owned()
    } else {
        "working".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_effort_work_mode_normalizations_match_tables() {
        assert_eq!(normalize_state("running"), "running");
        assert_eq!(normalize_state("idle"), "idle");
        assert_eq!(normalize_state("bogus"), "idle");

        assert_eq!(normalize_effort(None), "auto");
        assert_eq!(normalize_effort(Some("  ")), "auto");
        assert_eq!(normalize_effort(Some(" high ")), "high");

        assert_eq!(normalize_work_mode("LIGHT"), "economy");
        assert_eq!(normalize_work_mode("economy"), "economy");
        assert_eq!(normalize_work_mode("delivery"), "delivery");
        assert_eq!(normalize_work_mode("nope"), "balanced");

        assert!(is_light_runtime_profile("Lite"));
        assert!(!is_light_runtime_profile("delivery"));

        assert_eq!(normalize_goal_status("complete"), "complete");
        assert_eq!(normalize_goal_status("done"), "none");
    }

    #[test]
    fn persisted_phase_passes_whitelist_and_heuristic_falls_back() {
        assert_eq!(normalize_persisted_phase("paused"), "paused");
        assert_eq!(normalize_persisted_phase("we are planning"), "planning");
        assert_eq!(normalize_persisted_phase("executing steps"), "implementing");
        assert_eq!(normalize_persisted_phase("random"), "working");
    }
}

//! ACP-045 mode/approval drift reconciliation ordering
//! (`service.go:445-484`, `service.go:869-887`).
//!
//! A turn or a config rebuild can flip the controller-side plan/goal mode or
//! the tool-approval posture after the session's bookkeeping last read it. The
//! drift emitters reconcile that bookkeeping against the controller before the
//! session is exposed as idle; a concurrent config switch that wins the idle
//! race would otherwise rebuild from the stale mode/posture. Both emitters
//! serialize on `stateChangeMu` across the controller read and the bookkeeping
//! swap so an explicit user selection landing between the two is never
//! reverted. The controller reads, config-state snapshot, and meta persistence
//! are runtime wiring; this module owns the derivation table, the change-only
//! emission rule, and the finishTurn step order.

use crate::session_open::{MODE_GOAL, MODE_NORMAL, MODE_PLAN};

/// Derives the current collaboration mode from the controller's plan/goal
/// state (`service.go:456-463`): plan wins over goal, goal needs a running
/// goal or the draft-goal slot, otherwise normal.
pub fn derive_mode(plan_mode: bool, goal_running: bool, goal_draft: bool) -> &'static str {
    if plan_mode {
        return MODE_PLAN;
    }
    if goal_running || goal_draft {
        return MODE_GOAL;
    }
    MODE_NORMAL
}

/// Normalizes a tool-approval posture to its wire identity
/// (`session_config.go:489-502`): case-insensitive on the four recognized
/// spellings; anything else falls back to `ask`.
pub fn normalize_posture(raw: &str) -> String {
    match raw.trim().to_ascii_lowercase().as_str() {
        "auto" => "auto".to_owned(),
        "yolo" => "yolo".to_owned(),
        "readonly" => "readOnly".to_owned(),
        "dontask" => "dontAsk".to_owned(),
        _ => "ask".to_owned(),
    }
}

/// Mode drift: `Some(current)` only when the derived mode differs from the
/// session's recorded mode (`service.go:464-467`).
pub fn mode_drift(session_mode: &str, derived: &'static str) -> Option<&'static str> {
    (session_mode != derived).then_some(derived)
}

/// Approval drift: normalizes both sides first, then `Some(controller)` only
/// when the posture changed (`service.go:476-479`).
pub fn approval_drift(session_posture: &str, controller_posture: &str) -> Option<String> {
    let current = normalize_posture(controller_posture);
    (normalize_posture(session_posture) != current).then_some(current)
}

/// finishTurn reconciliation order (`service.go:876-887`). Drift is reconciled
/// before `finish` exposes the session as idle; pending config and the queued
/// reload drain after that; a second drift pass runs once a rebuild may have
/// normalized the state again.
pub const FINISH_TURN_STEPS: &[&str] = &[
    "emit_mode_drift",
    "emit_tool_approval_drift",
    "finish_expose_idle",
    "apply_pending_config",
    "drain_pending_reload",
    "emit_mode_drift",
    "emit_tool_approval_drift",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_derivation_prefers_plan_then_goal_then_normal() {
        assert_eq!(derive_mode(true, true, false), MODE_PLAN);
        assert_eq!(derive_mode(false, true, false), MODE_GOAL);
        assert_eq!(derive_mode(false, false, true), MODE_GOAL);
        assert_eq!(derive_mode(false, false, false), MODE_NORMAL);
    }

    #[test]
    fn posture_normalizes_case_and_spelling_with_ask_fallback() {
        assert_eq!(normalize_posture("  AUTO "), "auto");
        assert_eq!(normalize_posture("Yolo"), "yolo");
        assert_eq!(normalize_posture("ReadOnly"), "readOnly");
        assert_eq!(normalize_posture("dontAsk"), "dontAsk");
        assert_eq!(normalize_posture("bogus"), "ask");
        assert_eq!(normalize_posture(""), "ask");
    }

    #[test]
    fn drift_emits_only_on_change() {
        assert_eq!(mode_drift("normal", "plan"), Some(MODE_PLAN));
        assert_eq!(mode_drift("goal", "goal"), None);

        assert_eq!(approval_drift("ask", "AUTO"), Some("auto".to_owned()));
        assert_eq!(approval_drift("dontAsk", "dontask"), None);
    }

    #[test]
    fn finish_order_reconciles_before_idle_and_reruns_after_drain() {
        assert!(FINISH_TURN_STEPS.windows(2).all(|w| w[0] != w[1]));
        let first_mode = FINISH_TURN_STEPS
            .iter()
            .position(|s| *s == "emit_mode_drift")
            .unwrap();
        let first_approval = FINISH_TURN_STEPS
            .iter()
            .position(|s| *s == "emit_tool_approval_drift")
            .unwrap();
        let idle = FINISH_TURN_STEPS
            .iter()
            .position(|s| *s == "finish_expose_idle")
            .unwrap();
        let drain = FINISH_TURN_STEPS
            .iter()
            .position(|s| *s == "drain_pending_reload")
            .unwrap();
        let last_mode = FINISH_TURN_STEPS
            .iter()
            .rposition(|s| *s == "emit_mode_drift")
            .unwrap();
        let last_approval = FINISH_TURN_STEPS
            .iter()
            .rposition(|s| *s == "emit_tool_approval_drift")
            .unwrap();

        assert!(first_mode < idle);
        assert!(first_approval < idle);
        assert!(drain < last_mode);
        assert!(drain < last_approval);
        assert!(last_mode > drain && last_approval > drain);
    }

    #[test]
    fn posture_rank_reuses_ceiling_order() {
        use crate::permission::posture_rank;
        assert!(posture_rank("readOnly") < posture_rank("ask"));
        assert!(posture_rank("ask") < posture_rank("yolo"));
        assert_eq!(posture_rank("bogus"), 0);
    }
}

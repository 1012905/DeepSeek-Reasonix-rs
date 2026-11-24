//! ACP-045 behavior coverage: mode/approval derivation, change-only drift
//! emission, and the finishTurn reconciliation order
//! (`service.go:445-484`, `service.go:869-887`, `session_config.go:489-502`).

use reasonix_acp::drift::{
    FINISH_TURN_STEPS, approval_drift, derive_mode, mode_drift, normalize_posture,
};
use reasonix_acp::permission::posture_rank;
use reasonix_acp::session_open::{MODE_GOAL, MODE_NORMAL, MODE_PLAN};

#[test]
fn mode_derivation_prefers_plan_then_goal_then_normal() {
    assert_eq!(derive_mode(true, true, false), MODE_PLAN);
    assert_eq!(derive_mode(false, true, false), MODE_GOAL);
    assert_eq!(derive_mode(false, false, true), MODE_GOAL);
    assert_eq!(derive_mode(false, false, false), MODE_NORMAL);
}

#[test]
fn posture_normalization_is_case_insensitive_with_ask_fallback() {
    assert_eq!(normalize_posture("  AUTO "), "auto");
    assert_eq!(normalize_posture("Yolo"), "yolo");
    assert_eq!(normalize_posture("ReadOnly"), "readOnly");
    assert_eq!(normalize_posture("dontAsk"), "dontAsk");
    assert_eq!(normalize_posture("read only"), "ask");
    assert_eq!(normalize_posture("bogus"), "ask");
    assert_eq!(normalize_posture(""), "ask");
}

#[test]
fn mode_drift_emits_only_when_recorded_mode_differs() {
    assert_eq!(mode_drift("normal", MODE_PLAN), Some(MODE_PLAN));
    assert_eq!(mode_drift("goal", MODE_GOAL), None);
    assert_eq!(mode_drift("plan", MODE_NORMAL), Some(MODE_NORMAL));
}

#[test]
fn approval_drift_normalizes_both_sides_before_comparing() {
    assert_eq!(approval_drift("ask", "AUTO"), Some("auto".to_owned()));
    assert_eq!(approval_drift("dontAsk", "dontask"), None);
    assert_eq!(
        approval_drift("ask", "ReadOnly"),
        Some("readOnly".to_owned())
    );
    assert_eq!(approval_drift("yolo", "bogus"), Some("ask".to_owned()));
}

#[test]
fn finish_order_reconciles_drift_before_idle_and_reruns_after_drain() {
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
    let config = FINISH_TURN_STEPS
        .iter()
        .position(|s| *s == "apply_pending_config")
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
    assert!(idle < config);
    assert!(config < drain);
    assert!(drain < last_mode);
    assert!(drain < last_approval);
    assert!(last_mode < last_approval || last_approval < last_mode);
    assert!(FINISH_TURN_STEPS.windows(2).all(|w| w[0] != w[1]));
}

#[test]
fn drift_rank_stays_on_shared_ceiling_order() {
    assert!(posture_rank("readOnly") < posture_rank("ask"));
    assert!(posture_rank("ask") < posture_rank("yolo"));
    assert_eq!(posture_rank("bogus"), 0);
}

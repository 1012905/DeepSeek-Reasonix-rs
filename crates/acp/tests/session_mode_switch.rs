//! ACP-032: `session/set_mode` dispatch table, legacy posture mapping, ceiling
//! precheck, draft-goal arming, and update emission rule (`session.go:235-290`).

use reasonix_acp::errors::RpcErrorCode;
use reasonix_acp::session_mode::{
    ModeBackend, ModeDecisionContext, apply_mode, decide_mode, should_emit_mode_update,
};

struct RecordingBackend {
    events: Vec<String>,
    goal_status: String,
}

impl ModeBackend for RecordingBackend {
    fn set_plan_mode(&mut self, on: bool) {
        self.events.push(format!("set_plan_mode:{on}"));
    }
    fn clear_goal(&mut self) {
        self.events.push("clear_goal".to_owned());
    }
    fn set_auto_approve_tools(&mut self, on: bool) {
        self.events.push(format!("auto_approve:{on}"));
    }
    fn set_tool_approval_mode(&mut self, approval: &str) {
        self.events.push(format!("approval:{approval}"));
    }
    fn goal_status(&self) -> String {
        self.goal_status.clone()
    }
}

#[test]
fn normal_plan_goal_dispatch_to_fixed_controller_actions() {
    let ctx = ModeDecisionContext::default();

    let normal = decide_mode("normal", &ctx).unwrap();
    assert_eq!(normal.next_mode, "normal");
    assert_eq!(normal.set_plan_mode, Some(false));
    assert!(normal.clear_goal);
    assert!(normal.legacy_approval.is_none());

    let plan = decide_mode("plan", &ctx).unwrap();
    assert_eq!(plan.next_mode, "plan");
    assert_eq!(plan.set_plan_mode, Some(true));
    assert!(plan.clear_goal);

    let goal = decide_mode("goal", &ctx).unwrap();
    assert_eq!(goal.next_mode, "goal");
    assert_eq!(goal.set_plan_mode, Some(false));
    assert!(!goal.clear_goal);
}

#[test]
fn legacy_default_and_auto_normalize_to_normal_with_posture() {
    let ctx = ModeDecisionContext {
        ceiling: "yolo".to_owned(),
        goal_status: String::new(),
    };
    let default = decide_mode("default", &ctx).unwrap();
    assert_eq!(default.next_mode, "normal");
    assert_eq!(default.legacy_approval.as_deref(), Some("ask"));

    let auto = decide_mode("auto", &ctx).unwrap();
    assert_eq!(auto.next_mode, "normal");
    assert_eq!(auto.legacy_approval.as_deref(), Some("yolo"));
}

#[test]
fn ceiling_check_runs_before_controller_mutation() {
    let ctx = ModeDecisionContext {
        ceiling: "ask".to_owned(),
        goal_status: String::new(),
    };
    let err = decide_mode("auto", &ctx).unwrap_err();
    assert_eq!(err.code, RpcErrorCode::InvalidParams);
    assert!(err.message.contains("above the ceiling"));
    assert_eq!(err.data.as_ref().unwrap()["ceiling"], "ask");
}

#[test]
fn unknown_mode_id_message_is_verbatim() {
    let err = decide_mode("bogus", &ModeDecisionContext::default()).unwrap_err();
    assert_eq!(err.message, "session/set_mode: unknown modeId bogus");
}

#[test]
fn draft_goal_arms_only_when_no_running_goal() {
    let running = ModeDecisionContext {
        ceiling: String::new(),
        goal_status: "running".to_owned(),
    };
    assert!(!decide_mode("goal", &running).unwrap().goal_draft_armed);

    let stopped = ModeDecisionContext {
        ceiling: String::new(),
        goal_status: "stopped".to_owned(),
    };
    assert!(decide_mode("goal", &stopped).unwrap().goal_draft_armed);
}

#[test]
fn apply_mode_runs_plan_clear_and_legacy_posture_in_order() {
    let mut backend = RecordingBackend {
        events: vec![],
        goal_status: String::new(),
    };
    let action = decide_mode(
        "auto",
        &ModeDecisionContext {
            ceiling: "yolo".to_owned(),
            goal_status: String::new(),
        },
    )
    .unwrap();
    apply_mode(&mut backend, &action);
    assert_eq!(
        backend.events,
        vec![
            "set_plan_mode:false",
            "clear_goal",
            "auto_approve:true",
            "approval:yolo",
        ]
    );
}

#[test]
fn current_mode_update_emits_only_on_change() {
    assert!(should_emit_mode_update("normal", "goal"));
    assert!(!should_emit_mode_update("goal", "goal"));
}

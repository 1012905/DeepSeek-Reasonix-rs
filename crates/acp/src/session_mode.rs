//! ACP-032 `session/set_mode` five-way dispatch, legacy posture mapping, and
//! draft-goal arming (`session.go:235-290`).
//!
//! The three collaboration modes and the two legacy aliases map onto a fixed
//! controller action set. Legacy aliases carry an implied tool-approval
//! posture that passes the startup ceiling check before any controller change;
//! a goal mode arms the draft-goal slot only when no goal is already running.
//! Real controller plan/goal setters, config-state publish, and sidecar
//! persistence are runtime wiring; this module owns the dispatch table, the
//! ceiling precheck, the draft condition, and the update emission rule.

use crate::errors::RpcError;
use crate::lifecycle::unknown_mode_id;
use crate::permission::{above_ceiling_error, within_ceiling};
use crate::session_open::{MODE_GOAL, MODE_NORMAL, MODE_PLAN};

/// Wire posture value for the legacy `default` alias (`session.go:263`).
const LEGACY_ASK: &str = "ask";
/// Wire posture value for the legacy `auto` alias (`session.go:271`).
const LEGACY_YOLO: &str = "yolo";
/// Goal status that keeps an installed goal live (`control.GoalStatusRunning`).
const GOAL_RUNNING: &str = "running";

/// Resolved action set the backend applies after the dispatch decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModeAction {
    /// Collaboration mode id the session ends up in.
    pub next_mode: String,
    /// Plan-mode toggle; `None` leaves the controller's plan state untouched.
    pub set_plan_mode: Option<bool>,
    /// Whether the controller's installed goal must be cleared.
    pub clear_goal: bool,
    /// Legacy implied approval posture, when the alias carried one.
    pub legacy_approval: Option<String>,
    /// Whether the draft-goal slot must be armed for a follow-up prompt.
    pub goal_draft_armed: bool,
}

/// Controller seam the mode action applies to.
pub trait ModeBackend {
    fn set_plan_mode(&mut self, on: bool);
    fn clear_goal(&mut self);
    fn set_auto_approve_tools(&mut self, on: bool);
    fn set_tool_approval_mode(&mut self, approval: &str);
    fn goal_status(&self) -> String;
}

/// Inputs the pure decision needs beyond the requested mode id.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ModeDecisionContext {
    /// Startup posture ceiling; empty means unrestricted.
    pub ceiling: String,
    /// Installed goal status, read before the dispatch changes anything.
    pub goal_status: String,
}

/// Resolves a requested mode id into the action set. The ceiling check runs
/// before any controller mutation (`session.go:259-260`, `session.go:267-268`).
pub fn decide_mode(raw_id: &str, ctx: &ModeDecisionContext) -> Result<ModeAction, RpcError> {
    let mut action = match raw_id {
        MODE_NORMAL => ModeAction {
            next_mode: MODE_NORMAL.to_owned(),
            set_plan_mode: Some(false),
            clear_goal: true,
            legacy_approval: None,
            goal_draft_armed: false,
        },
        MODE_PLAN => ModeAction {
            next_mode: MODE_PLAN.to_owned(),
            set_plan_mode: Some(true),
            clear_goal: true,
            legacy_approval: None,
            goal_draft_armed: false,
        },
        MODE_GOAL => ModeAction {
            next_mode: MODE_GOAL.to_owned(),
            set_plan_mode: Some(false),
            clear_goal: false,
            legacy_approval: None,
            goal_draft_armed: false,
        },
        "default" => {
            if !within_ceiling(LEGACY_ASK, &ctx.ceiling) {
                return Err(above_ceiling_error(
                    "session/set_mode",
                    LEGACY_ASK,
                    &ctx.ceiling,
                ));
            }
            ModeAction {
                next_mode: MODE_NORMAL.to_owned(),
                set_plan_mode: Some(false),
                clear_goal: true,
                legacy_approval: Some(LEGACY_ASK.to_owned()),
                goal_draft_armed: false,
            }
        }
        "auto" => {
            if !within_ceiling(LEGACY_YOLO, &ctx.ceiling) {
                return Err(above_ceiling_error(
                    "session/set_mode",
                    LEGACY_YOLO,
                    &ctx.ceiling,
                ));
            }
            ModeAction {
                next_mode: MODE_NORMAL.to_owned(),
                set_plan_mode: Some(false),
                clear_goal: true,
                legacy_approval: Some(LEGACY_YOLO.to_owned()),
                goal_draft_armed: false,
            }
        }
        other => return Err(unknown_mode_id(other)),
    };

    action.goal_draft_armed = action.next_mode == MODE_GOAL && ctx.goal_status != GOAL_RUNNING;
    Ok(action)
}

/// Applies the resolved action to the controller seam.
pub fn apply_mode(backend: &mut dyn ModeBackend, action: &ModeAction) {
    if let Some(on) = action.set_plan_mode {
        backend.set_plan_mode(on);
    }
    if action.clear_goal {
        backend.clear_goal();
    }
    if let Some(approval) = &action.legacy_approval {
        backend.set_auto_approve_tools(approval == LEGACY_YOLO);
        backend.set_tool_approval_mode(approval);
    }
}

/// `current_mode_update` is emitted only when the mode actually changes
/// (`session.go:285-287`).
pub fn should_emit_mode_update(current: &str, next: &str) -> bool {
    current != next
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::RpcErrorCode;

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
    fn three_collaboration_modes_dispatch_to_fixed_actions() {
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
    fn legacy_default_and_auto_map_to_normal_with_posture() {
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
    fn legacy_ceiling_check_runs_before_controller_mutation() {
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
    fn unknown_mode_id_is_rejected_verbatim() {
        let ctx = ModeDecisionContext::default();
        let err = decide_mode("bogus", &ctx).unwrap_err();
        assert_eq!(err.message, "session/set_mode: unknown modeId bogus");
    }

    #[test]
    fn goal_draft_arms_only_without_a_running_goal() {
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
    fn mode_update_emits_only_on_change() {
        assert!(should_emit_mode_update("normal", "goal"));
        assert!(!should_emit_mode_update("goal", "goal"));
    }
}

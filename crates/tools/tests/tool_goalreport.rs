//! The `update_goal` tool at the tools boundary (Go
//! `internal/tools/builtin/updategoal.go`, docs/contracts/tools.md §1.14):
//! schema/description, the status/reason/verified decision table, the recorder
//! routing, and the one refusal identity for a turn with no active goal turn.

use std::sync::{Arc, Mutex};

use reasonix_tools::abi::{CallContext, ContextualTool, Tool, plan_mode_safe};
use reasonix_tools::builtin::{
    GoalReport, GoalTurnRecorder, UPDATE_GOAL_DESCRIPTION, UPDATE_GOAL_TOOL_NAME, UpdateGoalTool,
    completion_note, goal_recorder_from, update_goal_tool,
};
use reasonix_tools::refusal::Refusal;
use reasonix_tools::registry::{FEATURE_GOAL_MODE, HostRequired};
use serde_json::json;

/// A recording recorder: proves the tool routes the parsed report out, and
/// lets the test decide what the recorder returns.
struct FakeRecorder {
    seen: Mutex<Vec<GoalReport>>,
    reply: String,
    fail: Option<Box<dyn std::error::Error + Send + Sync>>,
}

impl FakeRecorder {
    fn new(reply: &str) -> Self {
        Self {
            seen: Mutex::new(Vec::new()),
            reply: reply.to_owned(),
            fail: None,
        }
    }
}

impl GoalTurnRecorder for FakeRecorder {
    fn record_goal_report(
        &self,
        report: GoalReport,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        self.seen.lock().expect("recorder lock").push(report);
        match &self.fail {
            Some(err) => Err(Box::new(std::io::Error::other(err.to_string()))),
            None => Ok(self.reply.clone()),
        }
    }
}

fn ctx_with(recorder: Arc<FakeRecorder>) -> CallContext {
    let bound: Arc<Arc<dyn GoalTurnRecorder>> = Arc::new(recorder as Arc<dyn GoalTurnRecorder>);
    CallContext::new().with_value(bound)
}

#[test]
fn schema_description_and_flags_match_the_go_contract() {
    let t = UpdateGoalTool;
    assert_eq!(t.name(), UPDATE_GOAL_TOOL_NAME);
    assert_eq!(t.name(), "update_goal");
    assert_eq!(t.description(), UPDATE_GOAL_DESCRIPTION);
    assert!(t.description().contains("Report this turn's disposition"));
    assert!(t.description().contains("Delivery acceptance criteria"));

    let s = t.schema();
    assert_eq!(s["type"], "object");
    assert_eq!(s["required"], json!(["status"]));
    assert_eq!(
        s["properties"]["status"]["enum"],
        json!(["continue", "complete", "blocked"])
    );
    assert_eq!(s["properties"]["reason"]["type"], "string");
    assert_eq!(s["properties"]["next_action"]["type"], "string");
    assert!(s["properties"]["completion"]["type"].is_string());
    assert_eq!(
        s["properties"]["completion"]["properties"]["verified"]["type"],
        "array"
    );

    assert!(t.read_only(), "the tool records candidate state only");
    assert!(plan_mode_safe(&t), "goal reporting may run while planning");
    let host = t
        .capabilities()
        .get::<dyn HostRequired>()
        .expect("the tool declares its required feature");
    assert_eq!(host.host_required_by(), FEATURE_GOAL_MODE);
}

#[test]
fn without_a_recorder_it_fails_closed_with_the_one_identity() {
    let err = UpdateGoalTool
        .execute(&CallContext::new(), &json!({"status": "complete"}))
        .expect_err("no recorder means no report");
    let refusal = err
        .downcast_ref::<Refusal>()
        .expect("the unavailable arm carries a typed refusal");
    assert_eq!(refusal.code, "goal.no_active_turn");
    assert_eq!(
        refusal.message,
        "update_goal is only available while an active goal turn is running — no goal state was changed"
    );

    let reasoner = UpdateGoalTool
        .capabilities()
        .get::<dyn reasonix_tools::abi::ContextualReasoner>()
        .expect("the tool reasons about its own availability");
    let gate = reasoner.unavailable(&CallContext::new());
    assert_eq!(
        gate.code, refusal.code,
        "gate and stale path share one identity"
    );
    assert_eq!(gate.message, refusal.message);
}

#[test]
fn the_contextual_gate_follows_the_recorder_binding() {
    let gate = UpdateGoalTool
        .capabilities()
        .get::<dyn ContextualTool>()
        .expect("the tool exposes its contextual gate");
    assert!(
        !gate.provider_visible(&CallContext::new()),
        "no recorder, no provider-visible tool"
    );

    let recorder = Arc::new(FakeRecorder::new("recorded."));
    assert!(
        gate.provider_visible(&ctx_with(recorder)),
        "a bound recorder makes the tool visible"
    );
}

#[test]
fn an_unknown_status_is_rejected_before_any_state_changes() {
    let cases = [
        (
            "sideways",
            r#"update_goal: status must be one of continue|complete|blocked, got "sideways" — no goal state was changed"#,
        ),
        (
            "",
            r#"update_goal: status must be one of continue|complete|blocked, got "" — no goal state was changed"#,
        ),
    ];
    for (status, want) in cases {
        let err = UpdateGoalTool
            .execute(&CallContext::new(), &json!({"status": status}))
            .unwrap_err();
        assert_eq!(err.to_string(), want, "status {status:?}");
    }
}

#[test]
fn continue_and_blocked_require_a_reason() {
    for status in ["continue", "blocked"] {
        let err = UpdateGoalTool
            .execute(
                &CallContext::new(),
                &json!({"status": status, "reason": "  "}),
            )
            .unwrap_err();
        assert_eq!(
            err.to_string(),
            format!("update_goal: reason is required for {status} — no goal state was changed")
        );
    }

    let err = UpdateGoalTool
        .execute(&CallContext::new(), &json!({"status": "complete"}))
        .expect_err("still fails without a recorder");
    // Complete without a reason still reaches the missing-recorder arm, so
    // the reason rule did not fire: its identity is the goal refusal, not a
    // reason error.
    let refusal = err.downcast_ref::<Refusal>().expect("goal refusal");
    assert_eq!(refusal.code, "goal.no_active_turn");
}

#[test]
fn a_blank_verified_entry_names_its_index() {
    let err = UpdateGoalTool
        .execute(
            &CallContext::new(),
            &json!({"status": "complete", "completion": {"verified": ["echo ok", "  "]}}),
        )
        .unwrap_err();
    assert_eq!(
        err.to_string(),
        "update_goal: completion.verified[1] is empty — cite the command as it actually ran, or leave the list out"
    );
}

#[test]
fn the_recorder_receives_the_normalized_report_and_passes_its_reply_on() {
    let recorder = Arc::new(FakeRecorder::new("recorded: continuing"));
    let out = UpdateGoalTool
        .execute(
            &ctx_with(recorder.clone()),
            &json!({
                "status": " Continue ",
                "reason": "work in progress",
                "next_action": "check the log",
            }),
        )
        .expect("recorder accepts the report");

    let seen = recorder.seen.lock().expect("recorder lock");
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].status, "continue");
    assert_eq!(seen[0].reason, "work in progress");
    assert_eq!(seen[0].next_action, "check the log");
    assert_eq!(out, "recorded: continuing");
}

#[test]
fn a_complete_report_appends_the_completion_account_note() {
    let recorder = Arc::new(FakeRecorder::new("recorded: done"));
    let out = UpdateGoalTool
        .execute(
            &ctx_with(recorder),
            &json!({
                "status": "complete",
                "reason": "verified by tests",
                "completion": {
                    "verified": ["cargo test", "cargo clippy"],
                    "unverified": ["manual check"],
                    "risks": ["flaky network"],
                },
            }),
        )
        .expect("recorder accepts the report");
    assert_eq!(
        out,
        "recorded: done Completion account recorded (2 verified, 1 unverified, 1 risks); the verified commands are reconciled against this session's receipts."
    );
}

#[test]
fn a_complete_report_with_no_account_says_only_what_the_host_saw() {
    let recorder = Arc::new(FakeRecorder::new("recorded: done"));
    let out = UpdateGoalTool
        .execute(
            &ctx_with(recorder),
            &json!({"status": "complete", "reason": "done"}),
        )
        .expect("recorder accepts the report");
    assert_eq!(
        out,
        "recorded: done No completion account was given: the report will carry only what the host could observe."
    );
}

#[test]
fn a_recorder_error_passes_through_unchanged() {
    #[derive(Debug)]
    struct Boom;
    impl std::error::Error for Boom {}
    impl std::fmt::Display for Boom {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("goal turn rejected the report")
        }
    }
    let recorder = Arc::new(FakeRecorder {
        seen: Mutex::new(Vec::new()),
        reply: String::new(),
        fail: Some(Box::new(Boom)),
    });
    let err = UpdateGoalTool
        .execute(
            &ctx_with(recorder),
            &json!({"status": "blocked", "reason": "stuck"}),
        )
        .unwrap_err();
    assert!(err.to_string().contains("goal turn rejected the report"));
}

#[test]
fn the_note_renders_counts_with_an_empty_account_as_nothing() {
    assert_eq!(
        completion_note(0, 0, 0),
        " No completion account was given: the report will carry only what the host could observe."
    );
    assert_eq!(
        completion_note(3, 2, 1),
        " Completion account recorded (3 verified, 2 unverified, 1 risks); the verified commands are reconciled against this session's receipts."
    );
}

#[test]
fn the_registered_tool_is_the_same_instance_as_the_constructor() {
    let direct = update_goal_tool();
    assert_eq!(direct.name(), UPDATE_GOAL_TOOL_NAME);
    assert_eq!(direct.description(), UPDATE_GOAL_DESCRIPTION);
    assert_eq!(direct.schema(), UpdateGoalTool.schema());
    assert!(goal_recorder_from(&CallContext::new()).is_none());
}

#[test]
fn an_argument_type_mismatch_is_rejected_as_invalid_args() {
    let err = UpdateGoalTool
        .execute(
            &CallContext::new(),
            &json!({"status": "complete", "completion": {"verified": "not an array"}}),
        )
        .unwrap_err();
    assert!(err.to_string().starts_with("invalid update_goal args:"));
}

#[test]
fn a_full_registered_tool_reads_as_read_only_in_the_contract() {
    let registry = reasonix_tools::registry::Registry::new();
    registry.add(update_goal_tool());
    let entries = registry.contract_entries();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "update_goal");
    assert!(entries[0].read_only);
    assert_eq!(entries[0].schema["required"], json!(["status"]));
}

//! The plan face at the tools boundary (Go `internal/tools/builtin/todo.go`,
//! `completestep.go`, `internal/contract/evidence/`): tool cards and the
//! sequential/host-required decisions, todo validation and receipt rendering,
//! complete_step identity checks, and plan-mode visibility/refusal.

use std::sync::Arc;

use reasonix_tools::abi::{CallContext, ContextualReasoner, ContextualTool, Tool};
use reasonix_tools::builtin::{
    CODE_PLAN_MODE_ACTIVE, COMPLETE_STEP_TOOL_NAME, CompleteStepTool, PlanHost, PlanModeReader,
    TODO_WRITE_TOOL_NAME, TodoItem, TodoStepMatch, TodoWriteTool,
};
use serde_json::json;

struct FakePlan {
    todos: Vec<TodoItem>,
}

impl FakePlan {
    fn new(todos: Vec<TodoItem>) -> Self {
        Self { todos }
    }
}

impl PlanHost for FakePlan {
    fn latest_todos(&self) -> Vec<TodoItem> {
        self.todos.clone()
    }
    fn unverified_completed(&self, _current: &[TodoItem]) -> (Vec<TodoStepMatch>, bool) {
        (Vec::new(), true)
    }
}

struct ActiveMode(bool);

impl PlanModeReader for ActiveMode {
    fn plan_active(&self) -> bool {
        self.0
    }
}

fn plan_ctx(host: Arc<FakePlan>) -> CallContext {
    let bound: Arc<Arc<dyn PlanHost>> = Arc::new(host as Arc<dyn PlanHost>);
    CallContext::new().with_value(bound)
}

fn mode_ctx(active: bool) -> CallContext {
    let bound: Arc<Arc<dyn PlanModeReader>> =
        Arc::new(Arc::new(ActiveMode(active)) as Arc<dyn PlanModeReader>);
    CallContext::new().with_value(bound)
}

#[test]
fn todo_write_card_is_read_only_and_sequential() {
    let tool = TodoWriteTool;
    assert_eq!(tool.name(), TODO_WRITE_TOOL_NAME);
    assert!(tool.description().contains("structured task list"));
    assert!(tool.schema().is_object());
    assert!(tool.read_only());
}

#[test]
fn complete_step_card_is_read_only() {
    let tool = CompleteStepTool;
    assert_eq!(tool.name(), COMPLETE_STEP_TOOL_NAME);
    assert!(tool.description().contains("approved plan"));
    assert!(tool.schema().is_object());
    assert!(tool.read_only());
}

#[test]
fn todo_write_accepts_a_valid_two_level_list() {
    let args = json!({"todos": [
        {"content": "Phase", "status": "pending", "level": 0},
        {"content": "First sub-step", "status": "in_progress", "level": 1, "step_id": "plan_step_01"},
        {"content": "Second sub-step", "status": "pending", "level": 1}
    ]});
    let out = TodoWriteTool
        .execute(&CallContext::new(), &args)
        .expect("valid list accepted");
    assert!(out.contains("Todos updated: 3 total — 0 completed, 1 in progress, 2 pending"));
    assert!(out.contains("The in_progress item is \"plan_step_01\""));
}

#[test]
fn todo_write_rejects_an_invalid_status() {
    let args = json!({"todos": [{"content": "Do thing", "status": "done"}]});
    let err = TodoWriteTool
        .execute(&CallContext::new(), &args)
        .expect_err("bad status refused");
    assert!(err.to_string().contains("invalid status \"done\""));
}

#[test]
fn todo_write_rejects_empty_content() {
    let args = json!({"todos": [{"content": "", "status": "pending"}]});
    let err = TodoWriteTool
        .execute(&CallContext::new(), &args)
        .expect_err("empty content refused");
    assert!(err.to_string().contains("content is required"));
}

#[test]
fn complete_step_requires_an_identity_and_a_result() {
    let err = CompleteStepTool
        .execute(&CallContext::new(), &json!({"result": "done"}))
        .expect_err("missing identity refused");
    assert!(
        err.to_string()
            .contains("step_id, step, or step_index is required")
    );

    let err = CompleteStepTool
        .execute(
            &CallContext::new(),
            &json!({"step_id": "plan_step_01", "result": ""}),
        )
        .expect_err("missing result refused");
    assert!(err.to_string().contains("result is required"));
}

#[test]
fn complete_step_without_a_list_reports_no_advance() {
    let out = CompleteStepTool
        .execute(
            &CallContext::new(),
            &json!({"step_id": "plan_step_01", "result": "Tests pass"}),
        )
        .expect("sign-off records against an empty host list");
    assert!(out.starts_with("Step \"plan_step_01\" done."));
    assert!(out.contains("no task list to advance"));
}

#[test]
fn complete_step_is_hidden_while_plan_is_active() {
    let tool = CompleteStepTool;
    let visible: Arc<dyn ContextualTool> = tool
        .capabilities()
        .get()
        .expect("contextual capability installed");
    assert!(visible.provider_visible(&mode_ctx(false)));
    assert!(!visible.provider_visible(&mode_ctx(true)));

    let reasoner: Arc<dyn ContextualReasoner> = tool
        .capabilities()
        .get()
        .expect("reasoner capability installed");
    let refusal = reasoner.unavailable(&mode_ctx(true));
    assert_eq!(refusal.code, CODE_PLAN_MODE_ACTIVE);
    assert!(
        refusal
            .message
            .contains("only available after plan approval")
    );
}

#[test]
fn complete_step_matches_the_in_progress_item_from_the_host_list() {
    let host = Arc::new(FakePlan::new(vec![
        TodoItem {
            content: "First sub-step".into(),
            status: "in_progress".into(),
            active_form: "Running tests".into(),
            level: 1,
            step_id: "plan_step_01".into(),
        },
        TodoItem {
            content: "Second sub-step".into(),
            status: "pending".into(),
            active_form: "Finishing the second sub-step".into(),
            level: 1,
            step_id: "plan_step_02".into(),
        },
    ]));
    let out = CompleteStepTool
        .execute(
            &plan_ctx(host),
            &json!({"step_id": "plan_step_01", "result": "Tests pass"}),
        )
        .expect("current item signs off");
    assert!(out.contains("The host advanced the task list; the next step is 2"));
}

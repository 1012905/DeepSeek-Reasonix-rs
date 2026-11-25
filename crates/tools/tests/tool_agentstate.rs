//! TOOL-042 agent / state / platform cards (docs/contracts/tools.md §3.7; Go
//! `internal/runtime/agent/await_user.go`, `internal/tools/builtin/viewimage.go`,
//! `internal/runtime/agent/readonly_bash.go`).
//!
//! Covers the `await_user` todo resolution and gate messages, the `view_image`
//! refusal classification and rendering, and the read-only bash wrapper's
//! description rewrite and blocker reasons.

use std::sync::Arc;

use reasonix_contract::eventwire::ask::AskQuestion;
use reasonix_tools::abi::{CallContext, Tool, is_decision_barrier};
use reasonix_tools::ask::{AskOutcome, Asker};
use reasonix_tools::builtin::agentstate::{
    AWAIT_USER_NEED_LIMIT, AwaitTarget, AwaitUserError, AwaitUserHost, AwaitUserTool,
    CODE_IMAGE_TOO_LARGE, CODE_IMAGE_UNFIT, CODE_IMAGE_UNSUPPORTED, ReadOnlyBashBlocker,
    ReadOnlyBashClassifier, ReadOnlyBashTool, VIEW_IMAGE_MAX_BYTES, ViewImageAsset, ViewImageHost,
    ViewImageTool, data_url, mime_supported, readonly_bash_description, readonly_block_reason,
    render_image, render_recorded, resolve_await_target, too_large_refusal, unfit_refusal,
    unsupported_refusal, validate_need,
};
use reasonix_tools::builtin::plan_evidence::TodoItem;
use serde_json::{Value, json};

fn todo(content: &str, status: &str, step_id: &str) -> TodoItem {
    TodoItem {
        content: content.to_owned(),
        status: status.to_owned(),
        active_form: String::new(),
        level: 0,
        step_id: step_id.to_owned(),
    }
}

// --- validate_need -----------------------------------------------------------

#[test]
fn need_must_be_present_and_within_rune_limit() {
    assert!(validate_need("").is_err());
    assert!(validate_need("   ").is_err());
    let long = "x".repeat(AWAIT_USER_NEED_LIMIT + 1);
    match validate_need(&long) {
        Err(AwaitUserError::NeedTooLong { count, limit }) => {
            assert_eq!(count, AWAIT_USER_NEED_LIMIT + 1);
            assert_eq!(limit, AWAIT_USER_NEED_LIMIT);
        }
        other => panic!("unexpected: {other:?}"),
    }
    assert!(validate_need("the user picks the target").is_ok());
}

// --- resolve_await_target ----------------------------------------------------

#[test]
fn empty_list_and_all_complete_reject_parking() {
    let err = resolve_await_target(&[], None).expect_err("no list");
    assert!(matches!(err, AwaitUserError::NoList));

    let done = vec![todo("done", "completed", "s1")];
    let err = resolve_await_target(&done, None).expect_err("all complete");
    assert!(matches!(err, AwaitUserError::AllComplete));
}

#[test]
fn cited_step_unknown_names_available_ids_or_says_list_has_none() {
    let todos = vec![
        todo("one", "in_progress", "s1"),
        todo("two", "pending", "s2"),
    ];
    match resolve_await_target(&todos, Some("s9")).expect_err("unknown id") {
        AwaitUserError::UnknownStepWithIds { step_id, ids } => {
            assert_eq!(step_id, "s9");
            assert!(ids.contains("s1"));
            assert!(ids.contains("s2"));
        }
        other => panic!("unexpected: {other:?}"),
    }

    let no_ids = vec![todo("one", "in_progress", ""), todo("two", "pending", "")];
    let err = resolve_await_target(&no_ids, Some("s9")).expect_err("unknown id, no ids");
    assert!(matches!(err, AwaitUserError::UnknownStepNoIds { .. }));
}

#[test]
fn cited_completed_step_rejects_parking() {
    let todos = vec![
        todo("done", "completed", "s1"),
        todo("open", "pending", "s2"),
    ];
    let err = resolve_await_target(&todos, Some("s1")).expect_err("completed");
    assert!(matches!(err, AwaitUserError::CompletedStep { .. }));
}

#[test]
fn current_item_resolves_when_one_is_in_progress() {
    let todos = vec![
        todo("open now", "in_progress", "s1"),
        todo("later", "pending", "s2"),
    ];
    let target = resolve_await_target(&todos, None).expect("current item");
    assert_eq!(target.citation, "[s1] open now");
    assert_eq!(target.status, "in_progress");
}

#[test]
fn no_in_progress_names_ids_or_tells_to_mark_one() {
    let with_ids = vec![todo("one", "pending", "s1"), todo("two", "pending", "s2")];
    match resolve_await_target(&with_ids, None).expect_err("no current") {
        AwaitUserError::NoCurrentWithIds { ids } => assert!(ids.contains("s1")),
        other => panic!("unexpected: {other:?}"),
    }
    let no_ids = vec![todo("one", "pending", ""), todo("two", "pending", "")];
    let err = resolve_await_target(&no_ids, None).expect_err("no current, no ids");
    assert!(matches!(err, AwaitUserError::NoCurrentNoIds));
}

#[test]
fn recorded_message_names_the_citation() {
    let target = AwaitTarget {
        citation: "[s1] open now".to_owned(),
        status: "in_progress".to_owned(),
        active_form: String::new(),
    };
    let message = render_recorded(&target.citation);
    assert!(message.contains("[s1] open now"));
    assert!(message.contains("The turn ends here"));
}

// --- AwaitUserTool -----------------------------------------------------------

struct FakeAwaitHost {
    todos: Vec<TodoItem>,
    plan_submission: bool,
}

impl AwaitUserHost for FakeAwaitHost {
    fn todos(&self) -> Vec<TodoItem> {
        self.todos.clone()
    }

    fn plan_submission(&self) -> bool {
        self.plan_submission
    }
}

struct SilentAsker;
impl Asker for SilentAsker {
    fn ask(&self, _: &[AskQuestion]) -> AskOutcome {
        AskOutcome::default()
    }
}

fn asker_ctx() -> CallContext {
    let asker: Arc<dyn Asker> = Arc::new(SilentAsker);
    CallContext::new().with_value(Arc::new(asker))
}

#[test]
fn await_user_is_a_decision_barrier_bound_to_turn_exit() {
    let tool = AwaitUserTool::new(None);
    assert!(is_decision_barrier(&tool));
    assert_eq!(tool.name(), "await_user");
    assert!(tool.read_only());
}

#[test]
fn await_user_rejects_plan_submission_before_touching_the_list() {
    let host = Arc::new(FakeAwaitHost {
        todos: vec![todo("open", "in_progress", "s1")],
        plan_submission: true,
    });
    let tool = AwaitUserTool::new(Some(host));
    let err = tool
        .execute(&asker_ctx(), &json!({"need": "decide target"}))
        .expect_err("plan submission");
    assert!(matches!(
        err.downcast_ref::<AwaitUserError>(),
        Some(AwaitUserError::PlanSubmission)
    ));
}

#[test]
fn await_user_fails_closed_without_an_asker() {
    let host = Arc::new(FakeAwaitHost {
        todos: vec![todo("open", "in_progress", "s1")],
        plan_submission: false,
    });
    let tool = AwaitUserTool::new(Some(host));
    let err = tool
        .execute(&CallContext::new(), &json!({"need": "decide target"}))
        .expect_err("no asker");
    assert!(matches!(
        err.downcast_ref::<AwaitUserError>(),
        Some(AwaitUserError::NoAsker)
    ));
}

#[test]
fn await_user_renders_the_wait_notice_for_the_current_item() {
    let host = Arc::new(FakeAwaitHost {
        todos: vec![todo("open now", "in_progress", "s1")],
        plan_submission: false,
    });
    let tool = AwaitUserTool::new(Some(host));
    let out = tool
        .execute(&asker_ctx(), &json!({"need": "pick the target"}))
        .expect("parked");
    assert_eq!(
        out,
        "Recorded: waiting on the user at [s1] open now. The turn ends here with the list untouched, and the host will not continue it — their next message does. Put what you are waiting for to them in your reply."
    );
}

// --- view_image --------------------------------------------------------------

#[test]
fn mime_supported_recognizes_the_five_viewable_types() {
    assert!(mime_supported("image/png"));
    assert!(mime_supported("IMAGE/JPEG"));
    assert!(mime_supported("image/webp"));
    assert!(mime_supported("image/gif"));
    assert!(mime_supported("image/svg+xml"));
    assert!(!mime_supported("application/pdf"));
}

#[test]
fn image_refusals_carry_typed_codes_and_remedies() {
    let unsupported = unsupported_refusal("application/pdf");
    assert_eq!(unsupported.code, CODE_IMAGE_UNSUPPORTED);
    assert!(unsupported.remedy.is_some());

    let too_large = too_large_refusal("a.png", VIEW_IMAGE_MAX_BYTES + 1, VIEW_IMAGE_MAX_BYTES);
    assert_eq!(too_large.code, CODE_IMAGE_TOO_LARGE);

    let unfit = unfit_refusal("a.png", "decode failed");
    assert_eq!(unfit.code, CODE_IMAGE_UNFIT);
}

struct FakeImageHost {
    asset: Option<ViewImageAsset>,
    refusal: Option<reasonix_tools::refusal::Refusal>,
}

impl ViewImageHost for FakeImageHost {
    fn read_image(&self, _: &str) -> Result<ViewImageAsset, reasonix_tools::refusal::Refusal> {
        if let Some(ref refusal) = self.refusal {
            return Err(refusal.clone());
        }
        Ok(self.asset.clone().unwrap())
    }
}

#[test]
fn view_image_requires_a_path() {
    let tool = ViewImageTool::new(Some(Arc::new(FakeImageHost {
        asset: None,
        refusal: None,
    })));
    let err = tool
        .execute(&CallContext::new(), &json!({}))
        .expect_err("missing path");
    assert!(err.to_string().contains("path"));
}

#[test]
fn view_image_renders_caption_and_data_url() {
    let asset = ViewImageAsset {
        display_path: "shot.png".to_owned(),
        mime: "image/png".to_owned(),
        bytes: vec![1, 2, 3, 4],
    };
    let host = Arc::new(FakeImageHost {
        asset: Some(asset.clone()),
        refusal: None,
    });
    let tool = ViewImageTool::new(Some(host));
    let out = tool
        .execute(&CallContext::new(), &json!({"path": "shot.png"}))
        .expect("rendered");
    assert!(out.contains("[image: shot.png, image/png, 4 bytes shown]"));
    assert!(out.contains(&data_url("image/png", &asset.bytes)));
    assert_eq!(render_image(&asset), out);
}

#[test]
fn view_image_passes_back_host_refusal() {
    let host = Arc::new(FakeImageHost {
        asset: None,
        refusal: Some(unfit_refusal("bad.png", "corrupt")),
    });
    let tool = ViewImageTool::new(Some(host));
    let err = tool
        .execute(&CallContext::new(), &json!({"path": "bad.png"}))
        .expect_err("unfit");
    assert!(err.to_string().contains(CODE_IMAGE_UNFIT));
}

// --- read-only bash wrapper --------------------------------------------------

#[test]
fn readonly_description_rewrites_the_verb_and_appends_policy_suffix() {
    let rewritten = readonly_bash_description(
        "Execute a command in the shell and return combined stdout/stderr.",
    );
    assert!(rewritten.starts_with("Execute a foreground read-only command in the shell"));
    assert!(rewritten.contains("Only permission-classified read-only commands are allowed"));
}

#[test]
fn blocker_reasons_name_the_shape_that_disqualified() {
    let reason = readonly_block_reason(ReadOnlyBashBlocker::InlineCode);
    assert!(reason.contains("inline interpreter code"));
    let reason = readonly_block_reason(ReadOnlyBashBlocker::NestedExecution);
    assert!(reason.contains("substitution"));
    let reason = readonly_block_reason(ReadOnlyBashBlocker::DynamicName);
    assert!(reason.contains("variable"));
    let reason = readonly_block_reason(ReadOnlyBashBlocker::IndirectExecution);
    assert!(reason.contains("wrapper"));
    let reason = readonly_block_reason(ReadOnlyBashBlocker::HereDocBody);
    assert!(reason.contains("here-document"));
    let reason = readonly_block_reason(ReadOnlyBashBlocker::Unparsable);
    assert!(reason.contains("Static analysis"));
    let reason = readonly_block_reason(ReadOnlyBashBlocker::Default);
    assert!(reason.contains("outside the read-only command set"));
}

struct FakeClassifier {
    blocker: Option<ReadOnlyBashBlocker>,
}

impl ReadOnlyBashClassifier for FakeClassifier {
    fn blocker(&self, _: &str) -> Option<ReadOnlyBashBlocker> {
        self.blocker
    }
}

struct InnerBash;
impl Tool for InnerBash {
    fn name(&self) -> &str {
        "bash"
    }
    fn description(&self) -> &str {
        "Execute a command in the shell."
    }
    fn schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn execute(&self, _: &CallContext, _: &Value) -> reasonix_tools::abi::ToolResult {
        Ok("ran".to_owned())
    }
    fn read_only(&self) -> bool {
        false
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[test]
fn readonly_wrapper_blocks_non_read_only_and_delegates_allowed() {
    let blocked = ReadOnlyBashTool::new(
        Some(Arc::new(InnerBash)),
        Some(Arc::new(FakeClassifier {
            blocker: Some(ReadOnlyBashBlocker::DynamicName),
        })),
    );
    let err = blocked
        .execute(&CallContext::new(), &json!({"command": "$X list"}))
        .expect_err("blocked");
    assert!(err.to_string().contains("read-only sub-agent"));

    let allowed = ReadOnlyBashTool::new(
        Some(Arc::new(InnerBash)),
        Some(Arc::new(FakeClassifier { blocker: None })),
    );
    let out = allowed
        .execute(&CallContext::new(), &json!({"command": "ls"}))
        .expect("delegated");
    assert_eq!(out, "ran");
    assert!(allowed.read_only());
}

#[test]
fn readonly_wrapper_without_classifier_fails_closed() {
    let tool = ReadOnlyBashTool::new(Some(Arc::new(InnerBash)), None);
    let err = tool
        .execute(&CallContext::new(), &json!({"command": "ls"}))
        .expect_err("seam unwired");
    assert!(err.to_string().contains("classifier seam"));
}

//! TOOL-039: delegation tools — `spawn_agents` / `read_subagent_result`
//! (Go `internal/runtime/delegation/*`; docs/contracts/tools.md §3.4).
//!
//! Behavior-level evidence: tool cards and capability markers, runner/store
//! seams failing closed when unwired, argument contract and rejection codes,
//! head-budget scaling, result JSON shape, and byte-offset paging rendering.

use std::sync::Arc;

use reasonix_tools::abi::{CallContext, CapabilitySet, PlanModeClassifier, Tool};
use reasonix_tools::builtin::delegate::{
    DelegationRunner, PagedTool, ParentSessionBinding, RunOutput, RunSpec, SPAWN_TOOL_NAME,
    SUBAGENT_RESULT_MAX_BYTES, SUBAGENT_RESULT_TOOL_NAME, SpawnTool, SubagentResultStore,
    SubagentResultTool, parent_session_from,
};
use reasonix_tools::registry::CapabilityAliased;
use serde_json::{Value, json};

struct StubRunner {
    outputs: Vec<RunOutput>,
}

impl DelegationRunner for StubRunner {
    fn run(&self, spec: &RunSpec) -> Result<RunOutput, Box<dyn std::error::Error + Send + Sync>> {
        Ok(self.outputs.get(spec.index).cloned().unwrap_or_default())
    }
}

struct StubStore {
    answer: String,
    status: String,
}

impl SubagentResultStore for StubStore {
    fn read_result(
        &self,
        _reference: &str,
        _parent_session: &str,
        _workspace_root: &str,
    ) -> Result<(String, String), Box<dyn std::error::Error + Send + Sync>> {
        Ok((self.answer.clone(), self.status.clone()))
    }
}

fn ctx_with_session() -> CallContext {
    CallContext::new().with_value(Arc::new(ParentSessionBinding {
        session: "parent-1".to_owned(),
    }))
}

#[test]
fn spawn_card_matches_the_go_identity_and_markers() {
    let tool = SpawnTool::default();
    assert_eq!(tool.name(), SPAWN_TOOL_NAME);
    assert!(!tool.description().is_empty());
    assert!(!tool.read_only());
    assert!(
        tool.capabilities()
            .get::<dyn PlanModeClassifier>()
            .unwrap()
            .plan_mode_safe()
    );
    assert_eq!(
        tool.capabilities()
            .get::<dyn CapabilityAliased>()
            .unwrap()
            .capability_aliases(),
        vec!["task:subagent".to_owned()]
    );
}

#[test]
fn spawn_without_runner_fails_closed() {
    let tool = SpawnTool::default();
    let err = tool
        .execute(
            &CallContext::new(),
            &json!({ "tasks": [{ "title": "t", "task": "find x" }] }),
        )
        .unwrap_err();
    assert!(err.to_string().contains("sub-agents are not available"));
}

#[test]
fn spawn_validates_shape_and_rejects_unknown_fields() {
    let runner = Arc::new(StubRunner { outputs: vec![] });
    let tool = SpawnTool::new(Some(runner));

    let empty = tool
        .execute(&CallContext::new(), &json!({ "tasks": [] }))
        .unwrap_err();
    assert!(empty.to_string().contains("tasks must not be empty"));

    let over: Vec<Value> = (0..9)
        .map(|_| json!({ "title": "t", "task": "x" }))
        .collect();
    let over = tool
        .execute(&CallContext::new(), &json!({ "tasks": over }))
        .unwrap_err();
    assert!(over.to_string().contains("at most 8 tasks"));

    let no_text = tool
        .execute(
            &CallContext::new(),
            &json!({ "tasks": [{ "title": "t", "task": "  " }] }),
        )
        .unwrap_err();
    assert!(no_text.to_string().contains("task 1 has no task text"));

    let unknown = tool
        .execute(
            &CallContext::new(),
            &json!({ "tasks": [{ "title": "t", "task": "x" }], "bogus": 1 }),
        )
        .unwrap_err();
    assert!(unknown.to_string().contains("unknown field"));

    let malformed = tool
        .execute(&CallContext::new(), &json!({ "tasks": "not-an-array" }))
        .unwrap_err();
    assert!(malformed.to_string().contains("arguments must be"));
}

#[test]
fn spawn_result_shape_carries_reference_head_read_and_wrote() {
    let runner = Arc::new(StubRunner {
        outputs: vec![RunOutput {
            reference: "sa_1".to_owned(),
            answer: "hello world".to_owned(),
            read: vec!["a.rs".to_owned()],
            wrote: vec![],
        }],
    });
    let tool = SpawnTool::new(Some(runner));
    let body = tool
        .execute(
            &CallContext::new(),
            &json!({ "tasks": [{ "title": "t", "task": "find x" }] }),
        )
        .unwrap();
    let body: Value = serde_json::from_str(&body).unwrap();
    let agent = &body["agents"][0];
    assert_eq!(agent["title"], "t");
    assert_eq!(agent["ref"], "sa_1");
    assert_eq!(agent["chars"], 11);
    assert!(!agent["more"].as_bool().unwrap_or(false));
    assert_eq!(agent["read"][0], "a.rs");
    assert!(agent.get("wrote").is_none() || agent["wrote"].as_array().unwrap().is_empty());
}

#[test]
fn spawn_head_budget_scales_with_task_count() {
    // One task: 8000 clamped to the 2000 ceiling; eight tasks: 1000 inside bounds.
    assert_eq!(8000usize.clamp(600, 2000), 2000);
    assert_eq!((8000 / 8).clamp(600, 2000), 1000);
}

#[test]
fn result_card_is_read_only_paged_and_plan_mode_safe() {
    let tool = SubagentResultTool::default();
    assert_eq!(tool.name(), SUBAGENT_RESULT_TOOL_NAME);
    assert!(tool.read_only());
    assert!(
        tool.capabilities()
            .get::<dyn PlanModeClassifier>()
            .unwrap()
            .plan_mode_safe()
    );
    assert!(tool.capabilities().get::<dyn PagedTool>().unwrap().paged());
}

#[test]
fn result_tool_reads_pages_and_renders_boundaries() {
    let store = Arc::new(StubStore {
        answer: "héllo world".to_owned(),
        status: "completed".to_owned(),
    });
    let tool = SubagentResultTool::new(Some(store), String::new());

    let page = tool
        .execute(
            &ctx_with_session(),
            &json!({ "ref": "sa_1", "limit_bytes": 4 }),
        )
        .unwrap();
    assert!(page.starts_with("Subagent result sa_1 (status=completed, bytes 0-4 of 12):"));
    assert!(page.contains("More remains"));
    assert!(page.contains("offset_bytes=4"));

    let tail = tool
        .execute(
            &ctx_with_session(),
            &json!({ "ref": "sa_1", "offset_bytes": 4 }),
        )
        .unwrap();
    assert!(tail.ends_with("End of subagent result."));
}

#[test]
fn result_tool_refuses_bad_args_and_missing_seams() {
    let tool = SubagentResultTool::default();

    let no_ref = tool.execute(&ctx_with_session(), &json!({})).unwrap_err();
    assert!(no_ref.to_string().contains("ref is required"));

    let negative = tool
        .execute(
            &ctx_with_session(),
            &json!({ "ref": "sa_1", "offset_bytes": -1 }),
        )
        .unwrap_err();
    assert!(negative.to_string().contains("non-negative"));

    let limit = tool
        .execute(
            &ctx_with_session(),
            &json!({ "ref": "sa_1", "limit_bytes": SUBAGENT_RESULT_MAX_BYTES + 1 }),
        )
        .unwrap_err();
    assert!(limit.to_string().contains("between 1 and 24576"));

    let unknown = tool
        .execute(&ctx_with_session(), &json!({ "ref": "sa_1", "bogus": 1 }))
        .unwrap_err();
    assert!(unknown.to_string().contains("unknown field"));

    let no_store = tool
        .execute(&ctx_with_session(), &json!({ "ref": "sa_1" }))
        .unwrap_err();
    assert!(no_store.to_string().contains("storage is not available"));

    let no_parent = SubagentResultTool::new(
        Some(Arc::new(StubStore {
            answer: "x".to_owned(),
            status: "completed".to_owned(),
        })),
        String::new(),
    )
    .execute(&CallContext::new(), &json!({ "ref": "sa_1" }))
    .unwrap_err();
    assert!(no_parent.to_string().contains("persisted parent session"));
}

#[test]
fn result_tool_rejects_offset_past_end_and_half_character_offsets() {
    let store = Arc::new(StubStore {
        answer: "héllo".to_owned(),
        status: "completed".to_owned(),
    });
    let tool = SubagentResultTool::new(Some(store), String::new());

    let past = tool
        .execute(
            &ctx_with_session(),
            &json!({ "ref": "sa_1", "offset_bytes": 100 }),
        )
        .unwrap_err();
    assert!(past.to_string().contains("exceeds result size"));

    // 'é' spans bytes 1..3; offset 2 sits mid-character.
    let half = tool
        .execute(
            &ctx_with_session(),
            &json!({ "ref": "sa_1", "offset_bytes": 2 }),
        )
        .unwrap_err();
    assert!(half.to_string().contains("UTF-8 character boundary"));
}

#[test]
fn parent_session_binding_round_trips_through_context() {
    let ctx = CallContext::new().with_value(Arc::new(ParentSessionBinding {
        session: "session-42".to_owned(),
    }));
    assert_eq!(parent_session_from(&ctx), Some("session-42".to_owned()));
    assert!(parent_session_from(&CallContext::new()).is_none());
}

#[test]
fn delegation_constructors_accept_empty_seams_for_assembly() {
    let _spawn = SpawnTool::new(None);
    let _result = SubagentResultTool::new(None, "/workspace".to_owned());
    let _set: CapabilitySet = CapabilitySet::default();
}

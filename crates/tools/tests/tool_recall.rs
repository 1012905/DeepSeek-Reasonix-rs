//! The `recall` tool at the tools boundary (Go `internal/tools/builtin/recall.go`,
//! docs/contracts/tools.md §1.15): both variants' schemas and descriptions,
//! the validation order, request passthrough to the bound recaller, and the
//! two renderers plus the position list.

use std::sync::{Arc, Mutex};

use reasonix_tools::abi::{CallContext, plan_mode_safe};
use reasonix_tools::builtin::{
    ContextRecaller, MAX_RECALL_POSITIONS, RECALL_DESCRIPTION_FULL, RECALL_DESCRIPTION_SEARCH_OFF,
    RECALL_TOOL_NAME, RecallHit, RecallRequest, RecallResult, position_list, recall_tool,
    recall_without_search, recaller_from, render_recall, render_recall_search,
};
use serde_json::{Value, json};

struct FakeRecaller {
    seen: Mutex<Vec<RecallRequest>>,
    result: RecallResult,
    fail: Option<String>,
}

impl FakeRecaller {
    fn new(result: RecallResult) -> Self {
        Self {
            seen: Mutex::new(Vec::new()),
            result,
            fail: None,
        }
    }
}

impl ContextRecaller for FakeRecaller {
    fn recall_context(
        &self,
        request: RecallRequest,
    ) -> Result<RecallResult, Box<dyn std::error::Error + Send + Sync>> {
        self.seen.lock().expect("recaller lock").push(request);
        match &self.fail {
            Some(err) => Err(Box::new(std::io::Error::other(err.clone()))),
            None => Ok(self.result.clone()),
        }
    }
}

fn ctx_with(recaller: Arc<FakeRecaller>) -> CallContext {
    let bound: Arc<Arc<dyn ContextRecaller>> = Arc::new(recaller as Arc<dyn ContextRecaller>);
    CallContext::new().with_value(bound)
}

fn sample_result() -> RecallResult {
    RecallResult {
        text: "folded: the logging decision".to_owned(),
        recalled: vec![3, 5],
        missing: vec![7],
        hits: vec![RecallHit {
            position: 9,
            kind: "tool".to_owned(),
            tool: "grep".to_owned(),
            snippet: "log helper".to_owned(),
        }],
        searched: 12,
        tokens: 40,
        budget_left: 60,
    }
}

#[test]
fn both_variants_share_the_name_but_differ_in_description_and_schema() {
    let full = recall_tool();
    let off = recall_without_search();
    assert_eq!(full.name(), RECALL_TOOL_NAME);
    assert_eq!(off.name(), RECALL_TOOL_NAME);

    assert_eq!(full.description(), RECALL_DESCRIPTION_FULL);
    assert_eq!(off.description(), RECALL_DESCRIPTION_SEARCH_OFF);
    assert!(
        full.description()
            .contains("or search the whole folded region")
    );
    assert!(
        off.description()
            .contains("addressed by the #n positions listed")
    );

    let full_schema = full.schema();
    let off_schema = off.schema();
    assert_eq!(off_schema["required"], json!(["positions"]));
    assert!(off_schema["properties"].get("query").is_none());
    assert!(off_schema["properties"].get("limit").is_none());
    assert!(full_schema["properties"].get("query").is_some());
    assert!(full_schema["properties"].get("limit").is_some());
    assert_eq!(full_schema["required"], Value::Null);
    assert_eq!(
        full_schema["properties"]["positions"]["maxItems"],
        MAX_RECALL_POSITIONS
    );
}

#[test]
fn both_variants_are_read_only_and_plan_mode_safe() {
    for tool in [recall_tool(), recall_without_search()] {
        assert!(tool.read_only());
        assert!(plan_mode_safe(tool.as_ref()));
    }
}

#[test]
fn the_search_off_variant_refuses_a_query_then_a_missing_address() {
    let off = recall_without_search();
    let err = off
        .execute(
            &CallContext::new(),
            &json!({"query": "logging", "positions": [3]}),
        )
        .unwrap_err();
    assert_eq!(
        err.to_string(),
        "recall: this configuration reads addresses only"
    );

    let err = off
        .execute(&CallContext::new(), &json!({"positions": []}))
        .unwrap_err();
    assert_eq!(
        err.to_string(),
        "recall: positions must name at least one #n address"
    );
}

#[test]
fn the_full_variant_needs_either_a_query_or_positions() {
    let full = recall_tool();
    let err = full.execute(&CallContext::new(), &json!({})).unwrap_err();
    assert_eq!(
        err.to_string(),
        "recall: give positions to read, or a query to search for them"
    );
}

#[test]
fn more_than_eight_positions_are_refused_whole() {
    let full = recall_tool();
    let err = full
        .execute(
            &CallContext::new(),
            &json!({"positions": [1, 2, 3, 4, 5, 6, 7, 8, 9]}),
        )
        .unwrap_err();
    assert_eq!(
        err.to_string(),
        format!("recall: at most {MAX_RECALL_POSITIONS} positions per call")
    );
}

#[test]
fn without_a_recaller_the_tool_is_unavailable_outside_an_active_session() {
    let err = recall_tool()
        .execute(&CallContext::new(), &json!({"positions": [3]}))
        .unwrap_err();
    assert_eq!(
        err.to_string(),
        "recall is unavailable outside an active agent session"
    );
}

#[test]
fn the_read_path_routes_positions_and_renders_the_accounting_line_last() {
    let recaller = Arc::new(FakeRecaller::new(sample_result()));
    let out = recall_tool()
        .execute(&ctx_with(recaller.clone()), &json!({"positions": [3, 5]}))
        .expect("recaller answers");

    let seen = recaller.seen.lock().expect("recaller lock");
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].positions, vec![3, 5]);
    assert_eq!(seen[0].query, "");
    assert_eq!(
        seen[0].limit, 0,
        "the tool layer does not fill the limit default"
    );

    assert_eq!(
        out,
        "folded: the logging decision\n\n[recalled #3, #5; 40 tokens; 60 left before the next compaction; no longer addressable: #7]"
    );
}

#[test]
fn the_search_path_routes_the_query_and_renders_the_search_footer() {
    let recaller = Arc::new(FakeRecaller::new(sample_result()));
    let out = recall_tool()
        .execute(
            &ctx_with(recaller.clone()),
            &json!({"query": "logging", "limit": 12}),
        )
        .expect("recaller answers");

    let seen = recaller.seen.lock().expect("recaller lock");
    assert_eq!(seen[0].query, "logging");
    assert_eq!(seen[0].positions, Vec::<i64>::new());
    assert_eq!(
        seen[0].limit, 12,
        "the host-normalized limit rides through raw"
    );

    assert_eq!(
        out,
        "folded: the logging decision\n\n[searched 12 folded messages; 40 tokens; 60 left before the next compaction; read them with positions]"
    );
}

#[test]
fn the_search_off_path_keeps_rendering_as_a_read() {
    let recaller = Arc::new(FakeRecaller::new(sample_result()));
    let out = recall_without_search()
        .execute(&ctx_with(recaller), &json!({"positions": [9]}))
        .expect("recaller answers");
    assert!(out.starts_with("folded: the logging decision"));
    assert!(out.contains("[recalled #3, #5;"));
    assert!(!out.contains("searched"));
}

#[test]
fn a_recaller_error_passes_through_unchanged() {
    let recaller = Arc::new(FakeRecaller {
        seen: Mutex::new(Vec::new()),
        result: RecallResult::default(),
        fail: Some("recall budget exhausted".to_owned()),
    });
    let err = recall_tool()
        .execute(&ctx_with(recaller), &json!({"positions": [1]}))
        .unwrap_err();
    assert!(err.to_string().contains("recall budget exhausted"));
}

#[test]
fn renderers_and_the_position_list_match_the_go_format() {
    let res = sample_result();
    assert_eq!(
        render_recall(res.clone()),
        "folded: the logging decision\n\n[recalled #3, #5; 40 tokens; 60 left before the next compaction; no longer addressable: #7]"
    );
    assert_eq!(
        render_recall_search(res),
        "folded: the logging decision\n\n[searched 12 folded messages; 40 tokens; 60 left before the next compaction; read them with positions]"
    );
    assert_eq!(position_list(&[3, 5]), "#3, #5");
    assert_eq!(position_list(&[]), "nothing");
}

#[test]
fn an_argument_type_mismatch_is_rejected_as_invalid_args() {
    let err = recall_tool()
        .execute(&CallContext::new(), &json!({"positions": "not an array"}))
        .unwrap_err();
    assert!(err.to_string().starts_with("invalid recall args:"));
}

#[test]
fn the_context_gate_reads_the_bound_recaller() {
    assert!(recaller_from(&CallContext::new()).is_none());
    let recaller = Arc::new(FakeRecaller::new(RecallResult::default()));
    assert!(recaller_from(&ctx_with(recaller)).is_some());
}

#[test]
fn registered_recall_tools_resolve_and_are_read_only_in_the_contract() {
    let registry = reasonix_tools::registry::Registry::new();
    registry.add(recall_tool());
    let entries = registry.contract_entries();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "recall");
    assert!(entries[0].read_only);
}

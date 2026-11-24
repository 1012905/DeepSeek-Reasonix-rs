//! TOOL-017 context fold + context budget (Go `internal/tools/builtin/compress.go`,
//! `contextbudget.go`, docs/contracts/tools.md §1.22, §1.36): the two
//! model-facing built-ins and their agent-supplied seams.

#![forbid(unsafe_code)]

use std::any::Any;
use std::sync::Arc;

use reasonix_tools::abi::{CallContext, Tool, runs_sequentially};
use reasonix_tools::builtin::{
    COMPRESS_TOOL_NAME, CONTEXT_BUDGET_TOOL_NAME, CompressRequest, CompressResult, CompressTool,
    ContextBudget, ContextBudgetReporter, ContextBudgetTool, ContextCompressor, STATUS_OK,
    STATUS_UNMEASURED, compress_tool, context_budget_tool,
};
use reasonix_tools::registry::Registry;
use serde_json::{Value, json};

struct StubCompressor {
    seen: std::sync::Mutex<Vec<CompressRequest>>,
    result: CompressResult,
}

impl ContextCompressor for StubCompressor {
    fn compress_context(
        &self,
        req: &CompressRequest,
    ) -> Result<CompressResult, Box<dyn std::error::Error + Send + Sync>> {
        self.seen.lock().expect("seen lock").push(req.clone());
        Ok(self.result.clone())
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

struct StubReporter {
    budget: ContextBudget,
}

impl ContextBudgetReporter for StubReporter {
    fn context_budget(&self) -> ContextBudget {
        self.budget.clone()
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn measured_budget() -> ContextBudget {
    ContextBudget {
        status: STATUS_OK.to_owned(),
        tokens_remaining: Some(300),
        tokens_used: Some(700),
        compact_at: Some(1000),
        window: Some(128_000),
        reason: None,
    }
}

// ---- compress ----------------------------------------------------------------

#[test]
fn compress_schema_name_description_and_flags_match_the_go_contract() {
    let t = CompressTool;
    assert_eq!(t.name(), COMPRESS_TOOL_NAME);
    assert_eq!(t.name(), "compress");
    assert!(t.description().starts_with("Compress a selected part"));
    assert!(t.read_only(), "the fold changes only the projection");
    let s = t.schema();
    assert_eq!(s["type"], "object");
    assert_eq!(s["required"], json!(["direction", "anchor"]));
    assert_eq!(
        s["properties"]["direction"]["enum"],
        json!(["before", "after"])
    );
    assert_eq!(s["properties"]["anchor"]["maxLength"], 512);
    assert_eq!(s["properties"]["focus"]["maxLength"], 2000);
    let ctx = CallContext::new();
    assert!(runs_sequentially(&t, &ctx, &json!({})));
}

#[test]
fn compress_forwards_the_trimmed_request_and_echoes_the_result_json() {
    let result = CompressResult {
        status: "ok".to_owned(),
        direction: "before".to_owned(),
        anchor: "unique".to_owned(),
        messages: 4,
        source_tokens: 900,
        projection_tokens: 200,
        mode: "summarized".to_owned(),
        reason: String::new(),
    };
    let stub = Arc::new(StubCompressor {
        seen: std::sync::Mutex::new(Vec::new()),
        result,
    });
    let port: Arc<dyn ContextCompressor> = stub.clone();
    let ctx = CallContext::new().with_value(Arc::new(port));
    let out = CompressTool
        .execute(
            &ctx,
            &json!({"direction": "before", "anchor": "  unique excerpt  ", "focus": " keep decisions "}),
        )
        .expect("compress executes");
    let seen = stub.seen.lock().expect("seen lock").clone();
    assert_eq!(seen[0].direction, "before");
    assert_eq!(seen[0].anchor, "unique excerpt");
    assert_eq!(seen[0].focus, "keep decisions");
    let wire: Value = serde_json::from_str(&out).expect("decode");
    assert_eq!(wire["status"], "ok");
    assert_eq!(wire["messages"], 4);
    assert_eq!(wire["source_tokens"], 900);
    assert_eq!(wire["projection_tokens"], 200);
    assert_eq!(wire["mode"], "summarized");
}

#[test]
fn compress_rejects_bad_args_direction_empty_anchor_and_byte_overruns() {
    let ctx = CallContext::new();
    let cases: Vec<(Value, &str)> = vec![
        (json!("not an object"), "invalid compress args"),
        (json!({"direction": "around", "anchor": "x"}), "direction"),
        (json!({"direction": "Before", "anchor": "x"}), "direction"),
        (json!({"direction": "before", "anchor": "  "}), "empty"),
        (
            json!({"direction": "before", "anchor": "a".repeat(513)}),
            "512 bytes",
        ),
        (
            json!({"direction": "before", "anchor": "x", "focus": "b".repeat(2001)}),
            "2000 bytes",
        ),
    ];
    for (args, want) in cases {
        let err = CompressTool.execute(&ctx, &args).unwrap_err();
        assert!(err.to_string().contains(want), "{err}");
    }
}

#[test]
fn compress_is_unavailable_outside_an_active_agent_session() {
    let err = CompressTool
        .execute(
            &CallContext::new(),
            &json!({"direction": "after", "anchor": "unique"}),
        )
        .unwrap_err();
    assert!(err.to_string().contains("active agent session"));
}

// ---- context_budget -----------------------------------------------------------

#[test]
fn context_budget_schema_name_and_flags_match_the_go_contract() {
    let t = ContextBudgetTool;
    assert_eq!(t.name(), CONTEXT_BUDGET_TOOL_NAME);
    assert_eq!(t.name(), "context_budget");
    assert!(
        t.description()
            .starts_with("Report how much context room is left")
    );
    assert!(t.read_only());
    let s = t.schema();
    assert_eq!(s["type"], "object");
    assert_eq!(s["properties"], json!({}));
}

#[test]
fn context_budget_routes_the_reporter_budget_to_the_model() {
    let reporter = Arc::new(StubReporter {
        budget: measured_budget(),
    });
    let port: Arc<dyn ContextBudgetReporter> = reporter;
    let ctx = CallContext::new().with_value(Arc::new(port));
    let out = ContextBudgetTool
        .execute(&ctx, &json!({}))
        .expect("budget reports");
    let wire: Value = serde_json::from_str(&out).expect("decode");
    assert_eq!(wire["status"], "ok");
    assert_eq!(wire["tokens_remaining"], 300);
    assert_eq!(wire["tokens_used"], 700);
    assert_eq!(wire["compact_at"], 1000);
    assert_eq!(wire["window"], 128_000);
}

#[test]
fn context_budget_unmeasured_omits_every_figure_and_is_unknown() {
    let budget = ContextBudget::unmeasured(
        "the active provider declares no context window, so compaction is disabled",
    );
    assert!(!budget.known());
    let reporter = Arc::new(StubReporter { budget });
    let port: Arc<dyn ContextBudgetReporter> = reporter;
    let ctx = CallContext::new().with_value(Arc::new(port));
    let out = ContextBudgetTool
        .execute(&ctx, &json!({}))
        .expect("budget reports");
    let wire: Value = serde_json::from_str(&out).expect("decode");
    assert_eq!(wire["status"], STATUS_UNMEASURED);
    assert!(wire.get("tokens_remaining").is_none());
    assert!(wire.get("window").is_none());
}

#[test]
fn context_budget_is_unavailable_without_a_reporter() {
    let err = ContextBudgetTool
        .execute(&CallContext::new(), &json!({}))
        .unwrap_err();
    assert!(err.to_string().contains("active agent session"));
}

#[test]
fn both_tools_add_to_a_registry_by_their_model_names() {
    let registry = Registry::new();
    registry.add(compress_tool());
    registry.add(context_budget_tool());
    let mut names = registry.names();
    names.sort();
    assert_eq!(
        names,
        vec!["compress".to_owned(), "context_budget".to_owned()]
    );
    for name in ["compress", "context_budget"] {
        assert!(matches!(
            registry.resolve_call(name),
            reasonix_tools::registry::ResolveCall::Exact(_)
        ));
    }
}

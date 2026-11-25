//! TOOL-035 misc tools at the tools boundary (Go `internal/tools/builtin/`
//! `present_files.go`, `systemone.go`; docs/contracts/tools.md §2.12).
//!
//! Covers the frozen card (name, verbatim description, schema shape, read-only
//! classification), the argument validation, the pure backend selection, and
//! the host seam the integration slice binds.

use std::sync::Arc;

use reasonix_tools::abi::{CallContext, Tool, plan_mode_safe};
use reasonix_tools::builtin::misc::{
    CODE_SYSTEM_ONE_NOT_CONFIGURED, LOCAL_BACKEND, MAX_PRESENT_FILES, PRESENT_FILES_DESCRIPTION,
    PRESENT_FILES_TOOL_NAME, PresentFilesHost, PresentFilesTool, PresentedFile,
    SYSTEM_ONE_TOOL_NAME, SystemOneError, SystemOneEvaluator, SystemOneSpec, SystemOneTool,
    present_files_tool, system_one_description,
};
use serde_json::{Value, json};

struct FakePresentHost;

impl PresentFilesHost for FakePresentHost {
    fn present(
        &self,
        path: &str,
    ) -> Result<PresentedFile, Box<dyn std::error::Error + Send + Sync>> {
        Ok(PresentedFile {
            path: path.to_owned(),
            root: String::new(),
            description: String::new(),
        })
    }
}

#[test]
fn present_files_card_matches_the_go_contract() {
    let tool = present_files_tool();
    assert_eq!(tool.name(), PRESENT_FILES_TOOL_NAME);
    assert_eq!(tool.name(), "present_files");
    assert_eq!(tool.description(), PRESENT_FILES_DESCRIPTION);
    assert!(tool.read_only());

    let schema = tool.schema();
    assert_eq!(schema["type"], "object");
    let files = &schema["properties"]["files"];
    assert_eq!(files["minItems"], 1);
    assert_eq!(files["maxItems"], MAX_PRESENT_FILES);
    let item = &files["items"];
    assert_eq!(item["properties"]["path"]["maxLength"], 4096);
    assert_eq!(item["properties"]["description"]["maxLength"], 256);
    assert!(
        item["required"]
            .as_array()
            .unwrap()
            .contains(&json!("path"))
    );
    assert_eq!(item["additionalProperties"], false);
    assert!(
        schema["required"]
            .as_array()
            .unwrap()
            .contains(&json!("files"))
    );
}

#[test]
fn present_files_rejects_empty_overlong_and_malformed_arguments() {
    let tool = PresentFilesTool::default();
    let ctx = CallContext::new();

    let err = tool
        .execute(&ctx, &json!({"files": []}))
        .expect_err("empty files is refused");
    assert!(err.to_string().contains("requires 1 to 4 files"), "{err}");

    let five = json!({"files": [
        {"path": "a.txt"}, {"path": "b.txt"}, {"path": "c.txt"},
        {"path": "d.txt"}, {"path": "e.txt"}
    ]});
    let err = tool
        .execute(&ctx, &five)
        .expect_err("five files is refused");
    assert!(err.to_string().contains("requires 1 to 4 files"), "{err}");

    let long_path = json!({"files": [{"path": "x".repeat(4097)}]});
    let err = tool.execute(&ctx, &long_path).expect_err("overlong path");
    assert!(err.to_string().contains("too long"), "{err}");

    let long_description = json!({"files": [{"path": "a.txt", "description": "y".repeat(1025)}]});
    let err = tool
        .execute(&ctx, &long_description)
        .expect_err("overlong description");
    assert!(err.to_string().contains("too long"), "{err}");

    let err = tool
        .execute(&ctx, &json!({"files": "not-an-array"}))
        .expect_err("malformed args");
    assert!(err.to_string().contains("invalid args"), "{err}");
}

#[test]
fn present_files_without_host_reports_the_unwired_seam() {
    let tool = PresentFilesTool::default();
    let err = tool
        .execute(&CallContext::new(), &json!({"files": [{"path": "a.txt"}]}))
        .expect_err("no host seam");
    assert!(err.to_string().contains("host seam is not wired"), "{err}");
}

#[test]
fn present_files_resolves_through_the_host_seam_and_renders_the_delivery_card() {
    let host = Arc::new(FakePresentHost);
    let tool = PresentFilesTool::new(Some(host));
    let out = tool
        .execute(
            &CallContext::new(),
            &json!({"files": [{"path": "docs/a.md", "description": "The report"}]}),
        )
        .expect("host resolves the card");
    let wire: Value = serde_json::from_str(&out).expect("delivery json");
    assert_eq!(wire["version"], 1);
    assert_eq!(wire["files"][0]["path"], "docs/a.md");
    assert_eq!(wire["files"][0]["description"], "The report");
}

#[test]
fn system_one_backends_order_endpoint_then_local() {
    let spec = SystemOneSpec {
        endpoint: Some("reviewer".to_owned()),
        local: true,
    };
    assert_eq!(spec.backends(), vec!["reviewer", LOCAL_BACKEND]);
    assert!(spec.configured());

    let empty = SystemOneSpec::default();
    assert!(empty.backends().is_empty());
    assert!(!empty.configured());
}

#[test]
fn system_one_card_carries_the_dynamic_parenthetical_and_enum() {
    let spec = SystemOneSpec {
        endpoint: Some("reviewer".to_owned()),
        local: true,
    };
    let tool = SystemOneTool::new(spec, None);
    assert_eq!(tool.name(), SYSTEM_ONE_TOOL_NAME);
    assert_eq!(tool.name(), "system_one");
    assert!(tool.read_only());
    assert!(plan_mode_safe(&tool));
    assert!(tool.description().contains("reviewer, laya-local"));
    assert!(
        tool.description()
            .starts_with("Evaluate state with the configured System One decision backend")
    );

    let schema = tool.schema();
    let backend_enum = schema["properties"]["backend"]["enum"]
        .as_array()
        .expect("enum array");
    assert!(backend_enum.contains(&json!("reviewer")));
    assert!(backend_enum.contains(&json!(LOCAL_BACKEND)));
    assert!(
        schema["required"]
            .as_array()
            .unwrap()
            .contains(&json!("state"))
    );
    assert!(
        schema["required"]
            .as_array()
            .unwrap()
            .contains(&json!("questions"))
    );
}

#[test]
fn system_one_description_matches_the_zero_spec_snapshot() {
    let spec = SystemOneSpec::default();
    let desc = system_one_description(&spec);
    assert_eq!(
        desc,
        "Evaluate state with the configured System One decision backend (). Ask typed Noul, Choice, or Score questions and receive complete probabilities, confidence, routing metadata, and usage."
    );
}

#[test]
fn system_one_backend_selection_falls_back_to_the_single_configured() {
    let single = SystemOneSpec {
        endpoint: Some("reviewer".to_owned()),
        local: false,
    };
    assert_eq!(single.select_backend("").unwrap(), "reviewer");
    assert_eq!(single.select_backend("reviewer").unwrap(), "reviewer");

    let multi = SystemOneSpec {
        endpoint: Some("reviewer".to_owned()),
        local: true,
    };
    let err = multi
        .select_backend("")
        .expect_err("two backends need a choice");
    match err {
        SystemOneError::BackendRequired { available } => {
            assert_eq!(available, "reviewer, laya-local");
        }
        other => panic!("unexpected error: {other}"),
    }
    let err = multi
        .select_backend("ghost")
        .expect_err("unknown backend refused");
    match err {
        SystemOneError::BackendNotConfigured {
            requested,
            available,
        } => {
            assert_eq!(requested, "ghost");
            assert_eq!(available, "reviewer, laya-local");
        }
        other => panic!("unexpected error: {other}"),
    }
}

#[test]
fn system_one_unavailable_carries_the_decision_not_configured_refusal() {
    let tool = SystemOneTool::new(SystemOneSpec::default(), None);
    let err = tool
        .execute(&CallContext::new(), &json!({"state": {}, "questions": {}}))
        .expect_err("not configured");
    let refusal = err
        .downcast_ref::<reasonix_tools::refusal::Refusal>()
        .expect("typed refusal");
    assert_eq!(refusal.code, CODE_SYSTEM_ONE_NOT_CONFIGURED);
    assert_eq!(refusal.message, "no model is assigned to the decision role");
}

struct StubEvaluator {
    called: std::sync::Mutex<Option<String>>,
}

impl SystemOneEvaluator for StubEvaluator {
    fn evaluate(
        &self,
        backend: &str,
        _state: &Value,
        _questions: &Value,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        *self.called.lock().unwrap() = Some(backend.to_owned());
        Ok(json!({"backend": backend, "probabilities": {}}).to_string())
    }
}

#[test]
fn system_one_forwards_the_selected_backend_to_the_evaluator_seam() {
    let spec = SystemOneSpec {
        endpoint: Some("reviewer".to_owned()),
        local: false,
    };
    let evaluator = Arc::new(StubEvaluator {
        called: std::sync::Mutex::new(None),
    });
    let tool = SystemOneTool::new(spec, Some(evaluator.clone()));
    let out = tool
        .execute(
            &CallContext::new(),
            &json!({"backend": "reviewer", "state": "x", "questions": {}}),
        )
        .expect("evaluator answers");
    let wire: Value = serde_json::from_str(&out).expect("answer json");
    assert_eq!(wire["backend"], "reviewer");
    assert_eq!(
        *evaluator.called.lock().unwrap(),
        Some("reviewer".to_owned())
    );
}

#[test]
fn system_one_without_evaluator_reports_the_unwired_seam() {
    let spec = SystemOneSpec {
        endpoint: Some("reviewer".to_owned()),
        local: false,
    };
    let tool = SystemOneTool::new(spec, None);
    let err = tool
        .execute(&CallContext::new(), &json!({"state": "x", "questions": {}}))
        .expect_err("no evaluator seam");
    assert!(
        err.to_string().contains("evaluator seam is not wired"),
        "{err}"
    );
}

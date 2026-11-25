//! TOOL-041 extension runtime tools (docs/contracts/tools.md §3.6; Go
//! `internal/assembly/boot/extension_tools.go`, `extension_discovery_tool.go`).
//!
//! Covers the `ext__` name qualification, the normalized-JSON contract digest
//! recheck, the over-long offer filter, the discovery-tool name minting, and
//! the card/seam routing: serve check, contract recheck, reported error, and
//! the unwired seam failing closed.

use std::sync::{Arc, Mutex};

use reasonix_tools::abi::{CallContext, Tool};
use reasonix_tools::builtin::extension::{
    CONNECT_EXTENSION_PREFIX, ExtensionCall, ExtensionDiscoveryTool, ExtensionError, ExtensionHost,
    ExtensionPackageMetadata, ExtensionTool, MAX_TOOL_NAME_LEN, MintedExtension, RuntimeTool,
    contract_matches, decl_digest, digest, discovery_description, discovery_tool_name,
    extension_tool_name, mint_declared, name_offered, overlong_warning,
};
use serde_json::{Value, json};

fn decl(name: &str, description: &str, read_only: bool) -> RuntimeTool {
    RuntimeTool {
        name: name.to_owned(),
        description: description.to_owned(),
        input_schema: json!({"type": "object"}),
        read_only,
    }
}

#[test]
fn extension_tool_name_qualifies_plugin_with_namespace() {
    assert_eq!(
        extension_tool_name("acme.widget", "search"),
        "ext__acme_widget__search"
    );
    assert_eq!(extension_tool_name("plain", "run"), "ext__plain__run");
}

#[test]
fn digest_canonicalizes_key_order_and_is_stable() {
    let a = json!({"b": 1, "a": {"y": 2, "x": 3}, "c": "z"});
    let b = json!({"a": {"x": 3, "y": 2}, "b": 1, "c": "z"});
    assert_eq!(digest(&a), digest(&b));
    assert!(!digest(&a).is_empty());
    let changed = json!({"b": 1, "a": {"y": 2, "x": 3}, "c": "zz"});
    assert_ne!(digest(&a), digest(&changed));
}

#[test]
fn decl_digest_matches_only_when_contract_unchanged() {
    let registered = decl("search", "Search the web.", true);
    let same = decl("search", "Search the web.", true);
    assert!(contract_matches(&same, &registered));
    assert_eq!(decl_digest(&registered), decl_digest(&same));

    let edited_description = decl("search", "Changed wording.", true);
    assert!(!contract_matches(&edited_description, &registered));

    let edited_readonly = decl("search", "Search the web.", false);
    assert!(!contract_matches(&edited_readonly, &registered));
}

#[test]
fn discovery_name_prefixes_slug_and_eight_char_digest() {
    let name = discovery_tool_name("acme.widget");
    assert!(name.starts_with(CONNECT_EXTENSION_PREFIX));
    assert!(name.contains("acme_widget"));
    // The suffix is the first 8 digest chars of the plugin id.
    let digest_suffix = digest(&Value::String("acme.widget".to_owned()))
        .chars()
        .take(8)
        .collect::<String>();
    assert!(name.ends_with(&format!("_{digest_suffix}")));
    assert_eq!(name, discovery_tool_name("acme.widget"));
    assert_eq!(
        discovery_description("acme.widget"),
        "Connect extension acme.widget and list its available tools."
    );
}

#[test]
fn over_long_names_are_not_offered_and_warned() {
    assert!(name_offered("ext__short__run"));
    let long_tail = "x".repeat(MAX_TOOL_NAME_LEN);
    let long_name = extension_tool_name("p", &long_tail);
    assert!(!name_offered(&long_name));
    let warning = overlong_warning("p", &long_tail, &long_name);
    assert!(warning.contains("over the 64 characters"));

    let minted: Vec<MintedExtension> = mint_declared(
        "p",
        &[
            decl("ok", "fine", false),
            decl(&long_tail, "too long", false),
        ],
    );
    assert_eq!(minted.len(), 1);
    assert_eq!(minted[0].decl.name, "ok");
}

struct FakeHost {
    serves: bool,
    declared: Vec<RuntimeTool>,
    calls: Mutex<Vec<Value>>,
    result: ExtensionCall,
}

impl ExtensionHost for FakeHost {
    fn serves_tool(&self, _plugin_id: &str, tool_name: &str) -> bool {
        self.serves && self.declared.iter().any(|d| d.name == tool_name)
    }

    fn declared_tools(&self, _plugin_id: &str) -> Vec<RuntimeTool> {
        self.declared.clone()
    }

    fn call_tool(
        &self,
        _plugin_id: &str,
        _tool_name: &str,
        args: &Value,
    ) -> Result<ExtensionCall, Box<dyn std::error::Error + Send + Sync>> {
        self.calls.lock().expect("calls lock").push(args.clone());
        Ok(self.result.clone())
    }

    fn ensure_ready(
        &self,
        _plugin_id: &str,
    ) -> Result<Vec<RuntimeTool>, Box<dyn std::error::Error + Send + Sync>> {
        Ok(self.declared.clone())
    }
}

fn fake_host(declared: Vec<RuntimeTool>, result: ExtensionCall) -> Arc<dyn ExtensionHost> {
    Arc::new(FakeHost {
        serves: true,
        declared,
        calls: Mutex::new(Vec::new()),
        result,
    })
}

#[test]
fn extension_tool_routes_call_through_host_when_contract_matches() {
    let registered = decl("search", "Search the web.", true);
    let host = fake_host(
        vec![registered.clone()],
        ExtensionCall {
            is_error: false,
            content: "three hits".to_owned(),
        },
    );
    let tool = ExtensionTool::new(
        extension_tool_name("acme", "search"),
        "acme".to_owned(),
        registered,
        Some(host),
    );
    let out = tool
        .execute(&CallContext::new(), &json!({"query": "rust"}))
        .expect("call succeeds");
    assert_eq!(out, "three hits");
}

#[test]
fn extension_tool_fails_when_runtime_no_longer_serves() {
    let registered = decl("search", "Search the web.", true);
    let host = fake_host(vec![], ExtensionCall::default());
    let tool = ExtensionTool::new(
        extension_tool_name("acme", "search"),
        "acme".to_owned(),
        registered,
        Some(host),
    );
    let err = tool
        .execute(&CallContext::new(), &json!({}))
        .expect_err("no longer serves");
    assert!(matches!(
        err.downcast_ref::<ExtensionError>(),
        Some(ExtensionError::NoLongerServes { .. })
    ));
}

#[test]
fn extension_tool_rejects_changed_contract_before_dispatch() {
    let registered = decl("search", "Search the web.", true);
    let current = decl("search", "Edited wording.", true);
    let host = fake_host(vec![current], ExtensionCall::default());
    let tool = ExtensionTool::new(
        extension_tool_name("acme", "search"),
        "acme".to_owned(),
        registered,
        Some(host),
    );
    let err = tool
        .execute(&CallContext::new(), &json!({}))
        .expect_err("contract changed");
    assert!(matches!(
        err.downcast_ref::<ExtensionError>(),
        Some(ExtensionError::ContractChanged { .. })
    ));
}

#[test]
fn extension_tool_wraps_reported_sidecar_error() {
    let registered = decl("search", "Search the web.", true);
    let host = fake_host(
        vec![registered.clone()],
        ExtensionCall {
            is_error: true,
            content: "boom".to_owned(),
        },
    );
    let tool = ExtensionTool::new(
        extension_tool_name("acme", "search"),
        "acme".to_owned(),
        registered,
        Some(host),
    );
    let err = tool
        .execute(&CallContext::new(), &json!({}))
        .expect_err("reported error");
    match err.downcast_ref::<ExtensionError>() {
        Some(ExtensionError::CallReportedError { content, .. }) => assert_eq!(content, "boom"),
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn unwired_extension_host_fails_closed() {
    let registered = decl("search", "Search the web.", true);
    let tool = ExtensionTool::new(
        extension_tool_name("acme", "search"),
        "acme".to_owned(),
        registered,
        None,
    );
    let err = tool
        .execute(&CallContext::new(), &json!({}))
        .expect_err("seam unwired");
    assert!(matches!(
        err.downcast_ref::<ExtensionError>(),
        Some(ExtensionError::SeamUnwired)
    ));
}

#[test]
fn discovery_tool_exposes_declared_tools_and_package_metadata() {
    let declared = vec![decl("search", "Search.", true), decl("run", "Run.", false)];
    let host = fake_host(declared.clone(), ExtensionCall::default());
    let tool = ExtensionDiscoveryTool::new("acme.widget".to_owned(), Some(host));
    assert!(tool.name().starts_with(CONNECT_EXTENSION_PREFIX));
    assert!(!tool.read_only());

    let out = tool
        .execute(&CallContext::new(), &json!({}))
        .expect("ensure ready");
    let parsed: Value = serde_json::from_str(&out).expect("declared tools JSON");
    let names = parsed
        .as_array()
        .expect("array")
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(names, vec!["search", "run"]);

    let meta = tool
        .capabilities()
        .get::<dyn ExtensionPackageMetadata>()
        .expect("package metadata capability");
    assert_eq!(meta.extension_package_name(), "acme.widget");
}

#[test]
fn discovery_tool_without_host_fails_closed() {
    let tool = ExtensionDiscoveryTool::new("acme.widget".to_owned(), None);
    let err = tool
        .execute(&CallContext::new(), &json!({}))
        .expect_err("seam unwired");
    assert!(matches!(
        err.downcast_ref::<ExtensionError>(),
        Some(ExtensionError::SeamUnwired)
    ));
}

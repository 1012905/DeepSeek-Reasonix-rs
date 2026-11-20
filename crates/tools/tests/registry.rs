//! Registry oracle (Go `registry_test.go`, `registry_canon_test.go`,
//! `provider_surface_test.go`).

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use reasonix_tools::abi::{CallContext, CapabilitySet, Tool, ToolResult};
use reasonix_tools::{MCP_NAME_PREFIX, Registry};
use serde_json::{Value, json};

/// Minimal tool for registry tests (Go `stubTool`), with optional MCP
/// identity through the capability set.
#[derive(Default)]
struct StubTool {
    name: String,
    schema: Option<Value>,
    server: String,
    raw: String,
    visible: String,
    pkg: String,
    schema_calls: AtomicUsize,
}

impl StubTool {
    fn plain(name: &str) -> Arc<Self> {
        Arc::new(Self {
            name: name.to_owned(),
            ..Default::default()
        })
    }

    fn mcp(name: &str, server: &str, raw: &str, visible: &str, pkg: &str) -> Arc<Self> {
        Arc::new(Self {
            name: name.to_owned(),
            server: server.to_owned(),
            raw: raw.to_owned(),
            visible: visible.to_owned(),
            pkg: pkg.to_owned(),
            ..Default::default()
        })
    }
}

impl Tool for StubTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        "stub desc"
    }

    fn schema(&self) -> Value {
        self.schema_calls.fetch_add(1, Ordering::SeqCst);
        self.schema
            .clone()
            .unwrap_or_else(|| json!({"type":"object"}))
    }

    fn execute(&self, _ctx: &CallContext, _args: &Value) -> ToolResult {
        Ok(String::new())
    }

    fn read_only(&self) -> bool {
        true
    }

    fn capabilities(&self) -> CapabilitySet {
        let mut caps = CapabilitySet::default();
        if !self.server.is_empty() {
            let meta: Arc<dyn reasonix_tools::registry::McpMetadata> = Arc::new(McpMeta {
                server: self.server.clone(),
                raw: self.raw.clone(),
            });
            caps = caps.with("mcp-metadata", meta);
        }
        if !self.visible.is_empty() {
            let visible: Arc<dyn reasonix_tools::registry::McpVisibleMetadata> =
                Arc::new(McpVisible(self.visible.clone()));
            caps = caps.with("mcp-visible", visible);
        }
        if !self.pkg.is_empty() {
            let package: Arc<dyn reasonix_tools::registry::McpPackageMetadata> =
                Arc::new(McpPackage(self.pkg.clone()));
            caps = caps.with("mcp-package", package);
        }
        caps
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// MCP identity passed through the capability set (Go: stubTool's
/// MCPServerName/MCPRawToolName methods).
struct McpMeta {
    server: String,
    raw: String,
}

impl reasonix_tools::abi::Capability for McpMeta {}

impl reasonix_tools::registry::McpMetadata for McpMeta {
    fn mcp_server_name(&self) -> String {
        self.server.clone()
    }
    fn mcp_raw_tool_name(&self) -> String {
        self.raw.clone()
    }
}

struct McpVisible(String);

impl reasonix_tools::abi::Capability for McpVisible {}

impl reasonix_tools::registry::McpVisibleMetadata for McpVisible {
    fn mcp_visible_tool_name(&self) -> String {
        self.0.clone()
    }
}

struct McpPackage(String);

impl reasonix_tools::abi::Capability for McpPackage {}

impl reasonix_tools::registry::McpPackageMetadata for McpPackage {
    fn mcp_package_name(&self) -> String {
        self.0.clone()
    }
}

fn names_of(schemas: &[reasonix_tools::ToolSchema]) -> Vec<String> {
    schemas.iter().map(|s| s.name.clone()).collect()
}

#[test]
fn registry_resolves_portable_mcp_references_only_when_unique() {
    let r = Registry::new();
    let first = StubTool::mcp(
        "mcp__figma__get_design_context",
        "figma",
        "figma_get_design_context",
        "get_design_context",
        "figma",
    );
    r.add(first.clone());

    for reference in [
        "get_design_context",
        "figma_get_design_context",
        "figma/get_design_context",
        "mcp-tool:figma/figma_get_design_context",
        "mcp__plugin_figma_figma__get_design_context",
    ] {
        match r.resolve_call(reference) {
            reasonix_tools::ResolveCall::Exact(t) => assert_eq!(t.name(), first.name()),
            other => panic!(
                "ResolveCall({reference}) = {other:?}, want {}",
                first.name()
            ),
        }
    }

    // Exact registered names always win, even if they are also another MCP
    // tool's short/raw alias.
    r.add(StubTool::plain("get_design_context"));
    match r.resolve_call("get_design_context") {
        reasonix_tools::ResolveCall::Exact(t) => assert_eq!(t.name(), "get_design_context"),
        other => panic!("exact name did not win: {other:?}"),
    }

    r.add(StubTool::mcp(
        "mcp__other__get_design_context",
        "other",
        "get_design_context",
        "get_design_context",
        "",
    ));
    // A distinct raw name stays unambiguous, and the exact registered name
    // still suppresses alias ambiguity (Go asserts only that both resolve).
    match r.resolve_call("figma_get_design_context") {
        reasonix_tools::ResolveCall::Exact(_) => {}
        other => panic!("figma_get_design_context did not resolve: {other:?}"),
    }
    match r.resolve_call("get_design_context") {
        reasonix_tools::ResolveCall::Exact(t) => assert_eq!(t.name(), "get_design_context"),
        other => panic!("exact registered name should win: {other:?}"),
    }
    r.remove_prefix("get_design_context");
    match r.resolve_call("get_design_context") {
        reasonix_tools::ResolveCall::Ambiguous(c) => {
            assert_eq!(c.len(), 2, "want two candidates: {c:?}")
        }
        other => panic!("ambiguous short reference = {other:?}, want two candidates"),
    }
}

#[test]
fn registry_portable_aliases_do_not_change_provider_schemas() {
    let r = Registry::new();
    r.add(StubTool::mcp(
        "mcp__my_server_deadbeef__do_thing_deadbeef",
        "my.server",
        "do.thing",
        "do.thing",
        "",
    ));
    let before = r.schemas();
    match r.resolve_call("mcp__my_server__do_thing") {
        reasonix_tools::ResolveCall::Exact(_) => {}
        other => panic!("portable normalized reference did not resolve: {other:?}"),
    }
    let after = r.schemas();
    assert_eq!(before.len(), 1);
    assert_eq!(after.len(), 1);
    assert_eq!(before[0].name, after[0].name);
}

/// An MCP server's namespaced tools drop as a group on disconnect, leaving
/// built-ins and other servers' tools — and their insertion order — intact.
#[test]
fn registry_remove_prefix() {
    let r = Registry::new();
    r.add(StubTool::plain("bash"));
    r.add(StubTool::plain("mcp__fs__read"));
    r.add(StubTool::plain("mcp__fs__write"));
    r.add(StubTool::plain("mcp__stripe__charge"));

    assert_eq!(r.remove_prefix("mcp__fs__"), 2);
    assert_eq!(r.len(), 2);
    assert!(r.get("mcp__fs__read").is_none());
    assert!(r.get("mcp__stripe__charge").is_some());
    assert_eq!(r.names(), vec!["bash", "mcp__stripe__charge"]);

    // Removing a prefix that matches nothing is a no-op.
    assert_eq!(r.remove_prefix("mcp__nope__"), 0);
}

#[test]
fn registry_suspend_prefix_blocks_late_adds_until_resume() {
    let r = Registry::new();
    r.add(StubTool::plain("bash"));
    r.add(StubTool::plain("mcp__fs__connect"));

    assert_eq!(r.suspend_prefix("mcp__fs__"), 1);
    r.add(StubTool::plain("mcp__fs__read"));
    assert!(
        r.get("mcp__fs__read").is_none(),
        "suspended prefix accepted a late tool add; names={:?}",
        r.names()
    );
    assert!(
        r.get("bash").is_some(),
        "suspending an MCP prefix removed unrelated tools"
    );

    r.resume_prefix("mcp__fs__");
    r.add(StubTool::plain("mcp__fs__read"));
    assert!(
        r.get("mcp__fs__read").is_some(),
        "resumed prefix did not accept tool add"
    );
}

#[test]
fn registry_schema_revision_tracks_visible_changes() {
    let r = Registry::new();
    let initial = r.schema_revision();
    r.add(StubTool::plain("mcp__fs__read"));
    let after_add = r.schema_revision();
    assert!(after_add > initial);

    r.add(Arc::new(StubTool {
        name: "mcp__fs__read".to_owned(),
        schema: Some(json!({"type":"string"})),
        ..Default::default()
    }));
    let after_replace = r.schema_revision();
    assert!(after_replace > after_add);

    r.set_provider_visible_tools(Some(&["mcp__fs__read"]));
    let after_visibility = r.schema_revision();
    assert!(after_visibility > after_replace);

    r.set_provider_visible_tools(Some(&["mcp__fs__read"]));
    assert_eq!(
        r.schema_revision(),
        after_visibility,
        "no-op visibility update changed revision"
    );

    assert_eq!(r.remove_prefix("missing"), 0);
    assert_eq!(
        r.schema_revision(),
        after_visibility,
        "no-op removal changed revision"
    );

    assert_eq!(r.suspend_prefix("mcp__fs__"), 1);
    assert!(
        r.schema_revision() > after_visibility,
        "suspension did not advance revision"
    );
}

/// Schemas emit in deterministic alphabetical order regardless of insertion
/// order, so a logically identical tool set produces a stable provider
/// request prefix. Names() stays in insertion order.
#[test]
fn registry_schemas_sorted() {
    let r = Registry::new();
    let insertion = ["write", "bash", "read", "apply_patch"];
    for n in insertion {
        r.add(StubTool::plain(n));
    }

    let got = names_of(&r.schemas());
    assert_eq!(got, vec!["apply_patch", "bash", "read", "write"]);

    assert_eq!(
        r.names().iter().map(String::as_str).collect::<Vec<_>>(),
        insertion
    );
}

#[test]
fn registry_schemas_stable_and_canonical() {
    let r = Registry::new();
    r.add(Arc::new(StubTool {
        name: "zeta".to_owned(),
        schema: Some(json!({"type":"object","required":["b","a"],"properties":{"b":{"type":"string"},"a":{"type":"string"}}})),
        ..Default::default()
    }));
    r.add(Arc::new(StubTool {
        name: "alpha".to_owned(),
        schema: Some(json!({"required":["y","x"],"type":"object"})),
        ..Default::default()
    }));

    let schemas = r.schemas();
    assert_eq!(schemas.len(), 2);
    assert_eq!(schemas[0].name, "alpha");
    assert_eq!(schemas[1].name, "zeta");
    assert_eq!(
        schemas[0].parameters,
        json!({"properties":{},"required":["x","y"],"type":"object"})
    );
    assert_eq!(
        schemas[1].parameters,
        json!({"properties":{"a":{"type":"string"},"b":{"type":"string"}},"required":["a","b"],"type":"object"})
    );
}

#[test]
fn registry_schemas_canonicalizes_equivalent_ordering() {
    let first = Registry::new();
    first.add(Arc::new(StubTool {
        name: "same".to_owned(),
        schema: Some(json!({"type":"object","required":["b","a"],"properties":{"b":{"description":"bee","type":"string"},"a":{"type":"integer"}}})),
        ..Default::default()
    }));
    let second = Registry::new();
    second.add(Arc::new(StubTool {
        name: "same".to_owned(),
        schema: Some(json!({"properties":{"a":{"type":"integer"},"b":{"type":"string","description":"bee"}},"required":["a","b"],"type":"object"})),
        ..Default::default()
    }));

    let a = &first.schemas()[0].parameters;
    let b = &second.schemas()[0].parameters;
    assert_eq!(a, b, "equivalent schemas canonicalized differently");
}

/// Schema() must run exactly once per tool, at Add, no matter how many
/// times Schemas() is called — Schemas() runs every turn.
#[test]
fn schemas_canonicalizes_once_per_tool() {
    let r = Registry::new();
    let tool = Arc::new(StubTool {
        name: "alpha".to_owned(),
        schema: Some(
            json!({"type":"object","properties":{"b":{"type":"string"},"a":{"type":"string"}}}),
        ),
        ..Default::default()
    });
    r.add(tool.clone());
    assert_eq!(
        tool.schema_calls.load(Ordering::SeqCst),
        1,
        "Schema() called more than once at Add"
    );

    for _ in 0..50 {
        let schemas = r.schemas();
        assert_eq!(schemas.len(), 1);
        let got = schemas[0].parameters.to_string();
        let (ai, bi) = (got.find("\"a\""), got.find("\"b\""));
        assert!(
            ai.is_some() && bi.is_some() && ai < bi,
            "schema not canonicalized (keys unsorted): {got}"
        );
    }
    assert_eq!(
        tool.schema_calls.load(Ordering::SeqCst),
        1,
        "Schema() called after 50 Schemas() calls (caching regressed)"
    );
}

#[test]
fn split_mcp_name_and_connection_names() {
    assert_eq!(
        split_mcp_name("mcp__srv__tool").unwrap(),
        ("srv".to_owned(), "tool".to_owned())
    );
    assert!(split_mcp_name("bash").is_none());
    assert!(split_mcp_name("mcp__srv").is_none());
    assert!(split_mcp_name("mcp____tool").is_none());
    assert!(is_connection_name("mcp__srv__tool"));
    assert!(is_connection_name("ext__thing"));
    assert!(!is_connection_name("bash"));
    assert_eq!(MCP_NAME_PREFIX, "mcp__");
}

/// Re-export of the split helpers used by the tests above.
use reasonix_tools::registry::{is_connection_name, split_mcp_name};

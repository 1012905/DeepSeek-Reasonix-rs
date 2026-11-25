//! TOOL-043 assembly-conditional surface (docs/contracts/tools.md §3.8; Go
//! `internal/tools/websearch/search.go`, `internal/tools/fetchchain/chain.go`,
//! `internal/ext/command/slashtool.go`, `internal/ext/plugin/toolname.go`,
//! `internal/ext/installsource/install_source.go`).
//!
//! Covers MCP connect/model name identity, web_search validation and provider
//! seam, the fetchchain fallback order and refusals, slash-command catalog
//! building/listing/expansion, install_source argument normalization and the
//! unticketed-apply gate, plus the conditional-surface anchors resolving
//! against the frozen sixteen-step table.

use std::sync::Arc;

use reasonix_tools::abi::{CallContext, Tool};
use reasonix_tools::assembly::resolve_conditional_anchors;
use reasonix_tools::builtin::conditional::{
    BilledSink, CODE_FETCH_ALL_FAILED, CODE_FETCH_PAID_NEEDS_CONSENT, FallbackSink, FetchBackend,
    FetchChain, FetchConsent, FetchFallback, FetchPage, FetchVet, MCP_CONNECT_PREFIX, SearchResult,
    WEB_SEARCH_QUERY_MAX, WEB_SEARCH_TOOL_NAME, WebSearchProvider, WebSearchTool,
    conditional_surfaces, mcp_connect_permission_name, mcp_model_tool_name, mcp_tool_prefix,
    normalize_name, render_page, validate_web_query,
};
use reasonix_tools::builtin::conditional_slash::{
    INSTALL_SOURCE_TOOL_NAME, InstallApprover, InstallConnector, InstallError, InstallReceipt,
    InstallRequest, InstallSourceTool, NormalizedRequest, SlashCatalog, SlashCommandTool,
    SlashEntry, SlashGate, SlashRenderer, SlashVisibility, expand_frame, list_slash_commands,
    normalize_install_op, normalize_kind, normalize_mode, normalize_scope, normalize_slash_name,
    normalize_tier, normalize_transport, unticketed_apply, validate_install_request,
};
use reasonix_tools::refusal::Refusal;
use serde_json::json;

// --- MCP connect identity ----------------------------------------------------

#[test]
fn normalize_name_sanitizes_spelling_and_keeps_collision_hash() {
    assert_eq!(normalize_name("already_good"), "already_good");
    assert!(normalize_name("").starts_with("unnamed_"));
    let dirty = normalize_name("Acme DB!");
    assert!(dirty.starts_with("Acme_DB_"));
    // Two spellings that sanitize to the same bare form keep distinct hashes.
    let a = normalize_name("foo bar");
    let b = normalize_name("foo!bar");
    assert_ne!(a, b);
}

#[test]
fn mcp_prefix_and_connect_permission_name_are_namespaced() {
    assert_eq!(
        mcp_tool_prefix("acme db"),
        "mcp__acme_db_".to_owned() + &mcp_hash("acme db") + "__"
    );
    let connect = mcp_connect_permission_name("acme db");
    assert!(connect.starts_with(MCP_CONNECT_PREFIX));
    assert!(connect.contains("acme_db"));
}

fn mcp_hash(raw: &str) -> String {
    // Mirror the six-char fnv suffix for the prefix assertion.
    normalize_name(raw)
        .rsplit('_')
        .next()
        .unwrap_or_default()
        .to_owned()
}

#[test]
fn model_tool_name_bounds_to_128_chars_with_hash_suffix() {
    let long_raw = "tool".repeat(40);
    let name = mcp_model_tool_name("server", &long_raw);
    assert!(name.len() <= 128, "name too long: {}", name.len());
    assert!(name.starts_with("mcp__"));
    let short = mcp_model_tool_name("server", "search");
    assert!(short.len() <= 128);
}

// --- web_search ---------------------------------------------------------------

#[test]
fn web_query_must_be_present_and_within_byte_limit() {
    assert!(validate_web_query("").is_err());
    assert!(validate_web_query("   ").is_err());
    let long = "y".repeat(WEB_SEARCH_QUERY_MAX + 1);
    assert!(validate_web_query(&long).is_err());
    assert!(validate_web_query("rust lifetimes").is_ok());
}

struct FakeSearch {
    result: SearchResult,
}

impl WebSearchProvider for FakeSearch {
    fn search(&self, _: &str) -> Result<SearchResult, Box<dyn std::error::Error + Send + Sync>> {
        Ok(self.result.clone())
    }
}

#[test]
fn web_search_without_provider_fails_closed() {
    let tool = WebSearchTool::new(None);
    assert_eq!(tool.name(), WEB_SEARCH_TOOL_NAME);
    assert!(tool.read_only());
    let err = tool
        .execute(&CallContext::new(), &json!({"query": "rust"}))
        .expect_err("unavailable");
    assert!(err.to_string().contains("web search provider unavailable"));
}

#[test]
fn web_search_round_trips_provider_result() {
    let provider = Arc::new(FakeSearch {
        result: SearchResult {
            summary: "three hits".to_owned(),
            sources: "[1] example.com".to_owned(),
            truncated: false,
        },
    });
    let tool = WebSearchTool::new(Some(provider));
    let out = tool
        .execute(&CallContext::new(), &json!({"query": "rust lifetimes"}))
        .expect("result");
    let parsed: SearchResult = serde_json::from_str(&out).expect("search result JSON");
    assert_eq!(parsed.summary, "three hits");
    assert_eq!(parsed.sources, "[1] example.com");
}

// --- fetchchain ---------------------------------------------------------------

struct FakeBackend {
    name: String,
    paid: bool,
    page: Option<FetchPage>,
    fail: Option<String>,
}

impl FetchBackend for FakeBackend {
    fn name(&self) -> &str {
        &self.name
    }
    fn paid(&self) -> bool {
        self.paid
    }
    fn fetch(&self, _: &str) -> Result<FetchPage, Box<dyn std::error::Error + Send + Sync>> {
        if let Some(fail) = &self.fail {
            return Err(Box::new(std::io::Error::other(fail.clone())));
        }
        Ok(self.page.clone().unwrap())
    }
}

struct YesConsent;
impl FetchConsent for YesConsent {
    fn consent(&self, _: &str, _: &str) -> bool {
        true
    }
}

struct NoConsent;
impl FetchConsent for NoConsent {
    fn consent(&self, _: &str, _: &str) -> bool {
        false
    }
}

struct SilentVet;
impl FetchVet for SilentVet {
    fn vet(&self, _: &str) -> Result<(), Refusal> {
        Ok(())
    }
}

struct NoopSink;
impl FallbackSink for NoopSink {
    fn on_fallback(&self, _: FetchFallback) {}
}
impl BilledSink for NoopSink {
    fn on_billed(&self, _: &str) {}
}

fn page(title: &str, text: &str) -> FetchPage {
    FetchPage {
        title: title.to_owned(),
        text: text.to_owned(),
        status: 200,
        rendered: String::new(),
    }
}

#[test]
fn free_backend_answers_without_billed_marker() {
    let chain = FetchChain::new(
        vec![Arc::new(FakeBackend {
            name: "local".to_owned(),
            paid: false,
            page: Some(page("Home", "body")),
            fail: None,
        })],
        None,
        None,
        None,
        None,
    );
    let out = chain.run("https://example.com").expect("free answered");
    assert!(out.contains("[fetched by local]"));
    assert!(!out.contains("billed"));
}

#[test]
fn paid_backend_runs_after_free_fails_and_marks_billed() {
    let chain = FetchChain::new(
        vec![
            Arc::new(FakeBackend {
                name: "local".to_owned(),
                paid: false,
                page: None,
                fail: Some("transport".to_owned()),
            }),
            Arc::new(FakeBackend {
                name: "paid".to_owned(),
                paid: true,
                page: Some(page("Home", "body")),
                fail: None,
            }),
        ],
        Some(Arc::new(SilentVet)),
        Some(Arc::new(YesConsent)),
        Some(Arc::new(NoopSink)),
        Some(Arc::new(NoopSink)),
    );
    let out = chain.run("https://example.com").expect("paid answered");
    assert!(out.contains("[fetched by paid · billed]"));
}

#[test]
fn paid_without_consent_falls_back_to_consent_refusal() {
    let chain = FetchChain::new(
        vec![
            Arc::new(FakeBackend {
                name: "local".to_owned(),
                paid: false,
                page: None,
                fail: Some("transport".to_owned()),
            }),
            Arc::new(FakeBackend {
                name: "paid".to_owned(),
                paid: true,
                page: None,
                fail: Some("transport".to_owned()),
            }),
        ],
        Some(Arc::new(SilentVet)),
        Some(Arc::new(NoConsent)),
        Some(Arc::new(NoopSink)),
        None,
    );
    let err = chain.run("https://example.com").expect_err("needs consent");
    assert_eq!(err.refusal_code(), Some(CODE_FETCH_PAID_NEEDS_CONSENT));
}

#[test]
fn every_backend_failing_returns_all_failed_refusal() {
    let chain = FetchChain::new(
        vec![Arc::new(FakeBackend {
            name: "local".to_owned(),
            paid: false,
            page: None,
            fail: Some("transport".to_owned()),
        })],
        None,
        None,
        None,
        None,
    );
    let err = chain.run("https://example.com").expect_err("all failed");
    assert_eq!(err.refusal_code(), Some(CODE_FETCH_ALL_FAILED));
}

#[test]
fn fetchchain_requires_url() {
    let chain = FetchChain::new(vec![], None, None, None, None);
    assert!(chain.run("").is_err());
}

#[test]
fn render_page_marks_paid_and_passes_rendered_through() {
    let rendered = render_page("paid", true, &page("T", "text"));
    assert!(rendered.contains("· billed"));
    let direct = FetchPage {
        rendered: "already rendered".to_owned(),
        ..Default::default()
    };
    assert_eq!(render_page("local", false, &direct), "already rendered");
}

// --- slash_command ------------------------------------------------------------

struct FixedRenderer(String);
impl SlashRenderer for FixedRenderer {
    fn render(&self, _: &[String]) -> String {
        self.0.clone()
    }
}

struct HiddenEntry;
impl SlashVisibility for HiddenEntry {
    fn hidden(&self) -> bool {
        true
    }
}

struct RejectGate;
impl SlashGate for RejectGate {
    fn admit(&self, name: &str) -> Result<(), Refusal> {
        Err(Refusal::code("skill.gate", format!("skill {name} denied")))
    }
}

fn entry(name: &str, skill: bool, renderer: Option<Arc<dyn SlashRenderer>>) -> SlashEntry {
    SlashEntry {
        name: name.to_owned(),
        description: format!("the {name} command"),
        arg_hint: "[target]".to_owned(),
        skill,
        visibility: None,
        renderer,
    }
}

#[test]
fn slash_name_normalization_strips_leading_slash_and_space() {
    assert_eq!(normalize_slash_name("  /review  "), "review");
    assert_eq!(normalize_slash_name("git:commit"), "git:commit");
}

#[test]
fn catalog_builds_later_wins_and_lists_sorted() {
    let catalog = SlashCatalog::build(vec![
        entry(
            "/zoo",
            false,
            Some(Arc::new(FixedRenderer("zoo body".to_owned()))),
        ),
        entry(
            "review",
            false,
            Some(Arc::new(FixedRenderer("review body".to_owned()))),
        ),
        entry(
            "review",
            false,
            Some(Arc::new(FixedRenderer("later wins".to_owned()))),
        ),
    ]);
    let names = catalog.allowed_names(None);
    assert_eq!(names, vec!["review", "zoo"]);
    let listing = list_slash_commands(&catalog, None);
    assert!(listing.contains("- /review [target] — the review command"));
    assert!(listing.contains("- /zoo"));
}

#[test]
fn empty_catalog_lists_the_no_commands_message() {
    let catalog = SlashCatalog::build(vec![]);
    assert_eq!(
        list_slash_commands(&catalog, None),
        "No slash commands are configured in this project."
    );
}

#[test]
fn unknown_command_error_names_the_available_set() {
    let catalog = SlashCatalog::build(vec![entry(
        "review",
        false,
        Some(Arc::new(FixedRenderer("r".to_owned()))),
    )]);
    let tool = SlashCommandTool::new(catalog, None);
    let err = tool
        .execute(&CallContext::new(), &json!({"command": "missing"}))
        .expect_err("unknown");
    assert!(err.to_string().contains("\"missing\""));
    assert!(err.to_string().contains("review"));
}

#[test]
fn skill_entry_passes_gate_then_expands_frame() {
    let catalog = SlashCatalog::build(vec![SlashEntry {
        name: "review".to_owned(),
        description: "review".to_owned(),
        arg_hint: String::new(),
        skill: true,
        visibility: None,
        renderer: Some(Arc::new(FixedRenderer("check the diff".to_owned()))),
    }]);
    let tool = SlashCommandTool::new(catalog, Some(Arc::new(RejectGate)));
    let err = tool
        .execute(&CallContext::new(), &json!({"command": "review"}))
        .expect_err("gate refused");
    assert!(err.to_string().contains("skill review denied"));

    let open = SlashCommandTool::new(
        SlashCatalog::build(vec![SlashEntry {
            name: "review".to_owned(),
            description: "review".to_owned(),
            arg_hint: String::new(),
            skill: false,
            visibility: None,
            renderer: Some(Arc::new(FixedRenderer("check the diff".to_owned()))),
        }]),
        None,
    );
    let out = open
        .execute(
            &CallContext::new(),
            &json!({"command": "/review", "arguments": "src"}),
        )
        .expect("expanded");
    assert_eq!(out, expand_frame("review", "check the diff"));
}

#[test]
fn empty_skill_expansion_is_a_changed_while_expanding_error() {
    let catalog = SlashCatalog::build(vec![SlashEntry {
        name: "review".to_owned(),
        description: "review".to_owned(),
        arg_hint: String::new(),
        skill: true,
        visibility: None,
        renderer: Some(Arc::new(FixedRenderer(String::new()))),
    }]);
    let tool = SlashCommandTool::new(catalog, None);
    let err = tool
        .execute(&CallContext::new(), &json!({"command": "review"}))
        .expect_err("changed");
    assert!(
        err.to_string()
            .contains("changed while it was being expanded")
    );
}

#[test]
fn hidden_command_kept_out_of_listing_but_still_callable() {
    let catalog = SlashCatalog::build(vec![
        SlashEntry {
            name: "review".to_owned(),
            description: "review".to_owned(),
            arg_hint: String::new(),
            skill: false,
            visibility: Some(Arc::new(HiddenEntry)),
            renderer: Some(Arc::new(FixedRenderer("hidden body".to_owned()))),
        },
        entry(
            "visible",
            false,
            Some(Arc::new(FixedRenderer("visible body".to_owned()))),
        ),
    ]);
    assert_eq!(catalog.allowed_names(None), vec!["visible"]);
    assert!(catalog.get("review").is_some());
}

// --- install_source -----------------------------------------------------------

#[test]
fn install_normalizers_fall_back_to_auto_or_default() {
    assert_eq!(normalize_install_op(""), Ok("install"));
    assert_eq!(normalize_install_op("uninstall"), Ok("uninstall"));
    assert!(normalize_install_op("upgrade").is_err());
    assert_eq!(normalize_kind("mcp"), "mcp");
    assert_eq!(normalize_kind("weird"), "auto");
    assert_eq!(normalize_scope("project"), Some("project"));
    assert_eq!(normalize_scope("weird"), None);
    assert_eq!(normalize_mode("link"), "link");
    assert_eq!(normalize_transport("sse"), "sse");
    assert_eq!(normalize_tier("eager"), Some("eager"));
}

#[test]
fn install_request_gate_validates_op_source_and_name() {
    let missing_source = InstallRequest {
        op: Some("install".to_owned()),
        source: Some("  ".to_owned()),
        ..Default::default()
    };
    assert!(matches!(
        validate_install_request(&missing_source),
        Err(InstallError::MissingSource)
    ));

    let missing_name = InstallRequest {
        op: Some("uninstall".to_owned()),
        name: Some("  ".to_owned()),
        ..Default::default()
    };
    assert!(matches!(
        validate_install_request(&missing_name),
        Err(InstallError::MissingUninstallName)
    ));

    let unsupported = InstallRequest {
        op: Some("upgrade".to_owned()),
        source: Some("pkg".to_owned()),
        ..Default::default()
    };
    assert!(matches!(
        validate_install_request(&unsupported),
        Err(InstallError::OpUnsupported { .. })
    ));

    let ok = validate_install_request(&InstallRequest {
        op: None,
        source: Some("pkg".to_owned()),
        kind: Some("mcp".to_owned()),
        apply: Some(true),
        scope: Some("global".to_owned()),
        mode: Some("copy".to_owned()),
        name: None,
        transport: Some("stdio".to_owned()),
        tier: Some("background".to_owned()),
        plan_id: Some("plan-1".to_owned()),
    })
    .expect("valid");
    assert_eq!(ok.op, "install");
    assert_eq!(ok.kind, "mcp");
    assert!(ok.apply);
}

#[test]
fn unticketed_apply_detects_missing_plan_id() {
    assert!(unticketed_apply(true, true, "  "));
    assert!(!unticketed_apply(true, true, "plan-1"));
    assert!(!unticketed_apply(false, true, ""));
    assert!(!unticketed_apply(true, false, ""));
}

struct FakeConnector;
impl InstallConnector for FakeConnector {
    fn connect(
        &self,
        _: &NormalizedRequest,
    ) -> Result<InstallReceipt, Box<dyn std::error::Error + Send + Sync>> {
        Ok(InstallReceipt { tool_count: 3 })
    }
}

struct SilentApprover;
impl InstallApprover for SilentApprover {
    fn approve(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }
}

#[test]
fn install_source_unticketed_apply_returns_plan_instead_of_installing() {
    let tool = InstallSourceTool::new(
        Some(Arc::new(FakeConnector)),
        Some(Arc::new(SilentApprover)),
        true,
    );
    assert_eq!(tool.name(), INSTALL_SOURCE_TOOL_NAME);
    assert!(!tool.read_only());
    let out = tool
        .execute(
            &CallContext::new(),
            &json!({"op": "install", "source": "pkg", "apply": true}),
        )
        .expect("planned receipt");
    assert!(out.contains("\"status\":\"planned\""));
}

#[test]
fn install_source_apply_without_connector_fails_closed() {
    let tool = InstallSourceTool::new(None, None, true);
    let err = tool
        .execute(
            &CallContext::new(),
            &json!({"op": "install", "source": "pkg", "apply": true, "planId": "plan-1"}),
        )
        .expect_err("seam unwired");
    assert!(matches!(
        err.downcast_ref::<InstallError>(),
        Some(InstallError::SeamUnwired)
    ));
}

// --- assembly anchors ---------------------------------------------------------

#[test]
fn every_conditional_surface_anchors_to_a_frozen_supplier() {
    let surfaces = conditional_surfaces();
    let anchors = resolve_conditional_anchors(surfaces).expect("anchors resolve");
    assert_eq!(anchors.len(), surfaces.len());
    for anchor in anchors {
        assert_eq!(anchor.surface.anchor_step as u16, anchor.supplier.step);
        assert!(!anchor.surface.slot.is_empty());
    }
    let slugs: Vec<&str> = surfaces.iter().map(|s| s.slug).collect();
    assert!(slugs.contains(&"web_search"));
    assert!(slugs.contains(&"fetchchain"));
    assert!(slugs.contains(&"slash_command"));
    assert!(slugs.contains(&"mcp_connect"));
    assert!(slugs.contains(&"install_source"));
}

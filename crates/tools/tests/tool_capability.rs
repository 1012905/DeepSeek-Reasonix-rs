//! TOOL-037 use_capability resolution order + deferred index floor
//! (Go `internal/runtime/usecap`; docs/contracts/tools.md §3.1).
//!
//! Covers the five-tier resolve order, the typed refusal identities and
//! codes, action classification, and the deferred-index description built
//! from a registry: floor list, provider-visible exclusion, parenthetical cut,
//! byte boundary, and the web_fetch suffix.

use std::any::Any;
use std::collections::HashMap;
use std::sync::Arc;

use reasonix_tools::abi::{CallContext, Tool, ToolResult};
use reasonix_tools::registry::Registry;
use reasonix_tools::usecap::{
    ACTION_CALL, ACTION_DECLINE, ACTION_INSPECT, ACTION_LIST, ACTION_SEARCH,
    CAPABILITY_ID_EXAMPLES, CatalogLookup, DEFERRED_INDEX_TOOLS, DEFERRED_LEAD_BYTES,
    ResolutionError, ResolvedTarget, USE_CAPABILITY_BASE, USE_CAPABILITY_TOOL_NAME, bounded_lead,
    capability_id_required, deferred_index, is_catalog_read_action, is_idless_catalog_action,
    reason_required_for_decline, resolve_capability_id, unknown_action, use_capability_description,
};
use serde_json::{Value, json};

struct CardTool {
    name: String,
    description: String,
}

impl Tool for CardTool {
    fn name(&self) -> &str {
        &self.name
    }
    fn description(&self) -> &str {
        &self.description
    }
    fn schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn execute(&self, _ctx: &CallContext, _args: &Value) -> ToolResult {
        Ok(String::new())
    }
    fn read_only(&self) -> bool {
        true
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn seeded_registry(names: &[(&str, &str)]) -> Registry {
    let reg = Registry::new();
    for (name, description) in names {
        reg.add(Arc::new(CardTool {
            name: (*name).to_owned(),
            description: (*description).to_owned(),
        }));
    }
    reg
}

struct FakeCatalog {
    alias_to_tool: HashMap<String, String>,
}

impl CatalogLookup for FakeCatalog {
    fn catalog_tool_name(&self, id: &str) -> Option<String> {
        self.alias_to_tool.get(id).cloned()
    }
}

#[test]
fn tool_prefix_resolves_the_registry_tool_and_refuses_the_self_proxy() {
    let reg = seeded_registry(&[("grep", "Search file contents.")]);

    let target = resolve_capability_id("tool:grep", Some(&reg), None).expect("tool:grep resolves");
    assert_eq!(
        target,
        ResolvedTarget::RegistryTool {
            name: "grep".to_owned()
        }
    );

    let err = resolve_capability_id("tool:use_capability", Some(&reg), None)
        .expect_err("self proxy refused");
    assert_eq!(err, ResolutionError::SelfProxy);
    assert_eq!(err.code(), "usecap.self_proxy");

    let err = resolve_capability_id("tool:", Some(&reg), None).expect_err("empty tool name");
    assert!(matches!(err, ResolutionError::MissingToolName(_)));

    let err = resolve_capability_id("tool:ghost", Some(&reg), None).expect_err("unknown tool");
    assert_eq!(err, ResolutionError::ToolNotRegistered("ghost".to_owned()));
    assert_eq!(err.code(), "usecap.tool_not_registered");

    let err = resolve_capability_id("tool:grep", None, None).expect_err("no registry");
    assert_eq!(err, ResolutionError::RegistryUnavailable);
}

#[test]
fn skill_prefix_follows_the_run_then_read_only_preference() {
    let run = seeded_registry(&[("run_skill", "Run a skill.")]);
    let target =
        resolve_capability_id("skill:review", Some(&run), None).expect("run_skill present");
    assert_eq!(
        target,
        ResolvedTarget::Skill {
            skill_name: "review".to_owned(),
            tool_name: "run_skill".to_owned()
        }
    );

    let read_only = seeded_registry(&[("read_only_skill", "Read a skill.")]);
    let target = resolve_capability_id("skill:review", Some(&read_only), None)
        .expect("read_only_skill next");
    assert_eq!(
        target,
        ResolvedTarget::Skill {
            skill_name: "review".to_owned(),
            tool_name: "read_only_skill".to_owned()
        }
    );

    let read = seeded_registry(&[("read_skill", "Read a skill.")]);
    let target = resolve_capability_id("skill:review", Some(&read), None).expect("read_skill last");
    assert_eq!(
        target,
        ResolvedTarget::Skill {
            skill_name: "review".to_owned(),
            tool_name: "read_skill".to_owned()
        }
    );

    let empty = seeded_registry(&[]);
    let err =
        resolve_capability_id("skill:review", Some(&empty), None).expect_err("no skill runner");
    assert_eq!(err, ResolutionError::SkillUnavailable("review".to_owned()));

    let err = resolve_capability_id("skill:", Some(&empty), None).expect_err("empty skill");
    assert!(matches!(err, ResolutionError::MissingSkillName(_)));

    let err = resolve_capability_id("skill:review", None, None).expect_err("no registry");
    assert_eq!(err, ResolutionError::RegistryUnavailable);
}

#[test]
fn catalog_alias_resolves_before_the_mcp_tiers() {
    let reg = seeded_registry(&[("grep", "Search file contents.")]);
    let catalog = FakeCatalog {
        alias_to_tool: HashMap::from([("alias:grep".to_owned(), "grep".to_owned())]),
    };
    let target =
        resolve_capability_id("alias:grep", Some(&reg), Some(&catalog)).expect("catalog alias");
    assert_eq!(
        target,
        ResolvedTarget::CatalogAlias {
            id: "alias:grep".to_owned(),
            tool_name: "grep".to_owned()
        }
    );
}

#[test]
fn mcp_server_and_mcp_tool_prefixes_parse_in_order() {
    let target = resolve_capability_id("mcp-server:docs", None, None).expect("server connect");
    assert_eq!(
        target,
        ResolvedTarget::McpServer {
            server: "docs".to_owned()
        }
    );

    let target = resolve_capability_id("mcp-tool:docs/search", None, None).expect("concrete tool");
    assert_eq!(
        target,
        ResolvedTarget::McpTool {
            server: "docs".to_owned(),
            raw: "search".to_owned()
        }
    );

    let err = resolve_capability_id("mcp-tool:docs", None, None).expect_err("missing slash");
    assert_eq!(
        err,
        ResolutionError::InvalidMcpToolId("mcp-tool:docs".to_owned())
    );

    let err = resolve_capability_id("mcp-tool:/search", None, None).expect_err("empty server");
    assert!(matches!(err, ResolutionError::InvalidMcpToolId(_)));

    let err = resolve_capability_id("mcp-tool:docs/", None, None).expect_err("empty raw");
    assert!(matches!(err, ResolutionError::InvalidMcpToolId(_)));

    let err = resolve_capability_id("mcp-server:", None, None)
        .expect_err("empty server id is a server-id-as-tool refusal");
    assert!(matches!(err, ResolutionError::ServerIdAsTool(_)));

    let err = resolve_capability_id("grep", None, None).expect_err("plain name");
    assert_eq!(err, ResolutionError::NotMcpToolId("grep".to_owned()));
}

#[test]
fn action_classification_matches_the_go_sets() {
    assert!(is_catalog_read_action(ACTION_SEARCH));
    assert!(is_catalog_read_action(ACTION_LIST));
    assert!(is_catalog_read_action(ACTION_INSPECT));
    assert!(!is_catalog_read_action(ACTION_CALL));
    assert!(!is_catalog_read_action(ACTION_DECLINE));

    assert!(is_idless_catalog_action(ACTION_SEARCH));
    assert!(is_idless_catalog_action(ACTION_LIST));
    assert!(!is_idless_catalog_action(ACTION_INSPECT));
    assert!(!is_idless_catalog_action(ACTION_CALL));
}

#[test]
fn required_field_and_unknown_action_messages_are_frozen() {
    assert_eq!(
        capability_id_required(ACTION_INSPECT),
        "capability_id is required for action=inspect"
    );
    assert_eq!(
        capability_id_required(ACTION_CALL),
        "capability_id is required for action=call"
    );
    assert_eq!(
        capability_id_required(ACTION_DECLINE),
        "capability_id is required for action=decline"
    );
    assert_eq!(
        reason_required_for_decline(),
        "reason is required for action=decline"
    );
    assert_eq!(
        unknown_action("explode"),
        "unknown action \"explode\"; use search, list, inspect, call, or decline"
    );
}

#[test]
fn the_deferred_floor_lists_the_go_twelve_in_order() {
    assert_eq!(
        DEFERRED_INDEX_TOOLS,
        &[
            "web_fetch",
            "web_search",
            "browser_open",
            "browser_read",
            "browser_act",
            "grep",
            "glob",
            "ls",
            "present_files",
            "setting",
            "compress",
            "context_budget",
        ]
    );
}

#[test]
fn deferred_index_names_hidden_floor_tools_and_skips_provider_visible() {
    let reg = seeded_registry(&[
        ("web_fetch", "Read HTTP(S) text. Reduces HTML to text."),
        ("browser_open", "Load a page and return its snapshot."),
        ("grep", "Search file contents."),
        ("read_file", "Read a file."),
    ]);
    reg.set_provider_visible_tools(Some(&["read_file"]));

    let index = deferred_index(Some(&reg));
    assert!(index.contains("tool:web_fetch"));
    assert!(index.contains("Read HTTP(S) text."));
    assert!(index.contains("tool:browser_open"));
    assert!(index.contains("tool:grep"));
    assert!(!index.contains("tool:read_file"));
    assert!(index.contains(" Deferred tools, called by id: "));
    assert!(index.contains(
        " Read JSON or static text with tool:web_fetch; open a browser only for pages that need rendering, clicks or a login."
    ));
}

#[test]
fn deferred_index_is_empty_when_nothing_is_hidden_and_base_survives_nil() {
    let reg = seeded_registry(&[("web_fetch", "Read HTTP(S) text.")]);
    reg.set_provider_visible_tools(Some(&["web_fetch"]));
    assert_eq!(deferred_index(Some(&reg)), "");
    assert_eq!(use_capability_description(Some(&reg)), USE_CAPABILITY_BASE);

    let bare = use_capability_description(None);
    assert_eq!(bare, USE_CAPABILITY_BASE);
    assert!(bare.starts_with("Search and call deferred tools, skills, and MCP through stable IDs"));
}

#[test]
fn deferred_index_is_byte_stable_for_the_same_registry() {
    let reg = seeded_registry(&[
        ("web_fetch", "Read HTTP(S) text."),
        ("grep", "Search file contents."),
    ]);
    let a = use_capability_description(Some(&reg));
    let b = use_capability_description(Some(&reg));
    assert_eq!(a, b);
}

#[test]
fn a_row_is_cut_before_the_parenthetical() {
    let reg = seeded_registry(&[(
        "glob",
        "Find files matching a glob pattern (e.g. *.go). More text.",
    )]);
    reg.set_provider_visible_tools(Some(&[]));
    let index = deferred_index(Some(&reg));
    assert!(
        index.contains("tool:glob (Find files matching a glob pattern)"),
        "{index}"
    );
    assert!(!index.contains("e.g. *.go"), "{index}");
}

#[test]
fn bounded_lead_respects_the_byte_boundary_one_each_side() {
    let exactly: String = "a".repeat(DEFERRED_LEAD_BYTES);
    assert_eq!(bounded_lead(&exactly, DEFERRED_LEAD_BYTES), exactly);

    let over: String = "b".repeat(DEFERRED_LEAD_BYTES + 1);
    let lead = bounded_lead(&over, DEFERRED_LEAD_BYTES);
    assert!(lead.ends_with("…"), "{lead}");
    assert!(lead.len() <= DEFERRED_LEAD_BYTES + "…".len(), "{lead}");

    let sentence = "First sentence. Second sentence that runs on and on.";
    assert_eq!(
        bounded_lead(sentence, DEFERRED_LEAD_BYTES),
        "First sentence."
    );

    assert_eq!(bounded_lead("", DEFERRED_LEAD_BYTES), "");
}

#[test]
fn the_proxy_identity_and_examples_match_the_contract() {
    assert_eq!(USE_CAPABILITY_TOOL_NAME, "use_capability");
    assert_eq!(
        CAPABILITY_ID_EXAMPLES,
        ["tool:grep", "skill:review", "task:subagent"]
    );
}

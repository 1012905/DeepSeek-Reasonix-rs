//! TOOL-037: `use_capability` resolution order and the deferred index floor
//! (docs/contracts/tools.md §3.1; Go `internal/runtime/usecap`).
//!
//! This slice owns the pure resolution semantics: the five-tier capability-id
//! order, the typed refusal identities, the action classification, and the
//! deferred-index description built from the registry.
//!
//! TODO(wave39-b): the live catalog alias source, the MCP host session, the
//! capability ledger and audit, and the cross-turn inventory rebuild are
//! runtime substrates the assembly slice binds; resolution here stays pure —
//! no subprocess, no network, no permission gate before the real target is
//! named.

#![forbid(unsafe_code)]

use serde_json::Value;

use crate::registry::Registry;

/// `use_capability`, the proxy tool name.
pub const USE_CAPABILITY_TOOL_NAME: &str = "use_capability";

/// Go `useCapabilityBase`, verbatim.
pub const USE_CAPABILITY_BASE: &str = "Search and call deferred tools, skills, and MCP through stable IDs such as tool:grep, skill:review, task:subagent, and mcp-tool:server/tool. Before saying a tool, network, or integration is unavailable, search by natural-language goal; results include ranked IDs and available schemas. list enumerates the catalog, inspect loads one definition, call invokes it, and decline records why a preferred capability is skipped. task:subagent delegates 1-8 tasks that run in parallel. Calls retain all permission, sandbox, and execution checks.";

/// Go `CapabilityIDExamples`: the concrete ids the proxy description names.
pub const CAPABILITY_ID_EXAMPLES: [&str; 3] = ["tool:grep", "skill:review", "task:subagent"];

/// Go `UseCapabilityTool.Schema()`, stable across turns and MCP connects.
pub const USE_CAPABILITY_SCHEMA: &str = r#"{
	"type":"object",
	"properties":{
		"action":{"type":"string","description":"search | list | inspect | call | decline"},
		"query":{"type":"string","description":"Natural-language search goal"},
		"limit":{"type":"integer","minimum":1,"maximum":20,"description":"Result limit; default 5"},
		"capability_id":{"type":"string","description":"Stable ID required by inspect, call, and decline"},
		"arguments":{"type":"object","description":"Target arguments for call"},
		"reason":{"type":"string","description":"Non-empty decline reason"}
	},
	"required":["action"]
}"#;

/// `search` — lexical catalog read.
pub const ACTION_SEARCH: &str = "search";
/// `list` — full catalog enumeration.
pub const ACTION_LIST: &str = "list";
/// `inspect` — one definition loaded.
pub const ACTION_INSPECT: &str = "inspect";
/// `call` — resolve and invoke a target.
pub const ACTION_CALL: &str = "call";
/// `decline` — record why a preferred capability was skipped.
pub const ACTION_DECLINE: &str = "decline";

/// Go `CatalogReadActions`: resolve host-side without a target, execute
/// nothing.
pub const CATALOG_READ_ACTIONS: [&str; 3] = [ACTION_SEARCH, ACTION_LIST, ACTION_INSPECT];

/// Go `IDlessCatalogActions`: catalog reads that need no capability_id.
/// inspect is excluded on purpose: it names an id, so a restricted proxy must
/// gate it.
pub const IDLESS_CATALOG_ACTIONS: [&str; 2] = [ACTION_SEARCH, ACTION_LIST];

/// Whether the action is a catalog read.
pub fn is_catalog_read_action(action: &str) -> bool {
    CATALOG_READ_ACTIONS.contains(&action)
}

/// Whether the action may omit `capability_id`.
pub fn is_idless_catalog_action(action: &str) -> bool {
    IDLESS_CATALOG_ACTIONS.contains(&action)
}

/// The skill tool preference, in order (Go `resolveSkillCall`): full run
/// first, then the read-only variants a planner/plan mode exposes.
pub const SKILL_TOOL_PREFERENCE: [&str; 3] = ["run_skill", "read_only_skill", "read_skill"];

/// The floor list of built-ins a model reaches for by name before it searches:
/// hidden from the provider schema, so without a line here a directly visible
/// browser is the only way it can see to read a URL (Go `deferredIndexTools`).
pub const DEFERRED_INDEX_TOOLS: &[&str] = &[
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
];

/// Go `deferredLeadBytes`: one deferred row's description lead.
pub const DEFERRED_LEAD_BYTES: usize = 72;

/// The ordered resolution target behind a capability id (Go `resolveCall`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedTarget {
    /// `tool:<name>` — a registry tool.
    RegistryTool { name: String },
    /// `skill:<name>` — a skill runner in the preference order.
    Skill {
        skill_name: String,
        tool_name: String,
    },
    /// A catalog alias the listing itself answered to.
    CatalogAlias { id: String, tool_name: String },
    /// `mcp-server:<name>` — connect-and-list deferred target.
    McpServer { server: String },
    /// `mcp-tool:<server>/<raw>` — a concrete MCP tool.
    McpTool { server: String, raw: String },
}

/// Pure resolution failure with a typed code (Go the resolveCall error
/// strings). The message stays the frozen model-visible wording.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ResolutionError {
    /// The proxy asked to proxy itself.
    #[error("cannot proxy use_capability through itself")]
    SelfProxy,
    /// A `tool:` id carried no name.
    #[error("capability id {0:?} is missing a tool name")]
    MissingToolName(String),
    /// A `skill:` id carried no name.
    #[error("capability id {0:?} is missing a skill name")]
    MissingSkillName(String),
    /// No registry is bound to the proxy.
    #[error("tool registry is unavailable")]
    RegistryUnavailable,
    /// The registry does not serve the named tool.
    #[error("tool {0:?} is not registered in this session")]
    ToolNotRegistered(String),
    /// No skill runner is present for the named skill.
    #[error("skill tools are not available for {0:?}")]
    SkillUnavailable(String),
    /// A malformed `mcp-tool:` id.
    #[error("invalid mcp-tool id {0:?}; want mcp-tool:<server>/<tool>")]
    InvalidMcpToolId(String),
    /// A server id was passed where a tool id was expected.
    #[error(
        "{0:?} is a server id; call it directly to connect and list tools, or use mcp-tool:<server>/<tool>"
    )]
    ServerIdAsTool(String),
    /// An id the resolver does not recognize as an MCP tool reference.
    #[error("action=call requires an mcp-tool capability id, got {0:?}")]
    NotMcpToolId(String),
}

impl ResolutionError {
    /// The stable refusal code a host classifies on.
    pub fn code(&self) -> &'static str {
        match self {
            ResolutionError::SelfProxy => "usecap.self_proxy",
            ResolutionError::MissingToolName(_) => "usecap.missing_tool_name",
            ResolutionError::MissingSkillName(_) => "usecap.missing_skill_name",
            ResolutionError::RegistryUnavailable => "usecap.registry_unavailable",
            ResolutionError::ToolNotRegistered(_) => "usecap.tool_not_registered",
            ResolutionError::SkillUnavailable(_) => "usecap.skill_unavailable",
            ResolutionError::InvalidMcpToolId(_) => "usecap.invalid_mcp_tool_id",
            ResolutionError::ServerIdAsTool(_) => "usecap.server_id_as_tool",
            ResolutionError::NotMcpToolId(_) => "usecap.not_mcp_tool_id",
        }
    }
}

/// A catalog alias lookup the proxy shares with the listing surface (Go
/// `catalog.Lookup`). Only tool-kind entries with a non-empty registry name
/// resolve through this tier.
pub trait CatalogLookup: Send + Sync {
    /// The registry tool name behind a catalog id, when the entry is a live
    /// tool; `None` otherwise.
    fn catalog_tool_name(&self, id: &str) -> Option<String>;
}

/// Resolves a capability id in the frozen five-tier order:
///
/// 1. `tool:<name>` — registry tool (self-proxy refused, withdrawn slots
///    excluded by the registry itself).
/// 2. `skill:<name>` — skill runner, preference
///    [`SKILL_TOOL_PREFERENCE`].
/// 3. Catalog alias — the id the model read from list/inspect.
/// 4. `mcp-server:<name>` — deferred connect-and-list.
/// 5. `mcp-tool:<server>/<raw>` — concrete MCP tool; anything else is a typed
///    not-an-mcp-tool refusal.
pub fn resolve_capability_id(
    id: &str,
    registry: Option<&Registry>,
    catalog: Option<&dyn CatalogLookup>,
) -> Result<ResolvedTarget, ResolutionError> {
    let id = id.trim();

    // Tier 1: registry tool.
    if let Some(name) = id.strip_prefix("tool:") {
        let name = name.trim();
        if name.is_empty() {
            return Err(ResolutionError::MissingToolName(id.to_owned()));
        }
        if name == USE_CAPABILITY_TOOL_NAME {
            return Err(ResolutionError::SelfProxy);
        }
        let Some(reg) = registry else {
            return Err(ResolutionError::RegistryUnavailable);
        };
        if reg.get(name).is_none() {
            return Err(ResolutionError::ToolNotRegistered(name.to_owned()));
        }
        return Ok(ResolvedTarget::RegistryTool {
            name: name.to_owned(),
        });
    }

    // Tier 2: skill runner.
    if let Some(skill) = id.strip_prefix("skill:") {
        let skill = skill.trim();
        if skill.is_empty() {
            return Err(ResolutionError::MissingSkillName(id.to_owned()));
        }
        let Some(reg) = registry else {
            return Err(ResolutionError::RegistryUnavailable);
        };
        for tool_name in SKILL_TOOL_PREFERENCE {
            if reg.get(tool_name).is_some() {
                return Ok(ResolvedTarget::Skill {
                    skill_name: skill.to_owned(),
                    tool_name: tool_name.to_owned(),
                });
            }
        }
        return Err(ResolutionError::SkillUnavailable(skill.to_owned()));
    }

    // Tier 3: catalog alias.
    if let Some(lookup) = catalog
        && let Some(tool_name) = lookup.catalog_tool_name(id)
        && !tool_name.is_empty()
    {
        return Ok(ResolvedTarget::CatalogAlias {
            id: id.to_owned(),
            tool_name,
        });
    }

    // Tier 4: server-level connect.
    if let Some(server) = id.strip_prefix("mcp-server:") {
        let server = server.trim();
        if !server.is_empty() {
            return Ok(ResolvedTarget::McpServer {
                server: server.to_owned(),
            });
        }
        // An empty server id falls through to the generic MCP parse, the same
        // way ParseMCPServerCapabilityID reports no match.
    }

    // Tier 5: concrete MCP tool.
    if let Some(rest) = id.strip_prefix("mcp-tool:") {
        let rest = rest.trim();
        let Some((server, raw)) = rest.split_once('/') else {
            return Err(ResolutionError::InvalidMcpToolId(id.to_owned()));
        };
        if server.trim().is_empty() || raw.trim().is_empty() {
            return Err(ResolutionError::InvalidMcpToolId(id.to_owned()));
        }
        return Ok(ResolvedTarget::McpTool {
            server: server.trim().to_owned(),
            raw: raw.trim().to_owned(),
        });
    }
    if id.starts_with("mcp-server:") {
        return Err(ResolutionError::ServerIdAsTool(id.to_owned()));
    }
    Err(ResolutionError::NotMcpToolId(id.to_owned()))
}

/// `capability_id is required for action=<action>` (Go the inspect/call/
/// decline required-id messages; the nested-argument hint is added by the
/// argument-contract slice).
pub fn capability_id_required(action: &str) -> String {
    format!("capability_id is required for action={action}")
}

/// `reason is required for action=decline`.
pub const fn reason_required_for_decline() -> &'static str {
    "reason is required for action=decline"
}

/// `unknown action %q; use search, list, inspect, call, or decline`.
pub fn unknown_action(action: &str) -> String {
    format!("unknown action {action:?}; use search, list, inspect, call, or decline")
}

/// First-sentence lead bounded to `limit` bytes (Go `boundedLead`): cut
/// before a parenthetical happens at the caller; here the head is trimmed, cut
/// on a sentence boundary, then word-boundary truncated with a rune-safe
/// fallback for CJK text.
pub fn bounded_lead(description: &str, limit: usize) -> String {
    let lead = description.trim();
    if lead.is_empty() {
        return String::new();
    }
    let mut lead = lead.to_owned();
    if let Some((head, rest)) = lead.split_once(". ")
        && !rest.is_empty()
    {
        lead = format!("{head}.");
    }
    if lead.len() <= limit {
        return lead;
    }
    let mut bounded = lead[..limit.min(lead.len())].to_owned();
    if let Some(space) = bounded.rfind([' ', '\t'])
        && space > limit / 2
    {
        bounded.truncate(space);
    } else {
        // TrimPartialRune: drop a trailing partial UTF-8 continuation
        // sequence so the lead never splits a multibyte character.
        while matches!(bounded.as_bytes().last(), Some(b) if (b & 0xC0) == 0x80) {
            bounded.pop();
        }
    }
    let trimmed = bounded.trim_end_matches([' ', '\t', ',', ';', ':']);
    format!("{trimmed}…")
}

/// The deferred-index suffix for the registry (Go `deferredIndex`): one row
/// per floor tool the registry holds that is NOT provider-visible. Empty when
/// the registry is absent or nothing is deferred.
pub fn deferred_index(registry: Option<&Registry>) -> String {
    let Some(reg) = registry else {
        return String::new();
    };
    let mut rows: Vec<String> = Vec::new();
    let mut has_fetch = false;
    for name in DEFERRED_INDEX_TOOLS {
        let Some(tool) = reg.get(name) else {
            continue;
        };
        if reg.provider_visible(name) {
            continue;
        }
        if *name == "web_fetch" {
            has_fetch = true;
        }
        let description = tool.description();
        let description = description.split(" (").next().unwrap_or(description);
        let mut row = format!("tool:{name}");
        let lead = bounded_lead(description, DEFERRED_LEAD_BYTES);
        if !lead.is_empty() {
            row += &format!(" ({lead})");
        }
        rows.push(row);
    }
    if rows.is_empty() {
        return String::new();
    }
    let mut out = format!(" Deferred tools, called by id: {}.", rows.join("; "));
    if has_fetch {
        out += " Read JSON or static text with tool:web_fetch; open a browser only for pages that need rendering, clicks or a login.";
    }
    out
}

/// The full proxy description: the frozen base plus the deferred index for the
/// registry. A nil registry leaves the base alone (Go `UseCapabilityTool.Description`).
pub fn use_capability_description(registry: Option<&Registry>) -> String {
    format!("{}{}", USE_CAPABILITY_BASE, deferred_index(registry))
}

/// A placeholder proxy tool card for assembly-time contract checks. The live
/// proxy — call resolution, ledger, audit — arrives with the capability slice.
pub fn use_capability_schema_value() -> Value {
    serde_json::from_str(USE_CAPABILITY_SCHEMA).expect("the use_capability schema is valid JSON")
}

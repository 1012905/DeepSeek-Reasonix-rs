//! TOOL-038: the unified capability inventory for one routing turn
//! (docs/contracts/tools.md §3.2, Go `internal/runtime/capability/catalog.go`).
//!
//! The catalog merges live builtin tools, discovered skills and configured MCP
//! servers (including `auto_start=false`) into one entry table with a stable
//! sha256 fingerprint. Role profiles ride along for diagnostics only: a role
//! setting never hides an entry.
//!
//! Real remote catalog pulls and persisted schema-cache loading stay behind
//! the host assembly; this slice ships the entry shape, the merge/dedup/sort
//! decision table and the lookup contract.

#![forbid(unsafe_code)]

use sha2::{Digest, Sha256};

/// Role profile carried for diagnostics (Go `capability.Profile`). It selects
/// nothing: the directory is the same under every role setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Profile {
    #[default]
    Economy,
    Balanced,
    Delivery,
}

impl Profile {
    /// The profile's canonical lowercase spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Profile::Economy => "economy",
            Profile::Balanced => "balanced",
            Profile::Delivery => "delivery",
        }
    }
}

/// What kind of capability an entry describes (Go `capability.Kind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum CapabilityKind {
    #[default]
    Tool,
    Skill,
    McpTool,
    McpServer,
    Source,
}

impl CapabilityKind {
    pub fn as_str(self) -> &'static str {
        match self {
            CapabilityKind::Skill => "skill",
            CapabilityKind::McpServer => "mcp-server",
            CapabilityKind::McpTool => "mcp-tool",
            CapabilityKind::Tool => "tool",
            CapabilityKind::Source => "source",
        }
    }

    /// Parses a persisted cache string; the catalog loader that feeds it lands
    /// with the persisted schema-cache slice.
    #[allow(dead_code)]
    fn from_str(raw: &str) -> Option<Self> {
        match raw {
            "skill" => Some(CapabilityKind::Skill),
            "mcp-server" => Some(CapabilityKind::McpServer),
            "mcp-tool" => Some(CapabilityKind::McpTool),
            "tool" => Some(CapabilityKind::Tool),
            "source" => Some(CapabilityKind::Source),
            _ => None,
        }
    }
}

/// Server lifecycle state (Go `capability.Status`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum CapabilityStatus {
    #[default]
    Configured,
    Ready,
    Disabled,
    Failed,
    Stale,
}

impl CapabilityStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            CapabilityStatus::Ready => "ready",
            CapabilityStatus::Configured => "configured",
            CapabilityStatus::Disabled => "disabled",
            CapabilityStatus::Failed => "failed",
            CapabilityStatus::Stale => "stale",
        }
    }

    /// Parses a persisted cache string; the catalog loader that feeds it lands
    /// with the persisted schema-cache slice.
    #[allow(dead_code)]
    fn from_str(raw: &str) -> Option<Self> {
        match raw {
            "ready" => Some(CapabilityStatus::Ready),
            "configured" => Some(CapabilityStatus::Configured),
            "disabled" => Some(CapabilityStatus::Disabled),
            "failed" => Some(CapabilityStatus::Failed),
            "stale" => Some(CapabilityStatus::Stale),
            _ => None,
        }
    }
}

/// Author-declared auto-use policy (Go `capability.AutoUse`). The empty policy
/// appears only for seeded ledger rows that never carried a declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AutoUse {
    #[default]
    Off,
    Suggest,
    Prefer,
    Require,
    /// No declaration parsed; the router substitutes the skill default.
    Unset,
}

impl AutoUse {
    pub fn as_str(self) -> &'static str {
        match self {
            AutoUse::Off => "off",
            AutoUse::Suggest => "suggest",
            AutoUse::Prefer => "prefer",
            AutoUse::Require => "require",
            AutoUse::Unset => "",
        }
    }

    /// Rank used to order policies strongest-first (Go `rank`).
    pub fn rank(self) -> u8 {
        match self {
            AutoUse::Require => 3,
            AutoUse::Prefer => 2,
            AutoUse::Suggest => 1,
            AutoUse::Off | AutoUse::Unset => 0,
        }
    }
}

/// One inventory entry (Go `capability.Entry`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CapabilityEntry {
    pub id: String,
    pub kind: CapabilityKind,
    pub name: String,
    pub description: String,
    /// Origin scope/server this entry belongs to.
    pub source: String,
    pub status: CapabilityStatus,
    pub read_only: bool,
    pub destructive: bool,
    pub cost: String,
    pub auto_use: AutoUse,
    pub triggers: Vec<String>,
    pub negative_triggers: Vec<String>,
    /// The minted model tool name this entry answers to (MCP tools).
    pub tool_name: String,
    /// Extra capability IDs declared by the tool itself.
    pub aliases: Vec<String>,
    pub connect_source: String,
    pub connect_name: String,
    /// Capability IDs this skill depends on.
    pub requires: Vec<String>,
    /// `economy|balanced|delivery`; empty means all.
    pub profiles: Vec<String>,
    /// MCP: configured `auto_start`.
    pub auto_start: bool,
    /// Host-proven failure detail.
    pub failure_reason: String,
}

/// A builtin tool as the assembly hands it to the catalog (Go
/// `tool.ContractEntry` reduced to the fields this slice reads).
#[derive(Debug, Clone, Default)]
pub struct ToolCatalogInput {
    pub name: String,
    pub description: String,
    pub read_only: bool,
    pub aliases: Vec<String>,
    /// MCP binding, when the tool was registered from an MCP server.
    pub mcp_server: String,
    pub mcp_raw_name: String,
    pub capability_id: String,
}

/// A discovered skill as the store hands it to the catalog (Go `skill.Skill`
/// reduced to catalog-relevant fields).
#[derive(Debug, Clone, Default)]
pub struct SkillCatalogInput {
    pub name: String,
    pub description: String,
    pub scope: String,
    pub cost: String,
    pub auto_use: String,
    pub triggers: Vec<String>,
    pub negative_triggers: Vec<String>,
    pub requires: Vec<String>,
    pub profiles: Vec<String>,
}

/// One cached or proxy-observed concrete MCP tool (Go `plugin.CachedTool`).
#[derive(Debug, Clone, Default)]
pub struct CachedMcpTool {
    pub name: String,
    pub description: String,
    pub read_only: bool,
    pub destructive: bool,
    /// Minted model tool name, e.g. `acme__search`.
    pub tool_name: String,
}

/// A configured MCP server and its lifecycle state (Go `config.PluginEntry`
/// plus the connected/failed/disabled/cached maps).
#[derive(Debug, Clone, Default)]
pub struct McpServerInput {
    pub name: String,
    pub auto_start: bool,
    pub connected: bool,
    /// Host-proven failure reason; empty when the server did not fail.
    pub failed: String,
    pub disabled: bool,
    /// Whether the persisted schema cache key still matches the server spec.
    pub cache_key_ok: bool,
    pub cached_tools: Vec<CachedMcpTool>,
    /// Live tools observed through the Delivery proxy (proxied servers never
    /// register, so these are absent from the tool input list).
    pub proxy_tools: Vec<CachedMcpTool>,
}

/// Inputs that assemble one catalog (Go `capability.CatalogOptions`).
#[derive(Debug, Clone, Default)]
pub struct CatalogOptions {
    pub tools: Vec<ToolCatalogInput>,
    pub skills: Vec<SkillCatalogInput>,
    pub servers: Vec<McpServerInput>,
    /// Diagnostics only.
    pub profile: Profile,
    /// Whether the skill entry tools (`run_skill` / `read_skill` /
    /// `read_only_skill`) are registered; when absent skill entries report
    /// configured rather than ready.
    pub skill_tools_ready: bool,
}

/// The unified capability directory for one routing turn (Go `capability.Catalog`).
#[derive(Debug, Clone, Default)]
pub struct CapabilityCatalog {
    pub entries: Vec<CapabilityEntry>,
    /// First 16 hex chars of the sha256 over `id|kind|status|autoUse` lines.
    pub fingerprint: String,
}

/// Builds the merged, deduplicated, sorted catalog (Go `BuildCatalog`).
pub fn build_catalog(opts: CatalogOptions) -> CapabilityCatalog {
    let mut entries = tool_entries(&opts.tools);
    for entry in entries.iter_mut() {
        if entry.kind == CapabilityKind::McpTool {
            let name = entry.source.as_str();
            let server = opts.servers.iter().find(|s| s.name == name);
            match server {
                Some(s) if s.disabled => entry.status = CapabilityStatus::Disabled,
                Some(s) if !s.failed.is_empty() => {
                    entry.status = CapabilityStatus::Failed;
                    entry.failure_reason = s.failed.clone();
                }
                _ => {}
            }
        }
    }
    entries.extend(skill_entries(&opts.skills, opts.skill_tools_ready));
    entries.extend(mcp_server_entries(&opts));

    // Dedup by ID, preferring the higher lifecycle rank.
    let mut by_id: std::collections::HashMap<String, CapabilityEntry> =
        std::collections::HashMap::new();
    let mut order: Vec<String> = Vec::with_capacity(entries.len());
    for entry in entries {
        match by_id.get(&entry.id) {
            Some(prev) if status_rank(entry.status) <= status_rank(prev.status) => {}
            _ => {
                if !by_id.contains_key(&entry.id) {
                    order.push(entry.id.clone());
                }
                by_id.insert(entry.id.clone(), entry);
            }
        }
    }

    let mut out: Vec<CapabilityEntry> = order
        .into_iter()
        .filter_map(|id| by_id.remove(&id))
        .collect();
    out.sort_by(|a, b| {
        a.kind
            .as_str()
            .cmp(b.kind.as_str())
            .then_with(|| a.id.cmp(&b.id))
    });

    let fingerprint = catalog_fingerprint(&out);
    CapabilityCatalog {
        entries: out,
        fingerprint,
    }
}

/// Builtin tool entries (Go `ToolEntries`).
fn tool_entries(tools: &[ToolCatalogInput]) -> Vec<CapabilityEntry> {
    tools
        .iter()
        .map(|t| {
            let mcp = !t.mcp_server.is_empty();
            CapabilityEntry {
                id: if mcp {
                    t.capability_id.clone()
                } else {
                    format!("tool:{}", t.name)
                },
                kind: if mcp {
                    CapabilityKind::McpTool
                } else {
                    CapabilityKind::Tool
                },
                name: if mcp {
                    format!("{}/{}", t.mcp_server, t.mcp_raw_name)
                } else {
                    t.name.clone()
                },
                description: t.description.trim().to_owned(),
                source: if mcp {
                    t.mcp_server.clone()
                } else {
                    String::new()
                },
                status: CapabilityStatus::Ready,
                read_only: t.read_only,
                tool_name: t.name.clone(),
                aliases: clean_list(&t.aliases),
                connect_name: if mcp {
                    t.mcp_server.clone()
                } else {
                    String::new()
                },
                ..Default::default()
            }
        })
        .collect()
}

/// Skill entries plus the frontmatter a catalog reader sees (Go
/// `skillCatalogEntries` / `SkillEntries`).
fn skill_entries(skills: &[SkillCatalogInput], tools_ready: bool) -> Vec<CapabilityEntry> {
    skills
        .iter()
        .map(|sk| {
            let (status, connect_source) = if tools_ready {
                (CapabilityStatus::Ready, String::new())
            } else {
                (CapabilityStatus::Configured, "skills".to_owned())
            };
            let mut auto = normalize_auto_use(&sk.auto_use);
            if auto == AutoUse::Unset {
                auto = if !sk.triggers.is_empty() {
                    AutoUse::Prefer
                } else {
                    AutoUse::Suggest
                };
            }
            CapabilityEntry {
                id: format!("skill:{}", sk.name),
                kind: CapabilityKind::Skill,
                name: sk.name.clone(),
                description: sk.description.clone(),
                source: sk.scope.clone(),
                status,
                cost: sk.cost.trim().to_owned(),
                auto_use: auto,
                triggers: clean_list(&sk.triggers),
                negative_triggers: clean_list(&sk.negative_triggers),
                tool_name: "run_skill".to_owned(),
                connect_source,
                requires: clean_list(&sk.requires),
                profiles: normalize_profiles(&sk.profiles),
                ..Default::default()
            }
        })
        .collect()
}

/// Server entries plus concrete tools surfaced before any connection exists
/// (Go `MCPServerEntries`).
fn mcp_server_entries(opts: &CatalogOptions) -> Vec<CapabilityEntry> {
    let mut out: Vec<CapabilityEntry> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for server in &opts.servers {
        let name = server.name.trim().to_owned();
        if name.is_empty() || !seen.insert(name.clone()) {
            continue;
        }
        let status = if server.disabled {
            CapabilityStatus::Disabled
        } else if !server.failed.is_empty() {
            CapabilityStatus::Failed
        } else if server.connected {
            CapabilityStatus::Ready
        } else if !server.cache_key_ok && !server.cached_tools.is_empty() {
            CapabilityStatus::Stale
        } else {
            CapabilityStatus::Configured
        };
        let mut entry = CapabilityEntry {
            id: format!("mcp-server:{name}"),
            kind: CapabilityKind::McpServer,
            name: name.clone(),
            description: format!("MCP server {name}"),
            source: name.clone(),
            status,
            connect_source: "mcp".to_owned(),
            connect_name: name.clone(),
            auto_start: server.auto_start,
            failure_reason: if server.failed.is_empty() {
                String::new()
            } else {
                server.failed.clone()
            },
            ..Default::default()
        };
        out.push(entry.clone());

        let registry_has_tools = opts.tools.iter().any(|t| t.mcp_server == name);
        let (tool_src, tool_status) = match status {
            CapabilityStatus::Ready if !server.proxy_tools.is_empty() && !registry_has_tools => {
                (&server.proxy_tools[..], CapabilityStatus::Ready)
            }
            _ if status != CapabilityStatus::Ready => (&server.cached_tools[..], status),
            _ => (&[][..], status),
        };
        for ct in tool_src {
            let raw = ct.name.trim().to_owned();
            if raw.is_empty() {
                continue;
            }
            entry = CapabilityEntry {
                id: format!("mcp-tool:{name}/{raw}"),
                kind: CapabilityKind::McpTool,
                name: format!("{name}/{raw}"),
                description: ct.description.trim().to_owned(),
                source: name.clone(),
                status: tool_status,
                read_only: ct.read_only,
                destructive: ct.destructive,
                tool_name: ct.tool_name.clone(),
                connect_source: "mcp".to_owned(),
                connect_name: name.clone(),
                auto_start: server.auto_start,
                ..Default::default()
            };
            out.push(entry);
        }
    }
    out
}

impl CapabilityCatalog {
    /// Exact ID first, then declared aliases, then a unique tool-name match.
    /// Exact IDs win over aliases so a namespace can never shadow a real entry
    /// (Go `Catalog.Lookup`). Two tools answering to one name is refused, not
    /// guessed.
    pub fn lookup(&self, id: &str) -> Option<&CapabilityEntry> {
        let id = id.trim();
        if id.is_empty() {
            return None;
        }
        if let Some(entry) = self.entries.iter().find(|e| e.id == id) {
            return Some(entry);
        }
        if let Some(entry) = self
            .entries
            .iter()
            .find(|e| e.aliases.iter().any(|a| a == id))
        {
            return Some(entry);
        }
        let by_name: Vec<&CapabilityEntry> = self
            .entries
            .iter()
            .filter(|e| !e.tool_name.is_empty() && e.tool_name == id)
            .collect();
        if by_name.len() == 1 {
            Some(by_name[0])
        } else {
            None
        }
    }

    /// Whether every dependency exists and is ready (Go `RequiresReady`).
    pub fn requires_ready(&self, requires: &[String]) -> (bool, Vec<String>) {
        let mut missing = Vec::new();
        for dep in requires {
            let dep = dep.trim();
            if dep.is_empty() {
                continue;
            }
            match self.lookup(dep) {
                Some(entry) if entry.status == CapabilityStatus::Ready => {}
                _ => missing.push(dep.to_owned()),
            }
        }
        (missing.is_empty(), missing)
    }
}

fn status_rank(status: CapabilityStatus) -> u8 {
    match status {
        CapabilityStatus::Ready => 4,
        CapabilityStatus::Configured => 3,
        CapabilityStatus::Stale => 2,
        CapabilityStatus::Failed => 1,
        CapabilityStatus::Disabled => 0,
    }
}

fn catalog_fingerprint(entries: &[CapabilityEntry]) -> String {
    let mut hasher = Sha256::new();
    for entry in entries {
        hasher.update(entry.id.as_bytes());
        hasher.update(b"|");
        hasher.update(entry.kind.as_str().as_bytes());
        hasher.update(b"|");
        hasher.update(entry.status.as_str().as_bytes());
        hasher.update(b"|");
        hasher.update(entry.auto_use.as_str().as_bytes());
        hasher.update(b"\n");
    }
    let digest = hasher.finalize();
    hex_encode_16(&digest)
}

/// First 16 lowercase hex chars of a digest (Go `hex.EncodeToString(... )[:16]`).
fn hex_encode_16(digest: &[u8]) -> String {
    let mut out = String::with_capacity(16);
    for byte in digest.iter().take(8) {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

fn clean_list(inp: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for raw in inp {
        let v = raw.trim().to_owned();
        if v.is_empty() || !seen.insert(v.clone()) {
            continue;
        }
        out.push(v);
    }
    out
}

fn normalize_auto_use(raw: &str) -> AutoUse {
    match raw.trim().to_lowercase().as_str() {
        "off" => AutoUse::Off,
        "suggest" => AutoUse::Suggest,
        "prefer" => AutoUse::Prefer,
        "require" => AutoUse::Require,
        _ => AutoUse::Unset,
    }
}

fn normalize_profiles(inp: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for raw in inp {
        let p = raw.trim().to_lowercase();
        if matches!(p.as_str(), "economy" | "balanced" | "delivery") && seen.insert(p.clone()) {
            out.push(p);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server(name: &str) -> McpServerInput {
        McpServerInput {
            name: name.to_owned(),
            ..Default::default()
        }
    }

    #[test]
    fn fingerprint_is_stable_and_short() {
        let catalog = build_catalog(CatalogOptions {
            tools: vec![ToolCatalogInput {
                name: "read_file".to_owned(),
                description: "Read a file".to_owned(),
                ..Default::default()
            }],
            skills: vec![SkillCatalogInput {
                name: "commit".to_owned(),
                description: "Commit playbook".to_owned(),
                triggers: vec!["commit".to_owned()],
                ..Default::default()
            }],
            servers: vec![server("acme")],
            skill_tools_ready: true,
            ..Default::default()
        });
        assert_eq!(catalog.fingerprint.len(), 16);
        let again = build_catalog(CatalogOptions {
            tools: vec![ToolCatalogInput {
                name: "read_file".to_owned(),
                description: "Read a file".to_owned(),
                ..Default::default()
            }],
            skills: vec![SkillCatalogInput {
                name: "commit".to_owned(),
                description: "Commit playbook".to_owned(),
                triggers: vec!["commit".to_owned()],
                ..Default::default()
            }],
            servers: vec![server("acme")],
            skill_tools_ready: true,
            ..Default::default()
        });
        assert_eq!(catalog.fingerprint, again.fingerprint);
    }

    #[test]
    fn lookup_prefers_exact_id_then_alias_then_unique_tool_name() {
        let catalog = build_catalog(CatalogOptions {
            tools: vec![ToolCatalogInput {
                name: "read_file".to_owned(),
                description: "Read".to_owned(),
                aliases: vec!["files:read".to_owned()],
                ..Default::default()
            }],
            ..Default::default()
        });
        assert!(catalog.lookup("tool:read_file").is_some());
        assert!(catalog.lookup("files:read").is_some());
        assert!(catalog.lookup("read_file").is_some());
        assert!(catalog.lookup("missing").is_none());
    }

    #[test]
    fn requires_ready_reports_missing_and_unready() {
        let catalog = build_catalog(CatalogOptions {
            tools: vec![ToolCatalogInput {
                name: "read_file".to_owned(),
                description: "Read".to_owned(),
                ..Default::default()
            }],
            servers: vec![McpServerInput {
                name: "dead".to_owned(),
                failed: "boom".to_owned(),
                ..Default::default()
            }],
            ..Default::default()
        });
        let (ready, missing) = catalog.requires_ready(&["tool:read_file".to_owned()]);
        assert!(ready);
        assert!(missing.is_empty());
        let (ready, missing) = catalog.requires_ready(&[
            "tool:read_file".to_owned(),
            "mcp-server:dead".to_owned(),
            "ghost".to_owned(),
        ]);
        assert!(!ready);
        assert_eq!(missing, vec!["mcp-server:dead", "ghost"]);
    }
}

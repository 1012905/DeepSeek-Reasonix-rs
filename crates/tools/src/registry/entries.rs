//! MCP host-identity entries (tools.md §1.7): binding discovery, portable
//! aliases, and the disabled/held policy records. Split out of the original
//! `registry.rs` so that file stays under the per-source line ceiling; the
//! public paths in `crate::registry` are unchanged by the move.

use crate::abi::Tool;
use crate::refusal::Refusal;

/// `mcp.tool_disabled_by_config` — configuration explicitly disabled that
/// MCP tool.
pub const CODE_MCP_TOOL_DISABLED: &str = "mcp.tool_disabled_by_config";

/// `mcp.tool_definition_held` — the definition is new or differs from the
/// one the user approved for its server.
pub const CODE_MCP_TOOL_HELD: &str = "mcp.tool_definition_held";

/// The namespace every MCP tool name carries: `mcp__<server>__<tool>`.
pub const MCP_NAME_PREFIX: &str = "mcp__";

/// Marks a tool a plugin runtime serves, the way [`MCP_NAME_PREFIX`] marks
/// one an MCP server does.
pub const EXTENSION_NAME_PREFIX: &str = "ext__";

/// Whether the name belongs to a namespace a connection supplies once it is
/// up, which a catalog read before then does not list.
pub fn is_connection_name(name: &str) -> bool {
    name.starts_with(MCP_NAME_PREFIX) || name.starts_with(EXTENSION_NAME_PREFIX)
}

/// Splits `mcp__<server>__<tool>` into its parts. `None` for non-MCP names
/// and for malformed names missing either part.
pub fn split_mcp_name(name: &str) -> Option<(String, String)> {
    let rest = name.strip_prefix(MCP_NAME_PREFIX)?;
    let (server, tool) = rest.split_once("__")?;
    if server.is_empty() || tool.is_empty() {
        return None;
    }
    Some((server.to_owned(), tool.to_owned()))
}

/// The host identity of one MCP capability (tools.md §1.7). Host metadata
/// only: it never adds aliases to provider schemas or alters ordering.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct McpBinding {
    pub package: String,
    pub server: String,
    pub raw_name: String,
    pub visible_name: String,
    pub callable_name: String,
    pub capability_id: String,
}

/// The declared MCP identity behind a tool, from its capability set
/// (Go `MCPBindingFor`).
pub fn mcp_binding_for(tool: &dyn Tool) -> Option<McpBinding> {
    let caps = tool.capabilities();
    let meta = caps.get::<dyn McpMetadata>()?;
    let server = meta.mcp_server_name().trim().to_owned();
    let raw = meta.mcp_raw_tool_name().trim().to_owned();
    if server.is_empty() || raw.is_empty() {
        return None;
    }
    let visible = caps
        .get::<dyn McpVisibleMetadata>()
        .map(|v| v.mcp_visible_tool_name().trim().to_owned())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| raw.clone());
    let package = caps
        .get::<dyn McpPackageMetadata>()
        .map(|p| p.mcp_package_name().trim().to_owned())
        .unwrap_or_default();
    Some(McpBinding {
        capability_id: format!("mcp-tool:{server}/{raw}"),
        package,
        server,
        raw_name: raw,
        visible_name: visible,
        callable_name: tool.name().to_owned(),
    })
}

/// MCP identity traits a tool implements through its capability set.
pub trait McpMetadata: crate::abi::Capability {
    fn mcp_server_name(&self) -> String;
    fn mcp_raw_tool_name(&self) -> String;
}
pub trait McpVisibleMetadata: crate::abi::Capability {
    fn mcp_visible_tool_name(&self) -> String;
}
pub trait McpPackageMetadata: crate::abi::Capability {
    fn mcp_package_name(&self) -> String;
}

/// Accepted portable references for a binding (tools.md §1.7). The canonical
/// provider-visible name remains `callable_name`.
pub fn mcp_binding_aliases(b: &McpBinding) -> Vec<String> {
    let mut aliases = vec![
        b.raw_name.clone(),
        b.visible_name.clone(),
        format!("{}/{}", b.server, b.raw_name),
        format!("{}/{}", b.server, b.visible_name),
        b.capability_id.clone(),
        format!("mcp-tool:{}/{}", b.server, b.visible_name),
        format!(
            "{}{}__{}",
            MCP_NAME_PREFIX,
            portable_mcp_part(&b.server),
            portable_mcp_part(&b.raw_name)
        ),
        format!(
            "{}{}__{}",
            MCP_NAME_PREFIX,
            portable_mcp_part(&b.server),
            portable_mcp_part(&b.visible_name)
        ),
    ];
    if !b.package.is_empty() {
        let prefix = format!(
            "{}plugin_{}_{}__",
            MCP_NAME_PREFIX,
            portable_mcp_part(&b.package),
            portable_mcp_part(&b.server)
        );
        aliases.push(format!("{}{}", prefix, portable_mcp_part(&b.raw_name)));
        aliases.push(format!("{}{}", prefix, portable_mcp_part(&b.visible_name)));
    }
    aliases
}

/// Maps every rune outside `[A-Za-z0-9_-]` to `_` (Go `portableMCPPart`).
pub fn portable_mcp_part(s: &str) -> String {
    s.chars()
        .map(|r| {
            if r.is_ascii_alphanumeric() || r == '_' || r == '-' {
                r
            } else {
                '_'
            }
        })
        .collect()
}

/// A tool withheld until its changed definition is re-approved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeldMcp {
    pub binding: McpBinding,
    pub class: HeldMcpClass,
}

/// How a held tool differs from the approved definitions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeldMcpClass {
    Added,
    Changed,
    Unverifiable,
}

impl std::fmt::Display for HeldMcpClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            HeldMcpClass::Added => "added",
            HeldMcpClass::Changed => "changed",
            HeldMcpClass::Unverifiable => "unverifiable",
        })
    }
}

/// The typed refusal for one held tool.
pub fn held_mcp_refusal_for(h: &HeldMcp) -> Refusal {
    Refusal::code(
        CODE_MCP_TOOL_HELD,
        format!(
            "blocked: tool {:?} of MCP server {:?} is held ({}): its definition was not approved by the user",
            h.binding.callable_name, h.binding.server, h.class
        ),
    )
}

/// The server-qualified aliases for one binding. Bare raw and visible names
/// are deliberately excluded: they are ambiguous across servers and may also
/// name an ordinary built-in or a hallucinated tool.
pub(crate) fn disabled_mcp_names(binding: &McpBinding) -> Vec<String> {
    let raw = binding.raw_name.trim();
    let visible = binding.visible_name.trim();
    let mut names: Vec<String> = std::iter::once(binding.callable_name.clone())
        .chain(mcp_binding_aliases(binding))
        .map(|n| n.trim().to_owned())
        .filter(|n| !n.is_empty())
        .collect();
    names.sort();
    names.dedup();
    names.retain(|n| n != raw && n != visible);
    names
}

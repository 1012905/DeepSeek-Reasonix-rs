//! Tool naming, normalization, and roots projection.
//!
//! Oracle: `internal/ext/plugin/plugin.go:1464-1511`, `toolname.go`,
//! `protocol_inbound.go:100-136`. Every permission/hook/audit surface must
//! build names through these functions so the collision hash is never skipped.

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt;

use super::types::McpRoot;

/// Provider limit for a model-visible tool name (`^[a-zA-Z0-9_-]{1,128}$`).
pub const MAX_MODEL_TOOL_NAME: usize = 128;

/// Typed failures from tool catalog naming validation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NamingError {
    EmptyToolName,
    Collision {
        previous: String,
        candidate: String,
        shared: String,
    },
}

impl fmt::Display for NamingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyToolName => f.write_str("MCP tool catalog has empty or colliding names: tools/list returned an empty tool name"),
            Self::Collision { previous, candidate, shared } => write!(
                f,
                "MCP tool catalog has empty or colliding names: tools/list names {previous:?} and {candidate:?} share model-visible name {shared:?}"
            ),
        }
    }
}

impl std::error::Error for NamingError {}

fn invalid_name_chars(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// FNV-1a over the raw bytes, rendered as lowercase hex and cut to six chars.
pub fn short_name_hash(s: &str) -> String {
    let mut hash: u32 = 0x811c9dc5;
    for byte in s.as_bytes() {
        hash ^= u32::from(*byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    let full = alloc::format!("{hash:08x}");
    full[..6].to_string()
}

/// Maps invalid characters to `_`, trims edge `_`, and appends a hash when the
/// raw name changed so sanitized names cannot collide with already-safe ones.
pub fn normalize_name(s: &str) -> String {
    let replaced = invalid_name_chars(s);
    let trimmed = replaced.trim_matches('_');
    let mut out = if trimmed.is_empty() {
        "unnamed".to_string()
    } else {
        trimmed.to_string()
    };
    if out != s {
        out.push('_');
        out.push_str(&short_name_hash(s));
    }
    out
}

/// Namespace prefix for every tool from a server.
pub fn tool_prefix(server: &str) -> String {
    alloc::format!("mcp__{}__", normalize_name(server))
}

/// Permission/hook identity for starting a server on demand. Deliberately
/// outside the `mcp__` tool namespace so permission rules cannot collide.
pub fn mcp_connect_permission_name(server: &str) -> String {
    alloc::format!("mcp_connect__{}", normalize_name(server))
}

/// Shortens the tail so prefix+tail fits the provider limit, preserving the
/// prefix and a hash of the full tail so siblings stay distinct.
pub fn bounded_tool_name(prefix: &str, tail: &str) -> String {
    if prefix.len() + tail.len() <= MAX_MODEL_TOOL_NAME {
        return alloc::format!("{prefix}{tail}");
    }
    let hash = short_name_hash(tail);
    let keep = MAX_MODEL_TOOL_NAME as isize - prefix.len() as isize - hash.len() as isize - 1;
    if keep < 1 {
        return alloc::format!("{prefix}{tail}");
    }
    alloc::format!("{}{}_{}", prefix, &tail[..keep as usize], hash)
}

/// Canonical model-visible name `mcp__<server>__<tool>`.
pub fn tool_name(server: &str, raw: &str) -> String {
    bounded_tool_name(&tool_prefix(server), &normalize_name(raw))
}

/// Canonical model-visible name; the single entry point for every surface.
pub fn model_tool_name(server: &str, raw: &str) -> String {
    tool_name(server, raw)
}

/// Rejects an empty catalog name or two raw names sharing a model-visible name.
pub fn validate_mcp_tool_names(
    server: &str,
    strip_raw_prefix: &str,
    names: &[String],
) -> Result<(), NamingError> {
    let mut seen: Vec<(String, String)> = Vec::new();
    for candidate in names {
        if candidate.trim().is_empty() {
            return Err(NamingError::EmptyToolName);
        }
        let raw = candidate
            .strip_prefix(strip_raw_prefix)
            .unwrap_or(candidate);
        let visible = tool_name(server, raw);
        if let Some((previous, _)) = seen.iter().find(|(_, v)| v == &visible) {
            return Err(NamingError::Collision {
                previous: previous.clone(),
                candidate: candidate.clone(),
                shared: visible,
            });
        }
        seen.push((candidate.clone(), visible));
    }
    Ok(())
}

/// Turns an absolute workspace path into the roots projection (a `file://` URL).
///
/// On Windows a drive volume gets a leading slash; a `//host/share` path maps
/// its first component to the URL host.
pub fn mcp_roots_from_absolute(absolute: &str) -> Vec<McpRoot> {
    let trimmed = absolute.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    let path = trimmed.replace('\\', "/");
    let (host, url_path) = if let Some(after) = path.strip_prefix("//") {
        match after.split_once('/') {
            Some((h, rest)) => (h.to_string(), alloc::format!("/{rest}")),
            None => (after.to_string(), "/".to_string()),
        }
    } else {
        let has_volume = path.len() >= 2
            && path.as_bytes()[0].is_ascii_alphabetic()
            && path.as_bytes()[1] == b':';
        if has_volume {
            (String::new(), alloc::format!("/{path}"))
        } else {
            (String::new(), path.clone())
        }
    };
    let uri = if host.is_empty() {
        alloc::format!("file://{url_path}")
    } else {
        alloc::format!("file://{host}{url_path}")
    };
    let name = path.trim_end_matches('/');
    let name = if name.is_empty() {
        path.clone()
    } else {
        name.rsplit('/')
            .find(|part| !part.is_empty())
            .unwrap_or(name)
            .to_string()
    };
    vec![McpRoot { uri, name }]
}

/// Trim a workspace root and project it. Empty and whitespace-only roots are omitted.
pub fn mcp_roots(workspace_root: &str) -> Vec<McpRoot> {
    let trimmed = workspace_root.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    mcp_roots_from_absolute(trimmed)
}

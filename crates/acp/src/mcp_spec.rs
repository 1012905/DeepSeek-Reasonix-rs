//! ACP-043 MCP spec conversion and transport normalization
//! (`service.go:1494-1536`).
//!
//! The inbound [`McpServerSpec`] stays the loose wire shape; this module converts
//! it into the runtime [`NormalizedMcpSpec`]: the transport is trimmed,
//! lowercased, and folded onto one of `stdio` / `http` / `sse`, command and URL
//! are trimmed, args/env/headers are cloned, and `dir` + `workspace_root` are
//! pinned to the session cwd. The real plugin host is runtime wiring; validation
//! and normalization are checkable here over the wire DTOs.

use std::collections::BTreeMap;

use crate::wire::McpServerSpec;

/// Normalized transport kinds accepted after lowercasing/trim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    Stdio,
    Http,
    Sse,
}

impl Transport {
    /// Wire identifier the runtime spec carries.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Stdio => "stdio",
            Self::Http => "http",
            Self::Sse => "sse",
        }
    }
}

/// Folds a raw `type` value onto the accepted transport set. `None` means the
/// transport is unsupported (`service.go:1501-1521`).
pub fn normalize_transport(raw: &str) -> Option<Transport> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "" => Some(Transport::Stdio),
        "stdio" => Some(Transport::Stdio),
        "http" | "streamable-http" | "streamable_http" => Some(Transport::Http),
        "sse" => Some(Transport::Sse),
        _ => None,
    }
}

/// One converted plugin spec handed to the runtime (`plugin.Spec`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NormalizedMcpSpec {
    pub name: String,
    pub transport: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub url: String,
    pub headers: BTreeMap<String, String>,
    pub dir: String,
    pub workspace_root: String,
}

fn clone_map(input: Option<&BTreeMap<String, String>>) -> BTreeMap<String, String> {
    input.cloned().unwrap_or_default()
}

/// Converts the inbound spec list, applying the per-spec validation order:
/// name first, then transport-specific required field (`command` for stdio,
/// `url` for http/sse). The whole list fails on the first bad entry, matching
/// `mcpSpecs` (`service.go:1495-1535`).
pub fn convert(specs: &[McpServerSpec], cwd: &str) -> Result<Vec<NormalizedMcpSpec>, String> {
    if specs.is_empty() {
        return Ok(Vec::new());
    }
    let mut out = Vec::with_capacity(specs.len());
    for m in specs {
        let name = m.name.trim().to_owned();
        if name.is_empty() {
            return Err("MCP server name is required".to_owned());
        }
        let raw_type = m.r#type.clone().unwrap_or_default();
        let transport = normalize_transport(&raw_type).ok_or_else(|| {
            format!("MCP server {name:?} uses unsupported transport {raw_type:?}")
        })?;

        match transport {
            Transport::Stdio => {
                if m.command.as_deref().unwrap_or("").trim().is_empty() {
                    return Err(format!("MCP server {name:?} command is required"));
                }
            }
            Transport::Http | Transport::Sse => {
                if m.url.as_deref().unwrap_or("").trim().is_empty() {
                    return Err(format!("MCP server {name:?} url is required"));
                }
            }
        }

        out.push(NormalizedMcpSpec {
            name,
            transport: transport.as_str().to_owned(),
            command: m.command.as_deref().unwrap_or("").trim().to_owned(),
            args: m.args.clone().unwrap_or_default(),
            env: clone_map(m.env.as_ref().map(|e| &e.0)),
            url: m.url.as_deref().unwrap_or("").trim().to_owned(),
            headers: clone_map(m.headers.as_ref().map(|h| &h.0)),
            dir: cwd.to_owned(),
            workspace_root: cwd.to_owned(),
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::{McpEnv, McpHeaders};

    fn spec(
        name: &str,
        r#type: Option<&str>,
        command: Option<&str>,
        url: Option<&str>,
    ) -> McpServerSpec {
        McpServerSpec {
            name: name.to_owned(),
            r#type: r#type.map(str::to_owned),
            command: command.map(str::to_owned),
            args: None,
            env: None,
            url: url.map(str::to_owned),
            headers: None,
        }
    }

    #[test]
    fn empty_type_defaults_to_stdio_and_requires_command() {
        let err = convert(&[spec("s", None, None, None)], "/proj").unwrap_err();
        assert_eq!(err, r#"MCP server "s" command is required"#);

        let ok = convert(
            &[spec("s", Some("  STDIO "), Some("  npx  "), None)],
            "/proj",
        )
        .unwrap();
        assert_eq!(ok[0].transport, "stdio");
        assert_eq!(ok[0].command, "npx");
        assert_eq!(ok[0].dir, "/proj");
        assert_eq!(ok[0].workspace_root, "/proj");
    }

    #[test]
    fn http_aliases_fold_to_http_and_sse_stays_sse() {
        for alias in ["http", "streamable-http", "streamable_http"] {
            let out = convert(&[spec("w", Some(alias), None, Some("https://x"))], "/p").unwrap();
            assert_eq!(out[0].transport, "http");
        }
        let sse = convert(
            &[spec("w", Some("sse"), None, Some("https://x/stream"))],
            "/p",
        )
        .unwrap();
        assert_eq!(sse[0].transport, "sse");
    }

    #[test]
    fn url_required_for_http_and_sse() {
        let http_err = convert(&[spec("w", Some("http"), None, None)], "/p").unwrap_err();
        assert_eq!(http_err, r#"MCP server "w" url is required"#);
        let sse_err = convert(&[spec("w", Some("sse"), None, None)], "/p").unwrap_err();
        assert_eq!(sse_err, r#"MCP server "w" url is required"#);
    }

    #[test]
    fn unsupported_transport_reports_raw_type() {
        let err = convert(&[spec("w", Some("grpc"), None, None)], "/p").unwrap_err();
        assert_eq!(err, r#"MCP server "w" uses unsupported transport "grpc""#);
    }

    #[test]
    fn blank_name_is_reported_before_transport_check() {
        let blank = convert(&[spec("   ", Some("grpc"), None, None)], "/p").unwrap_err();
        assert_eq!(blank, "MCP server name is required");
    }

    #[test]
    fn env_and_headers_are_cloned_onto_the_spec() {
        let mut with_env = spec("s", None, Some("npx"), None);
        with_env.env = Some(McpEnv(BTreeMap::from([(
            "HOME".to_owned(),
            "/tmp".to_owned(),
        )])));
        with_env.headers = Some(McpHeaders(BTreeMap::from([(
            "Authorization".to_owned(),
            "Bearer t".to_owned(),
        )])));
        with_env.args = Some(vec!["-y".to_owned()]);
        let out = convert(&[with_env], "/p").unwrap();
        assert_eq!(out[0].env["HOME"], "/tmp");
        assert_eq!(out[0].headers["Authorization"], "Bearer t");
        assert_eq!(out[0].args, vec!["-y"]);
    }
}

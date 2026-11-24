//! ACP-043 behavior coverage: MCP spec conversion, transport normalization,
//! required-field order, and runtime spec pinning (`service.go:1494-1536`).

use reasonix_acp::mcp_spec::{Transport, convert, normalize_transport};
use reasonix_acp::wire::{McpEnv, McpHeaders, McpServerSpec};
use std::collections::BTreeMap;

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
fn transport_folding_table_matches_reference() {
    assert_eq!(normalize_transport(""), Some(Transport::Stdio));
    assert_eq!(normalize_transport("  stdIO "), Some(Transport::Stdio));
    assert_eq!(normalize_transport("http"), Some(Transport::Http));
    assert_eq!(
        normalize_transport("streamable-http"),
        Some(Transport::Http)
    );
    assert_eq!(
        normalize_transport("streamable_http"),
        Some(Transport::Http)
    );
    assert_eq!(normalize_transport("SSE"), Some(Transport::Sse));
    assert_eq!(normalize_transport("grpc"), None);
    assert_eq!(Transport::Http.as_str(), "http");
    assert_eq!(Transport::Sse.as_str(), "sse");
    assert_eq!(Transport::Stdio.as_str(), "stdio");
}

#[test]
fn blank_name_is_reported_before_any_transport_check() {
    let err = convert(&[spec("   ", Some("grpc"), None, None)], "/p").unwrap_err();
    assert_eq!(err, "MCP server name is required");
}

#[test]
fn stdio_requires_command_and_trims_it() {
    let err = convert(&[spec("fs", None, None, None)], "/p").unwrap_err();
    assert_eq!(err, r#"MCP server "fs" command is required"#);

    let ok = convert(&[spec("fs", Some("  STDIO "), Some("  npx  "), None)], "/p").unwrap();
    assert_eq!(ok[0].transport, "stdio");
    assert_eq!(ok[0].command, "npx");
}

#[test]
fn http_aliases_and_sse_require_url_with_folded_transport() {
    for alias in ["http", "streamable-http", "streamable_http"] {
        let err = convert(&[spec("w", Some(alias), None, None)], "/p").unwrap_err();
        assert_eq!(err, r#"MCP server "w" url is required"#);
    }
    let sse_err = convert(&[spec("w", Some("sse"), None, None)], "/p").unwrap_err();
    assert_eq!(sse_err, r#"MCP server "w" url is required"#);

    let http = convert(
        &[spec(
            "w",
            Some("streamable_http"),
            None,
            Some("  https://x  "),
        )],
        "/p",
    )
    .unwrap();
    assert_eq!(http[0].transport, "http");
    assert_eq!(http[0].url, "https://x");

    let sse = convert(
        &[spec("w", Some("sse"), None, Some("https://x/stream"))],
        "/p",
    )
    .unwrap();
    assert_eq!(sse[0].transport, "sse");
}

#[test]
fn unsupported_transport_reports_raw_unfolded_type() {
    let err = convert(&[spec("w", Some("GRPC"), None, None)], "/p").unwrap_err();
    assert_eq!(err, r#"MCP server "w" uses unsupported transport "GRPC""#);
}

#[test]
fn converted_spec_pins_dir_and_workspace_root_to_cwd() {
    let out = convert(&[spec("fs", None, Some("mcp-fs"), None)], "/proj").unwrap();
    assert_eq!(out[0].dir, "/proj");
    assert_eq!(out[0].workspace_root, "/proj");
}

#[test]
fn env_headers_and_args_are_cloned_onto_spec() {
    let mut with_env = spec("fs", None, Some("npx"), None);
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

#[test]
fn empty_spec_list_converts_to_empty_vector() {
    assert!(convert(&[], "/p").unwrap().is_empty());
}

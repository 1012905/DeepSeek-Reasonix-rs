//! Real-process stdio transport lifecycle and env/PATH semantics (MCP-008,
//! MCP-009; contract §4).
//!
//! The peer is this test binary re-executed through the shared support harness,
//! so spawn, the initialize handshake, a tool call, and close run against an
//! actual subprocess rather than an in-memory double.

mod support;

use std::path::PathBuf;
use std::time::Duration;

use reasonix_ext::mcp::stdio::{AGENT_CHILD_ENV, StdioError, StdioOptions, StdioTransport};
use serde_json::{Value, json};
use support::{ENV_MODES, ENV_PROTOCOL, helper_args, helper_exe};

/// Re-exec peer entry: when the helper marker is set the harness serves the
/// protocol loop and exits there; under the normal test run the marker is
/// absent and this is a no-op.
#[test]
fn helper_peer() {
    let _ = support::run_peer_from_env();
}

fn peer_options(
    modes: &str,
    extra_env: Vec<(String, String)>,
    dir: Option<PathBuf>,
    require_approval: bool,
    workspace: &std::path::Path,
) -> StdioOptions {
    let mut env = vec![
        (ENV_PROTOCOL.to_string(), "mcp".to_string()),
        (ENV_MODES.to_string(), modes.to_string()),
    ];
    env.extend(extra_env);
    StdioOptions {
        name: "mcp-peer".into(),
        command: helper_exe().display().to_string(),
        args: helper_args(),
        env,
        dir: dir.map(|dir| dir.display().to_string()).unwrap_or_default(),
        workspace_root: workspace.display().to_string(),
        require_launch_approval: require_approval,
        default_call_timeout: Duration::from_secs(10),
    }
}

fn inspect_env(transport: &StdioTransport) -> Value {
    transport
        .call_result(
            "tools/call",
            Some(&json!({"name": "inspect_env", "arguments": {}})),
        )
        .expect("inspect_env reply")
}

#[test]
fn stdio_spawns_handshakes_lists_tools_and_runs_a_call() {
    let root = tempfile::tempdir().expect("temp workspace");
    let transport = StdioTransport::spawn(peer_options(
        "legacy_only",
        vec![],
        None,
        false,
        root.path(),
    ))
    .expect("spawn peer");

    let era = transport.connect().expect("initialize handshake");
    assert!(era.is_legacy(), "legacy_only peer falls back to initialize");

    let list = transport
        .call_result("tools/list", None)
        .expect("tools/list reply");
    let tools = list["tools"].as_array().expect("tools array");
    assert!(tools.iter().any(|tool| tool["name"] == json!("echo")));

    let call = transport
        .call_result(
            "tools/call",
            Some(&json!({"name": "echo", "arguments": {"msg": "hello"}})),
        )
        .expect("tools/call reply");
    let text = call["content"][0]["text"].as_str().expect("text block");
    assert!(text.contains("hello"), "{text}");

    transport.close();
    support::wait_until("peer process exits", Duration::from_secs(5), || {
        transport.has_exited()
    });
    assert!(transport.has_exited());
}

#[test]
fn stdio_modern_peer_serves_calls_without_initialize() {
    let root = tempfile::tempdir().expect("temp workspace");
    let transport = StdioTransport::spawn(peer_options(
        "modern_only",
        vec![],
        None,
        false,
        root.path(),
    ))
    .expect("spawn peer");

    let era = transport.connect().expect("discover handshake");
    assert!(
        era.is_modern(),
        "modern_only peer negotiates the modern era"
    );

    let list = transport
        .call_era("tools/list", None)
        .expect("tools/list reply");
    let tools = list["tools"].as_array().expect("tools array");
    assert!(tools.iter().any(|tool| tool["name"] == json!("echo")));

    let call = transport
        .call_era(
            "tools/call",
            Some(&json!({"name": "echo", "arguments": {"msg": "modern"}})),
        )
        .expect("tools/call reply");
    let text = call["content"][0]["text"].as_str().expect("text block");
    assert!(text.contains("modern"), "{text}");

    transport.close();
    support::wait_until("peer process exits", Duration::from_secs(5), || {
        transport.has_exited()
    });
}

#[test]
fn stdio_env_injects_overrides_and_identity_marker_cannot_be_forged() {
    let root = tempfile::tempdir().expect("temp workspace");
    let transport = StdioTransport::spawn(peer_options(
        "legacy_only",
        vec![
            ("REASONIX_TEST_INJECTED".into(), "manifest-value".into()),
            (AGENT_CHILD_ENV.into(), "spoofed".into()),
        ],
        None,
        false,
        root.path(),
    ))
    .expect("spawn peer");
    transport.connect().expect("handshake");

    let inspect = inspect_env(&transport);
    let env = inspect["structuredContent"]["env"]
        .as_object()
        .expect("env object");
    assert!(
        !env.get("PATH")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .is_empty(),
        "host PATH is inherited by the child"
    );
    assert_eq!(
        env.get("REASONIX_TEST_INJECTED").and_then(Value::as_str),
        Some("manifest-value"),
        "manifest env reaches the child"
    );
    assert_eq!(
        env.get(AGENT_CHILD_ENV).and_then(Value::as_str),
        Some("1"),
        "the forced identity marker wins over a manifest spelling"
    );

    transport.close();
    support::wait_until("peer process exits", Duration::from_secs(5), || {
        transport.has_exited()
    });
}

#[test]
fn stdio_explicit_dir_becomes_the_child_cwd() {
    let root = tempfile::tempdir().expect("temp workspace");
    let explicit = tempfile::tempdir().expect("explicit cwd");
    let transport = StdioTransport::spawn(peer_options(
        "legacy_only",
        vec![],
        Some(explicit.path().to_path_buf()),
        false,
        root.path(),
    ))
    .expect("spawn peer");
    transport.connect().expect("handshake");

    let inspect = inspect_env(&transport);
    let cwd = inspect["structuredContent"]["cwd"]
        .as_str()
        .expect("child cwd");
    assert_eq!(
        std::fs::canonicalize(cwd).expect("child cwd"),
        std::fs::canonicalize(explicit.path()).expect("explicit dir")
    );

    transport.close();
    support::wait_until("peer process exits", Duration::from_secs(5), || {
        transport.has_exited()
    });
}

#[test]
fn stdio_approval_server_falls_back_to_workspace_cwd() {
    let root = tempfile::tempdir().expect("workspace root");
    let transport =
        StdioTransport::spawn(peer_options("legacy_only", vec![], None, true, root.path()))
            .expect("spawn peer");
    transport.connect().expect("handshake");

    let inspect = inspect_env(&transport);
    let cwd = inspect["structuredContent"]["cwd"]
        .as_str()
        .expect("child cwd");
    assert_eq!(
        std::fs::canonicalize(cwd).expect("child cwd"),
        std::fs::canonicalize(root.path()).expect("workspace root")
    );

    transport.close();
    support::wait_until("peer process exits", Duration::from_secs(5), || {
        transport.has_exited()
    });
}

#[test]
fn stdio_bare_command_resolves_through_path() {
    let root = tempfile::tempdir().expect("temp workspace");
    let bin_root = tempfile::tempdir().expect("bin root");
    let bin_dir = bin_root.path().join("bin");
    std::fs::create_dir_all(&bin_dir).expect("bin dir");
    let exe_name = if cfg!(windows) {
        "mcp-peer-bin.exe"
    } else {
        "mcp-peer-bin"
    };
    let copied = bin_dir.join(exe_name);
    std::fs::copy(helper_exe(), &copied).expect("copy peer binary");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&copied, std::fs::Permissions::from_mode(0o755))
            .expect("executable bit");
    }

    let path = std::env::join_paths(std::iter::once(bin_dir.as_path()))
        .expect("PATH")
        .to_string_lossy()
        .to_string();
    let mut options = peer_options(
        "legacy_only",
        vec![("PATH".into(), path)],
        None,
        false,
        root.path(),
    );
    options.command = "mcp-peer-bin".into();

    let transport = StdioTransport::spawn(options).expect("spawn peer via PATH lookup");
    transport
        .connect()
        .expect("handshake over PATH-resolved peer");

    transport.close();
    support::wait_until("peer process exits", Duration::from_secs(5), || {
        transport.has_exited()
    });
}

#[test]
fn stdio_missing_bare_command_is_refused_by_identity() {
    let root = tempfile::tempdir().expect("temp workspace");
    let mut options = peer_options("legacy_only", vec![], None, false, root.path());
    options.command = "definitely-no-such-reasonix-peer-xyzzy".into();

    let err = StdioTransport::spawn(options).expect_err("missing command must fail");
    assert!(
        matches!(err, StdioError::CommandMissing { .. }),
        "want CommandMissing identity, got {err:?}"
    );
}

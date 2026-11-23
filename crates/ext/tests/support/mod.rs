//! Shared test support: a re-exec NDJSON peer and installed-package fixtures.
//!
//! The peer is this test binary re-executed with `REASONIX_HELPER_PROTOCOL` set
//! (the standard helper-process pattern), so no third-party MCP server or
//! extension binary is needed and every test stays hermetic. Behavior is steered
//! entirely through environment variables the fixture writes into a package
//! manifest.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

/// Marker that turns the helper test function into the peer.
pub const ENV_PROTOCOL: &str = "REASONIX_HELPER_PROTOCOL";
/// Comma-separated failure injections.
pub const ENV_MODES: &str = "REASONIX_HELPER_MODES";
/// Raw `InitializeResult` JSON the sidecar peer answers with.
pub const ENV_INIT: &str = "REASONIX_HELPER_INIT";
/// Exits before serving anything.
pub const ENV_CRASH_ON_BOOT: &str = "REASONIX_HELPER_CRASH_ON_BOOT";

/// Arguments that re-execute this test binary as the helper.
///
/// `--nocapture` is load-bearing: the default harness buffers stdout, which
/// would hold the protocol frames until the process exits.
pub fn helper_args() -> Vec<String> {
    vec![
        "helper_peer".to_string(),
        "--exact".to_string(),
        "--nocapture".to_string(),
    ]
}

/// The absolute path of this test binary.
pub fn helper_exe() -> PathBuf {
    std::env::current_exe().expect("test binary path")
}

/// Runs the peer when the helper marker is set; returns whether it did.
///
/// Every test binary in this crate calls this from one `#[test]` named
/// `helper_peer`; the fixture targets that name with `--exact`.
pub fn run_peer_from_env() -> bool {
    let Ok(protocol) = std::env::var(ENV_PROTOCOL) else {
        return false;
    };
    let modes = Modes::parse(&std::env::var(ENV_MODES).unwrap_or_default());
    if std::env::var(ENV_CRASH_ON_BOOT).is_ok() {
        std::process::exit(3);
    }
    if modes.stderr_flood {
        // Push past any reasonable tail, then leave a credential-shaped line
        // last: that line has to survive in the ring and be redacted on the way
        // out, which proves the bound and the scrub together.
        let mut stderr = std::io::stderr();
        for index in 0..4096 {
            let _ = writeln!(stderr, "helper boot line {index} padding padding padding");
        }
        let _ = writeln!(
            stderr,
            "boot failed: api_key=sk-abcdef1234567890SECRETKEY is invalid"
        );
    }
    let init = std::env::var(ENV_INIT).unwrap_or_else(|_| {
        r#"{"protocolVersion":"2","name":"helper","version":"1.0.0","stateSchemaVersion":0}"#
            .to_string()
    });
    let mut peer = Peer::new();
    match protocol.as_str() {
        "sidecar" => run_sidecar(&mut peer, &init, &modes),
        "mcp" => run_mcp(&mut peer, &modes),
        other => panic!("unknown helper protocol {other:?}"),
    }
    true
}

#[derive(Debug, Default, Clone)]
struct Modes {
    early_request: bool,
    early_notify: bool,
    stall_handshake: bool,
    ignore_shutdown: bool,
    crash_after_init: bool,
    stderr_flood: bool,
    probe_unanswered: bool,
    legacy_only: bool,
    modern_only: bool,
    server_request: bool,
    stall_intercept: bool,
}

impl Modes {
    fn parse(raw: &str) -> Self {
        let mut modes = Self::default();
        for mode in raw
            .split(',')
            .map(str::trim)
            .filter(|mode| !mode.is_empty())
        {
            match mode {
                "early_request" => modes.early_request = true,
                "early_notify" => modes.early_notify = true,
                "stall_handshake" => modes.stall_handshake = true,
                "ignore_shutdown" => modes.ignore_shutdown = true,
                "crash_after_init" => modes.crash_after_init = true,
                "stderr_flood" => modes.stderr_flood = true,
                "probe_unanswered" => modes.probe_unanswered = true,
                "legacy_only" => modes.legacy_only = true,
                "modern_only" => modes.modern_only = true,
                "server_request" => modes.server_request = true,
                "stall_intercept" => modes.stall_intercept = true,
                other => panic!("unknown helper mode {other:?}"),
            }
        }
        modes
    }
}

struct Peer {
    out: std::io::Stdout,
}

impl Peer {
    fn new() -> Self {
        Self {
            out: std::io::stdout(),
        }
    }

    fn send(&mut self, value: &Value) {
        let mut stdout = self.out.lock();
        let _ = serde_json::to_writer(&mut stdout, value);
        let _ = stdout.write_all(b"\n");
        let _ = stdout.flush();
    }

    fn respond(&mut self, id: &Value, result: Value) {
        self.send(&json!({"jsonrpc": "2.0", "id": id, "result": result}));
    }

    fn respond_error(&mut self, id: &Value, code: i64, message: &str) {
        self.send(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {"code": code, "message": message}
        }));
    }
}

fn run_sidecar(peer: &mut Peer, init: &str, modes: &Modes) {
    let stdin = std::io::stdin();
    let mut reader = BufReader::new(stdin.lock());
    let init_value: Value = serde_json::from_str(init).expect("helper init is JSON");
    loop {
        let Some(frame) = read_frame(&mut reader) else {
            return;
        };
        let Some(method) = frame.get("method").and_then(Value::as_str) else {
            continue;
        };
        let id = frame.get("id").cloned().unwrap_or(Value::Null);
        let is_request = frame.get("id").is_some();
        match method {
            "extension/initialize" => {
                if modes.stall_handshake {
                    continue;
                }
                if modes.early_request {
                    peer.send(&json!({
                        "jsonrpc": "2.0",
                        "id": 9001,
                        "method": "host/content/read",
                        "params": {"contentRef": "content_none", "offset": 0}
                    }));
                }
                if modes.early_notify {
                    peer.send(&json!({
                        "jsonrpc": "2.0",
                        "method": "extension/provider/stream/chunk",
                        "params": {"streamId": "s1", "seq": 1}
                    }));
                }
                peer.respond(&id, init_value.clone());
            }
            "extension/initialized" => {
                if modes.crash_after_init {
                    std::process::exit(0);
                }
            }
            "extension/intercept" => {
                if !modes.stall_intercept {
                    peer.respond(&id, json!({"decision": "continue"}));
                }
            }
            "extension/event" | "extension/resources/changed" => {}
            "extension/shutdown" => {
                if modes.ignore_shutdown {
                    continue;
                }
                peer.respond(&id, json!({"accepted": true}));
                return;
            }
            other => {
                if is_request {
                    peer.respond_error(&id, -32601, &format!("method not found: {other}"));
                }
            }
        }
    }
}

fn run_mcp(peer: &mut Peer, modes: &Modes) {
    let stdin = std::io::stdin();
    let mut reader = BufReader::new(stdin.lock());
    loop {
        let Some(frame) = read_frame(&mut reader) else {
            return;
        };
        let Some(method) = frame.get("method").and_then(Value::as_str) else {
            continue;
        };
        let id = frame.get("id").cloned().unwrap_or(Value::Null);
        let is_request = frame.get("id").is_some();
        match method {
            "initialize" => {
                peer.respond(
                    &id,
                    json!({
                        "protocolVersion": "2025-06-18",
                        "capabilities": {"tools": {}},
                        "serverInfo": {"name": "helper", "version": "1.0.0"},
                        "instructions": "helper server"
                    }),
                );
            }
            "server/discover" => {
                if modes.probe_unanswered {
                    continue;
                }
                if modes.legacy_only {
                    // A legacy server may answer a pre-initialize method with
                    // an implementation-defined code; that must fall back to
                    // the initialize handshake rather than read as modern.
                    peer.respond_error(&id, -32601, "method not found");
                    continue;
                }
                let versions = if modes.modern_only {
                    json!(["2026-07-28"])
                } else {
                    json!(["2026-07-28", "2025-11-25"])
                };
                peer.respond(
                    &id,
                    json!({
                        "supportedVersions": versions,
                        "capabilities": {"tools": {}},
                        "instructions": "helper server"
                    }),
                );
            }
            "tools/list" => {
                peer.respond(
                    &id,
                    json!({"tools": [{"name": "echo", "description": "echo", "inputSchema": {"type": "object"}}]}),
                );
                if modes.server_request {
                    peer.send(&json!({"jsonrpc": "2.0", "id": 7001, "method": "ping"}));
                }
            }
            "ping" => {
                peer.respond(&id, json!({}));
            }
            "notifications/initialized" | "notifications/cancelled" => {}
            other => {
                if is_request {
                    peer.respond_error(&id, -32601, &format!("method not found: {other}"));
                }
            }
        }
    }
}

fn read_frame(reader: &mut impl BufRead) -> Option<Value> {
    let mut line = Vec::new();
    match reader.read_until(b'\n', &mut line) {
        Ok(0) | Err(_) => None,
        Ok(_) => {
            let payload = line.trim_ascii();
            if payload.is_empty() {
                return Some(Value::Null);
            }
            serde_json::from_slice(payload).ok()
        }
    }
}

/// A temporary Reasonix home with one installed runtime package.
pub struct Home {
    dir: tempfile::TempDir,
}

impl Home {
    pub fn new() -> Self {
        Self {
            dir: tempfile::tempdir().expect("temp home"),
        }
    }

    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    /// Installs one package whose runtime command is this test binary.
    ///
    /// `init` is the raw `InitializeResult` a sidecar peer answers with; an
    /// empty string keeps the peer's default declaration.
    pub fn install(&self, name: &str, init: &str, modes: &[&str]) -> PackageFixture {
        self.install_with(name, init, modes, |_| {})
    }

    /// Same as `install`, with a hook to adjust the runtime declaration.
    pub fn install_with(
        &self,
        name: &str,
        init: &str,
        modes: &[&str],
        configure: impl FnOnce(&mut BTreeMap<String, Value>),
    ) -> PackageFixture {
        let root = self.path().join("plugins").join(name);
        std::fs::create_dir_all(&root).expect("package root");
        let mut runtime = BTreeMap::new();
        runtime.insert(
            "command".to_string(),
            json!(helper_exe().display().to_string()),
        );
        runtime.insert("args".to_string(), json!(helper_args()));
        let mut env = BTreeMap::new();
        env.insert(ENV_PROTOCOL.to_string(), json!("sidecar"));
        env.insert(ENV_MODES.to_string(), json!(modes.join(",")));
        if !init.is_empty() {
            env.insert(ENV_INIT.to_string(), json!(init));
        }
        runtime.insert("env".to_string(), json!(env));
        configure(&mut runtime);
        let manifest = json!({
            "apiVersion": "reasonix.io/plugin/v2",
            "name": name,
            "version": "1.0.0",
            "runtime": runtime,
        });
        std::fs::write(
            root.join("reasonix-plugin.json"),
            serde_json::to_vec_pretty(&manifest).expect("manifest"),
        )
        .expect("write manifest");
        self.register(name, true);
        PackageFixture {
            name: name.to_string(),
            root,
        }
    }

    /// Registers one installed-state entry; the runtime comes from the manifest.
    pub fn register(&self, name: &str, enabled: bool) {
        let state_path = self.path().join("plugin-packages.json");
        let mut state: Value = std::fs::read(&state_path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_else(|| json!({"version": 1, "plugins": []}));
        let plugins = state["plugins"].as_array_mut().expect("plugins array");
        plugins.retain(|entry| entry["name"] != json!(name));
        plugins.push(json!({
            "name": name,
            "root": format!("plugins/{name}"),
            "version": "1.0.0",
            "enabled": enabled
        }));
        std::fs::write(
            &state_path,
            serde_json::to_vec_pretty(&state).expect("state"),
        )
        .expect("write state");
    }
}

/// The installed package a fixture created.
pub struct PackageFixture {
    pub name: String,
    pub root: PathBuf,
}

/// The session every fixture uses: a temp workspace is enough identity.
pub fn session(home: &Home) -> reasonix_ext::lifecycle::SessionContext {
    reasonix_ext::lifecycle::SessionContext {
        session_id: "test-session".into(),
        workspace_root: home.path().display().to_string(),
        generation: 1,
        epoch: String::new(),
    }
}

/// A valid declaration for a plugin whose manifest declares these capabilities.
pub fn init_result(name: &str, body: &str) -> String {
    format!(
        r#"{{"protocolVersion":"2","name":"{name}","version":"1.0.0","stateSchemaVersion":0{body}}}"#
    )
}

/// Waits until `predicate` holds, or panics after the budget.
pub fn wait_until(label: &str, budget: std::time::Duration, mut predicate: impl FnMut() -> bool) {
    let started = std::time::Instant::now();
    while started.elapsed() < budget {
        if predicate() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("timed out waiting for {label}");
}

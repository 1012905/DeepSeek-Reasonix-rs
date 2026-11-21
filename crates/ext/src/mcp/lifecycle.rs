//! Spec, Host, and start lifecycle: policy, phases, and typed state.
//!
//! Oracle: `internal/ext/plugin/plugin.go:38-65,70-165,250-575,769-1326`,
//! `runtime_spec.go`. Subprocess/network work is deferred to the transport
//! wave; this module owns the testable policy and state transitions.

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt;

use crate::mcp::naming::{normalize_name, tool_prefix};

/// Finite call deadline when neither context nor config supplies one (ms).
pub const DEFAULT_CALL_TIMEOUT_MS: u64 = 300_000;
/// Process-storm guardrail for batch startup.
pub const DEFAULT_START_CONCURRENCY: usize = 8;
/// Per-plugin budget used by `StartAvailable` (ms).
pub const DEFAULT_START_TIMEOUT_MS: u64 = 5_000;
/// Bounded startup window for servers that advertise but list no tools (ms).
pub const ADVERTISED_TOOLS_EMPTY_LIST_RETRY_DELAYS_MS: [u64; 3] = [50, 150, 300];

/// Named phases of a start, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum StartPhase {
    /// Resolve launcher, authorization, transport, and connect.
    Launch,
    /// Legacy initialize handshake (modern era skips it).
    Initialize,
    /// `tools/list`, cached before phase B.
    ToolsList,
    /// Async `prompts/list` and `resources/list`, when advertised.
    PromptsResources,
}

/// All start phases in execution order.
pub const START_PHASES: [StartPhase; 4] = [
    StartPhase::Launch,
    StartPhase::Initialize,
    StartPhase::ToolsList,
    StartPhase::PromptsResources,
];

impl StartPhase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Launch => "launch",
            Self::Initialize => "initialize",
            Self::ToolsList => "tools/list",
            Self::PromptsResources => "surface-b",
        }
    }
}

/// How a local stdio MCP process is launched. Host is the product default.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum McpProcessMode {
    #[default]
    Host,
    Confined,
}

impl McpProcessMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Host => "host",
            Self::Confined => "confined",
        }
    }

    pub fn parse(raw: &str) -> Self {
        match raw {
            "confined" => Self::Confined,
            _ => Self::Host,
        }
    }
}

/// Canonical transport kind. Empty spec type defaults to stdio.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TransportKind {
    #[default]
    Stdio,
    StreamableHttp,
    Sse,
}

impl TransportKind {
    pub fn canonical(self) -> &'static str {
        match self {
            Self::Stdio => "stdio",
            Self::StreamableHttp => "streamable-http",
            Self::Sse => "sse",
        }
    }
}

/// Parses a spec transport type; unknown values are an error, not a guess.
pub fn parse_transport_type(raw: &str) -> Result<TransportKind, LifecycleError> {
    match raw {
        "" | "stdio" => Ok(TransportKind::Stdio),
        "http" | "streamable-http" | "streamable_http" => Ok(TransportKind::StreamableHttp),
        "sse" => Ok(TransportKind::Sse),
        other => Err(LifecycleError::UnknownTransportType(other.to_string())),
    }
}

/// Canonical runtime transport for identity comparison: empty/stdio → stdio,
/// HTTP aliases → streamable-http, sse → sse, else lowercased as-is.
pub fn canonical_mcp_runtime_transport(raw: &str) -> String {
    match raw.to_ascii_lowercase().as_str() {
        "" | "stdio" => "stdio".to_string(),
        "http" | "streamable-http" | "streamable_http" => "streamable-http".to_string(),
        "sse" => "sse".to_string(),
        other => other.to_string(),
    }
}

/// Declares one external MCP server. Durations are milliseconds; host-only
/// resources (writers, manager pointers, callbacks) are carried as flags.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Spec {
    pub name: String,
    /// Installed package provenance; excluded from fingerprints.
    pub package: String,
    pub transport: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub url: String,
    pub headers: BTreeMap<String, String>,
    /// Exact server-local names excluded before schema registration.
    pub disabled_tools: Vec<String>,
    pub default_startup_timeout_ms: u64,
    pub startup_timeout_ms: u64,
    pub default_call_timeout_ms: u64,
    pub call_timeout_ms: u64,
    /// Per raw tool name (server-local names).
    pub tool_timeouts_ms: BTreeMap<String, u64>,
    pub dir: String,
    pub workspace_root: String,
    pub writable_roots: Vec<String>,
    pub config_source: String,
    /// The single runtime authorization result; tools carry no trust bit.
    pub authorized: bool,
    pub require_launch_approval: bool,
    /// A launch check was configured (callback body belongs to the host wave).
    pub has_launch_check: bool,
    pub launch_args: Vec<String>,
    pub launcher_identity_args: Vec<String>,
    pub launcher_locator: String,
    pub launcher_resolved_version: String,
    pub launcher_digest: String,
    pub process_mode: McpProcessMode,
    pub state_dir: String,
    pub process_state_dir: String,
    pub oauth_allow_missing_pkce_metadata: bool,
    pub oauth_scope_upgrade: bool,
    /// Prefix removed from raw names before namespacing; protocol calls keep
    /// the original.
    pub strip_raw_prefix: String,
    pub low_priority: bool,
}

impl Spec {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Self::default()
        }
    }

    /// Empty mode resolves to host; only confined is ever wrapped.
    pub fn resolved_process_mode(&self) -> McpProcessMode {
        self.process_mode
    }

    pub fn transport_kind(&self) -> Result<TransportKind, LifecycleError> {
        parse_transport_type(&self.transport)
    }

    pub fn effective_startup_timeout_ms(&self) -> u64 {
        if self.startup_timeout_ms > 0 {
            self.startup_timeout_ms
        } else if self.default_startup_timeout_ms > 0 {
            self.default_startup_timeout_ms
        } else {
            DEFAULT_START_TIMEOUT_MS
        }
    }

    pub fn effective_call_timeout_ms(&self) -> u64 {
        if self.call_timeout_ms > 0 {
            self.call_timeout_ms
        } else if self.default_call_timeout_ms > 0 {
            self.default_call_timeout_ms
        } else {
            DEFAULT_CALL_TIMEOUT_MS
        }
    }

    pub fn server_authorized(&self) -> bool {
        self.authorized
    }

    pub fn tool_enabled(&self, raw: &str) -> bool {
        !self.disabled_tools.iter().any(|d| d == raw)
    }
}

/// Batch startup policy. The zero value disables every safeguard.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StartPolicy {
    pub per_plugin_timeout_ms: u64,
    /// Zero or negative means no cap in Go; represented as `None` here.
    pub concurrency: Option<usize>,
    pub abort_on_error: bool,
    pub skip_persistence: bool,
}

impl StartPolicy {
    /// `StartAll` semantics: concurrency 8, abort on first error.
    pub fn start_all() -> Self {
        Self {
            per_plugin_timeout_ms: 0,
            concurrency: Some(DEFAULT_START_CONCURRENCY),
            abort_on_error: true,
            skip_persistence: false,
        }
    }

    /// `StartAvailable` semantics: 5s per plugin, keep going on failure.
    pub fn start_available() -> Self {
        Self {
            per_plugin_timeout_ms: DEFAULT_START_TIMEOUT_MS,
            concurrency: Some(DEFAULT_START_CONCURRENCY),
            abort_on_error: false,
            skip_persistence: false,
        }
    }
}

/// Typed lifecycle failures, each with an independent identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LifecycleError {
    ServerAlreadyConnected(String),
    SpawningInFlight(String),
    DeferredSpawnCancelled,
    HostClosed,
    ServerRemoved,
    UnknownTransportType(String),
    AmbiguousCatalog(String),
    Startup {
        server: String,
        phase: &'static str,
        detail: String,
    },
}

impl fmt::Display for LifecycleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ServerAlreadyConnected(name) => {
                write!(f, "plugin server already connected: {name:?}")
            }
            Self::SpawningInFlight(name) => {
                write!(f, "server spawn already in progress: {name:?}")
            }
            Self::DeferredSpawnCancelled => f.write_str("deferred MCP spawn cancelled"),
            Self::HostClosed => {
                f.write_str("the MCP host shut down (session ended or runtime rebuilt)")
            }
            Self::ServerRemoved => {
                f.write_str("this MCP server was removed, disabled, or reconnected")
            }
            Self::UnknownTransportType(raw) => {
                write!(f, "unknown transport type {raw:?} (want stdio|http|sse)")
            }
            Self::AmbiguousCatalog(detail) => f.write_str(detail),
            Self::Startup {
                server,
                phase,
                detail,
            } => {
                write!(f, "start plugin {server:?} failed in {phase}: {detail}")
            }
        }
    }
}

impl std::error::Error for LifecycleError {}

/// A failure recorded with the phase and elapsed time that produced it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StartupFailure {
    pub phase: &'static str,
    pub elapsed_ms: u64,
    pub stderr_summary: String,
    pub detail: String,
}

pub fn new_startup_failure(
    phase: &'static str,
    elapsed_ms: u64,
    stderr_summary: String,
    detail: String,
) -> StartupFailure {
    StartupFailure {
        phase,
        elapsed_ms,
        stderr_summary,
        detail,
    }
}

/// One spec's slot in a start plan. Result ordering matches spec order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StartPlanEntry {
    pub index: usize,
    pub spec: Spec,
}

/// Testable projection of a batch start: capped concurrency and stable order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StartPlan {
    pub entries: Vec<StartPlanEntry>,
    pub effective_concurrency: usize,
    pub policy: StartPolicy,
    pub phases: Vec<StartPhase>,
}

/// Effective concurrency: no cap → all specs; otherwise capped to len(specs).
pub fn effective_concurrency(policy: StartPolicy, spec_count: usize) -> usize {
    match policy.concurrency {
        None => spec_count,
        Some(n) if n == 0 || n > spec_count => spec_count,
        Some(n) => n,
    }
}

/// Builds a start plan without spawning anything.
pub fn build_start_plan(
    specs: Vec<Spec>,
    policy: StartPolicy,
) -> Result<StartPlan, LifecycleError> {
    for spec in &specs {
        spec.transport_kind()?;
    }
    let count = specs.len();
    let entries = specs
        .into_iter()
        .enumerate()
        .map(|(index, spec)| StartPlanEntry { index, spec })
        .collect();
    Ok(StartPlan {
        entries,
        effective_concurrency: effective_concurrency(policy, count),
        policy,
        phases: START_PHASES.to_vec(),
    })
}

/// Result of a removal: the namespace prefix reclaimed and whether it existed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Removal {
    pub prefix: String,
    pub found: bool,
}

/// A connected server's recorded lifecycle state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConnectedServer {
    pub spec: Spec,
    pub generation: u64,
    pub phase: StartPhase,
    pub tools_listed: bool,
}

/// Host state, kept independent of any transport for testability. `Host::new`
/// guarantees pointer stability for the session; servers are keyed by name.
#[derive(Clone, Debug, Default)]
pub struct Host {
    servers: BTreeMap<String, ConnectedServer>,
    generation: u64,
    closed: bool,
}

impl Host {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_closed(&self) -> bool {
        self.closed
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn get(&self, name: &str) -> Option<&ConnectedServer> {
        self.servers.get(name)
    }

    pub fn len(&self) -> usize {
        self.servers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.servers.is_empty()
    }

    pub fn names(&self) -> Vec<&str> {
        self.servers.keys().map(String::as_str).collect()
    }

    /// Adds a connected server, refusing a duplicate name by identity.
    pub fn connect(&mut self, spec: Spec) -> Result<(), LifecycleError> {
        if self.closed {
            return Err(LifecycleError::HostClosed);
        }
        if self.servers.contains_key(&spec.name) {
            return Err(LifecycleError::ServerAlreadyConnected(spec.name.clone()));
        }
        let name = spec.name.clone();
        self.generation += 1;
        self.servers.insert(
            name,
            ConnectedServer {
                spec,
                generation: self.generation,
                phase: StartPhase::Launch,
                tools_listed: false,
            },
        );
        Ok(())
    }

    pub fn advance_phase(&mut self, name: &str, phase: StartPhase) -> Result<(), LifecycleError> {
        let server = self
            .servers
            .get_mut(name)
            .ok_or(LifecycleError::ServerRemoved)?;
        server.phase = phase;
        if phase == StartPhase::ToolsList {
            server.tools_listed = true;
        }
        Ok(())
    }

    /// Removes a server and returns the namespaced prefix it owned.
    pub fn remove(&mut self, name: &str) -> Removal {
        let found = self.servers.remove(name).is_some();
        self.generation += 1;
        Removal {
            prefix: tool_prefix(name),
            found,
        }
    }

    /// Closes the host. Idempotent; cancels deferred spawns.
    pub fn close(&mut self) {
        self.closed = true;
        self.generation += 1;
    }
}

/// A deferred spawn carries its generation; a stale one is cancelled.
pub fn check_deferred_generation(
    host_generation: u64,
    spawn_generation: u64,
) -> Result<(), LifecycleError> {
    if host_generation == spawn_generation {
        Ok(())
    } else {
        Err(LifecycleError::DeferredSpawnCancelled)
    }
}

/// Name used for a single-flight spawn guard: the normalized server name.
pub fn spawn_key(name: &str) -> String {
    normalize_name(name)
}

//! One live sidecar connection: handshake gate, launch, and bounded teardown.
//!
//! Parity target: `internal/ext/extension/sidecar/client.go`. The host sends
//! `extension/initialize` first — its only opening move. Any extension-to-host
//! request or notification before `extension/initialized` poisons the
//! connection: the gate runs on the read loop, so the decision observes wire
//! arrival order, and the process tree is killed so a pending handshake
//! unwinds immediately instead of waiting out its budget.

use std::io::BufReader;
use std::process::{ChildStdin, ChildStdout};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::enums::{InterceptEvent, PROTOCOL_ID, PROTOCOL_VERSION, UIHostKind};
use crate::error::{ConnError, ErrorReason, ProtocolError, ProtocolErrorData, rpc_code};
use crate::lifecycle::{
    DEPENDENCY_SCHEMA_VERSION, HandshakePhase, HostCapabilities, InitializeParams,
    InitializeResult, InitializedParams, ManifestExpectation, SessionContext,
    validate_handshake_result,
};
use crate::rawjson::RawJson;
use crate::redact::redact_error_text;
use crate::rpcwire::{Conn, ConnOptions, HandlerResponse, RpcHandlerError};
use crate::sidecar::installed::{CapabilityRef, InstalledRuntimePackage};
use crate::sidecar::process::{CLOSE_WAIT, SidecarProcess, SpawnError};
use crate::sidecar::{StartupFailure, startup_failure};

/// Budget for the `extension/initialize` round trip.
pub const DEFAULT_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(30);
/// Budget for `extension/shutdown` before the bounded process close takes over.
pub const DEFAULT_SHUTDOWN_REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
/// Ordered notification queue depth; a full queue fails the connection.
pub const QUEUED_NOTIFICATIONS: usize = 256;
/// Bound on an outbound write that makes no progress (a sidecar not reading).
pub const DEFAULT_WRITE_STALL_BOUND: Duration = Duration::from_secs(10);
/// How long teardown waits for the serve loop to unwind.
pub const SERVE_EXIT_WAIT: Duration = Duration::from_secs(5);

/// Wire methods this slice speaks.
pub mod method {
    pub const INITIALIZE: &str = "extension/initialize";
    pub const INITIALIZED: &str = "extension/initialized";
    pub const SHUTDOWN: &str = "extension/shutdown";
    pub const INTERCEPT: &str = "extension/intercept";
    pub const EVENT: &str = "extension/event";
    pub const RESOURCES_CHANGED: &str = "extension/resources/changed";
}

/// Host-side UI calls belong to a later slice; naming the answer keeps the
/// degradation explicit instead of a missing registration.
pub const UNAVAILABLE_UI_MESSAGE: &str = "extension UI is not available on this host";

/// Options for starting one sidecar.
#[derive(Clone)]
pub struct ClientOptions {
    /// Session the extension serves.
    pub session: SessionContext,
    /// Host surface family that renders extension UI; empty means headless.
    pub ui_host: Option<UIHostKind>,
    /// Overrides the 30s initialize budget.
    pub handshake_timeout: Option<Duration>,
    /// Overrides the 10s write-stall bound.
    pub write_stall_bound: Option<Duration>,
    /// Observer told once when a started sidecar dies unexpectedly.
    pub on_crash: Option<OnCrashHook>,
}

impl Default for ClientOptions {
    fn default() -> Self {
        Self {
            session: SessionContext {
                session_id: String::new(),
                workspace_root: String::new(),
                generation: 0,
                epoch: String::new(),
            },
            ui_host: None,
            handshake_timeout: None,
            write_stall_bound: None,
            on_crash: None,
        }
    }
}

impl std::fmt::Debug for ClientOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClientOptions")
            .field("session", &self.session.session_id)
            .field("ui_host", &self.ui_host)
            .field("handshake_timeout", &self.handshake_timeout)
            .field("write_stall_bound", &self.write_stall_bound)
            .finish_non_exhaustive()
    }
}

/// Why a sidecar client could not start or serve a call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientError {
    /// Invalid options or package.
    InvalidOptions(String),
    /// The manifest does not declare a runtime.
    NoRuntime,
    /// A spawn-contract or operating-system failure, with bounded diagnostics.
    Startup(StartupFailure),
    /// A protocol violation, including a poisoned handshake.
    Protocol { reason: ErrorReason, detail: String },
    /// The sidecar crashed, shut down, or stopped answering.
    Unavailable { reason: ErrorReason, detail: String },
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidOptions(detail) => f.write_str(detail),
            Self::NoRuntime => f.write_str("the installed package declares no runtime"),
            Self::Startup(failure) => f.write_str(&failure.message()),
            Self::Protocol { reason, detail } | Self::Unavailable { reason, detail } => {
                write!(f, "{}: {detail}", ProtocolError::new(*reason).message())
            }
        }
    }
}

impl std::error::Error for ClientError {}

impl ClientError {
    /// The typed identity a caller matches on, when the failure has one.
    pub fn reason(&self) -> Option<ErrorReason> {
        match self {
            Self::Protocol { reason, .. } | Self::Unavailable { reason, .. } => Some(*reason),
            _ => None,
        }
    }
}

impl From<StartupFailure> for ClientError {
    fn from(failure: StartupFailure) -> Self {
        Self::Startup(failure)
    }
}

/// Observer told once when a started sidecar dies unexpectedly.
pub(crate) type OnCrashHook = Arc<dyn Fn(&str, &ClientError) + Send + Sync>;

/// Handshake progress, shared with the wire gate.
#[derive(Debug, Default)]
struct HandshakeState {
    phase: HandshakePhase,
    violation: Option<String>,
}

impl HandshakeState {
    /// Records the first violation; later ones add nothing.
    fn poison(&mut self, detail: String) {
        if matches!(
            self.phase,
            HandshakePhase::Poisoned | HandshakePhase::Shutdown
        ) {
            return;
        }
        self.phase = HandshakePhase::Poisoned;
        self.violation = Some(detail);
    }
}

/// One live sidecar connection.
pub struct Client {
    plugin_id: String,
    version: String,
    package: InstalledRuntimePackage,
    session: SessionContext,
    ui_host: UIHostKind,
    handshake_timeout: Duration,
    process: Arc<SidecarProcess>,
    conn: Conn<BufReader<ChildStdout>, ChildStdin>,
    state: Arc<Mutex<HandshakeState>>,
    init_result: Mutex<Option<InitializeResult>>,
    crashed: Arc<AtomicBool>,
    shutdown_done: AtomicBool,
    intercept_seq: AtomicU64,
    serve_exited: Arc<AtomicBool>,
    supervisor: Mutex<Option<JoinHandle<()>>>,
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client")
            .field("plugin_id", &self.plugin_id)
            .field("phase", &self.phase())
            .field("crashed", &self.crashed())
            .finish()
    }
}

impl Client {
    /// Spawns the sidecar and runs the initialize handshake.
    ///
    /// On any failure the process tree is killed and reaped before this
    /// returns, so a process never outlives a failed start.
    pub fn start(
        package: &InstalledRuntimePackage,
        options: &ClientOptions,
    ) -> Result<Self, ClientError> {
        validate_options(package, options)?;
        let started = Instant::now();
        let runtime = package
            .manifest
            .runtime
            .as_ref()
            .ok_or(ClientError::NoRuntime)?;
        let process = match SidecarProcess::spawn(
            package.plugin_id(),
            runtime,
            &package.runtime_env(),
            &package.root,
            &runtime.env,
        ) {
            Ok(process) => Arc::new(process),
            Err(err) => {
                let stage = spawn_stage(&err);
                let tail = crate::sidecar::process::TailBuffer::new(0);
                return Err(ClientError::Startup(startup_failure(
                    stage,
                    started.elapsed(),
                    tail.redacted(),
                    err.to_string(),
                )));
            }
        };
        let (stdout, stdin) = process.take_pipes();
        let (Some(stdout), Some(stdin)) = (stdout, stdin) else {
            process.kill_tree();
            process.close();
            return Err(ClientError::InvalidOptions(
                "sidecar pipes were not created".into(),
            ));
        };
        let client = Self::build(package, options, Arc::clone(&process), stdout, stdin);
        client.supervise();
        if let Err(err) = client.handshake() {
            client.abandon();
            return Err(wrap_handshake_failure(err, started, &client));
        }
        Ok(client)
    }

    fn build(
        package: &InstalledRuntimePackage,
        options: &ClientOptions,
        process: Arc<SidecarProcess>,
        stdout: ChildStdout,
        stdin: ChildStdin,
    ) -> Self {
        let plugin_id = package.plugin_id().to_string();
        let state = Arc::new(Mutex::new(HandshakeState {
            phase: HandshakePhase::New,
            violation: None,
        }));
        let gate = Gate {
            state: Arc::clone(&state),
            process: Arc::clone(&process),
            plugin_id: plugin_id.clone(),
        };
        let request_gate = gate.clone();
        let notification_gate = gate;
        let stall_process = Arc::clone(&process);
        let conn_options = ConnOptions {
            name: format!("extension:{plugin_id}"),
            max_inbound_bytes: crate::limits::FRAME_BYTES,
            max_outbound_bytes: crate::limits::FRAME_BYTES,
            strict_jsonrpc: true,
            max_concurrent_handlers: crate::rpcwire::DEFAULT_MAX_CONCURRENT_HANDLERS,
            max_queued_notifications: QUEUED_NOTIFICATIONS,
            max_write_stall: Some(
                options
                    .write_stall_bound
                    .unwrap_or(DEFAULT_WRITE_STALL_BOUND),
            ),
            before_request: Some(Box::new(move |method: &str, _params| {
                request_gate.check(method)
            })),
            before_notification: Some(Box::new(move |method: &str, _params| {
                notification_gate.check(method)
            })),
            // A stalled write ends the connection; the peer is wedged, so its
            // tree is torn down rather than left holding the pipe.
            on_terminal: Some(Arc::new(move || stall_process.kill_tree())),
        };
        let conn = Conn::new(BufReader::new(stdout), stdin, conn_options);
        Self {
            plugin_id,
            version: package.version().to_string(),
            package: package.clone(),
            session: options.session.clone(),
            ui_host: options.ui_host.unwrap_or(UIHostKind::Headless),
            handshake_timeout: options
                .handshake_timeout
                .unwrap_or(DEFAULT_HANDSHAKE_TIMEOUT),
            process,
            conn,
            state,
            init_result: Mutex::new(None),
            crashed: Arc::new(AtomicBool::new(false)),
            shutdown_done: AtomicBool::new(false),
            intercept_seq: AtomicU64::new(0),
            serve_exited: Arc::new(AtomicBool::new(false)),
            supervisor: Mutex::new(None),
        }
    }

    /// Runs the read loop for the connection's life and records an unexpected end.
    fn supervise(&self) {
        let conn = self.conn.clone();
        let state = Arc::clone(&self.state);
        let process = Arc::clone(&self.process);
        let crashed = Arc::clone(&self.crashed);
        let exited = Arc::clone(&self.serve_exited);
        let handle = std::thread::spawn(move || {
            let outcome = conn.serve();
            let orderly = matches!(
                state.lock().expect("client mutex").phase,
                HandshakePhase::Shutdown
            );
            if !orderly {
                // An unexpected end can leave the process ALIVE but
                // unreachable, so the tree is killed: a sidecar never outlives
                // the connection that speaks for it.
                process.kill_tree();
                crashed.store(true, Ordering::SeqCst);
                let _ = outcome;
            }
            exited.store(true, Ordering::SeqCst);
        });
        *self.supervisor.lock().expect("client mutex") = Some(handle);
    }

    /// Runs `extension/initialize` and validates the sidecar's declaration.
    fn handshake(&self) -> Result<(), ClientError> {
        let params = self.initialize_params();
        let params_value = serde_json::to_value(&params)
            .map_err(|err| ClientError::InvalidOptions(err.to_string()))?;
        let body = match self.conn.request(
            method::INITIALIZE,
            Some(&params_value),
            Some(self.handshake_timeout),
        ) {
            Ok(body) => body,
            Err(err) => {
                if let Some(violation) = self.violation() {
                    return Err(ClientError::Protocol {
                        reason: ErrorReason::ProtocolError,
                        detail: violation,
                    });
                }
                return Err(self.map_request_error(err));
            }
        };
        let result: InitializeResult =
            serde_json::from_value(body).map_err(|err| ClientError::Protocol {
                reason: ErrorReason::ProtocolError,
                detail: format!("invalid initialize result: {err}"),
            })?;
        if let Err(err) =
            validate_handshake_result(&self.plugin_id, &result, &self.manifest_expectation())
        {
            return Err(ClientError::Protocol {
                reason: err.reason,
                detail: redact_error_text(&capability_detail(&result, &self.plugin_id)),
            });
        }
        {
            let mut state = self.state.lock().expect("client mutex");
            if state.phase == HandshakePhase::Poisoned {
                let violation = state.violation.clone().unwrap_or_default();
                drop(state);
                return Err(ClientError::Protocol {
                    reason: ErrorReason::ProtocolError,
                    detail: violation,
                });
            }
            state.phase = HandshakePhase::Ready;
        }
        *self.init_result.lock().expect("client mutex") = Some(result);
        let initialized = serde_json::to_value(InitializedParams::default())
            .map_err(|err| ClientError::InvalidOptions(err.to_string()))?;
        self.conn
            .notify(method::INITIALIZED, Some(&initialized))
            .map_err(|err| self.map_request_error(err))
    }

    /// Kills and reaps a sidecar whose start failed, bounded.
    fn abandon(&self) {
        self.conn.terminate();
        self.process.kill_tree();
        self.process.wait_for_exit(CLOSE_WAIT);
        if let Some(handle) = self.supervisor.lock().expect("client mutex").take() {
            let _ = handle.join();
        }
    }

    /// The protocol violation that poisoned this connection, if any.
    pub fn violation_detail(&self) -> Option<String> {
        self.state.lock().expect("client mutex").violation.clone()
    }

    fn violation(&self) -> Option<String> {
        self.violation_detail()
    }

    fn manifest_expectation(&self) -> ManifestExpectation {
        let runtime = self.package.runtime();
        let mut expectation = ManifestExpectation {
            intercepts: runtime.intercepts.clone(),
            replaces: runtime.replaces.clone(),
            capabilities: runtime.capabilities.clone(),
            tools: runtime.tool_names(),
            requires: self
                .package
                .manifest
                .requires
                .iter()
                .map(requirement_wire)
                .collect(),
            provides: self
                .package
                .manifest
                .provides
                .iter()
                .map(capability_wire)
                .collect(),
            ..ManifestExpectation::default()
        };
        for provided in &self.package.manifest.provides {
            match provided.kind.trim().to_ascii_lowercase().as_str() {
                "provider" => expectation.providers.push(capability_address(provided)),
                "uiaction" => expectation.ui_actions.push(provided.id.trim().to_string()),
                _ => {}
            }
        }
        expectation
    }

    fn initialize_params(&self) -> InitializeParams {
        InitializeParams {
            protocol_version: PROTOCOL_VERSION.to_string(),
            protocol_id: PROTOCOL_ID.to_string(),
            manifest: self.manifest_expectation(),
            session: self.session.clone(),
            capabilities: HostCapabilities {
                content_refs: true,
                ui_host: self.ui_host,
                protocol_version: PROTOCOL_VERSION.to_string(),
                dependency_schema_version: DEPENDENCY_SCHEMA_VERSION,
            },
            dependency_schema_version: DEPENDENCY_SCHEMA_VERSION,
        }
    }

    /// The installed plugin package name this client serves.
    pub fn plugin_id(&self) -> &str {
        &self.plugin_id
    }

    /// The version this sidecar reports.
    pub fn version(&self) -> &str {
        &self.version
    }

    /// The validated initialize result, once the handshake completed.
    pub fn handshake_result(&self) -> Option<InitializeResult> {
        self.init_result.lock().expect("client mutex").clone()
    }

    /// The handshake phase this connection observes.
    pub fn phase(&self) -> HandshakePhase {
        self.state.lock().expect("client mutex").phase
    }

    /// The installed package this client was launched from.
    pub fn package(&self) -> &InstalledRuntimePackage {
        &self.package
    }

    /// The connection handle, for a caller registering host-side handlers.
    pub fn connection(&self) -> &Conn<BufReader<ChildStdout>, ChildStdin> {
        &self.conn
    }

    /// Reports whether the connection ended unexpectedly.
    pub fn crashed(&self) -> bool {
        self.crashed.load(Ordering::SeqCst)
    }

    /// Reports whether the serve loop has ended.
    pub fn is_disconnected(&self) -> bool {
        self.serve_exited.load(Ordering::SeqCst)
    }

    /// Reports whether the sidecar process has been reaped.
    pub fn exited(&self) -> bool {
        self.process.has_exited()
    }

    /// The bounded, redacted stderr tail.
    pub fn stderr_tail(&self) -> String {
        self.process.stderr_tail()
    }

    /// Allocates the next host-monotonic intercept sequence number.
    pub fn next_intercept_seq(&self) -> u64 {
        self.intercept_seq.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// The per-point intercept budget: the manifest override clamped to the
    /// 60s ceiling, else the point family's default.
    pub fn intercept_timeout(&self, event: InterceptEvent) -> Duration {
        let override_ms = self.package.runtime().timeout_millis;
        if override_ms > 0 {
            let ceiling = Duration::from_millis(crate::intercept::timeouts::CEILING_MS);
            return Duration::from_millis(override_ms).min(ceiling);
        }
        Duration::from_millis(crate::intercept::default_timeout_ms(event))
    }

    /// Sends one blocking `extension/intercept` and returns the raw ruling.
    pub fn intercept(
        &self,
        event: InterceptEvent,
        payload: Option<&Value>,
        timeout: Option<Duration>,
    ) -> Result<Value, ClientError> {
        self.require_ready()?;
        let budget = timeout.unwrap_or_else(|| self.intercept_timeout(event));
        let params = json!({
            "event": event,
            "seq": self.next_intercept_seq(),
            "payload": payload.cloned().unwrap_or(Value::Null),
            "timeoutMillis": budget.as_millis() as i64,
        });
        match self
            .conn
            .request(method::INTERCEPT, Some(&params), Some(budget))
        {
            Ok(value) => Ok(value),
            Err(ConnError::TimedOut) => {
                if self.package.runtime().kill_on_timeout {
                    self.process.kill_tree();
                }
                Err(ClientError::Unavailable {
                    reason: ErrorReason::InterceptTimeout,
                    detail: format!(
                        "extension {} did not answer {event:?} within {}ms",
                        self.plugin_id,
                        budget.as_millis()
                    ),
                })
            }
            Err(err) => Err(self.map_request_error(err)),
        }
    }

    /// Enqueues one fire-and-forget `extension/event` observation.
    pub fn try_notify_event(
        &self,
        event: InterceptEvent,
        payload: Option<&Value>,
    ) -> Result<(), ClientError> {
        self.require_ready()?;
        let params = json!({"event": event, "payload": payload.cloned().unwrap_or(Value::Null)});
        self.conn
            .try_notify(method::EVENT, Some(&params))
            .map_err(|err| self.map_request_error(err))
    }

    /// Sends `extension/resources/changed`.
    pub fn notify_resources_changed(&self, paths: &[String]) -> Result<(), ClientError> {
        self.require_ready()?;
        let params = json!({"paths": paths});
        self.conn
            .notify(method::RESOURCES_CHANGED, Some(&params))
            .map_err(|err| self.map_request_error(err))
    }

    /// Sends `extension/shutdown`, then closes stdin, grants a short EOF grace,
    /// kills the tree, and reaps bounded. Idempotent.
    pub fn shutdown(&self, timeout: Option<Duration>) {
        if self.shutdown_done.swap(true, Ordering::SeqCst) {
            return;
        }
        let was_ready = {
            let mut state = self.state.lock().expect("client mutex");
            let ready = state.phase == HandshakePhase::Ready;
            if state.phase != HandshakePhase::Poisoned {
                state.phase = HandshakePhase::Shutdown;
            }
            ready
        };
        let budget = timeout.unwrap_or(DEFAULT_SHUTDOWN_REQUEST_TIMEOUT);
        if was_ready && !self.crashed() {
            let params = json!({"timeoutMillis": budget.as_millis() as i64});
            let _ = self
                .conn
                .request(method::SHUTDOWN, Some(&params), Some(budget));
        }
        self.conn.terminate();
        self.process.close();
        self.wait_for_disconnect(SERVE_EXIT_WAIT);
        if let Some(handle) = self.supervisor.lock().expect("client mutex").take() {
            let _ = handle.join();
        }
    }

    /// Shuts the sidecar down with the default budgets.
    pub fn close(&self) {
        self.shutdown(None);
    }

    fn wait_for_disconnect(&self, budget: Duration) {
        let started = Instant::now();
        while !self.is_disconnected() {
            if started.elapsed() >= budget {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn require_ready(&self) -> Result<(), ClientError> {
        if self.crashed() {
            return Err(ClientError::Unavailable {
                reason: ErrorReason::ProviderInterrupted,
                detail: format!("extension sidecar {} crashed", self.plugin_id),
            });
        }
        match self.phase() {
            HandshakePhase::Ready => Ok(()),
            HandshakePhase::Shutdown => Err(ClientError::Unavailable {
                reason: ErrorReason::ProviderInterrupted,
                detail: format!("extension sidecar {} is shut down", self.plugin_id),
            }),
            HandshakePhase::New | HandshakePhase::Initializing | HandshakePhase::Poisoned => {
                Err(ClientError::Protocol {
                    reason: ErrorReason::ProtocolError,
                    detail: format!("extension sidecar {} is not initialized", self.plugin_id),
                })
            }
        }
    }

    /// Maps a failed outbound call: a peer error keeps its frozen reason when
    /// the structured data validates, and its message never bypasses redaction.
    pub fn map_request_error(&self, err: ConnError) -> ClientError {
        match err {
            ConnError::Response(response) => {
                let message = redact_error_text(&response.message);
                let reason = response
                    .data
                    .as_ref()
                    .and_then(|data| serde_json::from_str::<ProtocolErrorData>(data.get()).ok())
                    .filter(|data| data.validate().is_ok())
                    .and_then(|data| ErrorReason::from_wire(&data.reason));
                match reason {
                    Some(reason) => ClientError::Protocol {
                        reason,
                        detail: message,
                    },
                    None => ClientError::Unavailable {
                        reason: ErrorReason::ProtocolError,
                        detail: format!("peer error {}: {message}", response.code),
                    },
                }
            }
            ConnError::TimedOut => ClientError::Unavailable {
                reason: ErrorReason::ProviderInterrupted,
                detail: format!("extension sidecar {} did not answer", self.plugin_id),
            },
            other => ClientError::Unavailable {
                reason: ErrorReason::ProviderInterrupted,
                detail: redact_error_text(&other.to_string()),
            },
        }
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        self.shutdown(None);
    }
}

/// The wire gate: only a ready connection may carry extension-to-host traffic.
///
/// Poisoning kills the tree, so a pending handshake unwinds immediately instead
/// of waiting out its budget.
#[derive(Clone)]
struct Gate {
    state: Arc<Mutex<HandshakeState>>,
    process: Arc<SidecarProcess>,
    plugin_id: String,
}

impl Gate {
    fn check(&self, method: &str) -> Result<(), RpcHandlerError> {
        {
            let mut state = self.state.lock().expect("client mutex");
            if state.phase == HandshakePhase::Ready {
                return Ok(());
            }
            state.poison(format!(
                "extension {} sent {method:?} before extension/initialized",
                self.plugin_id
            ));
        }
        self.process.kill_tree();
        let data = ProtocolErrorData::for_reason(ErrorReason::ProtocolError);
        let encoded = serde_json::to_string(&data).expect("protocol error data is serializable");
        let raw = RawJson::parse(&encoded).expect("protocol error data is JSON");
        Err(RpcHandlerError::with_data(
            rpc_code::DOMAIN,
            ProtocolError::new(ErrorReason::ProtocolError).message(),
            raw,
        ))
    }
}

fn validate_options(
    package: &InstalledRuntimePackage,
    options: &ClientOptions,
) -> Result<(), ClientError> {
    if package.manifest.runtime.is_none() {
        return Err(ClientError::NoRuntime);
    }
    if package.installed.name.trim().is_empty() {
        return Err(ClientError::InvalidOptions(
            "installed plugin name is required".into(),
        ));
    }
    if options.session.session_id.trim().is_empty()
        || options.session.workspace_root.trim().is_empty()
    {
        return Err(ClientError::InvalidOptions(
            "session context requires a session ID and workspace base".into(),
        ));
    }
    Ok(())
}

/// Wraps a handshake failure with the stage, elapsed budget, and redacted tail.
fn wrap_handshake_failure(err: ClientError, started: Instant, client: &Client) -> ClientError {
    match err {
        ClientError::Startup(failure) => ClientError::Startup(failure),
        ClientError::InvalidOptions(detail) => ClientError::InvalidOptions(detail),
        other => ClientError::Startup(startup_failure(
            "handshake",
            started.elapsed(),
            client.process.stderr_tail(),
            other.to_string(),
        )),
    }
}

fn spawn_stage(err: &SpawnError) -> &'static str {
    match err {
        SpawnError::Empty | SpawnError::NotAbsolute { .. } | SpawnError::Shell { .. } => "resolve",
        SpawnError::NotExecutable { .. } | SpawnError::Failed { .. } => "spawn",
    }
}

fn capability_address(reference: &CapabilityRef) -> String {
    format!(
        "{}/{}",
        reference.namespace.trim().trim_end_matches('/'),
        reference.id.trim().trim_start_matches('/')
    )
}

fn capability_wire(reference: &CapabilityRef) -> crate::lifecycle::CapabilityWire {
    crate::lifecycle::CapabilityWire {
        namespace: reference.namespace.trim().to_string(),
        kind: reference.kind.trim().to_string(),
        id: reference.id.trim().to_string(),
        version: reference.version.trim().to_string(),
        schema_hash: reference.schema_hash.trim().to_string(),
    }
}

fn requirement_wire(reference: &CapabilityRef) -> crate::lifecycle::RequirementWire {
    crate::lifecycle::RequirementWire {
        namespace: reference.namespace.trim().to_string(),
        kind: reference.kind.trim().to_string(),
        id: reference.id.trim().to_string(),
        version: reference.version.trim().to_string(),
        schema_hash: reference.schema_hash.trim().to_string(),
        version_range: reference.version_range.trim().to_string(),
        optional: reference.optional,
    }
}

/// Names the declaration that broke the manifest ceiling, for diagnostics.
fn capability_detail(result: &InitializeResult, plugin_id: &str) -> String {
    if !result.subscriptions.is_empty() {
        return format!(
            "extension {plugin_id} declared subscriptions {:?}",
            result.subscriptions
        );
    }
    if !result.replaces.is_empty() {
        return format!(
            "extension {plugin_id} declared replaces {:?}",
            result.replaces
        );
    }
    if !result.providers.is_empty() {
        return format!("extension {plugin_id} declared providers");
    }
    if !result.ui_actions.is_empty() {
        return format!("extension {plugin_id} declared UI actions");
    }
    if !result.tools.is_empty() {
        return format!("extension {plugin_id} declared tools {:?}", result.tools);
    }
    format!("extension {plugin_id} declared capabilities its manifest does not")
}

/// Silences the unused-import lint for the handler type used in tests only.
#[allow(dead_code)]
fn _assert_handler_response(_: HandlerResponse) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sidecar::installed::{InstalledPlugin, RuntimeManifest, RuntimeSpec};

    fn package(runtime: Option<RuntimeSpec>) -> InstalledRuntimePackage {
        InstalledRuntimePackage {
            installed: InstalledPlugin {
                name: "alpha".into(),
                version: "1.0.0".into(),
                enabled: true,
                ..InstalledPlugin::default()
            },
            manifest: RuntimeManifest {
                name: "alpha".into(),
                version: "1.0.0".into(),
                runtime,
                ..RuntimeManifest::default()
            },
            root: std::env::temp_dir(),
        }
    }

    fn options() -> ClientOptions {
        ClientOptions {
            session: SessionContext {
                session_id: "s-1".into(),
                workspace_root: std::env::temp_dir().display().to_string(),
                generation: 1,
                epoch: String::new(),
            },
            ..ClientOptions::default()
        }
    }

    #[test]
    fn missing_runtime_is_a_typed_refusal() {
        let err = Client::start(&package(None), &options()).unwrap_err();
        assert_eq!(err, ClientError::NoRuntime);
    }

    #[test]
    fn session_context_is_required_before_spawning() {
        let err = Client::start(
            &package(Some(RuntimeSpec {
                command: "/opt/alpha".into(),
                ..RuntimeSpec::default()
            })),
            &ClientOptions::default(),
        )
        .unwrap_err();
        assert!(matches!(err, ClientError::InvalidOptions(_)), "{err:?}");
        assert!(err.to_string().contains("session context"));
    }

    #[test]
    fn exec_contract_failures_are_reported_before_any_spawn() {
        let err = Client::start(
            &package(Some(RuntimeSpec {
                command: "relative-name".into(),
                ..RuntimeSpec::default()
            })),
            &options(),
        )
        .unwrap_err();
        match err {
            ClientError::Startup(failure) => {
                assert_eq!(failure.stage, "resolve");
                assert!(
                    failure.detail.contains("not an absolute path"),
                    "{failure:?}"
                );
            }
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn intercept_timeout_defaults_follow_the_point_family() {
        assert_eq!(
            crate::intercept::default_timeout_ms(InterceptEvent::ToolBefore),
            5_000
        );
        assert_eq!(
            crate::intercept::default_timeout_ms(InterceptEvent::SessionStart),
            30_000
        );
    }
}

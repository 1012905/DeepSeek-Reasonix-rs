//! Client session: dispatch, per-call timeouts, progress-token plumbing, and
//! the legacy initialize handshake (MCP-015).
//!
//! Oracle: `internal/ext/plugin/client_session.go`, `protocol_version.go`;
//! contract `docs/contracts/mcp.md` §10, negotiated under §1.1. The session
//! owns dispatch against one connection; the reconnect wrapper and the
//! subprocess/network transports are separate slices.

use alloc::string::{String, ToString};
use core::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::Value;

use super::lifecycle::{DEFAULT_CALL_TIMEOUT_MS, Spec};
use super::seam::{Transport, TransportError};
use super::version::{Era, VersionError, initialize_params, negotiated_version};

/// JSON-RPC implementation-defined code a double (and, later, the real seam)
/// uses to mark a gone HTTP session. The transport seam does not yet carry a
/// typed expiry variant, so [`SessionExpiredProbe`] is the typed judgement
/// point instead of message-text matching.
pub const CODE_SESSION_EXPIRED: i64 = -32023;

const INITIALIZE_METHOD: &str = "initialize";
const INITIALIZED_NOTIFICATION: &str = "notifications/initialized";
const TOOLS_CALL_METHOD: &str = "tools/call";
const PROGRESS_TOKEN_PREFIX: &str = "reasonix-";

const NANOS_PER_SEC: u128 = 1_000_000_000;
const NANOS_PER_MS: u128 = 1_000_000;
const NANOS_PER_US: u128 = 1_000;

/// Closure that unregisters a progress-token registration once when the
/// call ends.
type ProgressUnregister = Box<dyn FnOnce() + Send + Sync>;

/// Optional transport capability: states the negotiated revision on each
/// request (Streamable HTTP's `MCP-Protocol-Version` header). Mirrors Go's
/// `protocolVersioned` interface.
pub trait ProtocolVersioned: Send + Sync {
    /// Records the revision echoed until the next initialize.
    fn set_protocol_version(&self, version: &str);
}

/// Sink a tool's combined output is streamed to. Mirrors `tool.ProgressFunc`.
pub trait ProgressSink: Send + Sync {
    /// Delivers one produced chunk.
    fn send(&self, chunk: &str);
}

impl<F> ProgressSink for F
where
    F: Fn(&str) + Send + Sync,
{
    fn send(&self, chunk: &str) {
        self(chunk);
    }
}

/// Optional transport capability: routes a `notifications/progress` frame to
/// the sink registered under its token. Mirrors Go's `progressTransport`.
pub trait ProgressTransport: Send + Sync {
    /// Registers `token` → `sink`; the returned closure unregisters once when
    /// the call ends.
    fn register_progress(&self, token: &str, sink: &dyn ProgressSink) -> ProgressUnregister;
}

/// Typed judgement that a transport error means the server session is gone.
///
/// The seam currently erases the underlying `HttpError` variant when it maps
/// an expired session onto `TransportError::Io`, so production code cannot tell
/// expiry apart from any other io failure without matching message text, which
/// the parity rules forbid. The default probe therefore never reports expiry;
/// the real HTTP wiring lands once the seam carries a typed expiry variant.
pub trait SessionExpiredProbe: Send + Sync {
    fn is_session_expired(&self, err: &TransportError) -> bool;
}

/// Probe that treats no error as session expiry.
#[derive(Default)]
pub struct FreshSessionProbe;

impl SessionExpiredProbe for FreshSessionProbe {
    fn is_session_expired(&self, _err: &TransportError) -> bool {
        false
    }
}

/// Gate run before a replacement connection is dialed. The three launcher
/// gates (stored launcher lock, project launch authorization, declared launch)
/// belong to the launcher slice (MCP-034); this hook keeps redial from
/// bypassing them once that slice lands.
pub trait RedialGate: Send + Sync {
    /// Re-checks the authorization a replacement connection would start on.
    fn authorize_replacement(&self) -> Result<(), SessionError>;
}

/// Typed session failures, each with an independent identity. Message text
/// follows the Go oracle verbatim; the wrapped `context.DeadlineExceeded` is
/// replaced by the typed [`SessionError::DeadlineExceeded`] variant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionError {
    Transport(TransportError),
    Version(VersionError),
    /// `plugin "<name>": <version error>` (Go `fmt.Errorf("plugin %q: %w", ...)`).
    PluginVersion {
        name: String,
        source: VersionError,
    },
    CallTimeout {
        method: String,
        server: String,
        tool: Option<String>,
        timeout: Duration,
        source: Box<SessionError>,
    },
    ReinitializeFailed {
        source: Box<SessionError>,
        reinit: Box<SessionError>,
    },
    ModernDispatchUnimplemented {
        method: String,
    },
    RedialDenied {
        reason: String,
    },
    /// Typed replacement for Go's `context.DeadlineExceeded`.
    DeadlineExceeded,
}

impl fmt::Display for SessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Transport(err) => fmt::Display::fmt(err, f),
            Self::Version(err) => fmt::Display::fmt(err, f),
            Self::PluginVersion { name, source } => {
                write!(f, "plugin {name:?}: {source}")
            }
            Self::CallTimeout {
                method,
                server,
                tool,
                timeout,
                source,
            } => match tool {
                Some(raw) => {
                    let full = alloc::format!("{server}.{raw}");
                    write!(
                        f,
                        "MCP tool {full:?} timed out after {timeout}; \
                         increase tool_timeout_seconds or call_timeout_seconds to allow \
                         longer runs: {source}",
                        timeout = format_timeout(*timeout),
                    )
                }
                None => write!(
                    f,
                    "MCP method {method:?} on server {server:?} timed out after {timeout}; \
                     increase mcp_call_timeout_seconds or call_timeout_seconds to allow \
                     longer runs: {source}",
                    timeout = format_timeout(*timeout),
                ),
            },
            Self::ReinitializeFailed { source, reinit } => {
                write!(f, "{source}; reinitialize failed: {reinit}")
            }
            Self::ModernDispatchUnimplemented { method } => {
                write!(
                    f,
                    "modern-era dispatch for {method:?} is not implemented in this slice"
                )
            }
            Self::RedialDenied { reason } => f.write_str(reason),
            Self::DeadlineExceeded => f.write_str("context deadline exceeded"),
        }
    }
}

impl std::error::Error for SessionError {}

/// Reads the raw server-local tool name out of `tools/call` params; absent or
/// non-object params yield an empty string (Go `rawToolNameFromCallParams`).
pub fn raw_tool_name(params: Option<&Value>) -> String {
    params
        .and_then(Value::as_object)
        .and_then(|m| m.get("name"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// Formats a timeout Go `time.Duration` style: whole seconds render as `Ns`,
/// otherwise the Go duration string (e.g. `1.5s`, `300ms`).
pub fn format_timeout(timeout: Duration) -> String {
    let nanos = timeout.as_nanos();
    if nanos == 0 {
        return "0s".to_string();
    }
    let whole_secs = nanos / NANOS_PER_SEC;
    let sub = nanos % NANOS_PER_SEC;
    if sub == 0 {
        return alloc::format!("{whole_secs}s");
    }
    if whole_secs > 0 {
        let fraction = trim_trailing_zeroes(&alloc::format!("{sub:09}"));
        return alloc::format!("{whole_secs}.{fraction}s");
    }
    if sub >= NANOS_PER_MS {
        let ms = sub / NANOS_PER_MS;
        let us = (sub % NANOS_PER_MS) / NANOS_PER_US;
        return if us == 0 {
            alloc::format!("{ms}ms")
        } else {
            alloc::format!("{ms}.{us}ms")
        };
    }
    if sub >= NANOS_PER_US {
        let us = sub / NANOS_PER_US;
        let ns = sub % NANOS_PER_US;
        return if ns == 0 {
            alloc::format!("{us}µs")
        } else {
            alloc::format!("{us}.{ns}µs")
        };
    }
    alloc::format!("{nanos}ns")
}

fn trim_trailing_zeroes(digits: &str) -> String {
    let trimmed = digits.trim_end_matches('0');
    if trimmed.is_empty() {
        "0".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Resolves the per-call timeout (Go `contextWithCallTimeout`/`callTimeout`).
/// A caller-set deadline is honored unchanged and yields `None`. Otherwise
/// `tools/call` takes `tool_timeouts_ms[raw]` first, then `call_timeout_ms`,
/// then `default_call_timeout_ms`, then the 300s default; other methods skip
/// the per-tool map.
pub fn resolve_call_timeout(
    spec: &Spec,
    method: &str,
    params: Option<&Value>,
    existing_deadline: Option<Duration>,
) -> Option<Duration> {
    if existing_deadline.is_some() {
        return None;
    }
    let mut ms: u64 = 0;
    if method == TOOLS_CALL_METHOD {
        let raw = raw_tool_name(params);
        if !raw.is_empty()
            && let Some(&tool_ms) = spec.tool_timeouts_ms.get(&raw)
            && tool_ms > 0
        {
            ms = tool_ms;
        }
    }
    if ms == 0 && spec.call_timeout_ms > 0 {
        ms = spec.call_timeout_ms;
    }
    if ms == 0 && spec.default_call_timeout_ms > 0 {
        ms = spec.default_call_timeout_ms;
    }
    if ms == 0 {
        ms = DEFAULT_CALL_TIMEOUT_MS;
    }
    Some(Duration::from_millis(ms))
}

/// Reads `tools.listChanged` from the initialize result's tools capability.
/// Absent, non-object, or non-bool means the server never announces catalog
/// changes (Go `declaresToolListChanged`).
pub fn declares_tool_list_changed(raw: Option<&Value>) -> bool {
    let Some(obj) = raw.and_then(Value::as_object) else {
        return false;
    };
    match obj.get("listChanged") {
        Some(Value::Bool(flag)) => *flag,
        _ => false,
    }
}

/// Capabilities recorded from the startup initialize result. Interior
/// mutability keeps dispatch and handshake behind `&self`; the host layer owns
/// the session's concurrency policy.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct RecordedCapabilities {
    has_tools: bool,
    has_prompts: bool,
    has_resources: bool,
    declare_list_changed: bool,
    instructions: String,
}

/// One client's MCP session over a connection: dispatch, timeout resolution,
/// progress tokens, and the initialize handshake.
pub struct Session {
    spec: Spec,
    era: Era,
    transport: Arc<dyn Transport + Send + Sync>,
    versioned: Option<Arc<dyn ProtocolVersioned + Send + Sync>>,
    progress_router: Option<Arc<dyn ProgressTransport + Send + Sync>>,
    expiry: Arc<dyn SessionExpiredProbe>,
    progress_id: AtomicU64,
    recorded: Mutex<RecordedCapabilities>,
}

impl Session {
    /// Builds a session over the given connection. Optional transport
    /// capabilities attach through the builder methods.
    pub fn new(spec: Spec, era: Era, transport: Arc<dyn Transport + Send + Sync>) -> Self {
        Self {
            spec,
            era,
            transport,
            versioned: None,
            progress_router: None,
            expiry: Arc::new(FreshSessionProbe),
            progress_id: AtomicU64::new(0),
            recorded: Mutex::default(),
        }
    }

    /// Attaches the protocol-version header capability (Streamable HTTP).
    pub fn with_versioned(mut self, versioned: Arc<dyn ProtocolVersioned + Send + Sync>) -> Self {
        self.versioned = Some(versioned);
        self
    }

    /// Attaches the progress-token routing capability.
    pub fn with_progress_router(
        mut self,
        router: Arc<dyn ProgressTransport + Send + Sync>,
    ) -> Self {
        self.progress_router = Some(router);
        self
    }

    /// Attaches the typed session-expiry classifier.
    pub fn with_expiry_probe(mut self, probe: Arc<dyn SessionExpiredProbe>) -> Self {
        self.expiry = probe;
        self
    }

    pub fn spec(&self) -> &Spec {
        &self.spec
    }

    pub fn era(&self) -> &Era {
        &self.era
    }

    pub fn has_tools(&self) -> bool {
        self.recorded.lock().expect("recorded mutex").has_tools
    }

    pub fn has_prompts(&self) -> bool {
        self.recorded.lock().expect("recorded mutex").has_prompts
    }

    pub fn has_resources(&self) -> bool {
        self.recorded.lock().expect("recorded mutex").has_resources
    }

    pub fn declares_tool_list_changed(&self) -> bool {
        self.recorded
            .lock()
            .expect("recorded mutex")
            .declare_list_changed
    }

    pub fn instructions(&self) -> String {
        self.recorded
            .lock()
            .expect("recorded mutex")
            .instructions
            .clone()
    }

    /// Dispatches one request. Modern era routes to the modern path; legacy era
    /// runs the single-call path (Go `callOn`).
    pub fn call(
        &self,
        method: &str,
        params: Option<Value>,
        sink: Option<&dyn ProgressSink>,
    ) -> Result<Value, SessionError> {
        self.call_on(method, params, sink)
    }

    /// Sends a fire-and-forget notification without dispatch wrappers.
    pub fn notify(&self, method: &str, params: Option<Value>) -> Result<(), SessionError> {
        self.transport
            .notify(method, params.as_ref())
            .map_err(SessionError::Transport)
    }

    /// Runs the startup initialize handshake, recording advertised capabilities.
    pub fn initialize(&self) -> Result<(), SessionError> {
        self.initialize_session_on(true)
    }

    /// Runs the initialize handshake on `transport`. When `record_capabilities`
    /// is false (runtime session refresh after an expiry), startup-only
    /// capability flags are never rewritten.
    pub fn initialize_session_on(&self, record_capabilities: bool) -> Result<(), SessionError> {
        let params = initialize_params(&self.spec.workspace_root);
        if let Some(versioned) = &self.versioned {
            // The header belongs to a session; initialize is what opens one,
            // so a re-initialize starts over from a blank revision.
            versioned.set_protocol_version("");
        }
        let result = self
            .transport
            .call(INITIALIZE_METHOD, Some(&params))
            .map_err(SessionError::Transport)?;

        let reply_version = result
            .get("protocolVersion")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let version =
            negotiated_version(reply_version).map_err(|source| SessionError::PluginVersion {
                name: self.spec.name.clone(),
                source,
            })?;
        if let Some(versioned) = &self.versioned {
            versioned.set_protocol_version(&version);
        }

        if !record_capabilities {
            return self.notify_initialized();
        }

        // Presence of the capability key (even an empty object) signals
        // support. A non-object result reads as an empty capability set, the
        // same zero-value Go leaves after a failed unmarshal.
        let capabilities = result
            .get("capabilities")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let instructions = result
            .get("instructions")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string();

        let mut recorded = self.recorded.lock().expect("recorded mutex");
        recorded.has_tools = capabilities.contains_key("tools");
        recorded.has_prompts = capabilities.contains_key("prompts");
        recorded.has_resources = capabilities.contains_key("resources");
        recorded.declare_list_changed = declares_tool_list_changed(capabilities.get("tools"));
        recorded.instructions = instructions;
        // The notification leaves this crate's control; a transport callback
        // that reads the recorded flags must not deadlock on this guard.
        drop(recorded);

        self.notify_initialized()
    }

    /// Resolves this call's timeout against the spec, with no caller deadline.
    pub fn call_timeout(&self, method: &str, params: Option<&Value>) -> Duration {
        resolve_call_timeout(&self.spec, method, params, None)
            .expect("resolved without a caller deadline")
    }

    /// Builds the typed timeout error for a timed-out call (Go `timeoutError`).
    pub fn timeout_error(
        &self,
        method: &str,
        params: Option<&Value>,
        timeout: Duration,
    ) -> SessionError {
        let raw = raw_tool_name(params);
        let tool = if method == TOOLS_CALL_METHOD && !raw.is_empty() {
            Some(raw)
        } else {
            None
        };
        SessionError::CallTimeout {
            method: method.to_string(),
            server: self.spec.name.clone(),
            tool,
            timeout,
            source: Box::new(SessionError::DeadlineExceeded),
        }
    }

    /// Runs the replacement-connection authorization gate before any redial.
    /// The dial itself lives with the reconnect wave; MCP-034 supplies the
    /// three launcher gates behind this hook.
    pub fn authorize_redial(&self, gate: &dyn RedialGate) -> Result<(), SessionError> {
        gate.authorize_replacement()
    }

    fn call_on(
        &self,
        method: &str,
        params: Option<Value>,
        sink: Option<&dyn ProgressSink>,
    ) -> Result<Value, SessionError> {
        if self.era.is_modern() {
            // The modern input-round path (protocol_modern.go) lands in its own
            // slice; this branch owns the dispatch split only.
            return Err(SessionError::ModernDispatchUnimplemented {
                method: method.to_string(),
            });
        }
        self.call_once(method, params, sink)
    }

    fn call_once(
        &self,
        method: &str,
        params: Option<Value>,
        sink: Option<&dyn ProgressSink>,
    ) -> Result<Value, SessionError> {
        let (params, unregister) = self.with_progress(method, params, sink);
        let outcome = self.call_transport(method, &params);
        if let Some(unregister) = unregister {
            unregister();
        }
        outcome
    }

    /// Sends the request once, re-initializing a gone HTTP session and
    /// retrying exactly one time (Go `callTransport`).
    fn call_transport(&self, method: &str, params: &Option<Value>) -> Result<Value, SessionError> {
        let first = self
            .transport
            .call(method, params.as_ref())
            .map_err(SessionError::Transport);
        let expired = match first.as_ref().err() {
            Some(SessionError::Transport(err)) => {
                method != INITIALIZE_METHOD
                    && !self.era.is_modern()
                    && self.expiry.is_session_expired(err)
            }
            Some(_) => false,
            None => false,
        };
        if !expired {
            return first;
        }
        let original = first.expect_err("expired branch holds an error");
        match self.initialize_session_on(false) {
            Ok(()) => self
                .transport
                .call(method, params.as_ref())
                .map_err(SessionError::Transport),
            Err(reinit) => Err(SessionError::ReinitializeFailed {
                source: Box::new(original),
                reinit: Box::new(reinit),
            }),
        }
    }

    /// Injects the progress token for an eligible `tools/call`. Conditions:
    /// the method is `tools/call`, a sink is present, the transport supports
    /// registration, and params are an object. Every other case leaves params
    /// byte-for-byte untouched (Go `withProgress`).
    fn with_progress(
        &self,
        method: &str,
        params: Option<Value>,
        sink: Option<&dyn ProgressSink>,
    ) -> (Option<Value>, Option<ProgressUnregister>) {
        let (Some(sink), Some(router)) = (sink, self.progress_router.as_deref()) else {
            return (params, None);
        };
        if method != TOOLS_CALL_METHOD {
            return (params, None);
        }
        let Some(params) = params else {
            return (None, None);
        };
        let Some(call_params) = params.as_object().cloned() else {
            return (Some(params), None);
        };

        let token = alloc::format!(
            "{PROGRESS_TOKEN_PREFIX}{}",
            self.progress_id.fetch_add(1, Ordering::SeqCst) + 1
        );
        let mut meta = call_params
            .get("_meta")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        meta.insert("progressToken".into(), Value::String(token.clone()));
        let mut copied = call_params;
        copied.insert("_meta".into(), Value::Object(meta));

        let unregister = router.register_progress(&token, sink);
        (Some(Value::Object(copied)), Some(unregister))
    }

    fn notify_initialized(&self) -> Result<(), SessionError> {
        self.transport
            .notify(
                INITIALIZED_NOTIFICATION,
                Some(&Value::Object(Default::default())),
            )
            .map_err(SessionError::Transport)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    use serde_json::json;

    use super::super::version::{MODERN_PROTOCOL_VERSION, OLDEST_PROTOCOL_VERSION};
    use super::super::wire::RpcError;

    type Script = BTreeMap<String, Vec<Result<Value, TransportError>>>;

    #[derive(Default)]
    struct DoubleInner {
        calls: Vec<(String, Option<Value>)>,
        notifications: Vec<(String, Option<Value>)>,
        version_sets: Vec<String>,
        registrations: Vec<String>,
        unregisters: Vec<String>,
        script: Script,
    }

    /// Scripted transport double: records the wire-shaped (method, params) of
    /// every call and notification, the version-header calls, and the
    /// progress registration/unregistration sequence.
    struct Double {
        inner: Arc<Mutex<DoubleInner>>,
        era: Era,
    }

    impl Double {
        fn new(era: Era) -> Self {
            Self {
                inner: Arc::new(Mutex::new(DoubleInner::default())),
                era,
            }
        }

        fn script(&self, method: &str, replies: Vec<Result<Value, TransportError>>) {
            self.inner
                .lock()
                .expect("double mutex")
                .script
                .insert(method.to_string(), replies);
        }

        fn calls(&self) -> Vec<(String, Option<Value>)> {
            self.inner.lock().expect("double mutex").calls.clone()
        }

        fn notifications(&self) -> Vec<(String, Option<Value>)> {
            self.inner
                .lock()
                .expect("double mutex")
                .notifications
                .clone()
        }

        fn version_sets(&self) -> Vec<String> {
            self.inner
                .lock()
                .expect("double mutex")
                .version_sets
                .clone()
        }

        fn registrations(&self) -> Vec<String> {
            self.inner
                .lock()
                .expect("double mutex")
                .registrations
                .clone()
        }

        fn unregisters(&self) -> Vec<String> {
            self.inner.lock().expect("double mutex").unregisters.clone()
        }
    }

    impl Transport for Double {
        fn call(&self, method: &str, params: Option<&Value>) -> Result<Value, TransportError> {
            let mut inner = self.inner.lock().expect("double mutex");
            inner.calls.push((method.to_string(), params.cloned()));
            let replies = inner.script.entry(method.to_string()).or_default();
            if replies.is_empty() {
                return Err(TransportError::NoReply);
            }
            replies.remove(0)
        }

        fn notify(&self, method: &str, params: Option<&Value>) -> Result<(), TransportError> {
            let mut inner = self.inner.lock().expect("double mutex");
            inner
                .notifications
                .push((method.to_string(), params.cloned()));
            Ok(())
        }

        fn era(&self) -> &Era {
            &self.era
        }
    }

    impl ProtocolVersioned for Double {
        fn set_protocol_version(&self, version: &str) {
            self.inner
                .lock()
                .expect("double mutex")
                .version_sets
                .push(version.to_string());
        }
    }

    impl ProgressTransport for Double {
        fn register_progress(
            &self,
            token: &str,
            _sink: &dyn ProgressSink,
        ) -> Box<dyn FnOnce() + Send + Sync> {
            self.inner
                .lock()
                .expect("double mutex")
                .registrations
                .push(token.to_string());
            let inner = self.inner.clone();
            let token = token.to_string();
            Box::new(move || {
                inner.lock().expect("double mutex").unregisters.push(token);
            })
        }
    }

    struct ExpiryByCode;

    impl SessionExpiredProbe for ExpiryByCode {
        fn is_session_expired(&self, err: &TransportError) -> bool {
            matches!(err, TransportError::Rpc(r) if r.code == CODE_SESSION_EXPIRED)
        }
    }

    fn expired_error() -> TransportError {
        TransportError::Rpc(RpcError::new(CODE_SESSION_EXPIRED, "session expired"))
    }

    fn spec(name: &str, workspace_root: &str) -> Spec {
        let mut spec = Spec::new(name);
        spec.workspace_root = workspace_root.to_string();
        spec
    }

    fn legacy_session(spec: Spec, double: Arc<Double>) -> (Session, Arc<Double>) {
        let session = Session::new(spec, Era::legacy(), double.clone())
            .with_versioned(double.clone())
            .with_progress_router(double.clone())
            .with_expiry_probe(Arc::new(ExpiryByCode));
        (session, double)
    }

    #[test]
    fn initialize_sends_legacy_params_with_roots() {
        let double = Arc::new(Double::new(Era::legacy()));
        double.script(
            INITIALIZE_METHOD,
            vec![Ok(json!({
                "protocolVersion": "2025-11-25",
                "capabilities": {"tools": {}},
                "instructions": "  build stuff  "
            }))],
        );
        let (session, double) = legacy_session(spec("demo", "/home/reasonix/project"), double);

        session.initialize().expect("initialize succeeds");

        let calls = double.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, INITIALIZE_METHOD);
        let params = calls[0].1.as_ref().expect("initialize params present");
        assert_eq!(params["protocolVersion"], json!("2025-11-25"));
        let capabilities = &params["capabilities"];
        assert_eq!(capabilities["roots"]["listChanged"], json!(false));
        assert!(capabilities["elicitation"]["form"].is_object());
        assert!(capabilities["elicitation"]["url"].is_object());
        assert_eq!(params["clientInfo"]["name"], json!("reasonix"));
        assert_eq!(params["clientInfo"]["version"], json!("dev"));

        let notifications = double.notifications();
        assert_eq!(notifications.len(), 1);
        assert_eq!(notifications[0].0, INITIALIZED_NOTIFICATION);
        assert_eq!(notifications[0].1, Some(json!({})));

        assert_eq!(
            double.version_sets(),
            vec!["".to_string(), "2025-11-25".to_string()]
        );
        assert_eq!(session.instructions(), "build stuff");
    }

    #[test]
    fn initialize_omits_roots_without_workspace_root() {
        let double = Arc::new(Double::new(Era::legacy()));
        double.script(
            INITIALIZE_METHOD,
            vec![Ok(
                json!({"protocolVersion": "2025-11-25", "capabilities": {}}),
            )],
        );
        let (session, double) = legacy_session(spec("demo", ""), double);

        session.initialize().expect("initialize succeeds");

        let calls = double.calls();
        let params = calls[0].1.as_ref().expect("initialize params");
        let capabilities = &params["capabilities"];
        assert!(capabilities.get("roots").is_none());
        assert!(capabilities["elicitation"]["form"].is_object());
        assert!(capabilities["elicitation"]["url"].is_object());
        assert!(!session.has_tools());
    }

    #[test]
    fn initialize_empty_reply_negotiates_oldest_version() {
        let double = Arc::new(Double::new(Era::legacy()));
        double.script(INITIALIZE_METHOD, vec![Ok(json!({"protocolVersion": ""}))]);
        let (session, double) = legacy_session(spec("demo", ""), double);

        session.initialize().expect("empty reply negotiates oldest");

        assert_eq!(
            double.version_sets(),
            vec!["".to_string(), OLDEST_PROTOCOL_VERSION.to_string()]
        );
        assert_eq!(session.era(), &Era::legacy());
    }

    #[test]
    fn initialize_known_reply_is_kept_verbatim() {
        let double = Arc::new(Double::new(Era::legacy()));
        double.script(
            INITIALIZE_METHOD,
            vec![Ok(json!({"protocolVersion": "2025-06-18"}))],
        );
        let (session, double) = legacy_session(spec("demo", ""), double);

        session.initialize().expect("known version accepted");

        assert_eq!(
            double.version_sets(),
            vec!["".to_string(), "2025-06-18".to_string()]
        );
    }

    #[test]
    fn initialize_unsupported_version_fails_without_notifying() {
        let double = Arc::new(Double::new(Era::legacy()));
        double.script(
            INITIALIZE_METHOD,
            vec![Ok(json!({"protocolVersion": "2026-01-01"}))],
        );
        let (session, double) = legacy_session(spec("demo", ""), double);

        let err = session
            .initialize()
            .expect_err("unsupported version rejected");
        let SessionError::PluginVersion { name, source } = &err else {
            panic!("expected PluginVersion, got {err:?}");
        };
        assert_eq!(name, "demo");
        assert!(matches!(
            source,
            VersionError::UnsupportedProtocolVersion { .. }
        ));
        assert!(err.to_string().starts_with("plugin \"demo\": "));

        assert!(double.notifications().is_empty());
        assert_eq!(double.version_sets(), vec!["".to_string()]);
    }

    #[test]
    fn initialize_records_startup_capabilities_by_presence() {
        let double = Arc::new(Double::new(Era::legacy()));
        double.script(
            INITIALIZE_METHOD,
            vec![Ok(json!({
                "protocolVersion": "2025-11-25",
                "capabilities": {
                    "tools": {"listChanged": true},
                    "prompts": {},
                    "resources": {}
                },
                "instructions": "  catalog here  "
            }))],
        );
        let (session, double) = legacy_session(spec("demo", ""), double);

        session
            .initialize()
            .expect("handshake records capabilities");

        assert!(session.has_tools());
        assert!(session.has_prompts());
        assert!(session.has_resources());
        assert!(session.declares_tool_list_changed());
        assert_eq!(session.instructions(), "catalog here");
        assert_eq!(double.notifications().len(), 1);
    }

    #[test]
    fn runtime_refresh_does_not_rewrite_startup_capabilities() {
        let double = Arc::new(Double::new(Era::legacy()));
        double.script(
            INITIALIZE_METHOD,
            vec![
                Ok(json!({
                    "protocolVersion": "2025-11-25",
                    "capabilities": {
                        "tools": {"listChanged": true},
                        "prompts": {},
                        "resources": {}
                    }
                })),
                Ok(json!({"protocolVersion": "2025-11-25", "capabilities": {}})),
            ],
        );
        let (session, double) = legacy_session(spec("demo", ""), double);

        session.initialize().expect("startup handshake");
        session
            .initialize_session_on(false)
            .expect("runtime refresh");

        assert!(session.has_tools());
        assert!(session.has_prompts());
        assert!(session.has_resources());
        assert!(session.declares_tool_list_changed());
        let notifications = double.notifications();
        assert_eq!(notifications.len(), 2);
        assert!(notifications.iter().all(|(_, p)| *p == Some(json!({}))));
    }

    #[test]
    fn declares_tool_list_changed_reads_bool_only() {
        assert!(!declares_tool_list_changed(None));
        assert!(!declares_tool_list_changed(Some(&json!([]))));
        assert!(!declares_tool_list_changed(Some(
            &json!({"listChanged": "yes"})
        )));
        assert!(!declares_tool_list_changed(Some(
            &json!({"listChanged": false})
        )));
        assert!(declares_tool_list_changed(Some(
            &json!({"listChanged": true})
        )));
    }

    #[test]
    fn timeout_resolution_follows_spec_priority() {
        let mut spec = Spec::new("demo");
        spec.tool_timeouts_ms.insert("raw-a".to_string(), 5_000);
        spec.call_timeout_ms = 20_000;
        spec.default_call_timeout_ms = 10_000;

        let tool_a = Some(json!({"name": "raw-a"}));
        assert_eq!(
            resolve_call_timeout(&spec, TOOLS_CALL_METHOD, tool_a.as_ref(), None),
            Some(Duration::from_millis(5_000))
        );

        let tool_b = Some(json!({"name": "raw-b"}));
        assert_eq!(
            resolve_call_timeout(&spec, TOOLS_CALL_METHOD, tool_b.as_ref(), None),
            Some(Duration::from_millis(20_000))
        );

        assert_eq!(
            resolve_call_timeout(&spec, "ping", None, None),
            Some(Duration::from_millis(20_000))
        );

        let mut lean = Spec::new("demo");
        lean.default_call_timeout_ms = 10_000;
        assert_eq!(
            resolve_call_timeout(&lean, "ping", None, None),
            Some(Duration::from_millis(10_000))
        );

        let bare = Spec::new("demo");
        assert_eq!(
            resolve_call_timeout(&bare, "ping", None, None),
            Some(Duration::from_millis(DEFAULT_CALL_TIMEOUT_MS))
        );

        assert_eq!(
            resolve_call_timeout(
                &spec,
                TOOLS_CALL_METHOD,
                tool_a.as_ref(),
                Some(Duration::from_secs(1))
            ),
            None
        );
    }

    #[test]
    fn timeout_error_text_matches_go_oracle() {
        let session = Session::new(
            spec("demo", ""),
            Era::legacy(),
            Arc::new(Double::new(Era::legacy())),
        );

        let tool = Some(json!({"name": "raw-a"}));
        let tool_secs =
            session.timeout_error(TOOLS_CALL_METHOD, tool.as_ref(), Duration::from_secs(5));
        assert_eq!(
            tool_secs.to_string(),
            "MCP tool \"demo.raw-a\" timed out after 5s; increase tool_timeout_seconds \
             or call_timeout_seconds to allow longer runs: context deadline exceeded"
        );

        let method_secs = session.timeout_error("ping", None, Duration::from_secs(5));
        assert_eq!(
            method_secs.to_string(),
            "MCP method \"ping\" on server \"demo\" timed out after 5s; increase \
             mcp_call_timeout_seconds or call_timeout_seconds to allow longer runs: \
             context deadline exceeded"
        );

        let tool_fraction = session.timeout_error(
            TOOLS_CALL_METHOD,
            tool.as_ref(),
            Duration::from_millis(1_500),
        );
        assert!(tool_fraction.to_string().contains("timed out after 1.5s;"));

        let method_ms = session.timeout_error("ping", None, Duration::from_millis(300));
        assert!(method_ms.to_string().contains("timed out after 300ms;"));

        let tool_no_raw = session.timeout_error(TOOLS_CALL_METHOD, None, Duration::from_secs(5));
        assert!(
            tool_no_raw
                .to_string()
                .starts_with("MCP method \"tools/call\" on server \"demo\"")
        );
    }

    #[test]
    fn progress_token_injected_once_and_unregistered_after_call() {
        let double = Arc::new(Double::new(Era::legacy()));
        double.script(TOOLS_CALL_METHOD, vec![Ok(json!({"ok": true}))]);
        let (session, double) = legacy_session(spec("demo", ""), double);

        let sink: &dyn ProgressSink = &|_chunk: &str| {};
        let params = json!({
            "name": "raw-a",
            "arguments": {"x": 1},
            "_meta": {"existing": "keep"}
        });
        session
            .call(TOOLS_CALL_METHOD, Some(params), Some(sink))
            .expect("tool call succeeds");

        let calls = double.calls();
        let recorded = calls[0].1.as_ref().expect("params recorded");
        assert_eq!(recorded["_meta"]["progressToken"], json!("reasonix-1"));
        assert_eq!(recorded["_meta"]["existing"], json!("keep"));
        assert_eq!(recorded["arguments"]["x"], json!(1));
        assert_eq!(recorded["name"], json!("raw-a"));
        assert_eq!(double.registrations(), vec!["reasonix-1".to_string()]);
        assert_eq!(double.unregisters(), vec!["reasonix-1".to_string()]);
    }

    #[test]
    fn second_eligible_call_gets_next_token() {
        let double = Arc::new(Double::new(Era::legacy()));
        double.script(
            TOOLS_CALL_METHOD,
            vec![Ok(json!({"ok": true})), Ok(json!({"ok": true}))],
        );
        let (session, double) = legacy_session(spec("demo", ""), double);
        let sink: &dyn ProgressSink = &|_chunk: &str| {};

        for _ in 0..2 {
            session
                .call(
                    TOOLS_CALL_METHOD,
                    Some(json!({"name": "raw-a"})),
                    Some(sink),
                )
                .expect("tool call succeeds");
        }

        assert_eq!(
            double.registrations(),
            vec!["reasonix-1".to_string(), "reasonix-2".to_string()]
        );
        assert_eq!(
            double.unregisters(),
            vec!["reasonix-1".to_string(), "reasonix-2".to_string()]
        );
    }

    #[test]
    fn progress_token_skipped_without_sink_or_for_other_methods() {
        let double = Arc::new(Double::new(Era::legacy()));
        double.script(TOOLS_CALL_METHOD, vec![Ok(json!({"ok": true}))]);
        double.script("ping", vec![Ok(json!({}))]);
        let (session, double) = legacy_session(spec("demo", ""), double);

        let tool_params = json!({"name": "raw-a", "_meta": {"existing": "keep"}});
        session
            .call(TOOLS_CALL_METHOD, Some(tool_params.clone()), None)
            .expect("tool call without sink");
        let calls = double.calls();
        let recorded_tool = calls[0].1.as_ref().expect("params recorded");
        assert!(recorded_tool["_meta"].get("progressToken").is_none());
        assert_eq!(recorded_tool["_meta"]["existing"], json!("keep"));

        let sink: &dyn ProgressSink = &|_chunk: &str| {};
        session
            .call("ping", Some(json!({"data": 1})), Some(sink))
            .expect("ping with sink");
        let calls = double.calls();
        let recorded_ping = calls[1].1.as_ref().expect("params recorded");
        assert!(recorded_ping.get("_meta").is_none());
        assert!(double.registrations().is_empty());
    }

    #[test]
    fn expired_session_reinitializes_without_rewriting_flags_and_retries_once() {
        let double = Arc::new(Double::new(Era::legacy()));
        double.script(
            TOOLS_CALL_METHOD,
            vec![Err(expired_error()), Ok(json!({"ok": true}))],
        );
        double.script(
            INITIALIZE_METHOD,
            vec![Ok(
                json!({"protocolVersion": "2025-11-25", "capabilities": {}}),
            )],
        );
        let (session, double) = legacy_session(spec("demo", ""), double);

        let result = session
            .call(TOOLS_CALL_METHOD, Some(json!({"name": "raw-a"})), None)
            .expect("retry succeeds");
        assert_eq!(result, json!({"ok": true}));

        let calls = double.calls();
        assert_eq!(
            calls.iter().map(|(m, _)| m.as_str()).collect::<Vec<_>>(),
            vec![TOOLS_CALL_METHOD, INITIALIZE_METHOD, TOOLS_CALL_METHOD]
        );
        // Refresh must not rewrite startup capability flags.
        assert!(!session.has_tools());
        assert!(!session.has_prompts());
        assert!(!session.has_resources());
        let notifications = double.notifications();
        assert_eq!(notifications.len(), 1);
        assert_eq!(notifications[0].0, INITIALIZED_NOTIFICATION);
    }

    #[test]
    fn expired_reinit_failure_wraps_both_errors() {
        let double = Arc::new(Double::new(Era::legacy()));
        double.script(TOOLS_CALL_METHOD, vec![Err(expired_error())]);
        double.script(INITIALIZE_METHOD, vec![Err(TransportError::NoReply)]);
        let (session, double) = legacy_session(spec("demo", ""), double);

        let err = session
            .call(TOOLS_CALL_METHOD, Some(json!({"name": "raw-a"})), None)
            .expect_err("reinit failure surfaces");
        let rendered = err.to_string();
        assert!(rendered.contains("; reinitialize failed: "));
        assert!(rendered.contains("MCP server did not answer"));

        let calls = double.calls();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].0, TOOLS_CALL_METHOD);
        assert_eq!(calls[1].0, INITIALIZE_METHOD);
    }

    #[test]
    fn initialize_failure_is_not_retried() {
        let double = Arc::new(Double::new(Era::legacy()));
        double.script(INITIALIZE_METHOD, vec![Err(TransportError::NoReply)]);
        let (session, double) = legacy_session(spec("demo", ""), double);

        let err = session
            .initialize()
            .expect_err("initialize failure propagates");
        assert!(matches!(
            err,
            SessionError::Transport(TransportError::NoReply)
        ));
        assert_eq!(double.calls().len(), 1);
        assert!(double.notifications().is_empty());
    }

    #[test]
    fn modern_dispatch_is_reported_as_unimplemented() {
        let double = Arc::new(Double::new(Era::modern(MODERN_PROTOCOL_VERSION)));
        let session = Session::new(spec("demo", ""), double.era().clone(), double.clone());

        let err = session
            .call(TOOLS_CALL_METHOD, Some(json!({"name": "raw-a"})), None)
            .expect_err("modern path not implemented yet");
        assert!(matches!(
            err,
            SessionError::ModernDispatchUnimplemented { .. }
        ));
        assert!(double.calls().is_empty());
    }
}

//! MCP stdio transport: NDJSON over a subprocess's pipes (contract §4).
//!
//! Parity target: `internal/ext/plugin/transport_stdio.go` and
//! `transport_stdio_inbound.go`. One reader thread owns stdout and demuxes each
//! response to the waiting call by id, so a cancelled call abandons its wait at
//! once and calls overlap freely: no lock spans a round trip, so one slow tool
//! cannot hold the pipe against the whole server.
//!
//! Server-originated requests (`ping`, `roots/list`) and progress notifications
//! are answered from the reader thread through a bounded reply queue, never
//! straight to stdin: blocking the reader on the write lock deadlocks both
//! pipes once the server also blocks writing stdout.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::naming::mcp_roots;
use super::seam::TransportError;
use super::stdio_resolve;
use super::types::McpRoot;
use super::version::{
    Era, EraPlan, MODERN_PROTOCOL_VERSION, PROTOCOL_VERSION, ProbeReply, VersionError,
    check_input_bounds, classify_probe_result, client_meta, connect_era, discover_result, pick_era,
    with_meta,
};
use super::wire::{
    RpcError, RpcRequest, decode_inbound_message, decode_rpc_response, format_mcp_progress,
    is_notification_id, progress_token_key, server_request_reply,
};
use crate::redact::redact_credentials;
use crate::sidecar::{CLOSE_WAIT, GRACEFUL_CLOSE_WAIT, STDERR_TAIL_BYTES, TailBuffer};

/// Bound on buffered server-request replies; overflow drops the reply rather
/// than blocking the read loop.
pub const REPLY_QUEUE_BOUND: usize = 16;
/// How long a cancelled call waits for the wire-level cancel notification.
pub const CANCEL_NOTIFY_TIMEOUT: Duration = Duration::from_secs(5);
/// Tool name used for the wire-level cancellation notification.
pub const CANCEL_METHOD: &str = "notifications/cancelled";
/// Progress notification method.
pub const PROGRESS_METHOD: &str = "notifications/progress";
/// Method a client MUST NOT cancel (MCP spec).
const INITIALIZE_METHOD: &str = "initialize";
/// Identity marker forced onto every subprocess env; see `stdio_resolve`.
pub use super::stdio_resolve::AGENT_CHILD_ENV;

/// Why a stdio transport could not be created.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StdioError {
    #[error("stdio plugin {plugin:?}: command is required")]
    MissingCommand { plugin: String },
    #[error("stdio plugin {plugin:?}: command {command:?} was not found on PATH")]
    CommandMissing { plugin: String, command: String },
    #[error("stdio plugin {plugin:?}: spawn {command:?}: {message}")]
    Spawn {
        plugin: String,
        command: String,
        message: String,
    },
    #[error("stdio plugin {plugin:?}: pipes were not created")]
    NoPipes { plugin: String },
}

/// A progress sink the transport routes `notifications/progress` into.
pub type ProgressSink = Arc<dyn Fn(&str) + Send + Sync>;

/// What a call did, so a caller can tell an unanswered probe from a real answer.
#[derive(Debug, Clone, PartialEq)]
pub enum CallOutcome {
    Result(Value),
    RpcError(RpcError),
    NoReply,
}

/// Options for one stdio MCP server.
#[derive(Debug, Clone)]
pub struct StdioOptions {
    /// Server name, used in diagnostics.
    pub name: String,
    /// Executable to launch; never a shell.
    pub command: String,
    pub args: Vec<String>,
    /// Extra environment entries layered over the inherited environment.
    pub env: Vec<(String, String)>,
    /// Working directory; empty keeps the host's.
    pub dir: String,
    /// Workspace base exposed through `roots` and the modern capabilities.
    pub workspace_root: String,
    /// Repository-declared servers fall back to the workspace root as their
    /// cwd; user-installed servers keep the inherited host cwd (Go
    /// `stdioWorkingDir`).
    pub require_launch_approval: bool,
    /// Budget for one call when the caller supplies none.
    pub default_call_timeout: Duration,
}

impl Default for StdioOptions {
    fn default() -> Self {
        Self {
            name: String::new(),
            command: String::new(),
            args: Vec::new(),
            env: Vec::new(),
            dir: String::new(),
            workspace_root: String::new(),
            require_launch_approval: false,
            default_call_timeout: Duration::from_millis(super::lifecycle::DEFAULT_CALL_TIMEOUT_MS),
        }
    }
}

/// One live stdio MCP connection.
///
/// Dropping it kills the child's process tree; `close` performs the bounded
/// stdin-EOF grace then the tree kill.
pub struct StdioTransport {
    name: String,
    workspace_root: String,
    default_call_timeout: Duration,
    pid: u32,
    stdin: Arc<Mutex<Option<ChildStdin>>>,
    next_id: AtomicI64,
    pending: Arc<Mutex<HashMap<i64, Sender<CallOutcome>>>>,
    read_error: Arc<Mutex<Option<String>>>,
    era: Mutex<Era>,
    killed: AtomicBool,
    exited: Arc<AtomicBool>,
    reaper: Mutex<Option<JoinHandle<()>>>,
    reader: Mutex<Option<JoinHandle<()>>>,
    reply_writer: Mutex<Option<JoinHandle<()>>>,
    stderr_thread: Mutex<Option<JoinHandle<()>>>,
    stderr: Arc<TailBuffer>,
    progress: Arc<Mutex<Option<ProgressSink>>>,
    closed: AtomicBool,
}

impl std::fmt::Debug for StdioTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StdioTransport")
            .field("name", &self.name)
            .field("pid", &self.pid)
            .field("era", &self.era.lock().expect("era mutex"))
            .finish()
    }
}

impl StdioTransport {
    /// Spawns the server and starts the reader.
    ///
    /// The command runs in exec form with the inherited environment, so a
    /// wrapper script that invokes `npx` or `uvx` finds its tools; an empty
    /// command is refused rather than guessed at.
    pub fn spawn(options: StdioOptions) -> Result<Self, StdioError> {
        if options.command.trim().is_empty() {
            return Err(StdioError::MissingCommand {
                plugin: options.name.clone(),
            });
        }
        let env = stdio_resolve::build_env(&options.env);
        let work_dir = stdio_resolve::working_dir(
            &options.dir,
            &options.workspace_root,
            options.require_launch_approval,
        );
        let exe = stdio_resolve::resolve_executable(
            &options.name,
            &options.command,
            work_dir.as_deref(),
            &env,
        )?;

        let mut cmd = Command::new(&exe);
        cmd.args(&options.args);
        cmd.env_clear();
        for (key, value) in &env {
            cmd.env(key, value);
        }
        if let Some(dir) = &work_dir {
            cmd.current_dir(dir);
        }
        cmd.stdin(Stdio::piped());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        configure_child(&mut cmd);
        let mut child = cmd.spawn().map_err(|err| StdioError::Spawn {
            plugin: options.name.clone(),
            command: redact_credentials(&exe.display().to_string()),
            message: redact_credentials(&err.to_string()),
        })?;
        let pid = child.id();
        let stdin = child.stdin.take();
        let stdout = child.stdout.take();
        let stderr_pipe = child.stderr.take();
        let (Some(stdin), Some(stdout)) = (stdin, stdout) else {
            let _ = child.kill();
            return Err(StdioError::NoPipes {
                plugin: options.name.clone(),
            });
        };
        let pending: Arc<Mutex<HashMap<i64, Sender<CallOutcome>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let read_error = Arc::new(Mutex::new(None));
        let stdin = Arc::new(Mutex::new(Some(stdin)));
        let exited = Arc::new(AtomicBool::new(false));
        let stderr = Arc::new(TailBuffer::new(STDERR_TAIL_BYTES));
        let stderr_thread = stderr_pipe.map(|pipe| spawn_stderr_drain(pipe, Arc::clone(&stderr)));
        let reaper = spawn_reaper(child, Arc::clone(&exited));
        let (reply_tx, reply_rx) = mpsc::sync_channel::<Value>(REPLY_QUEUE_BOUND);
        let progress: Arc<Mutex<Option<ProgressSink>>> = Arc::new(Mutex::new(None));
        let roots = mcp_roots(&options.workspace_root);
        let reader = {
            let pending = Arc::clone(&pending);
            let read_error = Arc::clone(&read_error);
            let progress = Arc::clone(&progress);
            std::thread::spawn(move || {
                read_loop(stdout, pending, read_error, roots, reply_tx, progress);
            })
        };
        let reply_writer = {
            let stdin = Arc::clone(&stdin);
            std::thread::spawn(move || reply_loop(reply_rx, stdin))
        };
        Ok(Self {
            name: options.name,
            workspace_root: options.workspace_root,
            default_call_timeout: options.default_call_timeout,
            pid,
            stdin,
            next_id: AtomicI64::new(0),
            pending,
            read_error,
            era: Mutex::new(Era::legacy()),
            killed: AtomicBool::new(false),
            exited,
            reaper: Mutex::new(Some(reaper)),
            reader: Mutex::new(Some(reader)),
            reply_writer: Mutex::new(Some(reply_writer)),
            stderr_thread: Mutex::new(stderr_thread),
            stderr,
            progress,
            closed: AtomicBool::new(false),
        })
    }

    /// The server name this transport speaks for.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The process ID, for a bounded tree kill by a supervisor.
    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// Whether the child has already exited.
    pub fn has_exited(&self) -> bool {
        self.exited.load(Ordering::SeqCst)
    }

    /// Retained, redacted stderr tail, for startup diagnostics.
    pub fn stderr_tail(&self) -> String {
        self.stderr.redacted()
    }

    /// Installs the progress sink; `None` drops progress notifications.
    pub fn set_progress_sink(&self, sink: Option<ProgressSink>) {
        *self.progress.lock().expect("progress mutex") = sink;
    }

    /// The roots this connection advertises, for a caller building a reply.
    pub fn roots(&self) -> Vec<McpRoot> {
        mcp_roots(&self.workspace_root)
    }

    /// Reads the negotiated era. Empty means the legacy initialize handshake.
    pub fn era_version(&self) -> Era {
        self.era.lock().expect("era mutex").clone()
    }

    /// Sends one call and classifies the outcome without collapsing identities:
    /// an unanswered call is `NoReply`, a peer error is `RpcError`.
    pub fn call_outcome(
        &self,
        method: &str,
        params: Option<&Value>,
        timeout: Option<Duration>,
    ) -> CallOutcome {
        if self.read_error.lock().expect("read error mutex").is_some() {
            return CallOutcome::NoReply;
        }
        let id = self.next_id.fetch_add(1, Ordering::SeqCst) + 1;
        let (tx, rx) = mpsc::channel();
        self.pending.lock().expect("pending mutex").insert(id, tx);
        let request = RpcRequest::new(id, method, params.cloned());
        if self.write_frame(&request).is_err() {
            self.pending.lock().expect("pending mutex").remove(&id);
            return CallOutcome::NoReply;
        }
        let budget = timeout.unwrap_or(self.default_call_timeout);
        let outcome = match rx.recv_timeout(budget) {
            Ok(outcome) => outcome,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                self.send_cancel(id, method);
                CallOutcome::NoReply
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => CallOutcome::NoReply,
        };
        self.pending.lock().expect("pending mutex").remove(&id);
        outcome
    }

    /// Sends the wire-level cancellation notification, best effort.
    fn send_cancel(&self, id: i64, method: &str) {
        if !cancel_notifiable(method) {
            return;
        }
        let request = RpcRequest::notification(
            CANCEL_METHOD,
            Some(json!({"requestId": id, "reason": "client deadline exceeded"})),
        );
        let _ = self.write_frame(&request);
    }

    fn write_frame(&self, request: &RpcRequest) -> Result<(), TransportError> {
        let frame = request.to_wire();
        let mut guard = self.stdin.lock().expect("stdin mutex");
        let Some(stdin) = guard.as_mut() else {
            return Err(TransportError::Io("mcp connection ended".into()));
        };
        stdin
            .write_all(&frame)
            .and_then(|()| stdin.flush())
            .map_err(|err| TransportError::Io(redact_credentials(&err.to_string())))
    }

    /// Runs the era probe and returns what it learned.
    ///
    /// A probe that drew no reply at all is `EraPlan::Unanswered`: a server SDK
    /// may abort serving on a first non-initialize message while its process
    /// keeps running, so the caller restarts the child before the handshake.
    pub fn probe_era(&self) -> EraPlan {
        let meta = client_meta(MODERN_PROTOCOL_VERSION, &self.workspace_root);
        let params = json!({"_meta": meta});
        let outcome = self.call_outcome(
            super::version::DISCOVER_METHOD,
            Some(&params),
            Some(Duration::from_millis(super::version::MODERN_PROBE_WAIT_MS)),
        );
        classify_probe_result(match outcome {
            CallOutcome::Result(value) => ProbeReply::Result(value),
            CallOutcome::RpcError(error) => ProbeReply::RpcError(error),
            CallOutcome::NoReply => ProbeReply::NoReply,
        })
    }

    /// Runs the legacy `initialize` handshake and records the server's choice.
    pub fn initialize(&self, timeout: Option<Duration>) -> Result<String, TransportError> {
        let params = super::version::initialize_params(&self.workspace_root);
        let outcome = self.call_outcome(INITIALIZE_METHOD, Some(&params), timeout);
        let value = match outcome {
            CallOutcome::Result(value) => value,
            CallOutcome::RpcError(error) => return Err(TransportError::Rpc(error)),
            CallOutcome::NoReply => return Err(TransportError::NoReply),
        };
        let chosen = value
            .get("protocolVersion")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let version = super::version::negotiated_version(chosen)
            .map_err(|err: VersionError| TransportError::Io(err.to_string()))?;
        let _ = self.notify_result("notifications/initialized", Some(&json!({})));
        Ok(version)
    }

    /// Brings a fresh connection up in whichever era the server speaks.
    ///
    /// The probe runs first; an unanswered probe is reported so the caller can
    /// restart the child, and a modern server's era is recorded for later
    /// calls. SSE never speaks modern, but this transport is stdio only.
    pub fn connect(&self) -> Result<Era, TransportError> {
        let plan = self.probe_era();
        if matches!(plan, EraPlan::Unanswered) {
            return Err(TransportError::NoReply);
        }
        let era = match plan {
            EraPlan::Failed(err) => return Err(TransportError::Io(err.to_string())),
            other => now_era(other)?,
        };
        if era.is_legacy() {
            self.initialize(None)?;
        }
        *self.era.lock().expect("era mutex") = era.clone();
        Ok(era)
    }

    /// Sends one call with the wire shape the connection's era requires.
    ///
    /// A modern connection carries `_meta` on every request; a legacy one sends
    /// the params untouched.
    pub fn call_era(&self, method: &str, params: Option<&Value>) -> Result<Value, TransportError> {
        let era = self.era_version();
        let params = if era.is_modern() {
            let meta = client_meta(&era.0, &self.workspace_root);
            with_meta(params, &meta).map_err(|err| TransportError::Io(err.to_string()))?
        } else {
            params.cloned().unwrap_or(Value::Null)
        };
        self.call_result(method, Some(&params))
    }

    /// Bounded close: stdin EOF, a short grace, then the process-tree kill.
    pub fn close(&self) {
        if self.closed.swap(true, Ordering::SeqCst) {
            return;
        }
        self.stdin.lock().expect("stdin mutex").take();
        if !self.wait_for_exit(GRACEFUL_CLOSE_WAIT) {
            self.kill_tree();
            let _ = self.wait_for_exit(CLOSE_WAIT);
        }
        self.join_threads();
        self.fail_pending("mcp connection closed");
    }

    /// Terminates the whole process tree, bounded.
    pub fn kill_tree(&self) {
        if self.killed.swap(true, Ordering::SeqCst) {
            return;
        }
        kill_process_tree(self.pid);
    }

    fn wait_for_exit(&self, budget: Duration) -> bool {
        let started = Instant::now();
        while started.elapsed() < budget {
            if self.has_exited() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5).min(budget));
        }
        self.has_exited()
    }

    fn join_threads(&self) {
        if let Some(handle) = self.reaper.lock().expect("reaper mutex").take() {
            let _ = handle.join();
        }
        if let Some(handle) = self.reader.lock().expect("reader mutex").take() {
            let _ = handle.join();
        }
        if let Some(handle) = self.reply_writer.lock().expect("reply mutex").take() {
            let _ = handle.join();
        }
        if let Some(handle) = self.stderr_thread.lock().expect("stderr mutex").take() {
            let _ = handle.join();
        }
    }

    fn fail_pending(&self, message: &str) {
        let mut guard = self.read_error.lock().expect("read error mutex");
        if guard.is_none() {
            *guard = Some(message.to_string());
        }
        drop(guard);
        let pending = std::mem::take(&mut *self.pending.lock().expect("pending mutex"));
        for (_, sender) in pending {
            let _ = sender.send(CallOutcome::NoReply);
        }
    }
}

fn now_era(plan: EraPlan) -> Result<Era, TransportError> {
    match plan {
        EraPlan::Modern { era, .. } => Ok(era),
        EraPlan::Legacy | EraPlan::Unanswered => Ok(Era::legacy()),
        EraPlan::Failed(err) => Err(TransportError::Io(err.to_string())),
    }
}

impl StdioTransport {
    /// Sends one call through the seam's result shape.
    pub fn call_result(
        &self,
        method: &str,
        params: Option<&Value>,
    ) -> Result<Value, TransportError> {
        match self.call_outcome(method, params, None) {
            CallOutcome::Result(value) => Ok(value),
            CallOutcome::RpcError(error) => Err(TransportError::Rpc(error)),
            CallOutcome::NoReply => Err(TransportError::NoReply),
        }
    }

    /// Sends one fire-and-forget notification.
    pub fn notify_result(
        &self,
        method: &str,
        params: Option<&Value>,
    ) -> Result<(), TransportError> {
        self.write_frame(&RpcRequest::notification(method, params.cloned()))
    }
}

impl Drop for StdioTransport {
    fn drop(&mut self) {
        if !self.closed.load(Ordering::SeqCst) {
            self.kill_tree();
            self.closed.store(true, Ordering::SeqCst);
        }
    }
}

/// Reads one JSON-RPC message per line, routing notifications, answering
/// server requests, and handing each response to the waiting call.
fn read_loop(
    stdout: ChildStdout,
    pending: Arc<Mutex<HashMap<i64, Sender<CallOutcome>>>>,
    read_error: Arc<Mutex<Option<String>>>,
    roots: Vec<McpRoot>,
    replies: mpsc::SyncSender<Value>,
    progress: Arc<Mutex<Option<ProgressSink>>>,
) {
    let mut reader = BufReader::new(stdout);
    let mut line = Vec::new();
    loop {
        line.clear();
        match reader.read_until(b'\n', &mut line) {
            Ok(0) => break,
            Ok(_) => {}
            Err(err) => {
                let mut guard = read_error.lock().expect("read error mutex");
                if guard.is_none() {
                    *guard = Some(redact_credentials(&err.to_string()));
                }
                break;
            }
        }
        let payload = line.trim_ascii();
        if payload.is_empty() {
            continue;
        }
        let Some(message) = decode_inbound_message(payload) else {
            // An unparseable line cannot be routed; the transport stays alive
            // because a server may still answer the next call.
            continue;
        };
        if !message.method.is_empty() {
            let message_id = message.id.clone().unwrap_or(Value::Null);
            if is_notification_id(&message_id) {
                if message.method == PROGRESS_METHOD {
                    dispatch_progress(&progress, message.params.as_ref());
                }
                continue;
            }
            let reply = server_request_reply(message_id, &message.method, roots.clone());
            let value = serde_json::from_str(&reply.to_json()).unwrap_or(Value::Null);
            let _ = replies.try_send(value);
            continue;
        }
        // A response: route it to the call waiting on its id.
        let Some(response) = decode_rpc_response(payload) else {
            continue;
        };
        let Some(id) = response.id.as_i64() else {
            continue;
        };
        let sender = pending.lock().expect("pending mutex").remove(&id);
        let Some(sender) = sender else {
            continue;
        };
        let outcome = match response.error {
            Some(error) => CallOutcome::RpcError(error),
            None => CallOutcome::Result(response.result),
        };
        let _ = sender.send(outcome);
    }
    let mut guard = read_error.lock().expect("read error mutex");
    if guard.is_none() {
        *guard = Some("mcp connection ended".into());
    }
    drop(guard);
    let pending = std::mem::take(&mut *pending.lock().expect("pending mutex"));
    for (_, sender) in pending {
        let _ = sender.send(CallOutcome::NoReply);
    }
}

/// Routes a progress notification to the installed sink, if any.
fn dispatch_progress(progress: &Arc<Mutex<Option<ProgressSink>>>, params: Option<&Value>) {
    let Some(params) = params else { return };
    let token = params
        .get("progressToken")
        .map(progress_token_key)
        .unwrap_or_default();
    if token.is_empty() {
        return;
    }
    let progress_value = params.get("progress").and_then(Value::as_f64);
    let total = params.get("total").and_then(Value::as_f64);
    let message = params
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let guard = progress.lock().expect("progress mutex");
    if let Some(sink) = guard.as_ref() {
        sink(&format_mcp_progress(message, progress_value, total));
    }
}

/// Serialises server-request replies onto stdin, keeping the reader unblocked.
fn reply_loop(replies: mpsc::Receiver<Value>, stdin: Arc<Mutex<Option<ChildStdin>>>) {
    while let Ok(value) = replies.recv() {
        let Ok(text) = serde_json::to_string(&value) else {
            continue;
        };
        let mut guard = stdin.lock().expect("stdin mutex");
        let Some(stdin) = guard.as_mut() else {
            return;
        };
        let mut frame = text.into_bytes();
        frame.push(b'\n');
        if stdin
            .write_all(&frame)
            .and_then(|()| stdin.flush())
            .is_err()
        {
            return;
        }
    }
}

fn spawn_stderr_drain(mut pipe: ChildStderr, tail: Arc<TailBuffer>) -> JoinHandle<()> {
    std::thread::spawn(move || {
        let mut chunk = [0u8; 4096];
        loop {
            match pipe.read(&mut chunk) {
                Ok(0) => return,
                Ok(read) => tail.push(&chunk[..read]),
                Err(_) => return,
            }
        }
    })
}

fn spawn_reaper(mut child: Child, exited: Arc<AtomicBool>) -> JoinHandle<()> {
    std::thread::spawn(move || {
        let _ = child.wait();
        exited.store(true, Ordering::SeqCst);
    })
}

/// Whether a timed-out call may be cancelled on the wire. MCP spec: a client
/// MUST NOT cancel `initialize`.
fn cancel_notifiable(method: &str) -> bool {
    method != INITIALIZE_METHOD
}

#[cfg(windows)]
fn configure_child(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(unix)]
fn configure_child(cmd: &mut Command) {
    use std::os::unix::process::CommandExt;
    cmd.process_group(0);
}

#[cfg(not(any(windows, unix)))]
fn configure_child(_cmd: &mut Command) {}

#[cfg(windows)]
fn kill_process_tree(pid: u32) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut command = Command::new("taskkill");
    command.args(["/F", "/T", "/PID", &pid.to_string()]);
    command.stdin(Stdio::null());
    command.stdout(Stdio::null());
    command.stderr(Stdio::null());
    command.creation_flags(CREATE_NO_WINDOW);
    let _ = command.status();
}

#[cfg(unix)]
fn kill_process_tree(pid: u32) {
    for target in [format!("-{pid}"), pid.to_string()] {
        let _ = Command::new("kill")
            .args(["-KILL", &target])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

#[cfg(not(any(windows, unix)))]
fn kill_process_tree(_pid: u32) {}

/// Names the era a connection ended up in, for diagnostics.
pub fn describe_era(era: &Era) -> &'static str {
    if era.is_modern() { "modern" } else { "legacy" }
}

/// Reports whether a plan is the modern era.
pub fn plan_is_modern(plan: &EraPlan) -> bool {
    matches!(plan, EraPlan::Modern { .. })
}

/// Reads a server's advertised revisions, for the `pick_era` oracle.
pub fn plan_from_discover(value: &Value) -> Option<EraPlan> {
    let found = discover_result(value)?;
    let supported = found.supported_versions.clone();
    pick_era(&supported, Some(found)).ok()
}

/// Re-exported so a caller can build the era from a plan and transport kind.
pub fn era_for(
    transport: super::lifecycle::TransportKind,
    plan: EraPlan,
) -> Result<Era, VersionError> {
    connect_era(transport, plan)
}

/// Bounds one modern input round; a caller answers what it can and refuses the
/// rest by identity.
pub fn input_round_bounded(
    round: usize,
    input_requests: usize,
    request_state_size: usize,
) -> Result<(), VersionError> {
    check_input_bounds(round, input_requests, request_state_size)
}

/// Reads a progress token key from notification params.
pub fn progress_key(params: Option<&Value>) -> String {
    params
        .and_then(|params| params.get("progressToken"))
        .map(progress_token_key)
        .unwrap_or_default()
}

/// The legacy protocol revision this client offers.
pub const OFFERED_PROTOCOL_VERSION: &str = PROTOCOL_VERSION;

/// The raw wire frame for one request, exposed for tests.
pub fn frame_of(request: &RpcRequest) -> Vec<u8> {
    request.to_wire()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_command_is_refused_by_name() {
        let err = StdioTransport::spawn(StdioOptions {
            name: "demo".into(),
            ..StdioOptions::default()
        })
        .unwrap_err();
        assert_eq!(
            err,
            StdioError::MissingCommand {
                plugin: "demo".into()
            }
        );
    }

    #[test]
    fn era_helpers_name_the_plan() {
        assert_eq!(describe_era(&Era::legacy()), "legacy");
        assert_eq!(
            describe_era(&Era::modern(MODERN_PROTOCOL_VERSION)),
            "modern"
        );
        assert!(plan_is_modern(&EraPlan::Modern {
            era: Era::modern(MODERN_PROTOCOL_VERSION),
            discover: None,
        }));
        assert!(!plan_is_modern(&EraPlan::Legacy));
    }

    #[test]
    fn progress_key_reads_the_token() {
        assert_eq!(progress_key(Some(&json!({"progressToken": "t1"}))), "t1");
        assert_eq!(progress_key(Some(&json!({"progressToken": 7}))), "7");
        assert_eq!(progress_key(Some(&json!({}))), "");
    }

    #[test]
    fn cancel_never_notifies_initialize() {
        assert!(!cancel_notifiable(INITIALIZE_METHOD));
        assert!(cancel_notifiable("tools/call"));
        assert!(cancel_notifiable("ping"));
    }
}

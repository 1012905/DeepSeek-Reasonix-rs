//! One bidirectional JSON-RPC 2.0 NDJSON connection (contract §13.3).
//!
//! Parity target: `internal/ext/extension/rpcwire/conn.go`. Transport only: no
//! extension lifecycle or capability semantics. This owns request-ID
//! allocation and the pending map, handler registration, the notification
//! queue, per-call cancellation, one terminal error every caller observes, and
//! the write-stall watchdog over `FrameWriter`.
//!
//! Exactly one thread writes, fed by a bounded queue, so two frames never
//! interleave even when a caller abandons its request mid-flight. Teardown
//! cannot interrupt a blocked read on a pipe this type does not own; a host
//! that must unblock the reader kills the peer, which closes the pipe.

mod gate;
mod writer;

use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender, TrySendError};
use std::sync::{Arc, Mutex, Once};
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::error::{ConnError, ResponseError, rpc_code};
use crate::framing::{
    self, ErrorObject, Frame, FrameError, FrameWriter, OutboundEnvelope, RequestId, WriteError,
};
use crate::rawjson::RawJson;

use gate::{Gate, GateError, remaining};
use writer::{
    NotificationCall, WriteJob, WriteProgress, closed_error, conn_to_write_error, is_caller_abort,
    queue_full, stall_watchdog, write_error_to_conn, writer_loop,
};

/// Inbound handler slots when the caller sets no bound.
pub const DEFAULT_MAX_CONCURRENT_HANDLERS: usize = 64;
/// Outbound frames queued before a producer waits.
pub const WRITE_QUEUE_LIMIT: usize = 256;
/// Best-effort notifications outstanding before `try_notify` drops the event.
pub const BEST_EFFORT_NOTIFY_LIMIT: usize = 16;
/// How long teardown waits for an in-flight physical write.
pub const WRITER_EXIT_WAIT: Duration = Duration::from_millis(100);

/// A handler's typed JSON-RPC error response.
#[derive(Debug, Clone, PartialEq)]
pub struct RpcHandlerError {
    pub code: i32,
    pub message: String,
    pub data: Option<RawJson>,
}

impl RpcHandlerError {
    pub fn new(code: i32, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }

    pub fn with_data(code: i32, message: impl Into<String>, data: RawJson) -> Self {
        Self {
            code,
            message: message.into(),
            data: Some(data),
        }
    }
}

impl std::fmt::Display for RpcHandlerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for RpcHandlerError {}

/// Callback run exactly once after the response frame's physical write.
pub type AfterWrite = Box<dyn FnOnce(Result<(), WriteError>) + Send>;

/// What a request handler returns.
pub struct HandlerResponse {
    pub result: Value,
    pub after_write: Option<AfterWrite>,
}

impl HandlerResponse {
    pub fn new(result: Value) -> Self {
        Self {
            result,
            after_write: None,
        }
    }

    pub fn after_write(result: Value, callback: AfterWrite) -> Self {
        Self {
            result,
            after_write: Some(callback),
        }
    }
}

impl From<Value> for HandlerResponse {
    fn from(result: Value) -> Self {
        Self::new(result)
    }
}

type RequestHandlerFn =
    dyn Fn(Option<&RawJson>) -> Result<HandlerResponse, RpcHandlerError> + Send + Sync;
type NotificationHandlerFn = dyn Fn(Option<&RawJson>) + Send + Sync;
type GateFn = dyn Fn(&str, Option<&RawJson>) -> Result<(), RpcHandlerError> + Send + Sync;
type TerminalHook = dyn Fn() + Send + Sync;

/// Transport-only options; a zero frame limit means unlimited that direction.
#[derive(Default)]
pub struct ConnOptions {
    pub name: String,
    pub max_inbound_bytes: usize,
    pub max_outbound_bytes: usize,
    /// Validates the `jsonrpc` member and mutually exclusive frame shapes.
    pub strict_jsonrpc: bool,
    /// Bounds inbound request handlers; overload answers `-32099 server busy`.
    pub max_concurrent_handlers: usize,
    /// Enables ordered notification delivery through one bounded FIFO worker.
    pub max_queued_notifications: usize,
    /// Runs synchronously on the read loop before a request is dispatched.
    pub before_request: Option<Box<GateFn>>,
    /// Runs synchronously on the read loop before a notification is dispatched.
    pub before_notification: Option<Box<GateFn>>,
    /// Fails the connection when an outbound write makes no progress this long.
    pub max_write_stall: Option<Duration>,
    /// Runs when the connection ends with a transport failure so a host can
    /// tear down a peer whose pipe will never drain.
    pub on_terminal: Option<Arc<TerminalHook>>,
}

impl std::fmt::Debug for ConnOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConnOptions")
            .field("name", &self.name)
            .field("max_inbound_bytes", &self.max_inbound_bytes)
            .field("max_outbound_bytes", &self.max_outbound_bytes)
            .field("strict_jsonrpc", &self.strict_jsonrpc)
            .field("max_concurrent_handlers", &self.max_concurrent_handlers)
            .field("max_queued_notifications", &self.max_queued_notifications)
            .field("max_write_stall", &self.max_write_stall)
            .finish_non_exhaustive()
    }
}

/// State the writer, the watchdog, and every caller share.
struct Wire {
    name: String,
    started: Instant,
    progress: WriteProgress,
    write_slots: Gate,
    notify_slots: Gate,
    handler_slots: Gate,
    closed: AtomicBool,
    terminal: Mutex<Option<ConnError>>,
    close_once: Once,
    inflight: Mutex<HashMap<u64, Sender<Result<(), WriteError>>>>,
    next_write_id: AtomicU64,
    pending: Mutex<HashMap<i64, Sender<Result<Value, ConnError>>>>,
    on_terminal: Option<Arc<TerminalHook>>,
    max_outbound_bytes: usize,
    max_write_stall: Option<Duration>,
}

impl Wire {
    /// Records the first terminal cause, unblocks every producer and pending
    /// call, and fires the terminal hook once for a transport failure.
    fn shutdown(&self, err: Option<ConnError>) {
        let mut fire = false;
        self.close_once.call_once(|| {
            {
                let mut terminal = self.terminal.lock().expect("conn mutex");
                if terminal.is_none() {
                    *terminal = err.clone();
                    fire = err.is_some();
                }
            }
            self.write_slots.close();
            self.notify_slots.close();
            self.handler_slots.close();
            let inflight = std::mem::take(&mut *self.inflight.lock().expect("conn mutex"));
            for (_, sender) in inflight {
                let _ = sender.send(Err(WriteError::Io(closed_error())));
            }
            self.closed.store(true, Ordering::SeqCst);
            let terminal = self.terminal_error();
            let pending = std::mem::take(&mut *self.pending.lock().expect("conn mutex"));
            for (_, sender) in pending {
                let _ = sender.send(Err(terminal.clone()));
            }
        });
        if fire && let Some(hook) = &self.on_terminal {
            hook();
        }
    }

    fn terminal_error(&self) -> ConnError {
        if let Some(err) = self.terminal.lock().expect("conn mutex").clone() {
            return err;
        }
        ConnError::Closed {
            name: self.name.clone(),
        }
    }

    fn close_reason(&self) -> Option<ConnError> {
        self.terminal.lock().expect("conn mutex").clone()
    }
}

struct Inner<R, W: Write> {
    wire: Arc<Wire>,
    max_inbound_bytes: usize,
    strict: bool,
    before_request: Option<Box<GateFn>>,
    before_notification: Option<Box<GateFn>>,
    reader: Mutex<Option<R>>,
    writer: Mutex<Option<FrameWriter<W>>>,
    write_tx: Mutex<Option<SyncSender<WriteJob>>>,
    write_done: Mutex<Option<Receiver<()>>>,
    writer_started: Once,
    next_id: AtomicI64,
    requests: Mutex<HashMap<String, Arc<RequestHandlerFn>>>,
    notifications: Mutex<HashMap<String, Arc<NotificationHandlerFn>>>,
    notify_rx: Mutex<Option<Receiver<NotificationCall>>>,
    notify_tx: Mutex<Option<SyncSender<NotificationCall>>>,
}

impl<R, W: Write> Drop for Inner<R, W> {
    /// Dropping the last handle ends the connection: pending callers observe
    /// the terminal error instead of waiting for a writer nobody reads.
    fn drop(&mut self) {
        self.wire.shutdown(None);
    }
}

/// A cloneable handle to one connection.
pub struct Conn<R, W: Write> {
    inner: Arc<Inner<R, W>>,
}

impl<R, W: Write> Clone for Conn<R, W> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<R, W: Write> std::fmt::Debug for Conn<R, W> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Conn")
            .field("name", &self.inner.wire.name)
            .field("closed", &self.inner.wire.closed.load(Ordering::SeqCst))
            .finish()
    }
}

impl<R, W> Conn<R, W>
where
    R: BufRead + Send + 'static,
    W: Write + Send + 'static,
{
    /// Constructs a connection. Register handlers before calling `serve`.
    pub fn new(reader: R, writer: W, opts: ConnOptions) -> Self {
        let name = if opts.name.is_empty() {
            "rpcwire".to_string()
        } else {
            opts.name.clone()
        };
        let handlers = if opts.max_concurrent_handlers == 0 {
            DEFAULT_MAX_CONCURRENT_HANDLERS
        } else {
            opts.max_concurrent_handlers
        };
        let (notify_tx, notify_rx) = if opts.max_queued_notifications > 0 {
            let (tx, rx) = mpsc::sync_channel(opts.max_queued_notifications);
            (Some(tx), Some(rx))
        } else {
            (None, None)
        };
        let wire = Arc::new(Wire {
            name,
            started: Instant::now(),
            progress: WriteProgress::default(),
            write_slots: Gate::new(WRITE_QUEUE_LIMIT),
            notify_slots: Gate::new(BEST_EFFORT_NOTIFY_LIMIT),
            handler_slots: Gate::new(handlers),
            closed: AtomicBool::new(false),
            terminal: Mutex::new(None),
            close_once: Once::new(),
            inflight: Mutex::new(HashMap::new()),
            next_write_id: AtomicU64::new(0),
            pending: Mutex::new(HashMap::new()),
            on_terminal: opts.on_terminal,
            max_outbound_bytes: opts.max_outbound_bytes,
            max_write_stall: opts.max_write_stall,
        });
        Self {
            inner: Arc::new(Inner {
                wire,
                max_inbound_bytes: opts.max_inbound_bytes,
                strict: opts.strict_jsonrpc,
                before_request: opts.before_request,
                before_notification: opts.before_notification,
                reader: Mutex::new(Some(reader)),
                writer: Mutex::new(Some(FrameWriter::new(writer, usize::MAX))),
                write_tx: Mutex::new(None),
                write_done: Mutex::new(None),
                writer_started: Once::new(),
                next_id: AtomicI64::new(0),
                requests: Mutex::new(HashMap::new()),
                notifications: Mutex::new(HashMap::new()),
                notify_rx: Mutex::new(notify_rx),
                notify_tx: Mutex::new(notify_tx),
            }),
        }
    }

    /// Registers a request handler. Registering twice replaces the first.
    pub fn handle<F>(&self, method: &str, handler: F)
    where
        F: Fn(Option<&RawJson>) -> Result<HandlerResponse, RpcHandlerError> + Send + Sync + 'static,
    {
        self.inner
            .requests
            .lock()
            .expect("conn mutex")
            .insert(method.to_string(), Arc::new(handler));
    }

    /// Registers a notification handler.
    pub fn handle_notify<F>(&self, method: &str, handler: F)
    where
        F: Fn(Option<&RawJson>) + Send + Sync + 'static,
    {
        self.inner
            .notifications
            .lock()
            .expect("conn mutex")
            .insert(method.to_string(), Arc::new(handler));
    }

    /// Reads and dispatches frames until EOF, a read error, or connection close.
    pub fn serve(&self) -> Result<(), ConnError> {
        self.ensure_writer();
        let mut reader = match self.inner.reader.lock().expect("conn mutex").take() {
            Some(reader) => reader,
            None => return Err(self.inner.wire.terminal_error()),
        };
        let worker = self.spawn_notify_worker();
        let mut read_error: Option<ConnError> = None;
        loop {
            match framing::read_frame(&mut reader, self.inner.max_inbound_bytes) {
                Ok(Some((payload, _))) => self.dispatch(&payload),
                Ok(None) => break,
                Err(framing::ReadError::TooLarge(too_large)) => {
                    read_error = Some(ConnError::FrameTooLarge(too_large));
                    break;
                }
                Err(framing::ReadError::Io(io)) => {
                    read_error = Some(ConnError::Io(io.to_string()));
                    break;
                }
            }
            if self.is_closed() {
                break;
            }
        }
        if let Some(worker) = worker {
            self.inner.notify_tx.lock().expect("conn mutex").take();
            let _ = worker.join();
        }
        if let Some(err) = read_error {
            self.inner.wire.shutdown(Some(err.clone()));
            self.stop_writer();
            return Err(err);
        }
        self.stop_writer();
        self.inner.wire.shutdown(None);
        match self.inner.wire.close_reason() {
            Some(err) => Err(err),
            None => Ok(()),
        }
    }

    /// Sends a request and waits for its response, cancellation, or closure.
    ///
    /// A caller deadline fails only this call; the connection stays usable.
    pub fn request(
        &self,
        method: &str,
        params: Option<&Value>,
        timeout: Option<Duration>,
    ) -> Result<Value, ConnError> {
        if let Some(err) = self.inner.wire.close_reason() {
            return Err(err);
        }
        let params_raw = params.map(encode_params).transpose()?;
        let id = self.inner.next_id.fetch_add(1, Ordering::SeqCst) + 1;
        let (tx, rx) = mpsc::channel();
        {
            let mut pending = self.inner.wire.pending.lock().expect("conn mutex");
            if self.is_closed() {
                drop(pending);
                return Err(self.inner.wire.terminal_error());
            }
            pending.insert(id, tx);
        }
        let deadline = timeout.map(|budget| Instant::now() + budget);
        let envelope = OutboundEnvelope {
            jsonrpc: "2.0",
            id: Some(RequestId::Integer(id)),
            method: Some(method),
            params: params_raw.as_ref(),
            result: None,
            error: None,
        };
        if let Err(err) = self.write_envelope(&envelope, deadline) {
            self.inner
                .wire
                .pending
                .lock()
                .expect("conn mutex")
                .remove(&id);
            if !is_caller_abort(&err) {
                self.inner.wire.shutdown(Some(err.clone()));
            }
            return Err(err);
        }
        let outcome = match deadline {
            Some(deadline) => match remaining(deadline) {
                Some(left) => rx.recv_timeout(left),
                None => Err(RecvTimeoutError::Timeout),
            },
            None => rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
        };
        self.inner
            .wire
            .pending
            .lock()
            .expect("conn mutex")
            .remove(&id);
        match outcome {
            Ok(result) => result,
            Err(RecvTimeoutError::Timeout) => Err(ConnError::TimedOut),
            Err(RecvTimeoutError::Disconnected) => Err(self.inner.wire.terminal_error()),
        }
    }

    /// Sends a fire-and-forget notification on the blocking write path.
    pub fn notify(&self, method: &str, params: Option<&Value>) -> Result<(), ConnError> {
        let params_raw = params.map(encode_params).transpose()?;
        let envelope = OutboundEnvelope {
            jsonrpc: "2.0",
            id: None,
            method: Some(method),
            params: params_raw.as_ref(),
            result: None,
            error: None,
        };
        let result = self.write_envelope(&envelope, None);
        if let Err(err) = &result
            && !matches!(err, ConnError::FrameTooLarge(_))
        {
            self.inner.wire.shutdown(Some(err.clone()));
        }
        result
    }

    /// Enqueues a notification without waiting for the physical write.
    ///
    /// `Ok(())` means the bounded writer accepted the frame, not that the peer
    /// processed it. A full best-effort pool returns `QueueFull` so an
    /// observation-only caller can drop the event instead of stalling.
    pub fn try_notify(&self, method: &str, params: Option<&Value>) -> Result<(), ConnError> {
        let params_raw = params.map(encode_params).transpose()?;
        let envelope = OutboundEnvelope {
            jsonrpc: "2.0",
            id: None,
            method: Some(method),
            params: params_raw.as_ref(),
            result: None,
            error: None,
        };
        let frame = framing::encode_frame(&envelope, self.inner.wire.max_outbound_bytes)?;
        self.ensure_writer();
        let wire = Arc::clone(&self.inner.wire);
        if !wire.notify_slots.try_acquire() {
            return Err(queue_full(BEST_EFFORT_NOTIFY_LIMIT));
        }
        if !wire.write_slots.try_acquire() {
            wire.notify_slots.release();
            return Err(queue_full(BEST_EFFORT_NOTIFY_LIMIT));
        }
        let release: Arc<dyn Fn() + Send + Sync> = Arc::new(move || wire.notify_slots.release());
        match self.enqueue(frame, Arc::new(AtomicBool::new(false)), Some(release), None) {
            Ok(()) => Ok(()),
            Err(err) => {
                self.inner.wire.notify_slots.release();
                Err(err)
            }
        }
    }

    /// Reports whether the connection has ended.
    pub fn is_closed(&self) -> bool {
        self.inner.wire.closed.load(Ordering::SeqCst)
    }

    /// Ends the connection without a transport cause.
    ///
    /// Used at orderly teardown: pending callers observe `Closed` rather than
    /// waiting for a reader that will never report.
    pub fn terminate(&self) {
        self.inner.wire.shutdown(None);
        self.stop_writer();
    }

    fn spawn_notify_worker(&self) -> Option<std::thread::JoinHandle<()>> {
        let receiver = self.inner.notify_rx.lock().expect("conn mutex").take()?;
        Some(std::thread::spawn(move || {
            while let Ok(call) = receiver.recv() {
                (call.handler)(call.params.as_ref());
            }
        }))
    }

    fn dispatch(&self, line: &str) {
        let decoded = if self.inner.strict {
            framing::decode_frame(line)
        } else {
            decode_lenient(line)
        };
        match decoded {
            Ok(Frame::Request {
                id, method, params, ..
            }) => self.dispatch_request(id, &method, params.as_ref()),
            Ok(Frame::Notification { method, params, .. }) => {
                self.dispatch_notification(&method, params.as_ref());
            }
            Ok(Frame::Response {
                id, result, error, ..
            }) => self.resolve(&id, result.as_ref(), error.as_ref()),
            Err(FrameError::Parse) => {
                self.respond_error(RequestId::Null, rpc_code::PARSE, "parse error", None);
            }
            Err(_) => {
                let id = safe_request_id(line);
                self.respond_error(id, rpc_code::INVALID_REQUEST, "invalid request", None);
            }
        }
    }

    fn dispatch_request(&self, id: RequestId, method: &str, params: Option<&RawJson>) {
        if let Some(gate) = &self.inner.before_request
            && let Err(err) = gate(method, params)
        {
            self.respond_handler_error(id, &err);
            return;
        }
        let handler = self
            .inner
            .requests
            .lock()
            .expect("conn mutex")
            .get(method)
            .cloned();
        let Some(handler) = handler else {
            self.respond_error(
                id,
                rpc_code::METHOD_NOT_FOUND,
                &format!("method not found: {method}"),
                None,
            );
            return;
        };
        if !self.inner.wire.handler_slots.try_acquire() {
            self.respond_error(id, rpc_code::SERVER_BUSY, "server busy", None);
            return;
        }
        let conn = self.clone();
        let params = params.cloned();
        std::thread::spawn(move || {
            let outcome = handler(params.as_ref());
            conn.inner.wire.handler_slots.release();
            match outcome {
                Ok(response) => conn.respond_result(id, response),
                Err(err) => conn.respond_handler_error(id, &err),
            }
        });
    }

    fn dispatch_notification(&self, method: &str, params: Option<&RawJson>) {
        if let Some(gate) = &self.inner.before_notification
            && gate(method, params).is_err()
        {
            return;
        }
        let handler = self
            .inner
            .notifications
            .lock()
            .expect("conn mutex")
            .get(method)
            .cloned();
        let Some(handler) = handler else {
            return;
        };
        let sender = self.inner.notify_tx.lock().expect("conn mutex").clone();
        if let Some(queue) = sender {
            let call = NotificationCall {
                handler,
                params: params.cloned(),
            };
            if queue.try_send(call).is_err() {
                self.inner.wire.shutdown(Some(ConnError::Io(format!(
                    "{}: notification queue overflow",
                    self.inner.wire.name
                ))));
            }
            return;
        }
        if !self.inner.wire.handler_slots.try_acquire() {
            return;
        }
        let wire = Arc::clone(&self.inner.wire);
        let params = params.cloned();
        std::thread::spawn(move || {
            handler(params.as_ref());
            wire.handler_slots.release();
        });
    }

    fn resolve(&self, id: &RequestId, result: Option<&RawJson>, error: Option<&ErrorObject>) {
        let RequestId::Integer(id) = id else {
            return;
        };
        let sender = self
            .inner
            .wire
            .pending
            .lock()
            .expect("conn mutex")
            .remove(id);
        let Some(sender) = sender else {
            return;
        };
        let outcome = match error {
            Some(error) => Err(ConnError::Response(ResponseError {
                code: error.code,
                message: error.message.clone(),
                data: error.data.clone(),
            })),
            None => match result {
                Some(result) => serde_json::from_str(result.get())
                    .map_err(|err| ConnError::Io(format!("response result is not JSON: {err}"))),
                None => Ok(Value::Null),
            },
        };
        let _ = sender.send(outcome);
    }

    fn respond_result(&self, id: RequestId, response: HandlerResponse) {
        let HandlerResponse {
            result,
            after_write,
        } = response;
        let retry_id = id.clone();
        let encoded = match serde_json::to_string(&result) {
            Ok(encoded) => encoded,
            Err(err) => {
                self.respond_error(id, rpc_code::INTERNAL, "marshal result", None);
                if let Some(after) = after_write {
                    after(Err(WriteError::Io(std::io::Error::other(err.to_string()))));
                }
                return;
            }
        };
        let raw = match RawJson::parse(&encoded) {
            Ok(raw) => raw,
            Err(err) => {
                self.respond_error(id, rpc_code::INTERNAL, "marshal result", None);
                if let Some(after) = after_write {
                    after(Err(WriteError::Io(std::io::Error::other(err.to_string()))));
                }
                return;
            }
        };
        let envelope = OutboundEnvelope {
            jsonrpc: "2.0",
            id: Some(id),
            method: None,
            params: None,
            result: Some(&raw),
            error: None,
        };
        let written = self.write_envelope(&envelope, None);
        match &written {
            Err(ConnError::FrameTooLarge(_)) => self.respond_error(
                retry_id,
                rpc_code::INTERNAL,
                "response exceeds frame size limit",
                None,
            ),
            Err(err) => self.inner.wire.shutdown(Some(err.clone())),
            Ok(()) => {}
        }
        if let Some(after) = after_write {
            after(written.as_ref().map(|_| ()).map_err(conn_to_write_error));
        }
    }

    fn respond_handler_error(&self, id: RequestId, err: &RpcHandlerError) {
        self.respond_error(id, err.code, &err.message, err.data.as_ref());
    }

    fn respond_error(&self, id: RequestId, code: i32, message: &str, data: Option<&RawJson>) {
        let error = ErrorObject {
            code,
            message: message.to_string(),
            data: data.cloned(),
        };
        let retry_id = id.clone();
        let envelope = OutboundEnvelope {
            jsonrpc: "2.0",
            id: Some(id),
            method: None,
            params: None,
            result: None,
            error: Some(&error),
        };
        match self.write_envelope(&envelope, None) {
            Ok(()) => {}
            Err(ConnError::FrameTooLarge(_)) => {
                let fallback = ErrorObject {
                    code: rpc_code::INTERNAL,
                    message: "error response exceeds frame size limit".into(),
                    data: None,
                };
                let envelope = OutboundEnvelope {
                    jsonrpc: "2.0",
                    id: Some(retry_id),
                    method: None,
                    params: None,
                    result: None,
                    error: Some(&fallback),
                };
                if let Err(err) = self.write_envelope(&envelope, None) {
                    self.inner.wire.shutdown(Some(err));
                }
            }
            Err(err) => self.inner.wire.shutdown(Some(err)),
        }
    }

    fn write_envelope(
        &self,
        envelope: &OutboundEnvelope<'_>,
        deadline: Option<Instant>,
    ) -> Result<(), ConnError> {
        let frame = framing::encode_frame(envelope, self.inner.wire.max_outbound_bytes)?;
        self.ensure_writer();
        match self.inner.wire.write_slots.acquire(deadline) {
            Ok(()) => {}
            Err(GateError::TimedOut) => return Err(ConnError::TimedOut),
            Err(GateError::Closed) => return Err(self.inner.wire.terminal_error()),
        }
        self.enqueue(frame, Arc::new(AtomicBool::new(false)), None, deadline)
    }

    fn enqueue(
        &self,
        frame: Vec<u8>,
        aborted: Arc<AtomicBool>,
        release: Option<Arc<dyn Fn() + Send + Sync>>,
        deadline: Option<Instant>,
    ) -> Result<(), ConnError> {
        let (result_tx, result_rx) = mpsc::channel();
        let write_id = self.inner.wire.next_write_id.fetch_add(1, Ordering::SeqCst);
        self.inner
            .wire
            .inflight
            .lock()
            .expect("conn mutex")
            .insert(write_id, result_tx.clone());
        {
            let guard = self.inner.write_tx.lock().expect("conn mutex");
            let Some(queue) = guard.as_ref() else {
                drop(guard);
                self.remove_inflight(write_id);
                self.inner.wire.write_slots.release();
                return Err(self.inner.wire.terminal_error());
            };
            let job = WriteJob {
                frame,
                aborted: Arc::clone(&aborted),
                release,
                result: result_tx.clone(),
            };
            match queue.try_send(job) {
                Ok(()) => {}
                Err(TrySendError::Full(_)) => {
                    drop(guard);
                    self.remove_inflight(write_id);
                    self.inner.wire.write_slots.release();
                    return Err(queue_full(WRITE_QUEUE_LIMIT));
                }
                Err(TrySendError::Disconnected(_)) => {
                    drop(guard);
                    self.remove_inflight(write_id);
                    self.inner.wire.write_slots.release();
                    return Err(self.inner.wire.terminal_error());
                }
            }
        }
        let outcome = match deadline {
            Some(deadline) => match remaining(deadline) {
                Some(left) => result_rx.recv_timeout(left),
                None => Err(RecvTimeoutError::Timeout),
            },
            None => result_rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
        };
        self.remove_inflight(write_id);
        match outcome {
            Ok(Ok(())) => Ok(()),
            Ok(Err(err)) => Err(write_error_to_conn(err)),
            Err(RecvTimeoutError::Timeout) => {
                // The writer skips a frame that has not physically started and
                // finishes one it did: frames stay whole and ordered.
                aborted.store(true, Ordering::SeqCst);
                Err(ConnError::TimedOut)
            }
            Err(RecvTimeoutError::Disconnected) => Err(self.inner.wire.terminal_error()),
        }
    }

    fn remove_inflight(&self, write_id: u64) {
        self.inner
            .wire
            .inflight
            .lock()
            .expect("conn mutex")
            .remove(&write_id);
    }

    fn ensure_writer(&self) {
        let inner = &self.inner;
        inner.writer_started.call_once(|| {
            let (tx, rx) = mpsc::sync_channel::<WriteJob>(WRITE_QUEUE_LIMIT);
            let (done_tx, done_rx) = mpsc::channel();
            *inner.write_tx.lock().expect("conn mutex") = Some(tx);
            *inner.write_done.lock().expect("conn mutex") = Some(done_rx);
            let writer = inner.writer.lock().expect("conn mutex").take();
            let wire = Arc::clone(&inner.wire);
            std::thread::spawn(move || {
                if let Some(writer) = writer {
                    writer_loop(&wire, writer, rx);
                }
                let _ = done_tx.send(());
            });
            if let Some(bound) = inner.wire.max_write_stall {
                let weak = Arc::downgrade(&inner.wire);
                std::thread::spawn(move || stall_watchdog(weak, bound));
            }
        });
    }

    fn stop_writer(&self) {
        self.inner.write_tx.lock().expect("conn mutex").take();
        if let Some(done) = self.inner.write_done.lock().expect("conn mutex").as_ref() {
            let _ = done.recv_timeout(WRITER_EXIT_WAIT);
        }
    }
}

fn encode_params(value: &Value) -> Result<RawJson, ConnError> {
    let text = serde_json::to_string(value)
        .map_err(|err| ConnError::Io(format!("params are not JSON: {err}")))?;
    RawJson::parse(&text).map_err(|err| ConnError::Io(format!("params are not JSON: {err}")))
}

fn safe_request_id(line: &str) -> RequestId {
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        return RequestId::Null;
    };
    match value.get("id") {
        Some(Value::String(id)) => RequestId::String(id.clone()),
        Some(Value::Number(id)) => id
            .as_i64()
            .map(RequestId::Integer)
            .unwrap_or(RequestId::Null),
        _ => RequestId::Null,
    }
}

/// Classifies a frame without strict validation, for peers that disable it.
fn decode_lenient(line: &str) -> Result<Frame, FrameError> {
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        return Err(FrameError::Parse);
    };
    let Some(object) = value.as_object() else {
        return Err(FrameError::Parse);
    };
    let jsonrpc = object
        .get("jsonrpc")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let method = object
        .get("method")
        .and_then(Value::as_str)
        .filter(|method| !method.is_empty())
        .map(str::to_string);
    let id = match object.get("id") {
        Some(Value::String(id)) => Some(RequestId::String(id.clone())),
        Some(Value::Number(id)) => id.as_i64().map(RequestId::Integer),
        Some(Value::Null) => Some(RequestId::Null),
        _ => None,
    };
    let params = object
        .get("params")
        .and_then(|params| RawJson::parse(&params.to_string()).ok());
    let result = object
        .get("result")
        .and_then(|result| RawJson::parse(&result.to_string()).ok());
    let error = object
        .get("error")
        .and_then(|error| serde_json::from_value::<ErrorObject>(error.clone()).ok());
    match (method, id) {
        (Some(method), Some(id)) => Ok(Frame::Request {
            jsonrpc,
            id,
            method,
            params,
        }),
        (Some(method), None) => Ok(Frame::Notification {
            jsonrpc,
            method,
            params,
        }),
        (None, Some(id)) => Ok(Frame::Response {
            jsonrpc,
            id,
            result,
            error,
        }),
        (None, None) => Err(FrameError::InvalidRequest("missing method and id")),
    }
}

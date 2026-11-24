//! Legacy HTTP+SSE transport (MCP-012, contract §6).
//!
//! Oracle: `internal/ext/plugin/transport_sse.go`. The client keeps one
//! long-lived GET stream open; the server announces the POST endpoint through
//! an `event: endpoint` frame, and every JSON-RPC message travels to that
//! endpoint while responses and server messages return on the GET stream.
//! Responses demux by request id, so calls overlap freely. Server-originated
//! requests are answered by one bounded reply worker; a flooded queue drops
//! excess replies so the GET reader never stalls behind a blocked POST.
//!
//! This slice owns transport framing only: the initialize handshake,
//! progress-sink registration, and notification handlers attach in the
//! lifecycle layer.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use serde_json::{Value, json};

use super::http::{
    HttpClient, HttpClientConfig, HttpError, HttpReply, resolve_against, same_origin,
};
use super::naming::mcp_roots;
use super::seam::{OwnedTransport, Transport, TransportError};
use super::types::McpRoot;
use super::version::Era;
use super::wire::{
    InboundMessage, RpcRequest, RpcResponse, decode_inbound_message, decode_rpc_response,
    is_notification_id, server_request_reply,
};
use crate::redact::redact_credentials;

/// Bound on the server-request reply queue; overflow drops the reply rather
/// than blocking the GET reader (Go `sseReplyQueueBound`).
pub const SSE_REPLY_QUEUE_BOUND: usize = 16;
/// Reply POST deadline when no call timeout is configured.
const DEFAULT_REPLY_TIMEOUT: Duration = Duration::from_secs(30);
/// Call wait deadline when no call timeout is configured.
const DEFAULT_CALL_TIMEOUT: Duration = Duration::from_millis(300_000);
/// Budget for the initial GET to answer headers.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// Best-effort deadline for a wire-level cancellation notification.
const CANCEL_NOTIFY_TIMEOUT: Duration = Duration::from_secs(5);
/// Wire-level cancellation notification method.
const CANCEL_METHOD: &str = "notifications/cancelled";
/// A client MUST NOT cancel the initialize request.
const INITIALIZE_METHOD: &str = "initialize";

/// Typed SSE failure prose, matching the Go oracle's host-visible causes.
const ERR_CROSS_ORIGIN: &str = "server announced a cross-origin endpoint";
const ERR_CONTENT_TYPE: &str = "SSE response has an unexpected content type";
const ERR_ENDPOINT_MISSING: &str = "SSE stream ended before announcing an endpoint";

/// Options for one legacy HTTP+SSE MCP server.
#[derive(Clone, Debug)]
pub struct LegacySseConfig {
    /// The long-lived GET URL.
    pub url: String,
    /// Static headers sent on every request (credentials stay header-only).
    pub headers: std::collections::BTreeMap<String, String>,
    /// Era fixed at connect time; `Transport::era` reports this same value.
    pub era: Era,
    /// Per-call deadline; `None` falls back to the built-in default.
    pub call_timeout: Option<Duration>,
    /// Workspace base advertised through `roots` on server-request replies.
    pub workspace_root: String,
}

/// Endpoint announcement state, signaled once the reader has seen (or failed
/// on) the `endpoint` frame.
struct EndpointInner {
    ready: bool,
    endpoint: Option<String>,
    endpoint_err: Option<String>,
}

/// In-flight calls plus the latched read failure.
struct PendingState {
    pending: HashMap<i64, Sender<RpcResponse>>,
    read_error: Option<String>,
}

/// One live legacy HTTP+SSE connection.
pub struct LegacySseTransport {
    client: Arc<HttpClient>,
    endpoint: Arc<Mutex<EndpointInner>>,
    endpoint_ready: Arc<Condvar>,
    pending: Arc<Mutex<PendingState>>,
    next_id: AtomicI64,
    era: Era,
    reply_tx: Mutex<Option<SyncSender<InboundMessage>>>,
    call_timeout: Duration,
    static_headers: std::collections::BTreeMap<String, String>,
    reader: Mutex<Option<JoinHandle<()>>>,
    replier: Mutex<Option<JoinHandle<()>>>,
    closed: AtomicBool,
}

impl LegacySseTransport {
    /// Builds the HTTP client and starts the reader and reply worker. The GET
    /// stream opens on the reader thread; `call` waits for the endpoint
    /// announcement before posting.
    pub fn connect(config: LegacySseConfig) -> Result<Self, TransportError> {
        if config.url.trim().is_empty() {
            return Err(TransportError::Io("sse endpoint url is required".into()));
        }
        let call_timeout = config.call_timeout.unwrap_or(DEFAULT_CALL_TIMEOUT);
        let reply_timeout = config.call_timeout.unwrap_or(DEFAULT_REPLY_TIMEOUT);
        let client = Arc::new(
            HttpClient::new(HttpClientConfig {
                url: config.url.clone(),
                headers: config.headers.clone(),
                call_timeout,
            })
            .map_err(|err| TransportError::Io(redact_credentials(&err.to_string())))?,
        );
        let endpoint = Arc::new(Mutex::new(EndpointInner {
            ready: false,
            endpoint: None,
            endpoint_err: None,
        }));
        let endpoint_ready = Arc::new(Condvar::new());
        let pending = Arc::new(Mutex::new(PendingState {
            pending: HashMap::new(),
            read_error: None,
        }));
        let (reply_tx, reply_rx) = mpsc::sync_channel::<InboundMessage>(SSE_REPLY_QUEUE_BOUND);
        let roots = mcp_roots(&config.workspace_root);
        let static_headers = config.headers.clone();
        let base_url = config.url.clone();

        let reader = {
            let client = client.clone();
            let endpoint = endpoint.clone();
            let endpoint_ready = endpoint_ready.clone();
            let pending = pending.clone();
            let reply_tx = reply_tx.clone();
            let static_headers = static_headers.clone();
            std::thread::spawn(move || {
                read_loop(
                    &client,
                    &base_url,
                    &static_headers,
                    &endpoint,
                    &endpoint_ready,
                    &pending,
                    reply_tx,
                );
            })
        };
        let replier = {
            let client = client.clone();
            let endpoint = endpoint.clone();
            let pending = pending.clone();
            let static_headers = static_headers.clone();
            std::thread::spawn(move || {
                reply_loop(
                    reply_rx,
                    &client,
                    &endpoint,
                    &pending,
                    roots,
                    static_headers,
                    reply_timeout,
                );
            })
        };

        Ok(Self {
            client,
            endpoint,
            endpoint_ready,
            pending,
            next_id: AtomicI64::new(0),
            era: config.era,
            reply_tx: Mutex::new(Some(reply_tx)),
            call_timeout,
            static_headers,
            reader: Mutex::new(Some(reader)),
            replier: Mutex::new(Some(replier)),
            closed: AtomicBool::new(false),
        })
    }

    /// Blocks until the endpoint announcement settles; an announcement failure
    /// or a stream that ended before announcing surfaces here.
    fn wait_endpoint(&self) -> Result<(), TransportError> {
        let mut guard = self.endpoint.lock().expect("endpoint mutex");
        while !guard.ready {
            guard = self.endpoint_ready.wait(guard).expect("endpoint condvar");
        }
        if let Some(err) = &guard.endpoint_err {
            return Err(TransportError::Io(err.clone()));
        }
        if guard.endpoint.is_none() {
            return Err(TransportError::Io(ERR_ENDPOINT_MISSING.into()));
        }
        Ok(())
    }

    /// The announced absolute POST URL, if one settled.
    fn current_endpoint(&self) -> Option<String> {
        self.endpoint
            .lock()
            .expect("endpoint mutex")
            .endpoint
            .clone()
    }

    fn read_error(&self) -> Option<String> {
        self.pending
            .lock()
            .expect("pending mutex")
            .read_error
            .clone()
    }

    fn header_refs(headers: &[(String, String)]) -> Vec<(&str, &str)> {
        headers
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str()))
            .collect()
    }

    fn map_http(&self, err: HttpError) -> TransportError {
        match &err {
            HttpError::Status { rpc: Some(rpc), .. } => TransportError::Rpc(rpc.clone()),
            HttpError::Timeout => TransportError::NoReply,
            other => TransportError::Io(redact_credentials(&other.to_string())),
        }
    }

    /// Sends the wire-level cancellation notification, best effort, unless the
    /// method is the spec-protected initialize request.
    fn send_cancel(&self, id: i64, method: &str) {
        if method == INITIALIZE_METHOD {
            return;
        }
        let request = RpcRequest::notification(
            CANCEL_METHOD,
            Some(json!({"requestId": id, "reason": "client deadline exceeded"})),
        );
        let Ok(body) = serde_json::to_vec(&request) else {
            return;
        };
        let Some(endpoint) = self.current_endpoint() else {
            return;
        };
        let headers = self.static_header_pairs();
        let _ = self.client.post_to(
            &endpoint,
            &body,
            &Self::header_refs(&headers),
            CANCEL_NOTIFY_TIMEOUT,
        );
    }

    fn static_header_pairs(&self) -> Vec<(String, String)> {
        self.static_headers
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect()
    }
}

impl Transport for LegacySseTransport {
    fn call(&self, method: &str, params: Option<&Value>) -> Result<Value, TransportError> {
        self.wait_endpoint()?;
        if self.read_error().is_some() {
            // The stream died before this request was written; a replacement
            // connection can carry it without repeating a side effect.
            return Err(TransportError::Gone);
        }
        let id = self.next_id.fetch_add(1, Ordering::SeqCst) + 1;
        let (tx, rx) = mpsc::channel::<RpcResponse>();
        self.pending
            .lock()
            .expect("pending mutex")
            .pending
            .insert(id, tx);

        let request = RpcRequest::new(id, method, params.cloned());
        let body = serde_json::to_vec(&request)
            .map_err(|err| TransportError::Io(redact_credentials(&err.to_string())))?;
        let Some(endpoint) = self.current_endpoint() else {
            self.pending
                .lock()
                .expect("pending mutex")
                .pending
                .remove(&id);
            return Err(TransportError::Io(ERR_ENDPOINT_MISSING.into()));
        };
        let headers = self.static_header_pairs();
        let reply = self
            .client
            .post_to(
                &endpoint,
                &body,
                &Self::header_refs(&headers),
                self.call_timeout,
            )
            .map_err(|err| {
                self.pending
                    .lock()
                    .expect("pending mutex")
                    .pending
                    .remove(&id);
                self.map_http(err)
            })?;
        // Legacy SSE answers arrive on the GET stream; the POST body is drained
        // and discarded here.
        match reply {
            HttpReply::Json(_) | HttpReply::Events(_) => {}
        }

        let outcome = rx.recv_timeout(self.call_timeout);
        self.pending
            .lock()
            .expect("pending mutex")
            .pending
            .remove(&id);
        match outcome {
            Ok(response) => match response.error {
                Some(rpc) => Err(TransportError::Rpc(rpc)),
                None => Ok(response.result),
            },
            Err(mpsc::RecvTimeoutError::Timeout) => {
                self.send_cancel(id, method);
                Err(TransportError::NoReply)
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => match self.read_error() {
                Some(err) => Err(TransportError::Io(err)),
                None => Err(TransportError::NoReply),
            },
        }
    }

    fn notify(&self, method: &str, params: Option<&Value>) -> Result<(), TransportError> {
        self.wait_endpoint()?;
        if let Some(err) = self.read_error() {
            return Err(TransportError::Io(err));
        }
        let request = RpcRequest::notification(method, params.cloned());
        let body = serde_json::to_vec(&request)
            .map_err(|err| TransportError::Io(redact_credentials(&err.to_string())))?;
        let Some(endpoint) = self.current_endpoint() else {
            return Err(TransportError::Io(ERR_ENDPOINT_MISSING.into()));
        };
        let headers = self.static_header_pairs();
        self.client
            .post_to(
                &endpoint,
                &body,
                &Self::header_refs(&headers),
                self.call_timeout,
            )
            .map_err(|err| self.map_http(err))?;
        Ok(())
    }

    fn era(&self) -> &Era {
        &self.era
    }
}

impl OwnedTransport for LegacySseTransport {
    /// Shuts the connection down once: the reply queue drains, pending calls
    /// are released, and the HTTP worker thread and idle connections close.
    fn close(&self) {
        if self.closed.swap(true, Ordering::SeqCst) {
            return;
        }
        if let Some(tx) = self.reply_tx.lock().expect("reply tx mutex").take() {
            drop(tx);
        }
        fail_pending(&self.pending, Some("mcp connection closed"));
        self.client.shutdown();
        if let Some(handle) = self.replier.lock().expect("replier mutex").take() {
            let _ = handle.join();
        }
        if let Some(handle) = self.reader.lock().expect("reader mutex").take() {
            let _ = handle.join();
        }
    }
}

impl Drop for LegacySseTransport {
    fn drop(&mut self) {
        if !self.closed.load(Ordering::SeqCst) {
            self.close();
        }
    }
}

/// Reader thread: opens the GET stream, routes endpoint frames and inbound
/// messages, and latches the read failure on any stream end or read error.
fn read_loop(
    client: &HttpClient,
    base_url: &str,
    static_headers: &std::collections::BTreeMap<String, String>,
    endpoint: &Arc<Mutex<EndpointInner>>,
    endpoint_ready: &Arc<Condvar>,
    pending: &Arc<Mutex<PendingState>>,
    reply_tx: SyncSender<InboundMessage>,
) {
    let headers: Vec<(String, String)> = static_headers
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    let refs = LegacySseTransport::header_refs(&headers);
    let stream = match client.get_event_stream_at(base_url, &refs, CONNECT_TIMEOUT) {
        Ok(stream) => stream,
        Err(err) => {
            let message = classify_get_error(&err);
            set_endpoint(endpoint, endpoint_ready, None, Some(message.clone()));
            fail_pending(pending, Some(&message));
            return;
        }
    };

    let mut stream = stream;
    while let Some(event) = stream.next_event() {
        match event {
            Ok(event) => handle_event(
                &event.event,
                &event.data,
                base_url,
                endpoint,
                endpoint_ready,
                pending,
                &reply_tx,
            ),
            Err(err) => {
                let message = redact_credentials(&err.to_string());
                fail_pending(pending, Some(&message));
                return;
            }
        }
    }

    // The stream ended. If the endpoint never settled this is the missing-
    // endpoint failure; otherwise the quiet death latches for pending calls.
    let ready = endpoint.lock().expect("endpoint mutex").ready;
    if !ready {
        let message = ERR_ENDPOINT_MISSING.to_string();
        set_endpoint(endpoint, endpoint_ready, None, Some(message.clone()));
        fail_pending(pending, Some(&message));
    } else {
        fail_pending(pending, Some("mcp SSE stream ended"));
    }
}

/// Classifies the initial GET failure into the SSE typed causes; the HTTP
/// status variant keeps its bounded facts and the URL is redacted on the way
/// out.
fn classify_get_error(err: &HttpError) -> String {
    match err {
        HttpError::UnexpectedContentType { .. } => ERR_CONTENT_TYPE.to_string(),
        other => redact_credentials(&other.to_string()),
    }
}

fn handle_event(
    event_name: &str,
    payload: &str,
    base_url: &str,
    endpoint: &Arc<Mutex<EndpointInner>>,
    endpoint_ready: &Arc<Condvar>,
    pending: &Arc<Mutex<PendingState>>,
    reply_tx: &SyncSender<InboundMessage>,
) {
    match event_name {
        "endpoint" => {
            let reference = payload.trim();
            match resolve_against(base_url, reference) {
                Ok(resolved) => {
                    if !same_origin(base_url, &resolved) {
                        set_endpoint(
                            endpoint,
                            endpoint_ready,
                            None,
                            Some(ERR_CROSS_ORIGIN.to_string()),
                        );
                        fail_pending(pending, Some(ERR_CROSS_ORIGIN));
                    } else {
                        set_endpoint(endpoint, endpoint_ready, Some(resolved), None);
                    }
                }
                Err(_) => {
                    let message = "invalid SSE endpoint URL".to_string();
                    set_endpoint(endpoint, endpoint_ready, None, Some(message.clone()));
                    fail_pending(pending, Some(&message));
                }
            }
        }
        "message" | "" => {
            let Some(message) = decode_inbound_message(payload.as_bytes()) else {
                return;
            };
            if !message.method.is_empty() {
                let message_id = message.id.clone().unwrap_or(Value::Null);
                if is_notification_id(&message_id) {
                    // Progress and notification handlers attach in the lifecycle
                    // slice; the frame is skipped here, not routed.
                    return;
                }
                // The reply queue is bounded: a full queue drops the server
                // request so the reader keeps routing client responses.
                let _ = reply_tx.try_send(message);
                return;
            }
            let Some(response) = decode_rpc_response(payload.as_bytes()) else {
                return;
            };
            route_response(pending, response);
        }
        _ => {}
    }
}

fn set_endpoint(
    endpoint: &Arc<Mutex<EndpointInner>>,
    ready: &Arc<Condvar>,
    url: Option<String>,
    err: Option<String>,
) {
    let mut guard = endpoint.lock().expect("endpoint mutex");
    if guard.ready {
        return;
    }
    guard.endpoint = url;
    guard.endpoint_err = err;
    guard.ready = true;
    drop(guard);
    ready.notify_all();
}

fn route_response(pending: &Arc<Mutex<PendingState>>, response: RpcResponse) {
    let Some(id) = response.id.as_i64() else {
        return;
    };
    let sender = pending.lock().expect("pending mutex").pending.remove(&id);
    if let Some(sender) = sender {
        let _ = sender.send(response);
    }
}

/// Latches the read failure and releases every pending call by dropping its
/// sender; a blocked receiver observes the disconnect and reports the latch.
fn fail_pending(pending: &Arc<Mutex<PendingState>>, err: Option<&str>) {
    let mut guard = pending.lock().expect("pending mutex");
    if let Some(err) = err
        && guard.read_error.is_none()
    {
        guard.read_error = Some(err.to_string());
    }
    let drained = std::mem::take(&mut guard.pending);
    drop(guard);
    drop(drained);
}

/// Reply worker: one serialized POST per server-originated request, each under
/// the reply deadline. After a transport error the worker keeps draining the
/// queue so the GET reader never waits on it.
fn reply_loop(
    rx: Receiver<InboundMessage>,
    client: &HttpClient,
    endpoint: &Arc<Mutex<EndpointInner>>,
    pending: &Arc<Mutex<PendingState>>,
    roots: Vec<McpRoot>,
    static_headers: std::collections::BTreeMap<String, String>,
    reply_timeout: Duration,
) {
    let mut dead = false;
    for message in rx {
        if dead {
            continue;
        }
        let reply = server_request_reply(
            message.id.clone().unwrap_or(Value::Null),
            &message.method,
            roots.clone(),
        );
        let body = reply.to_json().into_bytes();
        let Some(endpoint_url) = endpoint.lock().expect("endpoint mutex").endpoint.clone() else {
            continue;
        };
        let headers: Vec<(String, String)> = static_headers
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        let refs: Vec<(&str, &str)> = headers
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str()))
            .collect();
        let result = client.post_to(&endpoint_url, &body, &refs, reply_timeout);
        if let Err(err) = result {
            fail_pending(pending, Some(&redact_credentials(&err.to_string())));
            dead = true;
        }
    }
}

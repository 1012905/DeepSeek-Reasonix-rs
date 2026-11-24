//! MCP Streamable HTTP transport: a blocking facade over one current-thread
//! tokio runtime owned by a dedicated worker thread.
//!
//! Oracle: `internal/ext/plugin/transport_http.go`. The synchronous seam
//! (`seam::Transport`) never crosses an async boundary; every public method
//! hands a job to the worker thread and waits on a std channel, so a caller on
//! any thread stays blocked the way the stdio transport blocks on its pipe.
//!
//! Sensitive-path note: configured `Authorization` / `X-API-Key` values are
//! headers only; they are never written into a diagnostic, and any URL rendered
//! in an error goes through the credential redaction pass before leaving this
//! crate.

use std::collections::BTreeMap;
use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use bytes::Bytes;
use http_body::{Body as HttpBody, Frame};
use hyper::body::Incoming;
use hyper::header::{ACCEPT, CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue, LOCATION};
use hyper::rt::{Read as HyperRead, Write as HyperWrite};
use hyper::{Method, Request, Uri};
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::{Connected, Connection};
use hyper_util::rt::{TokioExecutor, TokioIo};
use rustls::pki_types::ServerName;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;
use tower::Service;

use super::wire::RpcError;
use crate::redact::redact_credentials;

/// Redacts credential-shaped substrings before an io detail leaves the crate.
fn redact_safe(input: &str) -> String {
    redact_credentials(input)
}

/// Upper bound on a JSON / drained SSE response body, so a misbehaving server
/// cannot make the client buffer without bound (Go `maxHTTPBody`).
pub const MAX_HTTP_BODY: usize = 16 * 1024 * 1024;
/// First bytes kept from a non-2xx body for RPC error extraction (Go
/// `httpStatusError.BodyBytes`, 4 KiB).
pub const MAX_ERROR_BODY: usize = 4096;

/// How many same-origin redirects one request may follow.
const MAX_REDIRECTS: usize = 5;
/// Outgoing `Accept` for an MCP POST: either reply shape is legal.
const MCP_ACCEPT: &str = "application/json, text/event-stream";

/// One decoded SSE event: the optional `event:` name and the joined `data:`
/// payloads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SseEvent {
    pub event: String,
    pub data: String,
}

/// A streamed `text/event-stream` response. Frames arrive over a std channel
/// fed by the runtime worker; the facade parses SSE on the calling thread.
pub struct EventStream {
    rx: Receiver<Result<Bytes, HttpError>>,
    buffer: String,
    pending_event: String,
    pending_data: Vec<String>,
    finished: bool,
}

impl EventStream {
    /// Reads the next terminated SSE event. `None` means the stream ended
    /// cleanly with no further event; an `Err` carries the body read failure.
    pub fn next_event(&mut self) -> Option<Result<SseEvent, HttpError>> {
        loop {
            if let Some(event) = self.take_dispatched_event() {
                return Some(Ok(event));
            }
            if self.finished {
                return None;
            }
            match self.rx.recv() {
                Ok(Ok(bytes)) => self.buffer.push_str(&String::from_utf8_lossy(&bytes)),
                Ok(Err(err)) => return Some(Err(err)),
                Err(_) => {
                    self.finished = true;
                    if let Some(event) = self.flush_unterminated_event() {
                        return Some(Ok(event));
                    }
                    return None;
                }
            }
        }
    }

    /// Drops the stream; the worker forwarder stops once its sender is gone.
    pub fn close(self) {}

    fn take_dispatched_event(&mut self) -> Option<SseEvent> {
        loop {
            let newline = self.buffer.find('\n')?;
            let mut line: String = self.buffer.drain(..=newline).collect();
            if line.ends_with('\n') {
                line.pop();
            }
            if line.ends_with('\r') {
                line.pop();
            }
            if line.is_empty() {
                if self.pending_data.is_empty() && self.pending_event.is_empty() {
                    continue;
                }
                let event = SseEvent {
                    event: std::mem::take(&mut self.pending_event),
                    data: self.pending_data.join("\n"),
                };
                self.pending_data.clear();
                return Some(event);
            }
            self.push_line(&line);
        }
    }

    fn flush_unterminated_event(&mut self) -> Option<SseEvent> {
        let remainder = std::mem::take(&mut self.buffer);
        let remainder = remainder.trim_end_matches(['\r', '\n']);
        if !remainder.is_empty() {
            self.push_line(remainder);
        }
        if self.pending_data.is_empty() {
            return None;
        }
        let event = SseEvent {
            event: std::mem::take(&mut self.pending_event),
            data: self.pending_data.join("\n"),
        };
        self.pending_data.clear();
        Some(event)
    }

    fn push_line(&mut self, line: &str) {
        if let Some(rest) = line.strip_prefix("event:") {
            let value = rest.strip_prefix(' ').unwrap_or(rest);
            self.pending_event = value.to_string();
        } else if let Some(rest) = line.strip_prefix("data:") {
            let value = rest.strip_prefix(' ').unwrap_or(rest);
            self.pending_data.push(value.to_string());
        }
        // `id:`, `retry:`, and `:` comment lines carry no MCP payload.
    }
}

/// Static configuration for one MCP HTTP endpoint.
#[derive(Clone, Debug)]
pub struct HttpClientConfig {
    pub url: String,
    pub headers: BTreeMap<String, String>,
    pub call_timeout: Duration,
}

/// A completed non-streaming POST outcome, before the facade classifies it.
enum Posted {
    Json(Vec<u8>),
    Events(EventStream),
}

/// What a worker job hands back to the blocked caller.
enum JobReply {
    Posted(Posted),
    Streamed(EventStream),
    Deleted(u16),
}

enum Job {
    Post {
        body: Vec<u8>,
        headers: Vec<(String, String)>,
        tx: Sender<Result<JobReply, HttpError>>,
    },
    GetStream {
        headers: Vec<(String, String)>,
        tx: Sender<Result<JobReply, HttpError>>,
    },
    Delete {
        headers: Vec<(String, String)>,
        tx: Sender<Result<JobReply, HttpError>>,
    },
}

/// The public reply to a POST: one JSON body, or an open SSE stream.
pub enum HttpReply {
    Json(Vec<u8>),
    Events(EventStream),
}

/// Typed HTTP failures. `Display` reports bounded facts only — status code,
/// byte count, or an id — never text an external server wrote.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HttpError {
    #[error("invalid MCP endpoint URL")]
    InvalidUrl,
    #[error("http {status} (response body omitted; read {body_bytes} bytes)")]
    Status {
        status: u16,
        body_bytes: usize,
        rpc: Option<RpcError>,
    },
    #[error("http {status}: MCP session expired (response body omitted; read {body_bytes} bytes)")]
    SessionExpired { status: u16, body_bytes: usize },
    #[error("refusing cross-origin redirect")]
    CrossOriginRedirect,
    #[error("response body exceeds 16 MiB")]
    BodyTooLarge,
    #[error("unexpected content type {content_type:?}")]
    UnexpectedContentType { content_type: String },
    #[error("SSE stream ended without a response to id {id}")]
    SseEndedWithoutResponse { id: serde_json::Value },
    #[error("request timed out")]
    Timeout,
    #[error("too many redirects")]
    TooManyRedirects,
    #[error("http transport: {0}")]
    Io(String),
}

/// Resolves, connects, and TLS-wraps one connection, mirroring the provider
/// slice's hand-wired connector (AGENTS.md 3.3: rustls + ring, no native TLS).
#[derive(Clone)]
struct Connector {
    tls: Arc<rustls::ClientConfig>,
}

impl Connector {
    fn new() -> Result<Connector, HttpError> {
        let mut roots = rustls::RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let config = rustls::ClientConfig::builder_with_provider(provider)
            .with_protocol_versions(&[&rustls::version::TLS13, &rustls::version::TLS12])
            .map_err(|err| HttpError::Io(redact_safe(&err.to_string())))?
            .with_root_certificates(roots)
            .with_no_client_auth();
        Ok(Connector {
            tls: Arc::new(config),
        })
    }
}

impl Service<Uri> for Connector {
    type Response = ConnIo;
    type Error = io::Error;
    type Future = Pin<Box<dyn Future<Output = Result<ConnIo, io::Error>> + Send>>;

    fn poll_ready(
        &mut self,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        std::task::Poll::Ready(Ok(()))
    }

    fn call(&mut self, dst: Uri) -> Self::Future {
        let tls = self.tls.clone();
        Box::pin(async move {
            let scheme = dst.scheme_str().unwrap_or("https");
            let host = dst
                .host()
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no host"))?
                .to_owned();
            let port = dst
                .port_u16()
                .unwrap_or(if scheme == "https" { 443 } else { 80 });
            let addr = tokio::net::lookup_host((host.as_str(), port))
                .await?
                .next()
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no addresses"))?;
            let tcp = TcpStream::connect(addr).await?;
            tcp.set_nodelay(true).ok();
            let conn = if scheme != "https" {
                Conn::Plain(tcp)
            } else {
                let server_name = ServerName::try_from(host.clone()).map_err(|err| {
                    io::Error::new(io::ErrorKind::InvalidInput, format!("dns name: {err}"))
                })?;
                let connector = tokio_rustls::TlsConnector::from(tls);
                let session = connector.connect(server_name, tcp).await?;
                Conn::Tls(Box::new(session))
            };
            Ok(ConnIo::new(conn))
        })
    }
}

/// One established connection: plain TCP or TLS-wrapped TCP.
enum Conn {
    Plain(TcpStream),
    Tls(Box<tokio_rustls::client::TlsStream<TcpStream>>),
}

impl AsyncRead for Conn {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        match self.get_mut() {
            Conn::Plain(s) => Pin::new(s).poll_read(cx, buf),
            Conn::Tls(s) => Pin::new(s.as_mut()).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for Conn {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<io::Result<usize>> {
        match self.get_mut() {
            Conn::Plain(s) => Pin::new(s).poll_write(cx, buf),
            Conn::Tls(s) => Pin::new(s.as_mut()).poll_write(cx, buf),
        }
    }

    fn poll_flush(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        match self.get_mut() {
            Conn::Plain(s) => Pin::new(s).poll_flush(cx),
            Conn::Tls(s) => Pin::new(s.as_mut()).poll_flush(cx),
        }
    }

    fn poll_shutdown(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        match self.get_mut() {
            Conn::Plain(s) => Pin::new(s).poll_shutdown(cx),
            Conn::Tls(s) => Pin::new(s.as_mut()).poll_shutdown(cx),
        }
    }
}

/// hyper's IO traits over the raw connection, with the ALPN fact kept for h2.
struct ConnIo {
    io: TokioIo<Conn>,
    negotiated_h2: bool,
}

impl ConnIo {
    fn new(conn: Conn) -> Self {
        let negotiated_h2 = match &conn {
            Conn::Plain(_) => false,
            Conn::Tls(tls) => {
                let (_, session) = tls.get_ref();
                session.alpn_protocol() == Some(b"h2")
            }
        };
        Self {
            io: TokioIo::new(conn),
            negotiated_h2,
        }
    }
}

impl HyperRead for ConnIo {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: hyper::rt::ReadBufCursor<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().io).poll_read(cx, buf)
    }
}

impl HyperWrite for ConnIo {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<io::Result<usize>> {
        Pin::new(&mut self.get_mut().io).poll_write(cx, buf)
    }

    fn poll_flush(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().io).poll_flush(cx)
    }

    fn poll_shutdown(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().io).poll_shutdown(cx)
    }

    fn is_write_vectored(&self) -> bool {
        self.io.is_write_vectored()
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> std::task::Poll<io::Result<usize>> {
        Pin::new(&mut self.get_mut().io).poll_write_vectored(cx, bufs)
    }
}

impl Connection for ConnIo {
    fn connected(&self) -> Connected {
        let connected = Connected::new();
        if self.negotiated_h2 {
            connected.negotiated_h2()
        } else {
            connected
        }
    }
}

/// One one-shot request body.
struct OnceBody(Option<Bytes>);

impl HttpBody for OnceBody {
    type Data = Bytes;
    type Error = io::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        std::task::Poll::Ready(self.0.take().map(|data| Ok(Frame::data(data))))
    }

    fn is_end_stream(&self) -> bool {
        self.0.is_none()
    }

    fn size_hint(&self) -> http_body::SizeHint {
        match &self.0 {
            Some(bytes) => http_body::SizeHint::with_exact(bytes.len() as u64),
            None => http_body::SizeHint::with_exact(0),
        }
    }
}

/// Blocking MCP HTTP client. One runtime thread services every request; the
/// public methods are the synchronous facade the seam transport calls into.
pub struct HttpClient {
    jobs: Mutex<tokio::sync::mpsc::Sender<Job>>,
    session: Arc<Mutex<Option<String>>>,
    worker: Mutex<Option<JoinHandle<()>>>,
    base_url: String,
}

impl HttpClient {
    /// Builds the endpoint, validates the URL, and starts the runtime thread.
    pub fn new(cfg: HttpClientConfig) -> Result<Self, HttpError> {
        let uri: Uri = cfg.url.parse().map_err(|_| HttpError::InvalidUrl)?;
        let scheme = uri.scheme_str().unwrap_or_default();
        if scheme != "http" && scheme != "https" {
            return Err(HttpError::InvalidUrl);
        }
        if uri.host().is_none() {
            return Err(HttpError::InvalidUrl);
        }
        let (jobs, job_rx) = tokio::sync::mpsc::channel::<Job>(8);
        let session = Arc::new(Mutex::new(None));
        let base_url = cfg.url.clone();
        let static_headers = cfg.headers.clone();
        let worker = std::thread::spawn({
            let session = session.clone();
            let static_headers = static_headers.clone();
            move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("mcp http runtime");
                runtime.block_on(async move {
                    let connector = match Connector::new() {
                        Ok(connector) => connector,
                        Err(_) => return,
                    };
                    // No idle pool: peers in this slice close one connection
                    // per exchange, and a stale pooled conn would surface as
                    // a SendRequest failure on the next POST.
                    let client = Client::builder(TokioExecutor::new())
                        .pool_max_idle_per_host(0)
                        .build(connector);
                    worker_loop(client, job_rx, session, base_url, static_headers).await;
                });
            }
        });
        Ok(Self {
            jobs: Mutex::new(jobs),
            session,
            worker: Mutex::new(Some(worker)),
            base_url: cfg.url,
        })
    }

    /// The endpoint this client talks to, for diagnostics (callers redact
    /// before logging).
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// The session id captured from the most recent response, if any.
    pub fn session_id(&self) -> Option<String> {
        self.session.lock().expect("session mutex").clone()
    }

    /// Forgets a captured session after a 404 expiry.
    pub fn clear_session(&self) {
        *self.session.lock().expect("session mutex") = None;
    }

    /// POSTs one JSON-RPC body. A 2xx answer is either a JSON body or an open
    /// SSE stream, classified by `Content-Type`.
    pub fn post(
        &self,
        body: &[u8],
        headers: &[(&str, &str)],
        timeout: Duration,
    ) -> Result<HttpReply, HttpError> {
        let (tx, rx) = mpsc::channel();
        self.jobs
            .lock()
            .expect("jobs mutex")
            .try_send(Job::Post {
                body: body.to_vec(),
                headers: normalize_headers(headers),
                tx,
            })
            .map_err(|_| HttpError::Io("mcp http client shut down".into()))?;
        match rx.recv_timeout(timeout) {
            Ok(Ok(JobReply::Posted(Posted::Json(bytes)))) => Ok(HttpReply::Json(bytes)),
            Ok(Ok(JobReply::Posted(Posted::Events(stream)))) => Ok(HttpReply::Events(stream)),
            Ok(Ok(_)) => Err(HttpError::Io("unexpected http reply kind".into())),
            Ok(Err(err)) => Err(err),
            Err(_) => Err(HttpError::Timeout),
        }
    }

    /// Opens a long-lived GET SSE stream (reserved for the legacy SSE wave).
    pub fn get_event_stream(
        &self,
        headers: &[(&str, &str)],
        timeout: Duration,
    ) -> Result<EventStream, HttpError> {
        let (tx, rx) = mpsc::channel();
        self.jobs
            .lock()
            .expect("jobs mutex")
            .try_send(Job::GetStream {
                headers: normalize_headers(headers),
                tx,
            })
            .map_err(|_| HttpError::Io("mcp http client shut down".into()))?;
        match rx.recv_timeout(timeout) {
            Ok(Ok(JobReply::Streamed(stream))) => Ok(stream),
            Ok(Ok(_)) => Err(HttpError::Io("unexpected http reply kind".into())),
            Ok(Err(err)) => Err(err),
            Err(_) => Err(HttpError::Timeout),
        }
    }

    /// Sends a session-ending DELETE; the status is returned best-effort.
    pub fn delete(&self, headers: &[(&str, &str)], timeout: Duration) -> Result<u16, HttpError> {
        let (tx, rx) = mpsc::channel();
        self.jobs
            .lock()
            .expect("jobs mutex")
            .try_send(Job::Delete {
                headers: normalize_headers(headers),
                tx,
            })
            .map_err(|_| HttpError::Io("mcp http client shut down".into()))?;
        match rx.recv_timeout(timeout) {
            Ok(Ok(JobReply::Deleted(status))) => Ok(status),
            Ok(Ok(_)) => Err(HttpError::Io("unexpected http reply kind".into())),
            Ok(Err(err)) => Err(err),
            Err(_) => Err(HttpError::Timeout),
        }
    }

    /// Stops the worker thread and closes idle connections.
    pub fn shutdown(&self) {
        let (empty, _) = tokio::sync::mpsc::channel::<Job>(1);
        *self.jobs.lock().expect("jobs mutex") = empty;
        if let Some(handle) = self.worker.lock().expect("worker mutex").take() {
            let _ = handle.join();
        }
    }
}

impl Drop for HttpClient {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn normalize_headers(headers: &[(&str, &str)]) -> Vec<(String, String)> {
    headers
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect()
}

async fn worker_loop(
    client: Client<Connector, OnceBody>,
    mut jobs: tokio::sync::mpsc::Receiver<Job>,
    session: Arc<Mutex<Option<String>>>,
    base_url: String,
    static_headers: BTreeMap<String, String>,
) {
    while let Some(job) = jobs.recv().await {
        let tx = match &job {
            Job::Post { tx, .. } => tx.clone(),
            Job::GetStream { tx, .. } => tx.clone(),
            Job::Delete { tx, .. } => tx.clone(),
        };
        let result = match job {
            Job::Post { body, headers, .. } => {
                run_post(&client, &session, &base_url, &static_headers, body, headers)
                    .await
                    .map(JobReply::Posted)
            }
            Job::GetStream { headers, .. } => {
                run_get_stream(&client, &session, &base_url, &static_headers, headers)
                    .await
                    .map(JobReply::Streamed)
            }
            Job::Delete { headers, .. } => {
                run_delete(&client, &session, &base_url, &static_headers, headers)
                    .await
                    .map(JobReply::Deleted)
            }
        };
        let _ = tx.send(result);
    }
}

fn base_uri(base_url: &str) -> Result<Uri, HttpError> {
    base_url.parse().map_err(|_| HttpError::InvalidUrl)
}

async fn run_post(
    client: &Client<Connector, OnceBody>,
    session: &Arc<Mutex<Option<String>>>,
    base_url: &str,
    static_headers: &BTreeMap<String, String>,
    body: Vec<u8>,
    headers: Vec<(String, String)>,
) -> Result<Posted, HttpError> {
    let uri = base_uri(base_url)?;
    let resp = send(
        client,
        Method::POST,
        uri,
        static_headers,
        &headers,
        Some(Bytes::from(body)),
        Duration::from_secs(30),
    )
    .await?;
    capture_session(resp.headers(), session);
    let status = resp.status().as_u16();
    if status / 100 != 2 {
        let bytes = drain_body(resp.into_body(), MAX_ERROR_BODY)
            .await
            .unwrap_or_default();
        let rpc = extract_rpc_error(&bytes);
        return Err(HttpError::Status {
            status,
            body_bytes: bytes.len(),
            rpc,
        });
    }
    let content_type = content_type(resp.headers());
    if content_type.starts_with("application/json") {
        let bytes = drain_body(resp.into_body(), MAX_HTTP_BODY).await?;
        return Ok(Posted::Json(bytes));
    }
    if content_type.starts_with("text/event-stream") {
        return Ok(Posted::Events(spawn_forwarder(resp.into_body())));
    }
    Err(HttpError::UnexpectedContentType { content_type })
}

async fn run_get_stream(
    client: &Client<Connector, OnceBody>,
    session: &Arc<Mutex<Option<String>>>,
    base_url: &str,
    static_headers: &BTreeMap<String, String>,
    headers: Vec<(String, String)>,
) -> Result<EventStream, HttpError> {
    let uri = base_uri(base_url)?;
    let resp = send(
        client,
        Method::GET,
        uri,
        static_headers,
        &headers,
        None,
        Duration::from_secs(30),
    )
    .await?;
    capture_session(resp.headers(), session);
    let status = resp.status().as_u16();
    if status / 100 != 2 {
        let bytes = drain_body(resp.into_body(), MAX_ERROR_BODY)
            .await
            .unwrap_or_default();
        return Err(HttpError::Status {
            status,
            body_bytes: bytes.len(),
            rpc: extract_rpc_error(&bytes),
        });
    }
    let content_type = content_type(resp.headers());
    if !content_type.starts_with("text/event-stream") {
        return Err(HttpError::UnexpectedContentType { content_type });
    }
    Ok(spawn_forwarder(resp.into_body()))
}

async fn run_delete(
    client: &Client<Connector, OnceBody>,
    session: &Arc<Mutex<Option<String>>>,
    base_url: &str,
    static_headers: &BTreeMap<String, String>,
    headers: Vec<(String, String)>,
) -> Result<u16, HttpError> {
    let uri = base_uri(base_url)?;
    let resp = send(
        client,
        Method::DELETE,
        uri,
        static_headers,
        &headers,
        None,
        Duration::from_secs(10),
    )
    .await?;
    capture_session(resp.headers(), session);
    let status = resp.status().as_u16();
    let _ = drain_body(resp.into_body(), MAX_ERROR_BODY).await;
    Ok(status)
}

/// Issues one request, following same-origin redirects manually. A redirect to
/// another origin is refused rather than re-sending configured credentials.
async fn send(
    client: &Client<Connector, OnceBody>,
    method: Method,
    uri: Uri,
    static_headers: &BTreeMap<String, String>,
    headers: &[(String, String)],
    body: Option<Bytes>,
    timeout: Duration,
) -> Result<hyper::Response<Incoming>, HttpError> {
    let mut current = uri;
    for _ in 0..MAX_REDIRECTS {
        let request = build_request(
            method.clone(),
            &current,
            static_headers,
            headers,
            body.clone(),
        )?;
        let response = match tokio::time::timeout(timeout, client.request(request)).await {
            Ok(Ok(response)) => response,
            Ok(Err(err)) => return Err(HttpError::Io(redact_safe(&err.to_string()))),
            Err(_) => return Err(HttpError::Timeout),
        };
        let status = response.status();
        if !status.is_redirection() {
            return Ok(response);
        }
        let Some(location) = response
            .headers()
            .get(LOCATION)
            .and_then(|v| v.to_str().ok())
        else {
            return Ok(response);
        };
        let next = resolve_url(&current, location)?;
        if !same_origin(&current.to_string(), &next.to_string()) {
            return Err(HttpError::CrossOriginRedirect);
        }
        current = next;
    }
    Err(HttpError::TooManyRedirects)
}

fn build_request(
    method: Method,
    uri: &Uri,
    static_headers: &BTreeMap<String, String>,
    headers: &[(String, String)],
    body: Option<Bytes>,
) -> Result<Request<OnceBody>, HttpError> {
    let mut builder = Request::builder().method(method.clone()).uri(uri.clone());
    if method == Method::POST {
        builder = builder.header(CONTENT_TYPE, "application/json");
        builder = builder.header(ACCEPT, MCP_ACCEPT);
    } else if method == Method::GET {
        builder = builder.header(ACCEPT, "text/event-stream");
    }
    for (name, value) in static_headers {
        builder = builder.header(name.as_str(), value.as_str());
    }
    for (name, value) in headers {
        let header_name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| HttpError::Io(format!("header {name:?} is not a valid name")))?;
        let header_value = HeaderValue::from_str(value)
            .map_err(|_| HttpError::Io(format!("header {name:?} carries invalid characters")))?;
        builder = builder.header(header_name, header_value);
    }
    builder
        .body(OnceBody(body))
        .map_err(|err| HttpError::Io(redact_safe(&err.to_string())))
}

fn capture_session(headers: &HeaderMap, store: &Arc<Mutex<Option<String>>>) -> Option<String> {
    let sid = headers
        .get("mcp-session-id")
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    if let Some(sid) = &sid {
        *store.lock().expect("session mutex") = Some(sid.clone());
    }
    sid
}

fn content_type(headers: &HeaderMap) -> String {
    headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
}

fn extract_rpc_error(body: &[u8]) -> Option<RpcError> {
    let response = super::wire::decode_rpc_response(body)?;
    response.error
}

async fn drain_body(body: Incoming, limit: usize) -> Result<Vec<u8>, HttpError> {
    let mut out = Vec::new();
    let mut body = body;
    loop {
        match next_body_chunk(&mut body).await {
            Some(Ok(data)) => {
                out.extend_from_slice(&data);
                if out.len() > limit {
                    return Err(HttpError::BodyTooLarge);
                }
            }
            Some(Err(err)) => return Err(err),
            None => break,
        }
    }
    Ok(out)
}

fn spawn_forwarder(body: Incoming) -> EventStream {
    let (tx, rx) = mpsc::sync_channel::<Result<Bytes, HttpError>>(16);
    tokio::spawn(async move {
        let mut body = body;
        while let Some(chunk) = next_body_chunk(&mut body).await {
            if tx.send(chunk).is_err() {
                break;
            }
        }
    });
    EventStream {
        rx,
        buffer: String::new(),
        pending_event: String::new(),
        pending_data: Vec::new(),
        finished: false,
    }
}

async fn next_body_chunk(body: &mut Incoming) -> Option<Result<Bytes, HttpError>> {
    loop {
        let mut pinned = Pin::new(&mut *body);
        let frame = std::future::poll_fn(|cx| pinned.as_mut().poll_frame(cx)).await;
        match frame {
            None => return None,
            Some(Err(err)) => return Some(Err(HttpError::Io(redact_safe(&err.to_string())))),
            Some(Ok(frame)) => {
                if let Ok(data) = frame.into_data() {
                    return Some(Ok(data));
                }
            }
        }
    }
}

/// Same-origin check for the redirect policy: scheme, host, and effective
/// port must all match (Go `sameHTTPOrigin`).
pub fn same_origin(a: &str, b: &str) -> bool {
    let Ok(a) = a.parse::<Uri>() else {
        return false;
    };
    let Ok(b) = b.parse::<Uri>() else {
        return false;
    };
    let (Some(sa), Some(sb)) = (a.scheme_str(), b.scheme_str()) else {
        return false;
    };
    if !sa.eq_ignore_ascii_case(sb) {
        return false;
    }
    let (Some(ha), Some(hb)) = (a.host(), b.host()) else {
        return false;
    };
    if !ha.eq_ignore_ascii_case(hb) {
        return false;
    }
    effective_port(&a) == effective_port(&b)
}

fn effective_port(uri: &Uri) -> u16 {
    uri.port_u16().unwrap_or_else(|| match uri.scheme_str() {
        Some("https") => 443,
        _ => 80,
    })
}

fn resolve_url(base: &Uri, location: &str) -> Result<Uri, HttpError> {
    if let Ok(absolute) = location.parse::<Uri>()
        && absolute.scheme().is_some()
        && absolute.host().is_some()
    {
        return Ok(absolute);
    }
    let scheme = base.scheme_str().unwrap_or("http");
    let host = base.host().ok_or(HttpError::InvalidUrl)?;
    let port = base
        .port_u16()
        .map(|port| format!(":{port}"))
        .unwrap_or_default();
    let path = location.trim_start_matches('/');
    let joined = format!("{scheme}://{host}{port}/{path}");
    joined.parse().map_err(|_| HttpError::InvalidUrl)
}

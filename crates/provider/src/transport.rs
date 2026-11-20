//! Outbound HTTPS transport: hyper 1 client over a hand-wired TCP+rustls
//! connector (Go: `openai/newHTTPClient` via `safety/endpointclient`).
//!
//! TLS is rustls with the ring provider and the Mozilla root bundle
//! (AGENTS.md 3.3). The retry ladder is not ported here — one attempt per
//! call; replay budget stays with the future agent loop.

use std::error::Error as _;
use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use bytes::Bytes;
use http_body::{Body as HttpBody, Frame};
use hyper::body::Incoming;
use hyper::header::{HeaderMap, HeaderValue};
use hyper::rt::{Read as HyperRead, Write as HyperWrite};
use hyper::{Method, Request, Uri};
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::{Connected, Connection};
use hyper_util::rt::{TokioExecutor, TokioIo, TokioTimer};
use rustls::pki_types::ServerName;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;
use tower::Service;

use crate::error::ProviderError;

/// How long draining a non-OK response body may block (Go:
/// `errorBodyReadTimeout`): gateways under load send headers then stall.
pub const ERROR_BODY_READ_TIMEOUT: Duration = Duration::from_secs(10);

/// First bytes of a non-OK body kept for the error message (Go:
/// `readErrorBody`, 4 KiB).
pub const ERROR_BODY_LIMIT: usize = 4096;

/// Upper bound on a non-streaming response body.
pub(crate) const MAX_COMPLETION_BODY: usize = 16 * 1024 * 1024;

/// Key metadata the status ladder needs to type a non-OK response (Go:
/// `SendOptions`).
#[derive(Debug, Clone, Default)]
pub(crate) struct AuthMeta {
    pub key_env: String,
    pub key_source: String,
    pub has_key: bool,
}

/// A single-attempt request body (Go: `bytes.NewReader(body)`).
pub(crate) struct OnceBody(Option<Bytes>);

impl HttpBody for OnceBody {
    type Data = Bytes;
    type Error = std::convert::Infallible;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        Poll::Ready(self.0.take().map(|data| Ok(Frame::data(data))))
    }

    fn is_end_stream(&self) -> bool {
        self.0.is_none()
    }
}

/// One established connection: plain TCP or TLS-wrapped TCP.
pub(crate) enum Conn {
    Plain(TcpStream),
    Tls(Box<tokio_rustls::client::TlsStream<TcpStream>>),
}

impl AsyncRead for Conn {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Conn::Plain(s) => Pin::new(s).poll_read(cx, buf),
            Conn::Tls(s) => Pin::new(s.as_mut()).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for Conn {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            Conn::Plain(s) => Pin::new(s).poll_write(cx, buf),
            Conn::Tls(s) => Pin::new(s.as_mut()).poll_write(cx, buf),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Conn::Plain(s) => Pin::new(s).poll_flush(cx),
            Conn::Tls(s) => Pin::new(s.as_mut()).poll_flush(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Conn::Plain(s) => Pin::new(s).poll_shutdown(cx),
            Conn::Tls(s) => Pin::new(s.as_mut()).poll_shutdown(cx),
        }
    }
}

/// The connector response the legacy client reads: hyper's IO traits over the
/// raw connection, plus the ALPN fact for HTTP/2 negotiation.
pub(crate) struct ConnIo {
    io: TokioIo<Conn>,
    negotiated_h2: bool,
}

impl ConnIo {
    fn new(conn: Conn) -> ConnIo {
        let negotiated_h2 = match &conn {
            Conn::Plain(_) => false,
            Conn::Tls(tls) => {
                let (_, session) = tls.get_ref();
                session.alpn_protocol() == Some(b"h2")
            }
        };
        ConnIo {
            io: TokioIo::new(conn),
            negotiated_h2,
        }
    }
}

impl HyperRead for ConnIo {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: hyper::rt::ReadBufCursor<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().io).poll_read(cx, buf)
    }
}

impl HyperWrite for ConnIo {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.get_mut().io).poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().io).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().io).poll_shutdown(cx)
    }

    fn is_write_vectored(&self) -> bool {
        self.io.is_write_vectored()
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
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

/// Resolves, connects, and (for https) TLS-wraps one connection.
#[derive(Clone)]
pub(crate) struct Connector {
    tls: Arc<rustls::ClientConfig>,
}

impl Connector {
    pub(crate) fn new() -> Result<Connector, ProviderError> {
        let mut roots = rustls::RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let mut config = rustls::ClientConfig::builder_with_provider(provider)
            .with_protocol_versions(&[&rustls::version::TLS13, &rustls::version::TLS12])
            .map_err(|err| ProviderError::Config {
                message: format!("openai: rustls protocol versions: {err}"),
            })?
            .with_root_certificates(roots)
            .with_no_client_auth();
        config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
        Ok(Connector {
            tls: Arc::new(config),
        })
    }
}

impl Service<Uri> for Connector {
    type Response = ConnIo;
    type Error = io::Error;
    type Future = Pin<Box<dyn Future<Output = Result<ConnIo, io::Error>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
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

/// One-attempt POST transport over the hand-wired connector.
#[derive(Clone)]
pub(crate) struct Transport {
    client: Client<Connector, OnceBody>,
}

impl Transport {
    pub(crate) fn new() -> Result<Transport, ProviderError> {
        let connector = Connector::new()?;
        let client = Client::builder(TokioExecutor::new())
            .timer(TokioTimer::new())
            .build(connector);
        Ok(Transport { client })
    }

    /// POSTs one streaming request and returns the OK response (Go:
    /// `SendWithRetry` minus the retry ladder). A non-OK status or a
    /// header-phase timeout becomes a typed error.
    pub(crate) async fn post(
        &self,
        provider: &str,
        url: &str,
        headers: HeaderMap,
        body: Vec<u8>,
        header_timeout: Duration,
        auth: &AuthMeta,
    ) -> Result<Incoming, ProviderError> {
        let uri: Uri = url.parse().map_err(|err| ProviderError::Config {
            message: format!("openai: invalid chat URL {url:?}: {err}"),
        })?;
        let mut builder = Request::builder().method(Method::POST).uri(uri);
        for (name, value) in headers.iter() {
            builder = builder.header(name, value);
        }
        let request = builder
            .body(OnceBody(Some(Bytes::from(body))))
            .map_err(|err| ProviderError::Config {
                message: format!("openai: build request: {err}"),
            })?;
        let response =
            match tokio::time::timeout(header_timeout, self.client.request(request)).await {
                Ok(Ok(response)) => response,
                Ok(Err(err)) => {
                    return Err(request_failed(provider, &err.to_string()));
                }
                Err(_) => {
                    return Err(ProviderError::Timeout {
                        message: format!(
                            "{provider}: request failed: timeout waiting for response headers"
                        ),
                    });
                }
            };
        let status = response.status().as_u16();
        if status != 200 {
            let body = drain_error_body(response.into_body()).await;
            return Err(crate::wire::status_error(
                provider,
                status,
                &body,
                &auth.key_env,
                &auth.key_source,
                auth.has_key,
            ));
        }
        Ok(response.into_body())
    }

    /// POSTs a non-streaming request and returns the whole body.
    pub(crate) async fn post_once(
        &self,
        provider: &str,
        url: &str,
        headers: HeaderMap,
        body: Vec<u8>,
        header_timeout: Duration,
        auth: &AuthMeta,
    ) -> Result<Vec<u8>, ProviderError> {
        let mut incoming = self
            .post(provider, url, headers, body, header_timeout, auth)
            .await?;
        let mut out = Vec::new();
        while let Some(chunk) = next_body_chunk(&mut incoming).await {
            match chunk {
                Ok(bytes) => {
                    out.extend_from_slice(&bytes);
                    if out.len() > MAX_COMPLETION_BODY {
                        return Err(ProviderError::StreamInterrupted {
                            reason: crate::error::STREAM_INTERRUPT_PREMATURE_EOF.to_owned(),
                            message: format!("{provider}: completion body exceeds 16 MiB"),
                        });
                    }
                }
                Err(err) => return Err(err),
            }
        }
        Ok(out)
    }
}

/// Drains a non-OK body under the hard deadline, keeping the head (Go:
/// `readErrorBody`).
async fn drain_error_body(body: Incoming) -> String {
    let drain = async {
        let mut body = body;
        let mut out = Vec::new();
        while let Some(chunk) = next_body_chunk(&mut body).await {
            match chunk {
                Ok(bytes) => out.extend_from_slice(&bytes),
                Err(_) => break,
            }
            if out.len() >= ERROR_BODY_LIMIT {
                break;
            }
        }
        String::from_utf8_lossy(&out).into_owned()
    };
    tokio::time::timeout(ERROR_BODY_READ_TIMEOUT, drain)
        .await
        .unwrap_or_default()
}

/// The next data chunk of a body, already classified (Go: `ReadCut`).
pub(crate) async fn next_body_chunk(body: &mut Incoming) -> Option<Result<Bytes, ProviderError>> {
    loop {
        let mut pinned = Pin::new(&mut *body);
        let frame = std::future::poll_fn(|cx| pinned.as_mut().poll_frame(cx)).await;
        match frame {
            None => return None,
            Some(Err(err)) => return Some(Err(body_error(&err))),
            Some(Ok(frame)) => {
                if let Ok(data) = frame.into_data() {
                    return Some(Ok(data));
                }
            }
        }
    }
}

fn body_error(err: &hyper::Error) -> ProviderError {
    let reset = err
        .source()
        .and_then(|s| s.downcast_ref::<io::Error>())
        .is_some_and(|io| {
            io.kind() == io::ErrorKind::ConnectionReset
                || io.kind() == io::ErrorKind::ConnectionAborted
        });
    crate::error::read_cut("", &err.to_string(), reset)
}

/// Wraps a header-phase transport failure (Go: `SendWithRetry`'s
/// `request failed` wrap).
fn request_failed(provider: &str, detail: &str) -> ProviderError {
    ProviderError::StreamInterrupted {
        reason: crate::error::STREAM_INTERRUPT_CONNECTION_RESET.to_owned(),
        message: format!("{provider}: request failed: {detail}"),
    }
}

/// Builds the header map one chat POST carries (Go: `setChatHeaders` minus
/// the vendor identity extensions, which are a later slice).
pub(crate) fn chat_headers(
    base_url: &str,
    chat_url: &str,
    api_key: &str,
    key_env: &str,
    key_source: &str,
    headers: &std::collections::BTreeMap<String, String>,
    is_stream: bool,
) -> Result<HeaderMap, ProviderError> {
    let mut map = HeaderMap::new();
    if crate::vendor::is_deepseek(chat_url) {
        // DeepSeek's edge acknowledges an eagerly sent body with
        // WINDOW_UPDATE before its own SETTINGS (RFC 9113 §3.4); holding the
        // body for the 100 response lets the preface arrive first.
        map.insert("expect", HeaderValue::from_static("100-continue"));
    }
    map.insert("content-type", HeaderValue::from_static("application/json"));
    let key = api_key.trim();
    if !key.is_empty() {
        if crate::vendor::is_mimo(base_url) {
            map.insert(
                "api-key",
                HeaderValue::from_str(key).map_err(|_| ProviderError::Config {
                    message: format!(
                        "openai: {key_env} from {key_source} carries characters HTTP headers cannot"
                    ),
                })?,
            );
        } else {
            let bearer = format!("Bearer {key}");
            map.insert(
                "authorization",
                HeaderValue::from_str(&bearer).map_err(|_| ProviderError::Config {
                    message: format!(
                        "openai: {key_env} from {key_source} carries characters HTTP headers cannot"
                    ),
                })?,
            );
        }
    }
    let accept = if is_stream {
        "text/event-stream"
    } else {
        "application/json"
    };
    map.insert("accept", HeaderValue::from_static(accept));
    for (name, value) in crate::config::clean_custom_headers(headers) {
        let name = hyper::header::HeaderName::from_bytes(name.as_bytes()).map_err(|_| {
            ProviderError::Config {
                message: format!("openai: custom header {name:?} is not a valid header name"),
            }
        })?;
        let value = HeaderValue::from_str(&value).map_err(|_| ProviderError::Config {
            message: format!("openai: custom header {name:?} carries invalid characters"),
        })?;
        map.insert(name, value);
    }
    Ok(map)
}

//! Connection plumbing for the MCP HTTP client: the rustls/TLS-wrapped
//! connector, the plain-or-TLS connection enum, hyper's IO adapters, and the
//! one-shot request body.
//!
//! Moved verbatim out of `http.rs` to keep that file under the per-module
//! line bound; behavior is unchanged. The client runtime in `http.rs` builds
//! one [`Connector`] and services every request through it.

use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::Arc;

use bytes::Bytes;
use http_body::{Body as HttpBody, Frame};
use hyper::Uri;
use hyper::rt::{Read as HyperRead, Write as HyperWrite};
use hyper_util::client::legacy::connect::{Connected, Connection};
use hyper_util::rt::TokioIo;
use rustls::pki_types::ServerName;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;
use tower::Service;

use super::http::HttpError;
use crate::redact::redact_credentials;

/// Resolves, connects, and TLS-wraps one connection, mirroring the provider
/// slice's hand-wired connector (AGENTS.md 3.3: rustls + ring, no native TLS).
#[derive(Clone)]
pub(crate) struct Connector {
    tls: Arc<rustls::ClientConfig>,
}

impl Connector {
    pub(crate) fn new() -> Result<Connector, HttpError> {
        let mut roots = rustls::RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let config = rustls::ClientConfig::builder_with_provider(provider)
            .with_protocol_versions(&[&rustls::version::TLS13, &rustls::version::TLS12])
            .map_err(|err| HttpError::Io(redact_credentials(&err.to_string())))?
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
                .unwrap_or_else(|| if scheme == "https" { 443 } else { 80 });
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
pub(crate) struct ConnIo {
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
pub(crate) struct OnceBody(Option<Bytes>);

impl OnceBody {
    pub(crate) fn new(body: Option<Bytes>) -> Self {
        Self(body)
    }
}

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

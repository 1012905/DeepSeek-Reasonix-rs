//! Listening and shutdown for the hub's host (Go `listen.go`): serve until
//! the shutdown signal ends, then drain for up to ten seconds. Timeouts ride
//! the hyper layer — `ReadHeaderTimeout` 10s against slowloris, an idle
//! watchdog against keep-alive connections that never come back.

use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpListener;
use tokio::sync::watch;

use axum::Router;

/// Go `runGracefulListener`: the drain budget once the context ends.
const DRAIN_BUDGET: Duration = Duration::from_secs(10);
/// Go `http.Server.ReadHeaderTimeout`.
const HEADER_READ_TIMEOUT: Duration = Duration::from_secs(10);
/// Go `http.Server.IdleTimeout`: how long a keep-alive connection may sit
/// with nothing moving before the server closes it.
const IDLE_TIMEOUT: Duration = Duration::from_secs(120);
/// How often the idle watchdog looks at the connection's last activity.
const IDLE_POLL: Duration = Duration::from_secs(10);

/// Go `RunGracefulHandler`: serves the router on a caller-supplied listener —
/// one that already recorded its real bound address — until `shutdown`
/// completes, then drains active connections for up to ten seconds. A
/// shutdown that ran answers `Ok`; only a listener failure is an error.
pub async fn run_graceful_handler(
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
    listener: TcpListener,
    router: Router,
) -> io::Result<()> {
    let (stop_tx, stop_rx) = watch::channel(false);
    let live = Arc::new(AtomicUsize::new(0));
    let accept = tokio::spawn(accept_loop(listener, router, stop_rx, live.clone()));
    tokio::pin!(accept);
    let listener_failed = tokio::select! {
        _ = shutdown => false,
        outcome = &mut accept => {
            // The accept loop only ends early on an accept error.
            outcome.unwrap_or(Ok(()))?;
            true
        }
    };
    let _ = stop_tx.send(true);
    if !listener_failed {
        // Wait for the accept loop to observe the stop signal.
        let _ = accept.as_mut().await;
    }
    // Drain: in-flight work gets its budget, then whatever is left (an SSE
    // stream that never ends is always left) is closed with the process,
    // matching Go's Shutdown-then-give-up shape.
    let drained = tokio::time::timeout(DRAIN_BUDGET, async {
        while live.load(Ordering::SeqCst) > 0 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    if drained.is_err() {
        tracing::warn!(
            "serve: graceful shutdown exceeded its drain budget; closing remaining connections"
        );
    }
    Ok(())
}

async fn accept_loop(
    listener: TcpListener,
    router: Router,
    mut stopped: watch::Receiver<bool>,
    live: Arc<AtomicUsize>,
) -> io::Result<()> {
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (stream, _addr) = accepted?;
                let router = router.clone();
                let live = live.clone();
                live.fetch_add(1, Ordering::SeqCst);
                tokio::spawn(async move {
                    serve_connection(stream, router).await;
                    live.fetch_sub(1, Ordering::SeqCst);
                });
            }
            _ = stopped.changed() => {
                if *stopped.borrow() {
                    return Ok(());
                }
            }
        }
    }
}

async fn serve_connection(stream: tokio::net::TcpStream, router: Router) {
    let activity = Arc::new(std::sync::Mutex::new(Instant::now()));
    let stream = ActivityStream::new(stream, activity.clone());
    let service = hyper_util::service::TowerToHyperService::new(router);
    let mut builder =
        hyper_util::server::conn::auto::Builder::new(hyper_util::rt::TokioExecutor::new());
    builder
        .http1()
        .timer(hyper_util::rt::TokioTimer::new())
        .header_read_timeout(Some(HEADER_READ_TIMEOUT));
    let connection =
        builder.serve_connection_with_upgrades(hyper_util::rt::TokioIo::new(stream), service);
    tokio::pin!(connection);
    loop {
        tokio::select! {
            result = &mut connection => {
                if let Err(err) = result {
                    tracing::warn!("serve: connection error: {err}");
                }
                break;
            }
            _ = tokio::time::sleep(IDLE_POLL) => {
                let last = *activity
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                if last.elapsed() > IDLE_TIMEOUT {
                    tracing::debug!("serve: idle timeout closing a keep-alive connection");
                    break;
                }
            }
        }
    }
}

/// A stream wrapper that records every completed read and write, so the
/// connection task's watchdog can tell an idle keep-alive from active work.
/// The timestamp is bumped in the poll paths — no lock is held across I/O.
struct ActivityStream {
    inner: tokio::net::TcpStream,
    activity: Arc<std::sync::Mutex<Instant>>,
}

impl ActivityStream {
    fn new(inner: tokio::net::TcpStream, activity: Arc<std::sync::Mutex<Instant>>) -> Self {
        Self { inner, activity }
    }

    fn touch(&self) {
        let mut activity = self.activity.lock().unwrap_or_else(|e| e.into_inner());
        *activity = Instant::now();
    }
}

impl AsyncRead for ActivityStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let before = buf.filled().len();
        let result = AsyncRead::poll_read(Pin::new(&mut this.inner), cx, buf);
        if buf.filled().len() > before {
            this.touch();
        }
        result
    }
}

impl AsyncWrite for ActivityStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        let result = AsyncWrite::poll_write(Pin::new(&mut this.inner), cx, buf);
        if result.is_ready() {
            this.touch();
        }
        result
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        AsyncWrite::poll_flush(Pin::new(&mut self.get_mut().inner), cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        AsyncWrite::poll_shutdown(Pin::new(&mut self.get_mut().inner), cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn shutdown_signal_ends_serving_with_ok() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let router = Router::new();
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        let serve = run_graceful_handler(
            async move {
                let _ = rx.await;
            },
            listener,
            router,
        );
        tx.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(15), serve)
            .await
            .expect("graceful shutdown did not converge")
            .expect("shutdown ends with ok");
    }

    #[tokio::test]
    async fn in_flight_request_completes_within_the_drain() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let router = Router::new().route(
            "/slow",
            axum::routing::get(|| async {
                tokio::time::sleep(Duration::from_millis(300)).await;
                axum::response::IntoResponse::into_response("done")
            }),
        );
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        let serve = tokio::spawn(run_graceful_handler(
            async move {
                let _ = rx.await;
            },
            listener,
            router,
        ));
        tokio::time::sleep(Duration::from_millis(100)).await;
        // A request lands just before the shutdown signal.
        let client = tokio::spawn(async move {
            let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
            tokio::io::AsyncWriteExt::write_all(
                &mut stream,
                b"GET /slow HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
            )
            .await
            .unwrap();
            let mut buf = Vec::new();
            tokio::io::AsyncReadExt::read_to_end(&mut stream, &mut buf)
                .await
                .unwrap();
            String::from_utf8_lossy(&buf).into_owned()
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        tx.send(()).unwrap();
        serve.await.unwrap().unwrap();
        let response = tokio::time::timeout(Duration::from_secs(5), client)
            .await
            .expect("drain window closed before the request answered")
            .unwrap();
        assert!(
            response.contains("200"),
            "in-flight request must complete: {response}"
        );
    }
}

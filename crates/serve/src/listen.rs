//! Listening and shutdown for the hub's host (Go `listen.go`): serve until
//! the shutdown signal ends, then drain. Timeouts ride the hyper layer —
//! `ReadHeaderTimeout` 10s against slowloris, an idle watchdog against
//! keep-alive connections that never come back.

use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Mutex;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpListener;
use tokio::sync::watch;

use axum::Router;
use hyper_util::server::graceful::GracefulShutdown;

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
/// completes, then drains. A shutdown that ran answers `Ok`; only a listener
/// failure is an error.
pub async fn run_graceful_handler(
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
    listener: TcpListener,
    router: Router,
) -> io::Result<()> {
    let (stop_tx, stop_rx) = watch::channel(false);
    let accept = tokio::spawn(accept_loop(listener, router, stop_rx));
    tokio::pin!(accept);
    // The accept loop hands back the shutdown coordinator its connections
    // were watched with; on a listener error the error is the answer.
    let graceful: GracefulShutdown = tokio::select! {
        _ = shutdown => {
            let _ = stop_tx.send(true);
            flatten_join(accept.as_mut().await)?
        }
        outcome = &mut accept => {
            let _ = stop_tx.send(true);
            flatten_join(outcome)?
        }
    };
    // Go `srv.Shutdown`: idle keep-alive connections close immediately; only
    // in-flight requests get the drain budget. A stream that never ends (an
    // open /events) is always left, and is closed with the process.
    if tokio::time::timeout(DRAIN_BUDGET, graceful.shutdown())
        .await
        .is_err()
    {
        tracing::warn!(
            "serve: graceful shutdown exceeded its drain budget; closing remaining connections"
        );
    }
    Ok(())
}

fn flatten_join(
    joined: Result<Result<GracefulShutdown, io::Error>, tokio::task::JoinError>,
) -> io::Result<GracefulShutdown> {
    match joined {
        Ok(inner) => inner,
        // A panicking accept loop is a malfunction, not a shutdown condition.
        Err(join) => Err(io::Error::other(join)),
    }
}

async fn accept_loop(
    listener: TcpListener,
    router: Router,
    mut stopped: watch::Receiver<bool>,
) -> io::Result<GracefulShutdown> {
    let graceful = GracefulShutdown::new();
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (stream, _addr) = accepted?;
                // The watcher subscribes before it is sent to the task, so a
                // shutdown signal that lands before the task starts is still
                // seen (a fresh Receiver only misses sends made before the
                // subscription existed).
                let watcher = graceful.watcher();
                let router = router.clone();
                tokio::spawn(async move {
                    serve_connection(watcher, stream, router).await;
                });
            }
            _ = stopped.changed() => {
                if *stopped.borrow() {
                    return Ok(graceful);
                }
            }
        }
    }
}

async fn serve_connection(
    watcher: hyper_util::server::graceful::Watcher,
    stream: tokio::net::TcpStream,
    router: Router,
) {
    let activity = Arc::new(Mutex::new(Instant::now()));
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
    // Shutdown closes this connection when idle and waits for an in-flight
    // request; the idle watchdog closes it during normal operation.
    tokio::select! {
        result = watcher.watch(connection) => {
            if let Err(err) = result {
                tracing::warn!("serve: connection error: {err}");
            }
        }
        _ = tokio::time::sleep(IDLE_POLL) => {
            let last = *activity.lock().unwrap_or_else(|e| e.into_inner());
            if last.elapsed() > IDLE_TIMEOUT {
                tracing::debug!("serve: idle timeout closing a keep-alive connection");
            }
        }
    }
}

/// A stream wrapper that records every completed read and write, so the
/// connection task's idle watchdog can tell an idle keep-alive from active
/// work. The timestamp is bumped in the poll paths — no lock is held across
/// I/O.
struct ActivityStream {
    inner: tokio::net::TcpStream,
    activity: Arc<Mutex<Instant>>,
}

impl ActivityStream {
    fn new(inner: tokio::net::TcpStream, activity: Arc<Mutex<Instant>>) -> Self {
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
    use crate::broadcaster::Broadcaster;
    use reasonix_session::hub::Hub;
    use reasonix_session::hub::HubPaths;

    fn test_hub() -> Arc<Hub> {
        Arc::new(Hub::new(HubPaths {
            state_root: std::env::temp_dir().join("serve-listen-tests"),
        }))
    }

    // m-5 / OPT-002: a real TCP disconnect must release the SSE
    // subscription — no orphaned subscriber behind a gone socket.
    #[tokio::test]
    async fn a_disconnected_sse_client_releases_its_subscription() {
        let bc = Arc::new(Broadcaster::new());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        let serve = tokio::spawn(run_graceful_handler(
            async move {
                let _ = rx.await;
            },
            listener,
            crate::eventstream::session_routes(test_hub(), bc.clone()),
        ));
        tokio::time::sleep(Duration::from_millis(100)).await;
        let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        tokio::io::AsyncWriteExt::write_all(
            &mut stream,
            b"GET /events HTTP/1.1
Host: 127.0.0.1

",
        )
        .await
        .unwrap();
        // The stream stays open, so read bounded chunks under a deadline.
        let mut buf = vec![0u8; 512];
        let n = tokio::time::timeout(
            Duration::from_secs(5),
            tokio::io::AsyncReadExt::read(&mut stream, &mut buf),
        )
        .await
        .expect("no connect frame within 5s")
        .expect("socket closed before the connect frame");
        assert!(
            String::from_utf8_lossy(&buf[..n]).contains(": connected"),
            "the stream must open with the connect comment"
        );
        assert_eq!(bc.subscriber_count(), 1, "the stream holds one subscriber");
        drop(stream);
        tokio::time::timeout(Duration::from_secs(5), async {
            while bc.subscriber_count() != 0 {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("the subscription outlived its client");
        tx.send(()).unwrap();
        serve.await.unwrap().unwrap();
    }

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

    // m-3: an idle keep-alive connection must not hold the drain open — Go's
    // Shutdown closes it with the signal, so shutdown converges fast even
    // with a browser tab sitting on the port.
    #[tokio::test]
    async fn idle_keep_alive_connection_closes_immediately_on_shutdown() {
        use std::sync::atomic::AtomicUsize;
        use std::sync::atomic::Ordering;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let requests = Arc::new(AtomicUsize::new(0));
        let counter = requests.clone();
        let router = Router::new().layer(axum::middleware::from_fn(
            move |request: axum::extract::Request<axum::body::Body>,
                  next: axum::middleware::Next| {
                let counter = counter.clone();
                async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                    next.run(request).await
                }
            },
        ));
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        let serve = tokio::spawn(run_graceful_handler(
            async move {
                let _ = rx.await;
            },
            listener,
            router,
        ));
        tokio::time::sleep(Duration::from_millis(100)).await;
        // A keep-alive connection that makes one request and then sits idle.
        let idle = tokio::spawn(async move {
            let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
            tokio::io::AsyncWriteExt::write_all(
                &mut stream,
                b"GET /tree HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n",
            )
            .await
            .unwrap();
            let mut buf = vec![0u8; 4096];
            let _ = tokio::io::AsyncReadExt::read(&mut stream, &mut buf).await;
            // Hold the socket open without another request.
            tokio::time::sleep(Duration::from_secs(60)).await;
        });
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(requests.load(Ordering::SeqCst), 1, "the request landed");
        let start = Instant::now();
        tx.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(5), async move {
            serve.await.unwrap().unwrap();
        })
        .await
        .expect("shutdown did not converge inside its budget");
        let elapsed = start.elapsed();
        assert!(
            elapsed < DRAIN_BUDGET,
            "an idle keep-alive must close with the signal, not after the drain: {elapsed:?}"
        );
        idle.abort();
    }
}

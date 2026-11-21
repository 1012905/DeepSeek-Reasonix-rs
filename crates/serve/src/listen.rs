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
                // The stop receiver subscribes before it is sent to the task,
                // so a shutdown signal that lands before the task starts is
                // still seen (a fresh Receiver only misses sends made before
                // the subscription existed).
                let stopped = stopped.clone();
                let router = router.clone();
                let keeper = graceful.watcher();
                tokio::spawn(async move {
                    serve_connection(keeper, stopped, stream, router).await;
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
    keeper: hyper_util::server::graceful::Watcher,
    stopped: watch::Receiver<bool>,
    stream: tokio::net::TcpStream,
    router: Router,
) {
    serve_connection_with(keeper, stopped, stream, router, IDLE_TIMEOUT).await;
}

/// The idle budget of the connection task's watchdog, parametrized so the
/// tests can shrink `IDLE_TIMEOUT` without waiting minutes.
async fn serve_connection_with(
    _keeper: hyper_util::server::graceful::Watcher,
    mut stopped: watch::Receiver<bool>,
    stream: tokio::net::TcpStream,
    router: Router,
    idle_timeout: Duration,
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
    tokio::pin!(connection);
    // The connection is driven raw so both closers can reach it: shutdown
    // rides the `stopped` channel and idle rides the watchdog, each calling
    // `graceful_shutdown` (an idle keep-alive closes at once, an in-flight
    // request finishes) rather than dropping the future mid-request. Drain
    // accounting rides `keeper`: its receiver lives until this task ends,
    // which is what `GracefulShutdown::shutdown` waits on. The watchdog wakes
    // exactly when the idle budget would be spent and re-reads the activity
    // stamp first, so it completes only once nothing has moved for the whole
    // budget. A task accepted in the same select round that stopped the
    // accept loop starts already draining.
    let draining = *stopped.borrow_and_update();
    if draining {
        drain(connection, idle_timeout).await;
        return;
    }
    let watchdog = async {
        loop {
            let last = *activity.lock().unwrap_or_else(|e| e.into_inner());
            if last.elapsed() > idle_timeout {
                tracing::debug!("serve: idle timeout closing a keep-alive connection");
                return;
            }
            let wait = idle_timeout.saturating_sub(last.elapsed());
            tokio::time::sleep(wait).await;
        }
    };
    tokio::select! {
        result = connection.as_mut() => {
            if let Err(err) = result {
                tracing::warn!("serve: connection error: {err}");
            }
        }
        _ = watchdog => {
            drain(connection, idle_timeout).await;
        }
        _ = stopped.changed() => {
            drain(connection, idle_timeout).await;
        }
    }
}

/// Starts the connection's graceful shutdown and waits for it to end —
/// immediately for an idle keep-alive, after the in-flight request for busy
/// one — never past a second idle budget.
async fn drain<C>(mut connection: std::pin::Pin<&mut C>, idle_timeout: Duration)
where
    C: hyper_util::server::graceful::GracefulConnection,
    C::Error: std::fmt::Display,
{
    connection.as_mut().graceful_shutdown();
    if tokio::time::timeout(idle_timeout, connection.as_mut())
        .await
        .is_err()
    {
        tracing::warn!(
            "serve: a connection kept an in-flight request past the second budget; closing"
        );
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
    use axum::body::Body;
    use bytes::Bytes;

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
            crate::eventstream::events_routes(Some(bc.clone())),
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

    // ROUTE-020: a request still in flight must survive the watchdog's poll
    // ticks — only a connection whose idle budget is spent may be ended.
    #[tokio::test]
    async fn a_request_in_flight_survives_the_idle_polls() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let router = Router::new().route(
            "/slow",
            axum::routing::get(|| async {
                tokio::time::sleep(Duration::from_millis(200)).await;
                "done"
            }),
        );
        let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
        tokio::spawn(async move {
            let _stop_tx = stop_tx;
            let (stream, _) = listener.accept().await.unwrap();
            serve_connection_with(
                hyper_util::server::graceful::GracefulShutdown::new().watcher(),
                stop_rx,
                stream,
                router,
                Duration::from_millis(300),
            )
            .await;
        });
        let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        tokio::io::AsyncWriteExt::write_all(
            &mut stream,
            b"GET /slow HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
        )
        .await
        .unwrap();
        let mut buf = Vec::new();
        tokio::time::timeout(
            Duration::from_secs(5),
            tokio::io::AsyncReadExt::read_to_end(&mut stream, &mut buf),
        )
        .await
        .expect("the request never answered")
        .expect("the answering connection stayed readable");
        let response = String::from_utf8_lossy(&buf).into_owned();
        assert!(
            response.contains("200") && response.contains("done"),
            "an in-flight request must complete across idle polls: {response:?}"
        );
    }

    // ROUTE-020: activity counts writes too — a response still trickling out
    // (the shape of an SSE stream) must survive poll after poll.
    #[tokio::test]
    async fn a_slow_response_body_survives_repeated_idle_polls() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let router = Router::new().route(
            "/trickle",
            axum::routing::get(|| async {
                let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Result<Bytes, io::Error>>();
                tokio::spawn(async move {
                    for i in 0..10 {
                        tokio::time::sleep(Duration::from_millis(60)).await;
                        let _ = tx.send(Ok(Bytes::from(format!("chunk{i}\n"))));
                    }
                });
                Body::from_stream(tokio_stream::wrappers::UnboundedReceiverStream::new(rx))
            }),
        );
        let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
        tokio::spawn(async move {
            let _stop_tx = stop_tx;
            let (stream, _) = listener.accept().await.unwrap();
            serve_connection_with(
                hyper_util::server::graceful::GracefulShutdown::new().watcher(),
                stop_rx,
                stream,
                router,
                Duration::from_millis(300),
            )
            .await;
        });
        let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        tokio::io::AsyncWriteExt::write_all(
            &mut stream,
            b"GET /trickle HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
        )
        .await
        .unwrap();
        let mut buf = Vec::new();
        tokio::time::timeout(
            Duration::from_secs(5),
            tokio::io::AsyncReadExt::read_to_end(&mut stream, &mut buf),
        )
        .await
        .expect("the trickle never ended")
        .expect("the trickling connection stayed readable");
        let response = String::from_utf8_lossy(&buf).into_owned();
        for i in 0..10 {
            assert!(
                response.contains(&format!("chunk{i}")),
                "a poll tick must not cut a still-writing stream short: {response:?}"
            );
        }
    }

    // ROUTE-020: the watchdog's own verdict — a keep-alive connection that
    // sits with nothing moving is closed by the server, not left forever.
    #[tokio::test]
    async fn an_idle_keep_alive_connection_is_closed_by_the_watchdog() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let router = Router::new().route("/tree", axum::routing::get(|| async { "leaf" }));
        let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
        tokio::spawn(async move {
            let _stop_tx = stop_tx;
            let (stream, _) = listener.accept().await.unwrap();
            serve_connection_with(
                hyper_util::server::graceful::GracefulShutdown::new().watcher(),
                stop_rx,
                stream,
                router,
                Duration::from_millis(200),
            )
            .await;
        });
        let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        tokio::io::AsyncWriteExt::write_all(
            &mut stream,
            b"GET /tree HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n",
        )
        .await
        .unwrap();
        let mut buf = vec![0u8; 4096];
        let _ = tokio::io::AsyncReadExt::read(&mut stream, &mut buf).await;
        // No further request: the server closes the socket within the budget.
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                match tokio::io::AsyncReadExt::read(&mut stream, &mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
            }
        })
        .await
        .expect("the watchdog never closed the idle connection");
    }
}

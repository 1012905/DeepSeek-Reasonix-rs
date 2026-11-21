//! The event fan-out handle (Go serve `broadcaster.go`, docs/contracts/sse.md
//! §3-§4): one JSON frame per event, numbered when a client cannot afford to
//! miss it, and the subscription modes a client attaches with. The
//! broadcaster entity stays in serve; the hub names its handle through these
//! types, so a runtime's own routes drive the fan-out the open published
//! without serve reaching below itself for the concrete type.

use std::task::{Context, Poll, Waker};

use bytes::Bytes;

/// One JSON frame on its way to a client, carrying the number it was
/// assigned. `seq` is zero for frames that are not resumable — the transport
/// decides from it what to write as an SSE `id` and what to remember as a
/// resume point, without parsing the payload back.
#[derive(Debug, Clone)]
pub struct Frame {
    pub seq: i64,
    pub data: Bytes,
}

/// The five subscription modes (Go `SubscribeFrom`/`SubscribeInitialNotices`/
/// `SubscribeAttached`/`SubscribeResume`; plain `Subscribe` is `From(0)`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubscriptionMode {
    /// Resume after frame `after` — an SSE reconnect carries Last-Event-ID.
    /// Zero means "no cursor": no history is replayed.
    From(i64),
    /// Attach without a cursor; only the first such window per session
    /// receives pre-connection notices, as unnumbered copies.
    InitialNotices,
    /// `InitialNotices` that also states where it attached, as an unnumbered
    /// watermark ahead of every later frame.
    Attached,
    /// A window that stated its own position, zero included: it holds no
    /// numbered frame and is owed the whole log.
    Resume(i64),
}

/// The byte queue one subscription reads from. The broadcaster entity
/// implements it over its own subscriber state; a plain channel is the
/// always-closed fallback a test double hands out.
pub trait FrameQueue: Send {
    /// Polls the next frame — `None` once the subscription was closed
    /// (unsubscribed), which ends the client's stream.
    fn poll_recv(&mut self, cx: &mut Context<'_>) -> Poll<Option<Frame>>;
}

impl FrameQueue for tokio::sync::mpsc::Receiver<Frame> {
    fn poll_recv(&mut self, cx: &mut Context<'_>) -> Poll<Option<Frame>> {
        tokio::sync::mpsc::Receiver::poll_recv(self, cx)
    }
}

/// One client's live view of the fan-out (Go `Subscription`). Dropping it
/// unsubscribes — the Rust counterpart of Go's returned unsubscribe `func()`.
pub struct Subscription {
    queue: Box<dyn FrameQueue>,
    unsubscribe: Option<Box<dyn FnOnce() + Send>>,
}

impl Subscription {
    /// Builds the view over `queue`; `unsubscribe` runs once when the view
    /// drops, removing the subscriber and closing its queue.
    pub fn new(queue: Box<dyn FrameQueue>, unsubscribe: Box<dyn FnOnce() + Send>) -> Self {
        Self {
            queue,
            unsubscribe: Some(unsubscribe),
        }
    }

    /// The next frame, waiting while the queue is empty. `None` once the
    /// subscription was closed (unsubscribed) — the stream ends.
    pub async fn recv(&mut self) -> Option<Frame> {
        std::future::poll_fn(|cx| self.queue.poll_recv(cx)).await
    }

    /// The next frame if one is queued, without waiting. Used by benches and
    /// tests that drive the queue synchronously.
    pub fn try_recv(&mut self) -> Option<Frame> {
        match self
            .queue
            .poll_recv(&mut Context::from_waker(Waker::noop()))
        {
            Poll::Ready(frame) => frame,
            Poll::Pending => None,
        }
    }

    /// Removes the subscriber and closes its queue before the drop
    /// (Go's returned `func()`, which a caller may invoke directly).
    pub fn unsubscribe(&mut self) {
        if let Some(unsubscribe) = self.unsubscribe.take() {
            unsubscribe();
        }
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        if let Some(unsubscribe) = self.unsubscribe.take() {
            unsubscribe();
        }
    }
}

/// Event fan-out handle for one runtime (docs/contract.md §4.1): the
/// subscriber surface a runtime's own routes drive (sse.md §4). The handle
/// the hub publishes is the broadcaster the open machine minted — one per
/// runtime, never shared.
pub trait BroadcasterHandle: Send + Sync {
    /// Registers a client in one of the five modes; what the log still holds
    /// goes out before the live stream, so a resume has no seam.
    fn subscribe(&self, mode: SubscriptionMode) -> Subscription;

    /// The last numbered frame: how a client notices that the frame ending a
    /// turn never arrived — the one case no later frame would reveal on its
    /// own.
    fn watermark(&self) -> i64;

    /// Answers what a client missed after a given frame, and whether the log
    /// could still account for all of it. Serves the transport that has no
    /// connection to re-establish at the same log SSE resumes from.
    fn replay(&self, after: i64) -> (Vec<Bytes>, bool);
}

//! The event broadcaster (Go `broadcaster.go` + the queue half of
//! `eventstream.go`): one marshal, fanned out to every subscriber. A slow one
//! loses frames rather than back-pressuring the agent, so frames worth
//! keeping are numbered and held for a while — a drop becomes something the
//! client can ask for again, through the same numbers on either transport.
//!
//! Concurrency discipline (L0/OPT-001): `emit` never awaits and never blocks;
//! each subscriber has a byte-budgeted queue; locks are `std` mutexes held
//! across no await. The consumer side awaits a `Notify` between short
//! critical sections instead of a pump task.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use bytes::Bytes;

use reasonix_contract::eventwire::event::Event;
use reasonix_session::controller::BroadcasterHandle;
use reasonix_session::fanout::FrameQueue;

/// Bytes, not frames: frames here span three orders of magnitude, so a count
/// bounds nothing anyone cares about. 8MiB holds a whole session of
/// recoverable frames before a resume has to fall back to the transcript
/// (eventstream.go `replayBudget`).
pub const REPLAY_BUDGET: usize = 8 << 20;

/// The live queue budget (eventstream.go `queueBudget`): a subscriber that
/// falls behind this far sheds frames, oldest affordable first.
pub const QUEUE_BUDGET: usize = 4 << 20;

/// The depth of one subscriber's live window (eventstream.go `ch` buffer of
/// 64): a droppable frame behind a whole window of backlog describes text
/// already on screen and costs everything numbered behind it.
pub const SUBSCRIBER_WINDOW: usize = 64;

/// The reserved ToolProgress channel that carries sub-agent phase
/// transitions (event `SubagentProgressStatusName`). Nothing later restates
/// them: a client that misses the terminal one shows the child running
/// forever, so these frames are never droppable.
pub const SUBAGENT_PROGRESS_STATUS: &str = "reasonix.subagent.status";

pub use reasonix_session::fanout::{Frame, Subscription, SubscriptionMode};

/// Whether losing this frame degrades the view rather than breaking it
/// (Go `droppable`): a streaming delta is superseded by the text after it; a
/// sub-agent status channel is not — the phase values ride ToolProgress and
/// nothing else.
pub fn droppable(e: &Event) -> bool {
    match e.kind.as_str() {
        "tool_progress" => {
            e.tool.as_ref().map(|t| t.name.as_str()) != Some(SUBAGENT_PROGRESS_STATUS)
        }
        "text" | "reasoning" | "compaction_progress" => true,
        _ => false,
    }
}

/// One numbered frame kept for clients that have to resume.
struct ReplayFrame {
    seq: i64,
    kind: String,
    data: Bytes,
}

/// The bounded tail of the numbered stream (Go `replayLog`). It is the
/// difference between a dropped frame and a lost one.
#[derive(Default)]
struct ReplayLog {
    frames: VecDeque<ReplayFrame>,
    bytes: usize,
}

impl ReplayLog {
    fn add(&mut self, seq: i64, kind: String, data: Bytes) {
        self.bytes += data.len();
        self.frames.push_back(ReplayFrame { seq, kind, data });
        while self.bytes > REPLAY_BUDGET && self.frames.len() > 1 {
            if let Some(front) = self.frames.pop_front() {
                self.bytes -= front.data.len();
            }
        }
    }

    /// Go `replayLog.since`: the frames numbered after `seq`, and whether the
    /// log could still account for all of them. Incomplete means the client's
    /// next frame was evicted before it came back.
    fn since(&self, seq: i64) -> (Vec<&ReplayFrame>, bool) {
        let Some(first) = self.frames.front() else {
            return (Vec::new(), false);
        };
        if first.seq > seq.saturating_add(1) {
            return (self.frames.iter().collect(), false);
        }
        let from = self
            .frames
            .iter()
            .position(|f| f.seq > seq)
            .unwrap_or(self.frames.len());
        (self.frames.iter().skip(from).collect(), true)
    }

    fn reset(&mut self) {
        self.frames.clear();
        self.bytes = 0;
    }
}

/// While it waits, a frame keeps its shedding decision next to it (Go
/// `queuedFrame`), so eviction never has to re-derive it from the payload.
struct QueuedFrame {
    frame: Frame,
    droppable: bool,
}

struct SubscriberState {
    queue: VecDeque<QueuedFrame>,
    bytes: usize,
    closed: bool,
}

/// One client's outbound queue. `push` is called under the broadcaster's lock
/// and the subscriber's own — both `std` mutexes, never across an await — so
/// a client that stopped reading costs the producer one bounded append.
struct Subscriber {
    state: Mutex<SubscriberState>,
    /// The wake a push or a close hands out, capacity one: a token is pending
    /// while frames wait, and the poll side registers its waker inside the
    /// channel itself, so the wake survives the poll returning — a `Notify`
    /// future dropped between polls would unregister and lose it.
    doorbell: tokio::sync::mpsc::Sender<()>,
}

impl Subscriber {
    /// Go `subscriber.push`. `droppable` is passed rather than derived from
    /// `seq`: a connection-local replay carries no number and still must not
    /// be shed, since it is usually the prompt the client is waiting on.
    fn push(self: &Arc<Self>, frame: Frame, droppable: bool) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.closed {
            return;
        }
        // A queue a whole window deep means this client is out of step: a
        // delta behind that backlog arrives describing text already on
        // screen, and costs everything numbered behind it.
        if droppable && state.queue.len() >= SUBSCRIBER_WINDOW {
            return;
        }
        state.bytes += frame.data.len();
        state.queue.push_back(QueuedFrame { frame, droppable });
        while state.bytes > QUEUE_BUDGET && state.queue.len() > 1 {
            Self::evict(&mut state);
        }
        drop(state);
        let _ = self.doorbell.try_send(());
    }

    /// Go `subscriber.evictLocked`: the oldest frame the client can most
    /// afford to lose, preferring a droppable one wherever it sits and
    /// falling back to the oldest frame of any kind. The newest is never the
    /// candidate: it carries the state that would let the client stop
    /// waiting. Order among the survivors is never disturbed.
    fn evict(state: &mut SubscriberState) {
        let at = state
            .queue
            .iter()
            .take(state.queue.len() - 1)
            .position(|q| q.droppable)
            .unwrap_or(0);
        let removed = state.queue.remove(at);
        if let Some(removed) = removed {
            state.bytes -= removed.frame.data.len();
        }
    }

    fn close(&self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.closed = true;
        // Drain without handing the frames out: the client is gone.
        state.queue.clear();
        state.bytes = 0;
        drop(state);
        let _ = self.doorbell.try_send(());
    }
}

/// The broadcaster state guarded by one short-critical-section mutex. The
/// `Arc` wrapper lets a `Subscription` keep this state alive past its
/// broadcaster handle, so dropping a subscriber can never touch freed memory.
struct Inner {
    subs: HashMap<u64, Arc<Subscriber>>,
    next_sub: u64,
    seq: i64,
    replay: ReplayLog,
    initial_notices_replayed: bool,
    on_tree_changed: Option<Arc<dyn Fn() + Send + Sync>>,
}

/// The event fan-out one runtime's controller emits into. Seq is
/// TRANSPORT-scoped, not session-scoped: numbering outlives /new and /resume,
/// so a resume across a switch is told to rebuild rather than handed another
/// conversation's frames under numbers it already has.
pub struct Broadcaster {
    inner: Arc<Mutex<Inner>>,
}

impl Broadcaster {
    /// An empty broadcaster ready to accept subscribers.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                subs: HashMap::new(),
                next_sub: 0,
                seq: 0,
                replay: ReplayLog::default(),
                initial_notices_replayed: false,
                on_tree_changed: None,
            })),
        }
    }

    /// Numbers the event, records it for replay when it is one a client
    /// cannot afford to miss, and hands it to every subscriber. Never blocks;
    /// a marshal failure is dropped silently — one bad event must not stall
    /// the stream.
    pub fn emit(&self, e: Event) {
        let is_droppable = droppable(&e);
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let mut wire = e;
        if !is_droppable {
            inner.seq += 1;
            wire.seq = inner.seq;
        }
        let data = match serde_json::to_vec(&wire) {
            Ok(data) => Bytes::from(data),
            Err(_) => return,
        };
        if !is_droppable {
            let seq = inner.seq;
            let kind = wire.kind.clone();
            inner.replay.add(seq, kind, data.clone());
        }
        for sub in inner.subs.values() {
            sub.push(
                Frame {
                    seq: wire.seq,
                    data: data.clone(),
                },
                is_droppable,
            );
        }
        // The tree observer runs outside the lock (Go Emit's deferred call):
        // a peer message or a turn boundary re-opens what the session list
        // says, without any frame a person would see.
        let observer = inner.on_tree_changed.clone();
        let kind = wire.kind.as_str();
        let code = wire.code.as_str();
        drop(inner);
        if let Some(observer) = observer
            && (kind == "turn_started"
                || kind == "turn_done"
                || (kind == "notice" && peer_message_code(code)))
        {
            observer();
        }
    }

    /// Delivers an event only to the supplied subscriber: connection-local
    /// recovery, such as replaying a prompt to a browser that attached after
    /// it was asked. Unnumbered by definition — replaying it to a second
    /// client would be replaying a conversation that client never had.
    pub fn emit_to(&self, target: u64, e: Event) {
        let data = match serde_json::to_vec(&e) {
            Ok(data) => Bytes::from(data),
            Err(_) => return,
        };
        let is_droppable = droppable(&e);
        let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(sub) = inner.subs.get(&target) {
            sub.push(
                Frame {
                    seq: 0,
                    data: data.clone(),
                },
                is_droppable,
            );
        }
    }

    /// Registers a client in one of the five modes. What the log still holds
    /// goes out before the live stream and before `emit` can see the
    /// subscriber, so a resume has no seam. A gap the log cannot close is
    /// announced instead: the first sequence the client can trust. Subscribe
    /// and replay are one handoff under one lock hold.
    pub fn subscribe(&self, mode: SubscriptionMode) -> Subscription {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let id = inner.next_sub;
        inner.next_sub += 1;
        let (doorbell, doorbell_rx) = tokio::sync::mpsc::channel(1);
        let subscriber = Arc::new(Subscriber {
            state: Mutex::new(SubscriberState {
                queue: VecDeque::new(),
                bytes: 0,
                closed: false,
            }),
            doorbell,
        });
        let (initial_notices, attached, after) = match mode {
            SubscriptionMode::From(after) => (false, false, after),
            SubscriptionMode::InitialNotices => (true, false, 0),
            SubscriptionMode::Attached => (true, true, 0),
            SubscriptionMode::Resume(after) => (false, true, after),
        };
        if initial_notices && !inner.initial_notices_replayed {
            inner.initial_notices_replayed = true;
            for f in &inner.replay.frames {
                if f.kind != "notice" {
                    continue;
                }
                // Unnumbered copies: skipped non-notice history must not look
                // like a gap that prompts the client to replay it.
                let Ok(mut notice) = serde_json::from_slice::<Event>(&f.data) else {
                    continue;
                };
                notice.seq = 0;
                if let Ok(data) = serde_json::to_vec(&notice) {
                    subscriber.push(
                        Frame {
                            seq: 0,
                            data: Bytes::from(data),
                        },
                        false,
                    );
                }
            }
        } else if !initial_notices && (after > 0 || attached) && after < inner.seq {
            let (missed, complete) = inner.replay.since(after);
            if !complete {
                let from = missed.first().map(|f| f.seq).unwrap_or(inner.seq);
                if let Ok(data) = serde_json::to_vec(&gap_frame_json(from)) {
                    subscriber.push(
                        Frame {
                            seq: from,
                            data: Bytes::from(data),
                        },
                        false,
                    );
                }
            }
            for f in &missed {
                subscriber.push(
                    Frame {
                        seq: f.seq,
                        data: f.data.clone(),
                    },
                    false,
                );
            }
        }
        if attached {
            subscriber.push(
                Frame {
                    seq: 0,
                    data: Bytes::from(watermark_json(inner.seq)),
                },
                false,
            );
        }
        inner.subs.insert(id, subscriber.clone());
        drop(inner);
        // The queue owns the doorbell receiver; the unsubscribe closure holds
        // sibling handles over the same Arc'd subscriber state.
        let queue = LiveQueue {
            subscriber: subscriber.clone(),
            doorbell: doorbell_rx,
        };
        let unbind = {
            let inner = self.inner.clone();
            move || {
                let mut guard = inner.lock().unwrap_or_else(|e| e.into_inner());
                if guard.subs.remove(&id).is_some() {
                    drop(guard);
                    subscriber.close();
                }
            }
        };
        Subscription::new(Box::new(queue), Box::new(unbind))
    }

    /// The last numbered frame. A client compares it against what it has seen
    /// to notice that the frame ending a turn never arrived — the one case no
    /// later frame would reveal on its own.
    pub fn watermark(&self) -> i64 {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).seq
    }

    /// Answers what a client missed after a given frame, and whether the log
    /// could still account for all of it. Serves the transport that has no
    /// connection to re-establish: the shell's bus asks in a request instead
    /// of carrying Last-Event-ID into a new stream. Both arrive at the same
    /// log.
    pub fn replay(&self, after: i64) -> (Vec<Bytes>, bool) {
        let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if after >= inner.seq {
            return (Vec::new(), true);
        }
        let (missed, complete) = inner.replay.since(after);
        (missed.iter().map(|f| f.data.clone()).collect(), complete)
    }

    /// Go `ResetSession`: drops the replay tail and re-arms the
    /// initial-notices window for /new, /resume and /fork — those frames
    /// describe a conversation no client is looking at any more. The
    /// sequence keeps counting, so a client resuming across the switch lands
    /// before the log's first frame and is told to refetch.
    pub fn reset_session(&self) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.replay.reset();
        inner.initial_notices_replayed = false;
    }

    /// Go `observeTree`: the callback a hub pins so tree-relevant frames
    /// refresh the session list.
    pub fn set_on_tree_changed(&self, observer: Option<Arc<dyn Fn() + Send + Sync>>) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.on_tree_changed = observer;
    }

    /// Go `Subscribers`: the current connection count, for diagnostics.
    pub fn subscriber_count(&self) -> usize {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .subs
            .len()
    }
}

impl Default for Broadcaster {
    fn default() -> Self {
        Self::new()
    }
}

/// The fan-out handle the session hub names (docs/contract.md §4.1): the
/// entity lives in serve; the subscriber surface it carries is session's
/// [reasonix_session::fanout::BroadcasterHandle], so a runtime's routes
/// drive the broadcaster the open machine minted through the trait.
impl BroadcasterHandle for Broadcaster {
    fn subscribe(&self, mode: SubscriptionMode) -> Subscription {
        Broadcaster::subscribe(self, mode)
    }

    fn watermark(&self) -> i64 {
        Broadcaster::watermark(self)
    }

    fn replay(&self, after: i64) -> (Vec<Bytes>, bool) {
        Broadcaster::replay(self, after)
    }
}

/// The terminal sink of the emit pipeline (docs/contracts/sse.md §5): boot
/// wraps the broadcaster in mark-embedded-diffs → coalesce → sync → quote
/// before emitters ever see it, and `emit` is never blocking.
impl crate::sink::Sink for Broadcaster {
    fn emit(&self, e: Event) {
        Broadcaster::emit(self, e);
    }
}

/// Go `peer message` notice codes that re-open the session list
/// (broadcaster.go's observer switch).
fn peer_message_code(code: &str) -> bool {
    matches!(
        code,
        "session_recovery_forked"
            | "session_recovery_adopted"
            | "session_recovery_adopted_covered"
            | "session_shutdown_recovery_forked"
            | "session_continued_from_1x"
            | "peer_message_arrived"
    )
}

/// Go `gapFrame`: where the recoverable stream starts. `from` is the first
/// frame the client is about to receive (or the current watermark when the
/// log has nothing left), so everything before that has to come from the
/// transcript.
fn gap_frame_json(from: i64) -> Event {
    Event {
        kind: "stream_gap".to_owned(),
        seq: from,
        ..Event::default()
    }
}

/// Go `streamFrames`' keepalive frame: built by hand like Go's `fmt.Appendf`,
/// so the watermark number is present even when it is zero.
pub(crate) fn watermark_json(seq: i64) -> String {
    format!(r#"{{"kind":"stream_watermark","seq":{seq}}}"#)
}

/// The broadcaster-side queue of one live subscriber (the state Go's
/// `Subscription` holds): it answers session's [`FrameQueue`] over the same
/// VecDeque-plus-wake pair, and the view runs the unsubscribe on drop.
struct LiveQueue {
    subscriber: Arc<Subscriber>,
    doorbell: tokio::sync::mpsc::Receiver<()>,
}

impl FrameQueue for LiveQueue {
    fn poll_recv(&mut self, cx: &mut Context<'_>) -> Poll<Option<Frame>> {
        loop {
            let mut state = self
                .subscriber
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if let Some(queued) = state.queue.pop_front() {
                state.bytes -= queued.frame.data.len();
                return Poll::Ready(Some(queued.frame));
            }
            if state.closed {
                return Poll::Ready(None);
            }
            drop(state);
            // The waker registers inside the doorbell channel: a push that
            // lands after this poll still wakes the next one, and a token
            // parked before it is consumed on the way around.
            match self.doorbell.poll_recv(cx) {
                Poll::Ready(Some(())) => continue,
                // The doorbell closed only with the subscriber — the queue's
                // closed flag above is the answer either way.
                Poll::Ready(None) => continue,
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reasonix_contract::eventwire::tool::Tool;
    use std::time::Duration;

    fn text_event(text: &str) -> Event {
        Event {
            kind: "text".to_owned(),
            text: text.to_owned(),
            ..Event::default()
        }
    }

    fn tool_result(id: &str) -> Event {
        Event {
            kind: "tool_result".to_owned(),
            tool: Some(Tool {
                id: id.to_owned(),
                name: "bash".to_owned(),
                ..Tool::default()
            }),
            ..Event::default()
        }
    }

    /// Go broadcaster_test.go `read`: drains until `want` is seen or the
    /// deadline passes, the way a real SSE handler reads.
    async fn read_until(ch: &mut Subscription, want: &str) -> usize {
        let deadline = tokio::time::Duration::from_secs(2);
        let mut seen = 0;
        loop {
            match tokio::time::timeout(deadline, ch.recv()).await {
                Ok(Some(f)) => {
                    seen += 1;
                    let s = String::from_utf8_lossy(&f.data).into_owned();
                    if s.contains(want) {
                        return seen;
                    }
                }
                Ok(None) => panic!("stream closed before {want:?} arrived (after {seen} frames)"),
                Err(_) => panic!("timed out waiting for {want:?} (saw {seen} frames)"),
            }
        }
    }

    // Ported: broadcaster_test.go TestBroadcasterFanOut.
    #[tokio::test]
    async fn fan_out_reaches_every_subscriber() {
        let b = Arc::new(Broadcaster::new());
        let mut a = b.subscribe(SubscriptionMode::From(0));
        let mut d = b.subscribe(SubscriptionMode::From(0));
        assert_eq!(b.subscriber_count(), 2);
        b.emit(text_event("hi"));
        for ch in [&mut a, &mut d] {
            let f = ch.recv().await.expect("frame");
            let w: serde_json::Value = serde_json::from_slice(&f.data).unwrap();
            assert_eq!(w["kind"], "text");
            assert_eq!(w["text"], "hi");
        }
    }

    // Ported: broadcaster_test.go TestBroadcasterEmitsRetryingJSON.
    #[tokio::test]
    async fn retrying_event_travels_as_its_wire_json() {
        let b = Broadcaster::new();
        let mut ch = b.subscribe(SubscriptionMode::From(0));
        b.emit(Event {
            kind: "retrying".to_owned(),
            retry_attempt: 3,
            retry_max: 10,
            ..Event::default()
        });
        let f = ch.recv().await.unwrap();
        let s = String::from_utf8_lossy(&f.data).into_owned();
        for want in [
            r#""kind":"retrying""#,
            r#""retryAttempt":3"#,
            r#""retryMax":10"#,
        ] {
            assert!(s.contains(want), "retrying JSON {s} must contain {want}");
        }
    }

    // Ported: broadcaster_test.go TestBroadcasterUnsubscribe.
    #[tokio::test]
    async fn unsubscribe_drops_the_subscriber_and_emit_survives() {
        let b = Broadcaster::new();
        let mut ch = b.subscribe(SubscriptionMode::From(0));
        assert_eq!(b.subscriber_count(), 1);
        ch.unsubscribe();
        assert_eq!(b.subscriber_count(), 0);
        b.emit(Event {
            kind: "turn_done".to_owned(),
            ..Event::default()
        });
    }

    // Ported: broadcaster_test.go TestBroadcasterShedsDroppableFrames.
    #[tokio::test]
    async fn sheds_droppable_frames_rather_than_queueing_them_forever() {
        let b = Broadcaster::new();
        let mut ch = b.subscribe(SubscriptionMode::From(0));
        for _ in 0..10_000 {
            b.emit(text_event("x"));
        }
        b.emit(Event {
            kind: "turn_done".to_owned(),
            ..Event::default()
        });
        let n = read_until(&mut ch, r#""kind":"turn_done""#).await;
        assert!(n <= 1000, "client read {n} frames before the turn boundary");
    }

    // Ported: broadcaster_test.go TestBroadcasterKeepsPromptsBehindABacklog.
    #[tokio::test]
    async fn prompts_survive_behind_a_backlog() {
        use reasonix_contract::eventwire::approval::Approval;
        let b = Broadcaster::new();
        let mut ch = b.subscribe(SubscriptionMode::From(0));
        for _ in 0..200 {
            b.emit(text_event("x"));
        }
        b.emit(Event {
            kind: "approval_request".to_owned(),
            approval: Some(Approval {
                id: "a1".to_owned(),
                tool: "bash".to_owned(),
                ..Approval::default()
            }),
            ..Event::default()
        });
        b.emit(Event {
            kind: "turn_done".to_owned(),
            ..Event::default()
        });
        read_until(&mut ch, r#""kind":"approval_request""#).await;
        read_until(&mut ch, r#""kind":"turn_done""#).await;
    }

    // Ported: broadcaster_test.go TestBroadcasterKeepsOrderAcrossADrop.
    #[tokio::test]
    async fn order_survives_a_drop() {
        let b = Broadcaster::new();
        let mut ch = b.subscribe(SubscriptionMode::From(0));
        for _ in 0..500 {
            b.emit(text_event("x"));
        }
        for id in ["t1", "t2", "t3"] {
            b.emit(tool_result(id));
        }
        read_until(&mut ch, r#""id":"t1""#).await;
        read_until(&mut ch, r#""id":"t2""#).await;
        read_until(&mut ch, r#""id":"t3""#).await;
    }

    // Ported: broadcaster_test.go
    // TestBroadcasterEmitDoesNotBlockOnAStalledClient.
    #[tokio::test]
    async fn emit_does_not_block_on_a_stalled_client() {
        let b = Broadcaster::new();
        let _ch = b.subscribe(SubscriptionMode::From(0)); // never read
        let done = tokio::spawn(async move {
            for _ in 0..10_000 {
                b.emit(text_event("x"));
                b.emit(tool_result("t"));
            }
        });
        tokio::time::timeout(Duration::from_secs(10), done)
            .await
            .expect("emit blocked on a subscriber that never read")
            .expect("emit task panicked");
    }

    // Ported: broadcaster_test.go
    // TestBroadcasterKeepsSubagentStatusBehindABacklog.
    #[tokio::test]
    async fn subagent_status_survives_behind_a_backlog() {
        let b = Broadcaster::new();
        let mut ch = b.subscribe(SubscriptionMode::From(0));
        for _ in 0..500 {
            b.emit(Event {
                kind: "tool_progress".to_owned(),
                tool: Some(Tool {
                    id: "c1".to_owned(),
                    name: "reasonix.subagent.reasoning".to_owned(),
                    output: "x".to_owned(),
                    ..Tool::default()
                }),
                ..Event::default()
            });
        }
        b.emit(Event {
            kind: "tool_progress".to_owned(),
            tool: Some(Tool {
                id: "c1".to_owned(),
                name: SUBAGENT_PROGRESS_STATUS.to_owned(),
                output: "failed".to_owned(),
                ..Tool::default()
            }),
            ..Event::default()
        });
        read_until(&mut ch, r#""output":"failed""#).await;
    }

    // Ported: broadcaster_test.go
    // TestBroadcasterRefreshesTheListWhenAPeerMessageArrives.
    #[tokio::test]
    async fn peer_message_refreshes_the_list_once() {
        let b = Broadcaster::new();
        let refreshed = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = refreshed.clone();
        b.set_on_tree_changed(Some(Arc::new(move || {
            counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        })));
        b.emit(Event {
            kind: "notice".to_owned(),
            level: "info".to_owned(),
            audience: "operator".to_owned(),
            code: "peer_message_arrived".to_owned(),
            ..Event::default()
        });
        assert_eq!(refreshed.load(std::sync::atomic::Ordering::SeqCst), 1);
        b.emit(Event {
            kind: "notice".to_owned(),
            level: "info".to_owned(),
            code: "some_other_notice".to_owned(),
            ..Event::default()
        });
        assert_eq!(refreshed.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[test]
    fn droppable_set_matches_the_contract() {
        assert!(droppable(&text_event("x")));
        assert!(droppable(&Event {
            kind: "reasoning".to_owned(),
            text: "t".to_owned(),
            ..Event::default()
        }));
        assert!(droppable(&Event {
            kind: "compaction_progress".to_owned(),
            text: "t".to_owned(),
            ..Event::default()
        }));
        // Plain tool progress is droppable; the sub-agent status channel is not.
        assert!(droppable(&Event {
            kind: "tool_progress".to_owned(),
            tool: Some(Tool {
                id: "t".to_owned(),
                name: "bash".to_owned(),
                ..Tool::default()
            }),
            ..Event::default()
        }));
        assert!(!droppable(&Event {
            kind: "tool_progress".to_owned(),
            tool: Some(Tool {
                id: "c1".to_owned(),
                name: SUBAGENT_PROGRESS_STATUS.to_owned(),
                output: "running".to_owned(),
                ..Tool::default()
            }),
            ..Event::default()
        }));
        assert!(!droppable(&tool_result("t1")));
        assert!(!droppable(&Event::default()));
    }

    // Ported: assembly order Coalesce → Sync → [Broadcaster] (Go build.go:239
    // with the broadcaster as the frontend sink): a delta burst merges before
    // the boundary event, and the coalescer's window uses the tokio clock.
    #[tokio::test(start_paused = true)]
    async fn the_emit_pipeline_merges_bursts_before_the_broadcaster() {
        use crate::sink::{DEFAULT_STREAM_DELTA_WINDOW, coalesce, sync_sink};
        let b = Arc::new(Broadcaster::new());
        let mut ch = b.subscribe(SubscriptionMode::From(0));
        let pipeline = coalesce(
            sync_sink(Arc::clone(&b) as Arc<dyn crate::sink::Sink>),
            DEFAULT_STREAM_DELTA_WINDOW,
        );
        pipeline.emit(text_event("a"));
        pipeline.emit(text_event("b"));
        pipeline.emit(text_event("c"));
        pipeline.emit(Event {
            kind: "turn_done".to_owned(),
            ..Event::default()
        });
        let mut texts = String::new();
        for _ in 0..2 {
            let f = ch.recv().await.expect("frame");
            let w: serde_json::Value = serde_json::from_slice(&f.data).unwrap();
            if w["kind"] == "text" {
                texts.push_str(w["text"].as_str().unwrap());
            }
        }
        assert_eq!(texts, "abc", "burst must merge ahead of the boundary");
    }
}

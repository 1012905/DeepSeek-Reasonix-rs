//! Merges bursts of pure streaming deltas (docs/contracts/sse.md §5; Go
//! `coalesce.go`). The first delta of a burst forwards immediately
//! (time-to-first-token unchanged); later deltas buffer at most
//! [`DEFAULT_STREAM_DELTA_WINDOW`], flushing earlier on any other event (total
//! order preserved), a key switch, or the merged-byte cap.
//!
//! The window rides tokio's clock so tests pause and auto-advance it; outside
//! a runtime the clock degrades to wall time and the timer flush to a barrier
//! flush — buffering still never loses a delta.

use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::Duration;

use tokio::sync::watch;
use tokio::time::Instant;

use reasonix_contract::eventwire::event::Event;
use reasonix_contract::eventwire::tool::Tool;

use super::Sink;

/// Caps a merged delta so one event never carries an unbounded payload across
/// a frontend bridge (Go `coalesceMaxBytes`).
const COALESCE_MAX_BYTES: usize = 16 << 10;

/// Caps how often coalesced streaming deltas cross into a frontend: at most
/// one merged event per window under load — about one per display frame — so
/// a fast provider cannot flood a webview bridge, an SSE stream, or a
/// terminal redraw loop (Go `DefaultStreamDeltaWindow`).
pub const DEFAULT_STREAM_DELTA_WINDOW: Duration = Duration::from_millis(16);

/// Wraps `inner` so bursts of consecutive streaming deltas — text, reasoning,
/// compaction progress carrying nothing but a text payload, and one tool's
/// progress — merge into one event. A window of zero returns `inner`
/// unchanged.
pub fn coalesce(inner: Arc<dyn Sink>, window: Duration) -> Arc<dyn Sink> {
    if window.is_zero() {
        return inner;
    }
    let this = Arc::new(Coalescer::new(inner, window));
    let _ = this.self_weak.set(Arc::downgrade(&this));
    this
}

struct Coalescer {
    inner: Arc<dyn Sink>,
    window: Duration,
    // Guards buffering state and the outbound queue; inner.emit is never
    // called under it. A single drainer forwards FIFO, so a sink that
    // synchronously re-enters emit enqueues and returns instead of deadlocking.
    state: Mutex<CoalesceState>,
    timer_tx: watch::Sender<Option<Instant>>,
    // Set once in `coalesce`, so the lazy flush task can hold a weak handle
    // without a reference cycle.
    self_weak: OnceLock<Weak<Coalescer>>,
    timer_task: OnceLock<()>,
}

#[derive(Default)]
struct CoalesceState {
    key: DeltaKey,
    buf: String,
    pending: bool,
    /// The armed flush deadline; `None` means the buffer flushes on the next
    /// barrier only.
    deadline: Option<Instant>,
    /// The last immediate forward or flush; `None` means the first delta is
    /// the leading edge.
    last_forward: Option<Instant>,
    queue: Vec<Event>,
    draining: bool,
}

/// The identity a buffered burst merges under. Source belongs here rather
/// than to the payload: it names who emitted the delta, and two sources must
/// not merge. A key change flushes, so text, reasoning and one tool's progress
/// never merge into another's (Go `deltaKey`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct DeltaKey {
    kind: String,
    source: String,
    tool_id: String,
}

impl DeltaKey {
    /// Rebuilds the merged event from the key and the accumulated payload.
    fn event(&self, payload: String) -> Event {
        let mut e = Event {
            kind: self.kind.clone(),
            source: self.source.clone(),
            ..Event::default()
        };
        if self.kind == "tool_progress" {
            e.tool = Some(Tool {
                id: self.tool_id.clone(),
                output: payload,
                ..Tool::default()
            });
        } else {
            e.text = payload;
        }
        e
    }
}

/// Reports whether e is a pure streaming delta, with its merge key and
/// payload (Go `streamDelta`). Progress carrying more than id+output — a
/// sub-agent's status channel sets name, parentId and a rate — fails the
/// zero-probe, as does any event carrying an extra field.
fn stream_delta(e: &Event) -> Option<(DeltaKey, String)> {
    let mut key = DeltaKey {
        kind: e.kind.clone(),
        source: e.source.clone(),
        tool_id: String::new(),
    };
    let mut probe = e.clone();
    probe.source = String::new();
    let payload;
    let mut probe_tool = false;
    match e.kind.as_str() {
        "text" | "reasoning" | "compaction_progress" => {
            payload = e.text.clone();
            probe.text = String::new();
        }
        "tool_progress" => {
            let tool = e.tool.as_ref()?;
            key.tool_id = tool.id.clone();
            payload = tool.output.clone();
            if let Some(t) = probe.tool.as_mut() {
                t.id = String::new();
                t.output = String::new();
            }
            probe_tool = true;
        }
        _ => return None,
    }
    if payload.is_empty() || (e.kind == "tool_progress" && key.tool_id.is_empty()) {
        return None;
    }
    // The zero probe (Go `Event{Kind: e.Kind}`). The wire models a tool frame
    // as Option<Tool>, so a tool_progress probe compares against an empty
    // Some — a cleared tool. A text-kind event carrying an empty tool frame
    // fails the probe and passes through unmerged, where Go would merge it;
    // no emitter produces that shape.
    let zero = Event {
        kind: e.kind.clone(),
        tool: if probe_tool {
            Some(Tool::default())
        } else {
            None
        },
        ..Event::default()
    };
    if probe != zero {
        return None;
    }
    Some((key, payload))
}

impl Coalescer {
    fn new(inner: Arc<dyn Sink>, window: Duration) -> Self {
        let (timer_tx, _rx) = watch::channel(None);
        Self {
            inner,
            window,
            state: Mutex::new(CoalesceState::default()),
            timer_tx,
            self_weak: OnceLock::new(),
            timer_task: OnceLock::new(),
        }
    }

    fn emit(&self, e: Event) {
        let (key, payload, delta) = match stream_delta(&e) {
            Some((key, payload)) => (key, payload, true),
            None => (DeltaKey::default(), String::new(), false),
        };
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if !delta {
            self.enqueue_flush_locked(&mut state);
            state.queue.push(e);
            drop(state);
            self.drain_and_unlock();
            return;
        }
        if state.pending && state.key != key {
            self.enqueue_flush_locked(&mut state);
        }
        let leading = state
            .last_forward
            .is_none_or(|t| t.elapsed() >= self.window);
        if !state.pending && leading {
            state.last_forward = Some(Instant::now());
            state.queue.push(e);
            drop(state);
            self.drain_and_unlock();
            return;
        }
        if !state.pending {
            state.pending = true;
            state.key = key;
            self.arm_timer(&mut state);
        }
        state.buf.push_str(&payload);
        if state.buf.len() >= COALESCE_MAX_BYTES {
            self.enqueue_flush_locked(&mut state);
        }
        drop(state);
        self.drain_and_unlock();
    }

    // Arms the background flush one window out. Called only when the buffer
    // flips empty → pending; later deltas extend it without moving the
    // deadline, matching Go's timer reuse. Without a runtime the deadline is
    // never armed: the buffer flushes on the next barrier instead.
    fn arm_timer(&self, state: &mut CoalesceState) {
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let deadline = Instant::now() + self.window;
        state.deadline = Some(deadline);
        self.timer_tx.send_replace(Some(deadline));
        if let (Some(weak), true) = (self.self_weak.get(), self.timer_task.set(()).is_ok()) {
            handle.spawn(timer_loop(weak.clone(), self.timer_tx.subscribe()));
        }
    }

    fn flush(&self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        self.enqueue_flush_locked(&mut state);
        drop(state);
        self.drain_and_unlock();
    }

    // Moves the buffered delta (if any) onto the outbound queue.
    fn enqueue_flush_locked(&self, state: &mut CoalesceState) {
        if !state.pending {
            return;
        }
        state.deadline = None;
        self.timer_tx.send_replace(None);
        state
            .queue
            .push(state.key.event(std::mem::take(&mut state.buf)));
        state.pending = false;
        state.key = DeltaKey::default();
        state.last_forward = Some(Instant::now());
    }

    // Forwards queued events in FIFO order. Exactly one caller drains at a
    // time; others enqueue and return. The drain check is atomic under the
    // lock, so a drain started between a mutation and its call emits the
    // interleaved queue in the same FIFO order.
    fn drain_and_unlock(&self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.draining || state.queue.is_empty() {
            return;
        }
        state.draining = true;
        loop {
            let batch = std::mem::take(&mut state.queue);
            drop(state);
            for e in batch {
                self.inner.emit(e);
            }
            state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if state.queue.is_empty() {
                break;
            }
        }
        state.draining = false;
    }
}

/// The background flush: fires one window after the first buffered delta.
/// Re-arms land through the watch channel; a fire racing a re-arm flushes
/// whatever is pending, the same tolerance Go's timer.Stop/Reset race allows.
async fn timer_loop(weak: Weak<Coalescer>, mut rx: watch::Receiver<Option<Instant>>) {
    loop {
        let deadline = *rx.borrow_and_update();
        let Some(deadline) = deadline else {
            if rx.changed().await.is_err() {
                return;
            }
            continue;
        };
        tokio::select! {
            _ = tokio::time::sleep_until(deadline) => {
                let Some(this) = weak.upgrade() else {
                    return;
                };
                this.flush();
            }
            changed = rx.changed() => {
                if changed.is_err() {
                    return;
                }
            }
        }
    }
}

impl Sink for Coalescer {
    fn emit(&self, e: Event) {
        Coalescer::emit(self, e);
    }

    // Optional capabilities flush first so audits never overtake a buffered
    // delta, then forward to inner sinks that opt in.
    fn record_turn_completion(&self) {
        self.flush();
        self.inner.record_turn_completion();
    }

    fn record_memory_recall(&self, a: super::capabilities::MemoryRecallAudit) {
        self.flush();
        self.inner.record_memory_recall(a);
    }

    fn record_delegation_audit(&self, a: super::capabilities::DelegationAudit) {
        self.flush();
        self.inner.record_delegation_audit(a);
    }

    fn record_workspace_mutation(&self, m: super::capabilities::WorkspaceMutation) {
        self.flush();
        self.inner.record_workspace_mutation(m);
    }

    fn record_run_budget(&self, s: super::capabilities::RunBudgetSample) {
        self.flush();
        self.inner.record_run_budget(s);
    }

    fn quote_usage(&self, e: &Event) -> Option<reasonix_contract::pricing::CostQuote> {
        // A query, not a delivery: the buffered delta stays put.
        self.inner.quote_usage(e)
    }

    fn capture_usage_scope(&self) -> Option<Arc<dyn std::any::Any + Send + Sync>> {
        self.inner.capture_usage_scope()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    struct RecordSink {
        events: Mutex<Vec<Event>>,
        turns: AtomicUsize,
        workspace: AtomicUsize,
    }

    impl RecordSink {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                events: Mutex::new(Vec::new()),
                turns: AtomicUsize::new(0),
                workspace: AtomicUsize::new(0),
            })
        }

        fn snapshot(&self) -> Vec<Event> {
            self.events
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone()
        }
    }

    impl Sink for RecordSink {
        fn emit(&self, e: Event) {
            self.events
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(e);
        }

        fn record_turn_completion(&self) {
            self.turns.fetch_add(1, Ordering::SeqCst);
        }

        fn record_workspace_mutation(&self, _m: crate::sink::capabilities::WorkspaceMutation) {
            self.workspace.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn text(text: &str) -> Event {
        Event {
            kind: "text".to_owned(),
            text: text.to_owned(),
            ..Event::default()
        }
    }

    fn reasoning(text: &str) -> Event {
        Event {
            kind: "reasoning".to_owned(),
            text: text.to_owned(),
            ..Event::default()
        }
    }

    // Ported: coalesce_test.go TestCoalesceFirstDeltaForwardsImmediately.
    #[tokio::test]
    async fn first_delta_forwards_immediately() {
        let inner = RecordSink::new();
        let c = coalesce(inner.clone(), Duration::from_secs(3600));
        c.emit(text("hello"));
        let got = inner.snapshot();
        assert_eq!(got.len(), 1, "first delta must forward immediately");
        assert_eq!(got[0].text, "hello");
    }

    // Ported: coalesce_test.go TestCoalesceMergesBurstAndFlushesOnBarrier.
    #[tokio::test]
    async fn merges_burst_and_flushes_on_barrier() {
        let inner = RecordSink::new();
        let c = coalesce(inner.clone(), Duration::from_secs(3600));
        c.emit(reasoning("a")); // leading edge
        c.emit(reasoning("b"));
        c.emit(reasoning("c"));
        c.emit(Event {
            kind: "tool_dispatch".to_owned(),
            tool: Some(Tool {
                id: "t1".to_owned(),
                name: "bash".to_owned(),
                ..Tool::default()
            }),
            ..Event::default()
        }); // barrier
        let got = inner.snapshot();
        assert_eq!(
            got.len(),
            3,
            "want leading delta, merged burst, barrier: {got:?}"
        );
        assert_eq!(got[0].text, "a");
        assert_eq!(got[1].kind, "reasoning");
        assert_eq!(got[1].text, "bc");
        assert_eq!(
            got[2].kind, "tool_dispatch",
            "barrier arrives after the burst"
        );
    }

    // Ported: coalesce_test.go TestCoalesceKindSwitchFlushes.
    #[tokio::test]
    async fn kind_switch_flushes() {
        let inner = RecordSink::new();
        let c = coalesce(inner.clone(), Duration::from_secs(3600));
        c.emit(reasoning("think")); // leading edge
        c.emit(reasoning("ing"));
        c.emit(text("answer")); // switches kind: flush + buffer
        c.emit(Event {
            kind: "turn_done".to_owned(),
            ..Event::default()
        });
        let got = inner.snapshot();
        assert_eq!(got.len(), 4, "{got:?}");
        assert_eq!(got[1].kind, "reasoning");
        assert_eq!(got[1].text, "ing");
        assert_eq!(got[2].kind, "text");
        assert_eq!(got[2].text, "answer");
    }

    // Ported: coalesce_test.go TestCoalesceWindowFlushesBufferedTail.
    #[tokio::test(start_paused = true)]
    async fn window_flushes_buffered_tail() {
        let inner = RecordSink::new();
        let c = coalesce(inner.clone(), Duration::from_millis(20));
        c.emit(text("lead"));
        c.emit(text("tail"));
        tokio::time::sleep(Duration::from_millis(50)).await;
        let got = inner.snapshot();
        assert_eq!(got.len(), 2, "buffered tail must flush on the window");
        assert_eq!(got[1].text, "tail");
    }

    // Ported: coalesce_test.go TestCoalesceByteCapFlushes.
    #[tokio::test]
    async fn byte_cap_flushes() {
        let inner = RecordSink::new();
        let c = coalesce(inner.clone(), Duration::from_secs(3600));
        c.emit(text("lead"));
        c.emit(text(&"x".repeat(COALESCE_MAX_BYTES)));
        let got = inner.snapshot();
        assert_eq!(got.len(), 2, "byte cap must flush synchronously");
        assert_eq!(got[1].text.len(), COALESCE_MAX_BYTES);
    }

    // Ported: coalesce_test.go TestCoalesceCapabilitiesFlushFirstAndForward.
    #[tokio::test]
    async fn capabilities_flush_first_and_forward() {
        let inner = RecordSink::new();
        let c = coalesce(inner.clone(), Duration::from_secs(3600));
        c.emit(text("lead"));
        c.emit(text("tail"));
        c.record_turn_completion();
        c.record_workspace_mutation(crate::sink::capabilities::WorkspaceMutation {
            content: true,
            ..Default::default()
        });
        let got = inner.snapshot();
        assert_eq!(
            got.len(),
            2,
            "capability call must flush the buffered delta first: {got:?}"
        );
        assert_eq!(got[1].text, "tail");
        assert_eq!(inner.turns.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(inner.workspace.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    // Ported: coalesce_test.go TestCoalesceNonPureDeltaPassesThrough.
    #[tokio::test]
    async fn non_pure_delta_passes_through() {
        let inner = RecordSink::new();
        let c = coalesce(inner.clone(), Duration::from_secs(3600));
        c.emit(text("lead"));
        c.emit(text("buffered"));
        // A text event carrying any extra field is not a pure delta: it must
        // not merge, and it must flush the buffer ahead of itself.
        c.emit(Event {
            kind: "text".to_owned(),
            text: "detailed".to_owned(),
            detail: "diag".to_owned(),
            ..Event::default()
        });
        let got = inner.snapshot();
        assert_eq!(got.len(), 3, "{got:?}");
        assert_eq!(got[1].text, "buffered");
        assert_eq!(got[2].detail, "diag");
    }

    // Re-enters the wrapping sink from inside emit, the way a frontend
    // callback can synchronously call back into the controller.
    struct ReentrantSink {
        outer: Mutex<Option<Arc<dyn Sink>>>,
        events: Mutex<Vec<Event>>,
        fired: AtomicBool,
    }

    impl Sink for ReentrantSink {
        fn emit(&self, e: Event) {
            self.events
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(e.clone());
            if e.kind == "approval_request"
                && !self.fired.swap(true, Ordering::SeqCst)
                && let Some(outer) = self.outer.lock().unwrap_or_else(|e| e.into_inner()).clone()
            {
                outer.emit(Event {
                    kind: "notice".to_owned(),
                    text: "receipt".to_owned(),
                    ..Event::default()
                });
            }
        }
    }

    // Ported: coalesce_test.go TestCoalesceReentrantEmitDoesNotDeadlock.
    #[tokio::test]
    async fn reentrant_emit_does_not_deadlock() {
        let inner = Arc::new(ReentrantSink {
            outer: Mutex::new(None),
            events: Mutex::new(Vec::new()),
            fired: AtomicBool::new(false),
        });
        let c = coalesce(inner.clone(), Duration::from_secs(3600));
        *inner.outer.lock().unwrap_or_else(|e| e.into_inner()) = Some(c.clone());

        c.emit(text("lead"));
        c.emit(text("buffered"));
        c.emit(Event {
            kind: "approval_request".to_owned(),
            ..Event::default()
        });

        let got = inner
            .events
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let kinds: Vec<&str> = got.iter().map(|e| e.kind.as_str()).collect();
        assert_eq!(kinds, ["text", "text", "approval_request", "notice"]);
    }

    // Ported: coalesce_test.go TestCoalesceDisabledOrNil — a zero window
    // returns the inner sink unchanged.
    #[test]
    fn zero_window_returns_inner_unchanged() {
        let inner: Arc<dyn Sink> = Arc::new(crate::sink::Discard);
        let same = coalesce(inner.clone(), Duration::ZERO);
        assert!(Arc::ptr_eq(&inner, &same));
    }

    // Ported: coalesce_test.go TestCoalesceMergesCompactionProgress — a digest
    // merges like any other stream delta, and the barrier before
    // compaction_done still lands after everything written before it.
    #[tokio::test]
    async fn merges_compaction_progress() {
        let inner = RecordSink::new();
        let c = coalesce(inner.clone(), Duration::from_secs(3600));
        c.emit(Event {
            kind: "compaction_progress".to_owned(),
            text: "## ".to_owned(),
            ..Event::default()
        }); // leading edge
        c.emit(Event {
            kind: "compaction_progress".to_owned(),
            text: "Goal\n".to_owned(),
            ..Event::default()
        });
        c.emit(Event {
            kind: "compaction_progress".to_owned(),
            text: "- ship it".to_owned(),
            ..Event::default()
        });
        c.emit(Event {
            kind: "compaction_done".to_owned(),
            compaction: Some(reasonix_contract::eventwire::event::Compaction {
                trigger: "auto".to_owned(),
                ..Default::default()
            }),
            ..Event::default()
        });
        let got = inner.snapshot();
        assert_eq!(got.len(), 3, "{got:?}");
        assert_eq!(got[1].kind, "compaction_progress");
        assert_eq!(got[1].text, "Goal\n- ship it");
        assert_eq!(got[2].kind, "compaction_done");
    }

    // Ported: coalesce_test.go TestCoalesceMergesToolProgress — a build log is
    // hundreds of frames a second unless they merge.
    #[tokio::test]
    async fn merges_tool_progress() {
        let inner = RecordSink::new();
        let c = coalesce(inner.clone(), Duration::from_secs(3600));
        let progress = |id: &str, out: &str| Event {
            kind: "tool_progress".to_owned(),
            tool: Some(Tool {
                id: id.to_owned(),
                output: out.to_owned(),
                ..Tool::default()
            }),
            ..Event::default()
        };
        c.emit(progress("call-1", "line 1\n")); // leading edge
        for _ in 2..=100 {
            c.emit(progress("call-1", "line\n"));
        }
        c.emit(Event {
            kind: "tool_result".to_owned(),
            tool: Some(Tool {
                id: "call-1".to_owned(),
                output: "done".to_owned(),
                ..Tool::default()
            }),
            ..Event::default()
        });
        let got = inner.snapshot();
        assert_eq!(got.len(), 3, "{got:?}");
        assert_eq!(got[0].kind, "tool_progress");
        assert_eq!(got[0].tool.as_ref().unwrap().id, "call-1");
        assert_eq!(got[0].tool.as_ref().unwrap().output, "line 1\n");
        assert_eq!(got[1].tool.as_ref().unwrap().id, "call-1");
        assert_eq!(got[1].tool.as_ref().unwrap().output, "line\n".repeat(99));
    }

    // Ported: coalesce_test.go
    // TestCoalesceSeparatesToolProgressByCallID — two calls reporting at once
    // must not be concatenated into one another's card.
    #[tokio::test]
    async fn separates_tool_progress_by_call_id() {
        let inner = RecordSink::new();
        let c = coalesce(inner.clone(), Duration::from_secs(3600));
        let progress = |id: &str, out: &str| Event {
            kind: "tool_progress".to_owned(),
            tool: Some(Tool {
                id: id.to_owned(),
                output: out.to_owned(),
                ..Tool::default()
            }),
            ..Event::default()
        };
        c.emit(progress("a", "a1")); // leading edge
        c.emit(progress("a", "a2"));
        c.emit(progress("b", "b1"));
        c.emit(Event {
            kind: "tool_result".to_owned(),
            tool: Some(Tool {
                id: "b".to_owned(),
                output: "done".to_owned(),
                ..Tool::default()
            }),
            ..Event::default()
        });
        let got = inner.snapshot();
        assert_eq!(got.len(), 4, "{got:?}");
        assert_eq!(got[1].tool.as_ref().unwrap().id, "a");
        assert_eq!(got[1].tool.as_ref().unwrap().output, "a2");
        assert_eq!(got[2].tool.as_ref().unwrap().id, "b");
        assert_eq!(got[2].tool.as_ref().unwrap().output, "b1");
    }

    // Ported: coalesce_test.go
    // TestCoalescePassesThroughRichToolProgress — a sub-agent's status channel
    // carries name/parentId/rate, not just id+output; folding those would lose
    // the fields, so they must cross untouched.
    #[tokio::test]
    async fn passes_through_rich_tool_progress() {
        let inner = RecordSink::new();
        let c = coalesce(inner.clone(), Duration::from_secs(3600));
        for _ in 0..5 {
            c.emit(Event {
                kind: "tool_progress".to_owned(),
                tool: Some(Tool {
                    id: "sub".to_owned(),
                    name: crate::broadcaster::SUBAGENT_PROGRESS_STATUS.to_owned(),
                    output: "running".to_owned(),
                    rate_tokens_per_sec: 40,
                    ..Tool::default()
                }),
                ..Event::default()
            });
        }
        let got = inner.snapshot();
        assert_eq!(got.len(), 5, "rich progress must not merge: {got:?}");
        for e in &got {
            let tool = e.tool.as_ref().unwrap();
            assert_eq!(tool.name, crate::broadcaster::SUBAGENT_PROGRESS_STATUS);
            assert_eq!(tool.rate_tokens_per_sec, 40);
        }
    }

    // Ported: coalesce_test.go
    // TestCoalesceBypassesToolProgressWithoutID — progress without a call id
    // cannot be attributed, so it must not be buffered under an empty key
    // where every unrelated chunk would land.
    #[tokio::test]
    async fn bypasses_tool_progress_without_id() {
        let inner = RecordSink::new();
        let c = coalesce(inner.clone(), Duration::from_secs(3600));
        let orphan = || Event {
            kind: "tool_progress".to_owned(),
            tool: Some(Tool {
                output: "orphan".to_owned(),
                ..Tool::default()
            }),
            ..Event::default()
        };
        c.emit(orphan());
        c.emit(orphan());
        assert_eq!(inner.snapshot().len(), 2);
    }

    // Ported: coalesce_test.go
    // TestCoalesceMergesDeltasCarryingSource — every progress chunk an
    // executor emits carries Source, and merging must survive it.
    #[tokio::test]
    async fn merges_deltas_carrying_source() {
        let inner = RecordSink::new();
        let c = coalesce(inner.clone(), Duration::from_secs(3600));
        let progress = |out: &str| Event {
            kind: "tool_progress".to_owned(),
            source: "executor".to_owned(),
            tool: Some(Tool {
                id: "call-1".to_owned(),
                output: out.to_owned(),
                ..Tool::default()
            }),
            ..Event::default()
        };
        c.emit(progress("line 1\n")); // leading edge
        for _ in 0..99 {
            c.emit(progress("line\n"));
        }
        c.emit(Event {
            kind: "tool_result".to_owned(),
            tool: Some(Tool {
                id: "call-1".to_owned(),
                output: "done".to_owned(),
                ..Tool::default()
            }),
            ..Event::default()
        });
        let got = inner.snapshot();
        assert_eq!(got.len(), 3, "{got:?}");
        assert_eq!(got[1].source, "executor", "merged frame lost its source");
        assert_eq!(got[1].tool.as_ref().unwrap().output, "line\n".repeat(99));
    }

    // Ported: coalesce_test.go TestCoalesceMergesTextCarryingSource — a delta
    // from a named source must still merge.
    #[tokio::test]
    async fn merges_text_carrying_source() {
        let inner = RecordSink::new();
        let c = coalesce(inner.clone(), Duration::from_secs(3600));
        c.emit(Event {
            kind: "text".to_owned(),
            source: "executor".to_owned(),
            text: "a".to_owned(),
            ..Event::default()
        });
        for s in ["b", "c", "d"] {
            c.emit(Event {
                kind: "text".to_owned(),
                source: "executor".to_owned(),
                text: s.to_owned(),
                ..Event::default()
            });
        }
        c.emit(Event {
            kind: "turn_done".to_owned(),
            ..Event::default()
        });
        let got = inner.snapshot();
        assert_eq!(got.len(), 3, "{got:?}");
        assert_eq!(got[0].text, "a");
        assert_eq!(got[0].source, "executor");
        assert_eq!(got[1].text, "bcd");
        assert_eq!(got[1].source, "executor");
    }

    // Ported: coalesce_test.go TestCoalesceSeparatesBySource — different
    // sources must not merge into one another: a planner's text and an
    // executor's are attributable separately.
    #[tokio::test]
    async fn separates_by_source() {
        let inner = RecordSink::new();
        let c = coalesce(inner.clone(), Duration::from_secs(3600));
        c.emit(Event {
            kind: "text".to_owned(),
            source: "executor".to_owned(),
            text: "e1".to_owned(),
            ..Event::default()
        });
        c.emit(Event {
            kind: "text".to_owned(),
            source: "planner".to_owned(),
            text: "p1".to_owned(),
            ..Event::default()
        });
        c.emit(Event {
            kind: "turn_done".to_owned(),
            ..Event::default()
        });
        let got = inner.snapshot();
        assert_eq!(got.len(), 3, "{got:?}");
        assert_eq!(got[0].source, "executor");
        assert_eq!(got[0].text, "e1");
        assert_eq!(got[1].source, "planner");
        assert_eq!(got[1].text, "p1");
    }
}

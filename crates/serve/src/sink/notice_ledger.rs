//! Notice dedup across rebuilds (docs/contracts/sse.md §5; Go
//! `notice_ledger.go`). A session rebuild re-derives the same notices from the
//! same state; the ledger remembers which ones a lineage's previous build
//! showed, so a rebuild that re-derives one says nothing. A notice's identity
//! is its level, code, text and detail.
//!
//! sse.md §9: a notice repeated by a rebuild is intentionally silent — parity
//! tests must not expect it twice.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use reasonix_contract::eventwire::event::Event;
use reasonix_contract::pricing::CostQuote;

use super::Sink;
use super::capabilities::{
    AuditForwarder, DelegationAudit, MemoryRecallAudit, RunBudgetSample, WorkspaceMutation,
};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct NoticeKey {
    level: String,
    code: String,
    text: String,
    detail: String,
}

/// Remembers which notices a lineage's previous build showed (Go
/// `NoticeLedger`).
#[derive(Default)]
pub struct NoticeLedger {
    seen: Mutex<Option<HashSet<NoticeKey>>>,
}

impl NoticeLedger {
    /// An empty ledger: the first build emits everything.
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Forgets everything shown, for when the surface that showed it is gone:
    /// the next build emits every notice that still holds.
    pub fn reset(&self) {
        *self.seen.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    /// Opens one build's window over `sink`. Notices the previous successful
    /// build already showed are dropped; everything else, and every
    /// non-notice event, passes through.
    pub fn begin(self: &Arc<Self>, sink: Arc<dyn Sink>) -> Arc<NoticeWindow> {
        let prev = self
            .seen
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .unwrap_or_default();
        Arc::new(NoticeWindow {
            forwarder: AuditForwarder::new(sink.clone()),
            ledger: self.clone(),
            inner: sink,
            prev,
            state: Mutex::new(WindowState {
                cur: HashSet::new(),
                closed: false,
            }),
        })
    }
}

#[derive(Default)]
struct WindowState {
    cur: HashSet<NoticeKey>,
    closed: bool,
}

/// The sink for one build (Go `NoticeWindow`). After `close` it forwards
/// unchanged, so the runtime that outlives the build keeps emitting freely.
pub struct NoticeWindow {
    forwarder: AuditForwarder,
    ledger: Arc<NoticeLedger>,
    inner: Arc<dyn Sink>,
    prev: HashSet<NoticeKey>,
    state: Mutex<WindowState>,
}

impl NoticeWindow {
    /// Ends the window. A successful build's notices replace the ledger, so a
    /// notice that disappears and later returns is shown again; a failed
    /// build leaves the old runtime serving, so what it showed is added to
    /// what was seen.
    pub fn close(&self, ok: bool) {
        let cur = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            state.closed = true;
            state.cur.clone()
        };
        let mut seen = self.ledger.seen.lock().unwrap_or_else(|e| e.into_inner());
        if ok {
            *seen = Some(cur);
            return;
        }
        let mut merged = seen.take().unwrap_or_default();
        merged.extend(cur);
        *seen = Some(merged);
    }
}

impl Sink for NoticeWindow {
    fn emit(&self, e: Event) {
        if e.kind != "notice" {
            self.inner.emit(e);
            return;
        }
        let key = NoticeKey {
            level: e.level.clone(),
            code: e.code.clone(),
            text: e.text.clone(),
            detail: e.detail.clone(),
        };
        let shown = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if state.closed {
                drop(state);
                self.inner.emit(e);
                return;
            }
            let shown = self.prev.contains(&key);
            state.cur.insert(key);
            shown
        };
        if !shown {
            self.inner.emit(e);
        }
    }

    // The window only intercepts notice events; the host-only channels pass
    // through its embedded forwarder untouched (Go: the AuditForwarder embed).
    fn record_turn_completion(&self) {
        self.forwarder.record_turn_completion();
    }

    fn record_memory_recall(&self, a: MemoryRecallAudit) {
        self.forwarder.record_memory_recall(a);
    }

    fn record_delegation_audit(&self, a: DelegationAudit) {
        self.forwarder.record_delegation_audit(a);
    }

    fn record_workspace_mutation(&self, m: WorkspaceMutation) {
        self.forwarder.record_workspace_mutation(m);
    }

    fn record_run_budget(&self, s: RunBudgetSample) {
        self.forwarder.record_run_budget(s);
    }

    fn quote_usage(&self, e: &Event) -> Option<CostQuote> {
        self.forwarder.quote_usage(e)
    }

    fn capture_usage_scope(&self) -> Option<Arc<dyn std::any::Any + Send + Sync>> {
        self.forwarder.capture_usage_scope()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    // Ported: notice_ledger_test.go TestNoticeLedgerWindows.
    #[test]
    fn windows_dedup_across_rebuilds() {
        let got: Arc<StdMutex<Vec<Event>>> = Arc::default();
        let sink_seen = got.clone();
        let sink = Arc::new(FuncSinkShim(sink_seen));
        let ledger = NoticeLedger::new();
        let a = || Event {
            kind: "notice".to_owned(),
            code: "a".to_owned(),
            text: "x".to_owned(),
            ..Event::default()
        };

        let w = ledger.begin(sink.clone());
        w.emit(a());
        w.emit(Event {
            kind: "text".to_owned(),
            text: "delta".to_owned(),
            ..Event::default()
        });
        w.close(true);
        w.emit(a());
        {
            let seen = got.lock().unwrap_or_else(|e| e.into_inner());
            assert_eq!(
                seen.len(),
                3,
                "first build and post-close runtime must emit all"
            );
        }

        // A rebuild shows the changed notice, drops the exact repeat.
        got.lock().unwrap_or_else(|e| e.into_inner()).clear();
        let w = ledger.begin(sink.clone());
        w.emit(a());
        w.emit(Event {
            kind: "notice".to_owned(),
            code: "a".to_owned(),
            text: "x".to_owned(),
            detail: "changed".to_owned(),
            ..Event::default()
        });
        w.close(false);
        let seen = got.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(seen.len(), 1, "repeat dropped, changed kept");
        assert_eq!(seen[0].detail, "changed");
        drop(seen);

        // A failed build's notices count as shown: the old runtime keeps
        // serving what it already displayed.
        got.lock().unwrap_or_else(|e| e.into_inner()).clear();
        let w = ledger.begin(sink.clone());
        w.emit(Event {
            kind: "notice".to_owned(),
            code: "a".to_owned(),
            text: "x".to_owned(),
            detail: "changed".to_owned(),
            ..Event::default()
        });
        w.close(true);
        assert!(
            got.lock().unwrap_or_else(|e| e.into_inner()).is_empty(),
            "a failed build's notices count as shown"
        );
    }

    // Go notice_ledger.go: a notice that disappears and later returns is
    // shown again after a successful build (Close(true) replaces, never
    // accumulates).
    #[test]
    fn a_notice_that_disappears_and_returns_is_shown_again() {
        let got: Arc<StdMutex<Vec<Event>>> = Arc::default();
        let sink = Arc::new(FuncSinkShim(got.clone()));
        let ledger = NoticeLedger::new();
        let notice = |code: &str| Event {
            kind: "notice".to_owned(),
            code: code.to_owned(),
            text: "x".to_owned(),
            ..Event::default()
        };
        let w = ledger.begin(sink.clone());
        w.emit(notice("a"));
        w.emit(notice("b"));
        w.close(true);
        drop(w);

        let w = ledger.begin(sink.clone());
        w.emit(notice("a"));
        w.close(true);
        drop(w);

        // Rebuild three: b left the seen set when the second build never
        // emitted it, so its return is shown; a stays silent.
        got.lock().unwrap_or_else(|e| e.into_inner()).clear();
        let w = ledger.begin(sink.clone());
        w.emit(notice("a"));
        w.emit(notice("b"));
        w.close(true);
        let seen = got.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(seen.len(), 1, "only the returned notice is shown");
        assert_eq!(seen[0].code, "b");
    }

    // After Close the window forwards unchanged: the runtime that outlives the
    // build keeps emitting freely.
    #[test]
    fn post_close_runtime_emits_every_notice() {
        let got: Arc<StdMutex<Vec<Event>>> = Arc::default();
        let sink = Arc::new(FuncSinkShim(got.clone()));
        let ledger = NoticeLedger::new();
        let w = ledger.begin(sink);
        w.emit(Event {
            kind: "notice".to_owned(),
            code: "first".to_owned(),
            ..Event::default()
        });
        w.close(false);
        w.emit(Event {
            kind: "notice".to_owned(),
            code: "first".to_owned(),
            ..Event::default()
        });
        w.emit(Event {
            kind: "notice".to_owned(),
            code: "second".to_owned(),
            ..Event::default()
        });
        let seen = got.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(seen.len(), 3, "closed window forwards unchanged");
    }

    // reset() makes the next build emit everything again.
    #[test]
    fn reset_forgets_everything() {
        let got: Arc<StdMutex<Vec<Event>>> = Arc::default();
        let sink = Arc::new(FuncSinkShim(got.clone()));
        let ledger = NoticeLedger::new();
        let notice = || Event {
            kind: "notice".to_owned(),
            code: "a".to_owned(),
            ..Event::default()
        };
        let w = ledger.begin(sink.clone());
        w.emit(notice());
        w.close(true);
        ledger.reset();
        let w = ledger.begin(sink.clone());
        w.emit(notice());
        w.close(true);
        assert_eq!(got.lock().unwrap_or_else(|e| e.into_inner()).len(), 2);
    }

    /// The Go test's FuncSink: appends every event it receives.
    struct FuncSinkShim(Arc<StdMutex<Vec<Event>>>);

    impl Sink for FuncSinkShim {
        fn emit(&self, e: Event) {
            self.0.lock().unwrap_or_else(|e| e.into_inner()).push(e);
        }
    }
}

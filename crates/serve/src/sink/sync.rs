//! Serializes concurrent emission (docs/contracts/sse.md §5; Go `sync.go`).
//!
//! The base sink contract assumes serial emission — the agent's run loop
//! emits one event at a time — but background jobs emit from their own tasks
//! and can overlap a running turn. Wrapping the session sink once keeps the
//! serial-`emit` invariant every sink relies on (an SSE writer, a webview
//! bridge, a TUI channel) without each having to lock.

use std::sync::{Arc, Mutex};

use reasonix_contract::eventwire::event::Event;
use reasonix_contract::pricing::CostQuote;

use super::Sink;
use super::capabilities::{DelegationAudit, MemoryRecallAudit, RunBudgetSample, WorkspaceMutation};

/// Wraps `inner` so concurrent emits serialize. Capability forwarding
/// serializes under the same lock, keeping the audit channels ordered against
/// the event stream.
pub fn sync_sink(inner: Arc<dyn Sink>) -> Arc<dyn Sink> {
    Arc::new(SyncSink {
        inner,
        mu: Mutex::new(()),
    })
}

struct SyncSink {
    inner: Arc<dyn Sink>,
    mu: Mutex<()>,
}

impl Sink for SyncSink {
    fn emit(&self, e: Event) {
        let _guard = self.mu.lock().unwrap_or_else(|e| e.into_inner());
        self.inner.emit(e);
    }

    fn record_turn_completion(&self) {
        let _guard = self.mu.lock().unwrap_or_else(|e| e.into_inner());
        self.inner.record_turn_completion();
    }

    fn record_memory_recall(&self, a: MemoryRecallAudit) {
        let _guard = self.mu.lock().unwrap_or_else(|e| e.into_inner());
        self.inner.record_memory_recall(a);
    }

    fn record_delegation_audit(&self, a: DelegationAudit) {
        let _guard = self.mu.lock().unwrap_or_else(|e| e.into_inner());
        self.inner.record_delegation_audit(a);
    }

    fn record_workspace_mutation(&self, m: WorkspaceMutation) {
        let _guard = self.mu.lock().unwrap_or_else(|e| e.into_inner());
        self.inner.record_workspace_mutation(m);
    }

    fn record_run_budget(&self, s: RunBudgetSample) {
        let _guard = self.mu.lock().unwrap_or_else(|e| e.into_inner());
        self.inner.record_run_budget(s);
    }

    fn quote_usage(&self, e: &Event) -> Option<CostQuote> {
        let _guard = self.mu.lock().unwrap_or_else(|e| e.into_inner());
        self.inner.quote_usage(e)
    }

    fn capture_usage_scope(&self) -> Option<Arc<dyn std::any::Any + Send + Sync>> {
        let _guard = self.mu.lock().unwrap_or_else(|e| e.into_inner());
        self.inner.capture_usage_scope()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    // Ported: the invariant sync.go pins in its doc comment — overlapping
    // emitters never interleave inside one inner emit.
    #[tokio::test]
    async fn concurrent_emits_serialize_at_the_inner_sink() {
        let in_flight = Arc::new(AtomicUsize::new(0));
        let max_in_flight = Arc::new(AtomicUsize::new(0));

        struct Probe {
            in_flight: Arc<AtomicUsize>,
            max: Arc<AtomicUsize>,
        }
        impl Sink for Probe {
            fn emit(&self, _e: Event) {
                let now = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
                self.max.fetch_max(now, Ordering::SeqCst);
                std::thread::yield_now();
                self.in_flight.fetch_sub(1, Ordering::SeqCst);
            }
        }

        let sink = sync_sink(Arc::new(Probe {
            in_flight: in_flight.clone(),
            max: max_in_flight.clone(),
        }));
        let mut handles = Vec::new();
        for i in 0..16 {
            let sink = sink.clone();
            handles.push(tokio::spawn(async move {
                for _ in 0..50 {
                    sink.emit(Event {
                        kind: "text".to_owned(),
                        text: i.to_string(),
                        ..Event::default()
                    });
                }
            }));
        }
        for h in handles {
            h.await.expect("emit task");
        }
        assert_eq!(max_in_flight.load(Ordering::SeqCst), 1);
    }
}

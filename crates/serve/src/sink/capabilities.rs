//! The optional sink capability payloads and their forwarder
//! (docs/contracts/sse.md §5; Go `sink_capabilities.go`, `audit_forwarder.go`,
//! `record.go`, `run_budget.go`, `workspace_mutation.go`, `usage_quote.go`,
//! `usage_scope.go`). A wrapper that forgets to forward a channel is a silent
//! drop, so the registry here is the authority the completeness test reads —
//! never a list kept beside it.
//!
//! Host-only: none of these payloads ever serialize onto the event wire.

use std::any::Any;
use std::sync::Arc;

use reasonix_contract::eventwire::event::Event;
use reasonix_contract::pricing::CostQuote;

use super::Sink;

/// The registry (Go `capabilityContracts`): every optional channel a wrapper
/// must forward. The completeness test drives each one through
/// [`AuditForwarder`] — a name whose channel stops arriving fails the build.
pub const CAPABILITY_NAMES: &[&str] = &[
    "DelegationAudit",
    "MemoryRecall",
    "RunBudget",
    "TurnCompletion",
    "UsageQuote",
    "WorkspaceMutation",
    "UsageScope",
];

/// One completed sub-agent run, recorded as structured data so a benchmark can
/// compare orchestration arms without scraping prose (Go
/// `hostaudit.DelegationAudit`). Carries what the host observed, never what
/// the child claimed unchecked.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DelegationAudit {
    pub depth: i64,
    /// Host-recorded receipt counts for this child.
    pub tool_calls: i64,
    pub mutations: i64,
    /// Lets an aggregator detect two children touching one file.
    pub mutation_paths: Vec<String>,
    /// Writes the host saw outside the declared claim.
    pub claim_violations: i64,
    /// Directories the delegation narrowed the search to.
    pub parent_scope_hints: i64,
    /// Specific files the delegation text handed over.
    pub parent_named_files: i64,
    /// Every distinct path the child produced a receipt for.
    pub evidence_paths: i64,
    /// The subset of those no named file already covered.
    pub discovered_paths: i64,
}

/// One automatic-recall decision: identifiers, scores and budget numbers only —
/// never the query or fact text (Go `record.go`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MemoryRecallAudit {
    pub hits: Vec<MemoryRecallHit>,
    pub char_budget: i64,
    pub used_chars: i64,
    pub omitted: i64,
    pub omitted_by_limit: i64,
    pub omitted_by_budget: i64,
    /// Why recall stayed silent, as "<code>: <sentence>"; empty when hits were
    /// injected.
    pub suppressed: String,
    /// The typed code `suppressed` begins with; consumers match this, never
    /// the prose.
    pub suppression_code: String,
    /// The Retrieval V2 ranking (telemetry only, never served).
    pub shadow: Vec<MemoryRecallHit>,
}

/// One recalled fact's content-free fingerprint.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MemoryRecallHit {
    pub id: String,
    pub revision: i64,
    pub scope: String,
    pub kind: String,
    pub freshness: String,
    pub score: f64,
}

/// One scope's accumulated spend: counts and money, never content. `priced`
/// requires usage and prices for every observed round; otherwise `cost` holds
/// only the known priced subtotal, not an estimate.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct RunBudgetTotals {
    pub rounds: i64,
    pub requests: i64,
    #[serde(rename = "promptTokens")]
    pub prompt_tokens: i64,
    #[serde(rename = "outputTokens")]
    pub output_tokens: i64,
    pub cost: f64,
    pub priced: bool,
    #[serde(rename = "elapsedMs")]
    pub elapsed_ms: i64,
}

/// Both scopes after a round. `turn` is the current run; `task` spans every
/// run continuing the same work, because "continue" starts a new run and the
/// spend worth stopping accrues across them.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct RunBudgetSample {
    pub turn: RunBudgetTotals,
    pub task: RunBudgetTotals,
    pub currency: String,
}

/// Invalidates resources after a tool invocation may have changed them,
/// including through hooks (Go `workspace_mutation.go`). Neither proof of a
/// write nor a delivery receipt, and independent of tool-result ordering:
/// failed writers still qualify because they may have partially run.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WorkspaceMutation {
    pub tool_id: String,
    pub tool_name: String,
    pub paths: Vec<String>,
    pub all_paths: bool,
    pub content: bool,
    pub tree: bool,
    pub working_tree: bool,
    pub git_meta: bool,
}

/// Forwards every capability channel to the inner sink. Wrappers that only
/// pass host-side signals through delegate to one of these instead of
/// hand-writing forwarding — hand-written forwarding lost capabilities at
/// multiple Go wrappers even while their owning tests stayed green.
pub struct AuditForwarder {
    pub inner: Arc<dyn Sink>,
}

impl AuditForwarder {
    pub fn new(inner: Arc<dyn Sink>) -> Self {
        Self { inner }
    }

    pub fn record_turn_completion(&self) {
        self.inner.record_turn_completion();
    }

    pub fn record_memory_recall(&self, a: MemoryRecallAudit) {
        self.inner.record_memory_recall(a);
    }

    pub fn record_delegation_audit(&self, a: DelegationAudit) {
        self.inner.record_delegation_audit(a);
    }

    pub fn record_workspace_mutation(&self, m: WorkspaceMutation) {
        // The defensive copy mirrors Go: the caller keeps its own paths list
        // even though a consumer downstream could mutate what it received.
        let mut m = m;
        m.paths = m.paths.clone();
        self.inner.record_workspace_mutation(m);
    }

    pub fn record_run_budget(&self, s: RunBudgetSample) {
        self.inner.record_run_budget(s);
    }

    pub fn quote_usage(&self, e: &Event) -> Option<CostQuote> {
        quote_usage(self.inner.as_ref(), e)
    }

    pub fn capture_usage_scope(&self) -> Option<Arc<dyn Any + Send + Sync>> {
        capture_usage_scope(self.inner.as_ref())
    }
}

/// Preserves an existing quote or asks the sink's quoting owner; a sink
/// without [`Sink::quote_usage`] falls back to the no-price quote (Go
/// `usage_quote.go`).
pub fn quote_usage(sink: &dyn Sink, e: &Event) -> Option<CostQuote> {
    if let Some(existing) = e.usage.as_ref().and_then(|u| u.cost_quote.clone()) {
        return Some(existing);
    }
    sink.quote_usage(e)
        .or_else(|| super::costquote::ensure_cost_quote(e, None))
}

/// Reads the current accounting owner without changing it (Go
/// `CaptureUsageScope`).
pub fn capture_usage_scope(sink: &dyn Sink) -> Option<Arc<dyn Any + Send + Sync>> {
    sink.capture_usage_scope()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sink::costquote::ensure_cost_quote;
    use std::sync::Mutex;

    /// Overrides every capability so a chain test can prove a signal reached
    /// the far end rather than dying at some wrapper.
    struct CapturingSink {
        recorded: Mutex<Vec<&'static str>>,
    }

    impl CapturingSink {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                recorded: Mutex::new(Vec::new()),
            })
        }

        fn recorded(&self) -> Vec<&'static str> {
            self.recorded
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone()
        }
    }

    impl Sink for CapturingSink {
        fn emit(&self, _e: Event) {}

        fn record_turn_completion(&self) {
            self.recorded
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push("TurnCompletion");
        }

        fn record_memory_recall(&self, _a: MemoryRecallAudit) {
            self.recorded
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push("MemoryRecall");
        }

        fn record_delegation_audit(&self, _a: DelegationAudit) {
            self.recorded
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push("DelegationAudit");
        }

        fn record_workspace_mutation(&self, _m: WorkspaceMutation) {
            self.recorded
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push("WorkspaceMutation");
        }

        fn record_run_budget(&self, _s: RunBudgetSample) {
            self.recorded
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push("RunBudget");
        }

        fn quote_usage(&self, e: &Event) -> Option<CostQuote> {
            self.recorded
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push("UsageQuote");
            ensure_cost_quote(e, None)
        }

        fn capture_usage_scope(&self) -> Option<Arc<dyn Any + Send + Sync>> {
            self.recorded
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push("UsageScope");
            Some(Arc::new(()))
        }
    }

    fn usage_event() -> Event {
        Event {
            kind: "usage".to_owned(),
            usage: Some(reasonix_contract::eventwire::usage::Usage {
                prompt_tokens: 10,
                completion_tokens: 5,
                ..Default::default()
            }),
            ..Event::default()
        }
    }

    // Ported: sink_capabilities_test.go
    // TestAuditForwarderCoversEveryCapability. The forwarder reads the
    // registry rather than a list kept beside it, so a channel registered here
    // and forwarded by nothing fails this test.
    #[test]
    fn audit_forwarder_covers_every_capability() {
        let recorder = CapturingSink::new();
        let fwd = AuditForwarder::new(recorder.clone());
        for name in CAPABILITY_NAMES {
            match *name {
                "DelegationAudit" => fwd.record_delegation_audit(DelegationAudit::default()),
                "MemoryRecall" => fwd.record_memory_recall(MemoryRecallAudit::default()),
                "RunBudget" => fwd.record_run_budget(RunBudgetSample::default()),
                "TurnCompletion" => fwd.record_turn_completion(),
                "UsageQuote" => {
                    assert!(
                        fwd.quote_usage(&usage_event()).is_some(),
                        "UsageQuote produced no quote"
                    );
                }
                "WorkspaceMutation" => fwd.record_workspace_mutation(WorkspaceMutation::default()),
                "UsageScope" => {
                    assert!(
                        fwd.capture_usage_scope().is_some(),
                        "UsageScope captured nothing"
                    );
                }
                other => panic!("registry name {other:?} has no channel probe"),
            }
        }
        assert_eq!(
            recorder.recorded(),
            CAPABILITY_NAMES.to_vec(),
            "a registered channel never reached the recorder"
        );
    }

    // Ported: sink_capabilities_test.go TestSessionChainDeliversAudits,
    // mirroring boot's assembly order (Coalesce → Sync → CostQuote → recorder).
    #[test]
    fn session_chain_delivers_audits() {
        let recorder = CapturingSink::new();
        let chain = crate::sink::coalesce(
            crate::sink::sync_sink(crate::sink::cost_quote_sink(recorder.clone(), None)),
            std::time::Duration::from_millis(5),
        );
        chain.record_memory_recall(MemoryRecallAudit::default());
        assert_eq!(
            recorder.recorded(),
            ["MemoryRecall"],
            "memory recall never reached the recorder"
        );
    }

    // A sink that overrides no capability drops every channel silently —
    // the default the registry exists to catch, not a panic.
    #[test]
    fn sinks_without_capabilities_drop_channels_silently() {
        let sink = crate::sink::Discard;
        sink.record_memory_recall(MemoryRecallAudit::default());
        sink.record_run_budget(RunBudgetSample::default());
        sink.record_turn_completion();
    }
}

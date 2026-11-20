//! The sink pipeline between emitters and the broadcaster
//! (docs/contracts/sse.md §5; Go `internal/contract/event`): a chain of
//! wrappers every emitter shares, so coalescing, quoting, notice dedup and
//! diff tagging happen once for all frontends. Boot composes
//! `mark_embedded_diffs(goal_tee(coalesce(sync(cost_quote_sink(recorder(frontend))))))`.
//!
//! Capability model: Go delivers optional sink capabilities by type
//! assertion, and a wrapper that misses one silently drops that channel. Each
//! channel here is a default no-op method on [`Sink`] — overriding it is the
//! opt-in — and [`capabilities::AuditForwarder`] is the one component every
//! forwarding wrapper shares, held complete by its registry test.

pub mod capabilities;
pub mod coalesce;
pub mod costquote;
pub mod markdiff;
pub mod notice_ledger;
pub mod sync;

use std::any::Any;
use std::sync::Arc;

use reasonix_contract::eventwire::event::Event;
use reasonix_contract::pricing::CostQuote;

pub use capabilities::{
    AuditForwarder, CAPABILITY_NAMES, DelegationAudit, MemoryRecallAudit, MemoryRecallHit,
    RunBudgetSample, RunBudgetTotals, WorkspaceMutation, capture_usage_scope, quote_usage,
};
pub use coalesce::{DEFAULT_STREAM_DELTA_WINDOW, coalesce};
pub use costquote::{
    CostQuoteSink, DisplayRequest, QuoteContext, cost_quote_sink, ensure_cost_quote,
    normalize_currency,
};
pub use markdiff::mark_embedded_diffs;
pub use notice_ledger::NoticeLedger;
pub use sync::sync_sink;

/// Consumes a turn's events. Emitters call it serially from the run loop, so
/// an implementation need not be safe for concurrent `emit` unless it wraps
/// one that is (`sync::sync_sink` enforces the invariant for background
/// emitters). `emit` must not block indefinitely — a channel-backed sink
/// should be buffered or drained by a live reader.
///
/// The methods below are the optional host-only capability channels (Go
/// `sink_capabilities.go` registry). Defaults are silent drops — exactly the
/// failure the registry test exists to catch: a terminal sink overrides what
/// it consumes; a wrapper overrides every one it must forward.
pub trait Sink: Send + Sync {
    fn emit(&self, e: Event);

    /// One successfully admitted top-level controller run, for synchronous
    /// entry points that publish no turn_done event (Go `TurnCompletionSink`).
    fn record_turn_completion(&self) {}

    /// One automatic-recall decision: identifiers, scores, budget numbers —
    /// never the query or fact text (Go `MemoryRecallSink`).
    fn record_memory_recall(&self, _a: MemoryRecallAudit) {}

    /// One receipt per completed sub-agent run (Go `DelegationAuditSink`).
    fn record_delegation_audit(&self, _a: DelegationAudit) {}

    /// A workspace invalidation hint after a tool may have changed state (Go
    /// `WorkspaceMutationSink`). Neither proof of a write nor a delivery
    /// receipt; failed writers still qualify because they may have partially
    /// run.
    fn record_workspace_mutation(&self, _m: WorkspaceMutation) {}

    /// The per-round spend reading, turn- and task-scoped (Go
    /// `RunBudgetSink`). Shadow: observed, never enforced.
    fn record_run_budget(&self, _s: RunBudgetSample) {}

    /// The occurrence-time quote this sink would attach (Go `UsageQuoter`).
    /// `None` sends callers to [`quote_usage`]'s fallback chain.
    fn quote_usage(&self, _e: &Event) -> Option<CostQuote> {
        None
    }

    /// The opaque host accounting owner (Go `UsageScopeSource`). `None` means
    /// no scope capability; a token names work outside a goal. Host-only,
    /// never serialized onto an event.
    fn capture_usage_scope(&self) -> Option<Arc<dyn Any + Send + Sync>> {
        None
    }
}

/// Adapts a plain function (Go `FuncSink`).
pub struct FuncSink<F: Fn(Event) + Send + Sync>(F);

impl<F: Fn(Event) + Send + Sync> FuncSink<F> {
    pub fn new(f: F) -> Self {
        Self(f)
    }
}

impl<F: Fn(Event) + Send + Sync> Sink for FuncSink<F> {
    fn emit(&self, e: Event) {
        (self.0)(e);
    }
}

/// Drops every event. For tests and runs that only care about the final
/// session state (Go `Discard`).
pub struct Discard;

impl Sink for Discard {
    fn emit(&self, _e: Event) {}
}

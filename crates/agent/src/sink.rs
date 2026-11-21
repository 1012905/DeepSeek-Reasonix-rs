//! The kernel-side event sink (Go: `internal/contract/event` `Sink`).
//!
//! The agent emits [`Event`]s serially from the run loop; implementations
//! need not be concurrent-safe unless they wrap one that is. The richer
//! pipeline (coalesce, cost quote, notice ledger) lives in `reasonix-serve`
//! and wraps *around* a terminal sink at assembly time; serve may depend on
//! this crate, so the boot wave adapts its pipeline onto this trait — the
//! agent never depends on a frontend.

use reasonix_contract::eventwire::event::Event;

/// Consumes a turn's events. `emit` must not block indefinitely.
pub trait Sink: Send + Sync {
    fn emit(&self, e: Event);
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

/// Drops every event (Go `Discard`).
pub struct Discard;

impl Sink for Discard {
    fn emit(&self, _e: Event) {}
}

/// Wire names of the event kinds this crate emits (Go `eventwire/kinds.go`;
/// the closed vocabulary lives in `eventwire::KIND_NAMES`).
pub mod kind {
    pub const TURN_STARTED: &str = "turn_started";
    pub const REASONING: &str = "reasoning";
    pub const TEXT: &str = "text";
    pub const MESSAGE: &str = "message";
    pub const TOOL_DISPATCH: &str = "tool_dispatch";
    pub const TOOL_RESULT: &str = "tool_result";
    pub const USAGE: &str = "usage";
    pub const NOTICE: &str = "notice";
    pub const APPROVAL_REQUEST: &str = "approval_request";
    pub const ASK_REQUEST: &str = "ask_request";
    pub const TURN_PHASE: &str = "turn_phase";
    pub const STREAM_ATTEMPT: &str = "stream_attempt";
    pub const CONTEXT_MAINTENANCE: &str = "context_maintenance";
    pub const COMPACTION_STARTED: &str = "compaction_started";
    pub const COMPACTION_DONE: &str = "compaction_done";
}

/// Notice levels (Go `event.Level`; wire spelling `wireLevel`).
pub mod level {
    pub const INFO: &str = "info";
    pub const WARN: &str = "warn";
}

/// Stream-attempt lifecycle actions (Go `event.StreamAttemptAction`).
pub mod attempt_action {
    pub const BEGIN: &str = "begin";
    pub const DISCARD: &str = "discard";
    pub const COMMIT: &str = "commit";
}

/// Who authored a tool call (Go `event.ToolIssuer`).
pub mod issuer {
    pub const MODEL: &str = "model";
    pub const HOST: &str = "host";
}

/// Turn phase names (Go `event.TurnPhaseName`).
pub mod phase {
    pub const WORKING: &str = "working";
}

/// Notice codes the spine and the window machine emit (Go
/// `notice_codes.go`).
pub mod notice_code {
    pub const EMPTY_FINAL: &str = "empty_final";
    pub const TOOL_BUDGET: &str = "tool_budget";
    pub const COMPACT_HELD: &str = "compact_held";
    pub const CONTEXT_COMPOSITION: &str = "context_composition";
    pub const FOLD_NOT_REUSED_HISTORY: &str = "fold_not_reused_history";
    pub const FOLD_NOT_REUSED_LINEAGE: &str = "fold_not_reused_lineage";
    pub const DECISION_RECEIPT: &str = "decision_receipt";
}

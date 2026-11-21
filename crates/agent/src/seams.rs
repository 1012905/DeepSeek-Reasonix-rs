//! The typed seams later waves implement. Each one is `None` by default and
//! its absence carries Go-nil semantics: the capability degrades, the turn
//! does not. A seam that must be able to refuse returns `Err`; a seam that
//! only shapes output returns its value.

use std::sync::Arc;

use reasonix_contract::provider::Message;
use reasonix_provider::ChatCompletionRequest;
use reasonix_tools::Registry;

use crate::error::TurnError;

pub use crate::compose::DefaultComposer;

/// Composes the provider request from session state (Go:
/// `prepareSamplingRequest`). The default composition is the cache-stable
/// spine shape ([`DefaultComposer`], in [`crate::compose`]): the configured
/// system prompt, the registry's provider schemas, and the transcript's
/// provider projection. A host replaces the seam to add skills, memory
/// projections and turn-tail notes without re-shaping the loop: retries
/// replay the exact request this seam returned, so it must be deterministic
/// in one turn.
pub trait RequestComposer: Send + Sync {
    fn compose(
        &self,
        system: &str,
        session_messages: &[Message],
        registry: &Registry,
        base: &RequestBase,
    ) -> Result<ChatCompletionRequest, TurnError>;
}

/// The non-history parts of a request the loop already resolved (Go: the
/// provider/config fields `buildRequest` reads).
#[derive(Debug, Clone, Default)]
pub struct RequestBase {
    pub temperature: Option<f64>,
    pub max_tokens: i64,
}

/// Decides, per tool call, whether it may run (Go: `Gate`). `Ok(true)` runs
/// the call; `Ok(false, reason)` feeds the reason back to the model as a
/// blocked outcome; `Err` treats the call as blocked for that call only.
/// A `None` gate means no gating — every call runs — preserving headless
/// behaviour for callers that do not wire one in.
pub trait Gate: Send + Sync {
    fn check(
        &self,
        tool_name: &str,
        args: &serde_json::Value,
        read_only: bool,
    ) -> Result<bool, String>;
}

/// One answer to a structured question (Go: `event.AskAnswer`; host-side
/// only — it never crosses the wire).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AskAnswer {
    pub question_id: String,
    pub selected: Vec<String>,
    pub value: Option<serde_json::Value>,
}

/// Puts structured questions to the user and blocks for the answers (Go:
/// `Asker`). `None` is a headless run: a tool reaching for the user gets an
/// empty answer set and never an invented one.
pub trait Asker: Send + Sync {
    fn ask(&self, questions: &[reasonix_contract::eventwire::ask::AskQuestion]) -> Vec<AskAnswer>;
}

/// The context-maintenance seam (Go: `compact*.go`). Consulted between
/// rounds, after tool results land; `Ok(false)` means nothing ran. The
/// compaction wave owns the trigger estimation, the fold, and the commit.
pub trait Compactor: Send + Sync {
    fn maybe_compact(&self, turn: &crate::run::TurnState) -> Result<bool, TurnError>;
}

/// The fold transaction the compaction wave owns (Go: `compactToProjection`
/// and its region planning, summary call, and checkpoint install). The
/// context window calls it from Prepare once the trigger and receipt
/// machine have decided a fold should run; a `None` executor degrades the
/// fold — requests still go out, the provider rules — never the turn.
pub mod fold_executor {
    /// What a fold may waive (Go: `compactionScope`).
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
    pub struct FoldScope {
        /// Folds without waiting for the automatic trigger.
        pub ignore_threshold: bool,
        /// Folds even when the saving does not pay for it.
        pub ignore_economics: bool,
        /// Waives only how much was added (Go: `ignoreAmount` — a
        /// hand-typed compact on an unchanged context).
        pub ignore_amount: bool,
    }

    /// One fold transaction (Go: the `compactToProjection` arguments).
    #[derive(Debug, Clone)]
    pub struct FoldRequest {
        pub trigger: String,
        pub instructions: String,
        pub scope: FoldScope,
        /// Where this call would otherwise answer compaction-required, the
        /// fold is the only way out and a failed summary must degrade
        /// rather than strand the turn (Go: `mustFree`).
        pub must_free: bool,
    }

    /// How the fold fared (Go: `CompactionOutcome` + the noop reason). An
    /// install writes a projection; a noop folds nothing and names why.
    #[derive(Debug, Clone, Default, PartialEq, Eq)]
    pub struct FoldResult {
        pub installed: bool,
        /// Set only when `installed` is false (Go: `CompactionNoopReason`).
        pub noop_reason: Option<&'static str>,
    }

    /// Why a fold transaction failed. The variants are the identity (Go:
    /// `errCheckpointRejected` + `compactionFailureCode`'s sentinels); a
    /// frontend matches the code the window records, never the sentence.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum FoldError {
        /// The transcript changed during the summary call (Go:
        /// `errCompressStaleContext`).
        StaleContext,
        /// The candidate was judged not worth installing (Go:
        /// `checkpointRejection`). `code` is the typed verdict, `reason`
        /// its sentence.
        CheckpointRejected { code: &'static str, reason: String },
        /// A compaction extension refused the fold (Go:
        /// `errCompactionHookRefused`).
        HookRefused,
        /// The projection could not be persisted (Go:
        /// `errProjectionNotPersisted`).
        PersistFailed,
        /// The caller cancelled the turn (Go: `context.Canceled`).
        Cancelled,
        /// Nobody classified it (Go: the `FailUnclassified` fallback).
        Failed(String),
    }

    impl std::fmt::Display for FoldError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                FoldError::StaleContext => write!(f, "context changed during the fold"),
                FoldError::CheckpointRejected { reason, .. } => {
                    write!(f, "checkpoint candidate rejected: {reason}")
                }
                FoldError::HookRefused => write!(f, "compaction extension refused the fold"),
                FoldError::PersistFailed => write!(f, "projection could not be persisted"),
                FoldError::Cancelled => write!(f, "cancelled"),
                FoldError::Failed(msg) => write!(f, "{msg}"),
            }
        }
    }

    /// The fold machine (Go: `compactToProjection`). Deterministic per
    /// canonical state: the same transcript and scope reach the same
    /// verdict.
    pub trait FoldExecutor: Send + Sync {
        fn fold(&self, request: &FoldRequest) -> Result<FoldResult, FoldError>;
    }
}

/// Makes an admitted user message durable (Go: `AdmissionSaver`). `None`
/// leaves durability to the host's own save points; the message still lands
/// in the in-memory transcript either way.
pub trait TranscriptSaver: Send + Sync {
    fn save_admitted_message(&self);
}

/// Mid-turn guidance admitted while the turn runs (Go: `steerInbox` — the
/// trivial seam). `None` means no steering. The durable inbox wave replaces
/// the polling shape with item ids, loaders, and the unapplied path.
pub trait SteerInbox: Send + Sync {
    /// Drains one queued guidance item, oldest first.
    fn drain(&self) -> Option<String>;
}

/// Hands a sub-task to another agent (Go: the `task` machinery).
/// `None` refuses delegation with a typed answer instead of inventing a
/// sub-agent. The delegation wave implements the scheduler, the profile
/// boundary, and the child runtime.
pub trait Delegator: Send + Sync {
    fn delegate(
        &self,
        call_id: &str,
        args: &serde_json::Value,
        sink: Arc<dyn crate::sink::Sink>,
    ) -> Result<String, String>;
}

/// The refusal a `None` delegation seam answers with (Go: the unknown-tool
/// path — no `task` tool exists in the registry, so a call lands on the
/// unknown-tool outcome; this code exists for a registry that advertises
/// one without a seam behind it).
pub const DELEGATION_UNAVAILABLE: &str = "task.delegation_unavailable";

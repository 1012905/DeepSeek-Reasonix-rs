//! Turn failures and resumable stops (Go: the error types `Run` returns —
//! `EmptyFinalError`, `maxStepsPause`, `taskBudgetPause`, context.Canceled).
//!
//! Identity is the variant, never the message: a caller tells a provider
//! failure, a cancellation, and a resumable pause apart without matching
//! text.

use reasonix_provider::ProviderError;

/// Why a turn ended without a final answer (Go: the Run error surface).
#[derive(Debug)]
pub enum TurnError {
    /// The provider failed after every sampling attempt (Go: the
    /// sampling-recovery terminal). The failed attempts committed nothing:
    /// no assistant message, no executed tools.
    Provider(ProviderError),
    /// The caller cancelled the turn (Go: `context.Canceled`). Completed
    /// tool pairs stay in the transcript; nothing after the cancellation
    /// point is committed.
    Cancelled,
    /// The model produced no visible answer for `blocks` consecutive
    /// finals (Go: `EmptyFinalError`).
    EmptyFinal { blocks: i64, finish_reason: String },
    /// The request could not be composed (Go: prepareSamplingRequest
    /// failure — an empty transcript, a composer seam refusal).
    Composition(String),
    /// The model called context-unavailable tools again after the repair
    /// instruction and offered no visible answer (Go: the
    /// handleToolRound repeated-unavailable terminal).
    ContextToolLoop { tools: Vec<String> },
}

impl std::fmt::Display for TurnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TurnError::Provider(err) => write!(f, "{err}"),
            TurnError::Cancelled => write!(f, "cancelled"),
            TurnError::EmptyFinal {
                blocks,
                finish_reason,
            } => write!(
                f,
                "model returned no visible answer for {blocks} consecutive finals (finish={finish_reason})"
            ),
            TurnError::Composition(msg) => write!(f, "{msg}"),
            TurnError::ContextToolLoop { tools } => {
                write!(
                    f,
                    "model repeatedly called context-unavailable tools without a visible answer: {}",
                    tools.join(", ")
                )
            }
        }
    }
}

impl std::error::Error for TurnError {}

/// How a turn that reached a stop came to land (Go: `landCause` +
/// `gracePause`). The work so far is in the transcript; the next message
/// continues the task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopCause {
    /// The model gave a final answer the host accepted.
    Final,
    /// The configured tool-round ceiling fired and the grace round ended
    /// without a final answer (Go: `maxStepsPause`).
    MaxSteps { steps: i64, key: String },
    /// A caller-armed budget axis stopped the turn (Go: `taskBudgetPause`).
    Budget { axis: String, detail: String },
}

impl std::fmt::Display for StopCause {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StopCause::Final => write!(f, "final answer"),
            StopCause::MaxSteps { steps, key } => write!(
                f,
                "paused after reaching the tool-round budget ({key}={steps}) — the work so far is \
                 saved; send another message to continue"
            ),
            StopCause::Budget { axis, detail } => write!(
                f,
                "paused after reaching this task's {axis} budget ({detail}) — the work so far is \
                 saved; send another message to continue"
            ),
        }
    }
}

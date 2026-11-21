//! The agent loop (Go: `internal/runtime/agent` — the turn-loop spine).
//!
//! One `Agent` drives one conversation: a provider client, a tool registry,
//! and a sessionstore transcript wired into the machine that takes a user
//! turn, composes a chat request from session state, streams the provider
//! response, dispatches tool calls, and appends every landed message to the
//! transcript. Turn phases mirror `run_loop.go`: begin → sampling rounds →
//! tool rounds → final answer, with a grace round when a round ceiling
//! stops the turn.
//!
//! Cross-cutting machines (compaction, approval orchestration, delegation,
//! the durable steer inbox, prompt composition) join as typed seams in
//! [`seams`]; each is `None` by default with Go-nil semantics — a missing
//! seam degrades the capability, never the turn.

#![forbid(unsafe_code)]

pub mod cancel;
pub mod compose;
pub mod config;
pub mod error;
pub mod execute;
pub mod run;
pub mod seams;
pub mod sink;
pub mod stream;
pub mod usage;

pub use cancel::CancelToken;
pub use compose::DefaultComposer;
pub use config::{
    AgentConfig, MAX_EMPTY_FINAL_BLOCKS, MAX_SAMPLING_ATTEMPTS, MAX_TOOL_OUTPUT_BYTES,
};
pub use error::{StopCause, TurnError};
pub use execute::ToolOutcome;
pub use run::{Agent, TurnOutput};
pub use seams::{Asker, Compactor, Gate, RequestComposer, SteerInbox, TranscriptSaver};
pub use sink::{Discard, Sink};
pub use stream::StreamedTurn;
pub use usage::{finalize_sampling_usage, merge_sampling_usage};

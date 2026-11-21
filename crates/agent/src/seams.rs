//! The typed seams later waves implement. Each one is `None` by default and
//! its absence carries Go-nil semantics: the capability degrades, the turn
//! does not. A seam that must be able to refuse returns `Err`; a seam that
//! only shapes output returns its value.

use std::sync::Arc;

use reasonix_contract::provider::Message;
use reasonix_provider::ChatCompletionRequest;
use reasonix_tools::Registry;

use crate::error::TurnError;

/// Composes the provider request from session state (Go:
/// `prepareSamplingRequest`). The default composition is the cache-stable
/// spine shape: the configured system prompt, the registry's provider
/// schemas, and the transcript's provider projection.
///
/// The prompt-composition wave replaces the default — skills, memory
/// projections, turn-tail notes — without re-shaping the loop: retries
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

/// The default composition (Go: the inline body of
/// `prepareSamplingRequest` minus the machines not yet ported).
pub struct DefaultComposer;

impl RequestComposer for DefaultComposer {
    fn compose(
        &self,
        system: &str,
        session_messages: &[Message],
        registry: &Registry,
        base: &RequestBase,
    ) -> Result<ChatCompletionRequest, TurnError> {
        let tools = registry
            .schemas()
            .into_iter()
            .map(|schema| reasonix_provider::ToolSchema {
                name: schema.name,
                description: schema.description,
                parameters: serde_json::to_string(&schema.parameters)
                    .unwrap_or_else(|_| String::new()),
            })
            .collect();
        let mut messages = Vec::with_capacity(session_messages.len() + 1);
        if !system.is_empty() {
            messages.push(Message {
                role: reasonix_contract::provider::Role::SYSTEM,
                content: system.to_owned(),
                ..Message::default()
            });
        }
        messages.extend(reasonix_contract::provider::model_messages(session_messages).into_owned());
        Ok(ChatCompletionRequest {
            messages,
            tools,
            temperature: base.temperature,
            max_tokens: base.max_tokens,
            effort_override: String::new(),
        })
    }
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

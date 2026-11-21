//! The Asker seam a tool reaches through its call context (Go: the
//! `agent.Asker` carried in `context.Context` by `CallContext`). The tool
//! that asks owns the port; the host installs an implementation as one
//! typed binding, so `crates/tools` never reaches up into the agent.

use std::sync::Arc;

use reasonix_contract::eventwire::ask::AskQuestion;

use crate::abi::CallContext;

/// One answer to a structured question (Go: `event.AskAnswer`; host-side
/// only — it never crosses the wire).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AskAnswer {
    pub question_id: String,
    pub selected: Vec<String>,
    pub value: Option<serde_json::Value>,
}

/// What one ask resolved to. `timed_out` is the host's own cause, never an
/// empty answer: it names how long the wait was so the model can say the
/// question was skipped rather than dismissed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AskOutcome {
    pub answers: Vec<AskAnswer>,
    pub timed_out: Option<std::time::Duration>,
}

/// Puts structured questions to the user and blocks for the answers (Go:
/// `Asker`). A host with nobody to ask installs nothing, and the tool
/// leaves the decision unresolved instead of inventing an answer.
pub trait Asker: Send + Sync {
    fn ask(&self, questions: &[AskQuestion]) -> AskOutcome;
}

/// The asker bound to one call, when the host installed one.
pub fn asker_from(ctx: &CallContext) -> Option<Arc<dyn Asker>> {
    ctx.value()?.downcast_ref::<Arc<dyn Asker>>().cloned()
}

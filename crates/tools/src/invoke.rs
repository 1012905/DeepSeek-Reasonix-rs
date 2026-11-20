//! Invoker and nested calls (tools.md §1.13).

use std::fmt;

use serde_json::Value;

use crate::abi::CallContext;

/// Runs a tool call on behalf of a tool that is itself running, through
/// everything an ordinary call passes: permission, hooks, evidence and
/// mutation observation. Only the agent that owns the turn offers one.
///
/// Nested-call depth is the caller's policy, not this trait's: the owning
/// agent refuses invocations past its configured ceiling (reference kernel:
/// `max_subagent_depth`, default 2, 0 = unlimited) before handing an
/// invoker out. Every invoker handed through [`CallContext::with_value`]
/// therefore sees at most its own allowance.
pub trait Invoker: Send + Sync {
    fn invoke(
        &self,
        ctx: &CallContext,
        name: &str,
        args: &Value,
    ) -> Result<String, NestedCallError>;
}

/// A nested call that ran and failed, or was refused; `output` is what the
/// model would have read alongside the failure.
#[derive(Debug)]
pub struct NestedCallError {
    pub tool: String,
    pub output: String,
}

impl fmt::Display for NestedCallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}: {}", NESTED_CALL_FAILED, self.tool, self.output)
    }
}

impl std::error::Error for NestedCallError {}

/// The identity every nested-call failure carries (Go `ErrNestedCallFailed`);
/// callers classify with a downcast, never by matching the sentence.
pub const NESTED_CALL_FAILED: &str = "tool call failed";

/// The id a nested call runs under: its caller's id, a dot, and its 1-based
/// sequence within that caller. A recorded nested call carries no other mark
/// of its caller, so readers of a recording recognise it by this id shape.
pub fn nested_call_id(parent_id: &str, seq: u64) -> String {
    format!("{parent_id}.{seq}")
}

/// Whether `id` is a [`nested_call_id`] made under `parent_id`. The suffix
/// must be a canonical 1-based decimal — `call_7.01` and `call_7.0` are not
/// nested calls of `call_7`, and neither is a grandchild `call_7.1.2`.
pub fn is_nested_call_of(id: &str, parent_id: &str) -> bool {
    if parent_id.is_empty() {
        return false;
    }
    let Some(seq) = id.strip_prefix(parent_id) else {
        return false;
    };
    let Some(seq) = seq.strip_prefix('.') else {
        return false;
    };
    match seq.parse::<u64>() {
        Ok(n) => n > 0 && n.to_string() == seq,
        Err(_) => false,
    }
}

//! Withdrawn tools (tools.md §1.11): a schema-only placeholder holding a
//! provider-schema slot open for a tool that is gone.

use std::any::Any;
use std::sync::Arc;

use serde_json::Value;

use crate::abi::{CallContext, CapabilitySet, Tool, ToolResult};

/// A slot placeholder: the provider-visible array is fixed for the session,
/// so vacating an entry rewrites the cached prefix and costs every token
/// behind it. The entry stays byte for byte and its call fails with the
/// reason. Wrapping strips the rest — an MCP binding, a preview — so
/// catalogs stop offering it as live.
pub struct WithdrawnTool {
    inner: Arc<dyn Tool>,
    reason: String,
}

impl WithdrawnTool {
    /// Returns `tool` as a schema-only placeholder that fails with `reason`.
    /// Withdrawing an already-withdrawn tool restates the reason rather than
    /// wrapping it twice: the inner tool is the original's, and the schema
    /// held open is still the original tool's.
    pub fn wrap(tool: Arc<dyn Tool>, reason: impl Into<String>) -> Arc<dyn Tool> {
        Arc::new(Self {
            inner: withdrawn_inner(&tool).unwrap_or(tool),
            reason: reason.into(),
        })
    }
}

/// The tool a withdrawn slot was built from, so a restatement re-wraps it
/// instead of nesting a slot inside a slot.
pub fn withdrawn_inner(tool: &Arc<dyn Tool>) -> Option<Arc<dyn Tool>> {
    tool.as_any()
        .downcast_ref::<WithdrawnTool>()
        .map(|w| w.inner.clone())
}

/// Whether `tool` is only holding a schema slot. Callers that route by what
/// a tool can still do read this rather than calling it to find out.
pub fn is_withdrawn(tool: &Arc<dyn Tool>) -> bool {
    tool.as_any().is::<WithdrawnTool>()
}

impl Tool for WithdrawnTool {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn description(&self) -> &str {
        self.inner.description()
    }

    fn schema(&self) -> Value {
        self.inner.schema()
    }

    fn execute(&self, _ctx: &CallContext, _args: &Value) -> ToolResult {
        Err(self.reason.clone().into())
    }

    /// True because the call only states its reason: approving a withdrawn
    /// tool would be approving an error message.
    fn read_only(&self) -> bool {
        true
    }

    /// A slot is not a capability: nothing routes through it any more.
    fn capabilities(&self) -> CapabilitySet {
        CapabilitySet::default()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

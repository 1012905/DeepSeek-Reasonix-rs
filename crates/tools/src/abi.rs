//! The `Tool` abstraction and its optional capability traits (tools.md §1.1).

use std::any::{Any, TypeId};
use std::sync::Arc;

use serde_json::Value;

use crate::refusal::Refusal;

/// A schema a registry exposes to the provider (Go `provider.ToolSchema`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    /// Canonical JSON Schema, serialized once at registration.
    pub parameters: Value,
}

/// A capability the model can invoke.
///
/// Implementations must be `Send + Sync`: a registry hands tools across
/// threads and `execute` runs under the caller's async runtime.
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    /// The JSON Schema for the tool's parameters.
    fn schema(&self) -> Value;
    /// Parses the model-generated args and returns result text for the model.
    fn execute(&self, ctx: &CallContext, args: &Value) -> ToolResult;
    /// No observable side effects. A batch parallelizes only when every call
    /// in it is read-only, so write/read order survives; bash and plugin
    /// tools answer false because their effects are unknowable from args.
    fn read_only(&self) -> bool;

    /// Optional capabilities this tool provides, probeable by downcast via
    /// [`CapabilitySet::get`]. Go discovers these by type assertion; the
    /// frozen semantics (batch reads only read_only, preview walks the same
    /// argument path as execute, contextual availability fails closed) live
    /// in the capability trait docs.
    fn capabilities(&self) -> CapabilitySet {
        CapabilitySet::default()
    }

    /// Type-erased self, so host code can recognize concrete wrappers (the
    /// withdrawn placeholder) the way Go type-asserts a Tool.
    fn as_any(&self) -> &dyn Any;
}

/// Execute outcome: model-visible text or the error the host reports.
pub type ToolResult = Result<String, Box<dyn std::error::Error + Send + Sync>>;

/// Probeable optional-capability set (Go: type assertions against a `Tool`).
///
/// Entries are keyed by the `Arc<C>` wrapper type so a lookup with
/// `caps.get::<dyn ContextualTool>()` downcasts the stored object back to
/// that trait object. Tools declare capabilities once at construction, so a
/// host gate and a proxy resolver read the same table instead of drifting.
#[derive(Default)]
pub struct CapabilitySet {
    entries: Vec<(&'static str, TypeId)>,
    objects: Vec<Box<dyn Any + Send + Sync>>,
}

impl CapabilitySet {
    /// Declares one capability. `id` is diagnostic only — lookup is by type.
    /// The Arc must already be trait-coerced (`Arc::new(x) as Arc<dyn Trait>`)
    /// so its TypeId matches what `get` asks for.
    pub fn with<C: ?Sized + Capability>(mut self, id: &'static str, cap: Arc<C>) -> Self {
        self.entries.push((id, TypeId::of::<Arc<C>>()));
        self.objects.push(Box::new(cap));
        self
    }

    /// Returns the capability of this kind, or None when the tool does not
    /// provide it.
    pub fn get<C: ?Sized + Capability>(&self) -> Option<Arc<C>> {
        let wanted = TypeId::of::<Arc<C>>();
        self.entries
            .iter()
            .position(|(_, id)| *id == wanted)
            .and_then(|i| self.objects[i].downcast_ref::<Arc<C>>().cloned())
    }

    /// Whether a capability with this diagnostic id is declared.
    pub fn has_id(&self, id: &str) -> bool {
        self.entries.iter().any(|(name, _)| *name == id)
    }
}

impl std::fmt::Debug for CapabilitySet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list()
            .entries(self.entries.iter().map(|(name, _)| name))
            .finish()
    }
}

/// Marker bound shared by every optional capability trait.
pub trait Capability: Send + Sync + 'static {}

// Supertrait-bound traits (ContextualTool etc.) satisfy `Capability`
// automatically through their declaration; only Capability itself needs no
// entry here because nothing downcasts to it directly.

/// Execution-time availability contract for tools whose presence depends on
/// the active workflow context. Provider schemas stay static for cache
/// stability; the host must consult this before permissions, hooks, leases,
/// or execute, so stale transcripts fail closed.
pub trait ContextualTool: Capability {
    fn provider_visible(&self, ctx: &CallContext) -> bool;
}

/// A [`ContextualTool`] that can say what would make it available again, as a
/// typed [`Refusal`]. Only the tool knows that — the same state
/// `provider_visible` reads — so a name-keyed table of excuses elsewhere
/// would be half the contract kept where the other half drifts from it.
pub trait ContextualReasoner: ContextualTool {
    fn unavailable(&self, ctx: &CallContext) -> Refusal;
}

/// Computes, from the same args execute would receive, the change the call
/// *would* make — without touching disk. Callers pass the execute context so
/// a preview resolves through the same overlay and a user never approves a
/// diff that differs from what runs.
pub trait Previewer: Capability {
    fn preview(&self, ctx: &CallContext, args: &Value) -> Result<PreviewChange, String>;
}

/// A writer change for display and preimage capture (Go `diff.Change`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PreviewChange {
    pub path: String,
    pub kind: String,
    pub old_text: String,
    pub new_text: String,
    pub added: i64,
    pub removed: i64,
    /// Unified diff; empty when binary.
    pub diff: String,
    pub binary: bool,
}

/// A tool whose calls must keep provider order inside one reply. Read-only
/// answers approval, not batching: the two part company for a tool that
/// touches no file yet advances host state, or reads what an earlier call in
/// the same reply started.
pub trait SequentialTool: Capability {
    fn sequential(&self, ctx: &CallContext, args: &Value) -> bool;
}

/// A tool whose effect is writing the paths its own arguments name. Bash and
/// opaque plugin tools mutate without naming anything, so they answer false
/// and their effects stay unattributed.
pub trait PathWriter: Capability {
    fn writes_named_paths(&self) -> bool;
}

/// A tool whose result is the user's own answer. Running it ends the round:
/// every call written alongside it was written without that answer, so
/// carrying one over would reason from a decision nobody made yet (Go
/// `tool.IsDecisionBarrier`).
pub trait DecisionBarrier: Capability {
    fn decision_barrier(&self) -> bool;
}

/// Whether running this tool ends its round.
pub fn is_decision_barrier(tool: &dyn Tool) -> bool {
    tool.capabilities()
        .get::<dyn DecisionBarrier>()
        .is_some_and(|b| b.decision_barrier())
}

/// A tool whose result is the user's own answer, decided per call rather
/// than for the whole tool (Go `DecisionCallBarrier`). The static
/// [`DecisionBarrier`] still wins: a barrier tool ends the round on every
/// call, whatever these arguments say.
pub trait DecisionCallBarrier: Capability {
    fn decision_barrier_call(&self, args: &Value) -> bool;
}

/// Whether this call ends its round: the static barrier first, then the
/// per-arguments decision (Go `IsDecisionBarrierCall`).
pub fn is_decision_barrier_call(tool: &dyn Tool, args: &Value) -> bool {
    if is_decision_barrier(tool) {
        return true;
    }
    tool.capabilities()
        .get::<dyn DecisionCallBarrier>()
        .is_some_and(|b| b.decision_barrier_call(args))
}

/// A tool that may run during the planning phase (Go `PlanModeSafe`).
/// Deliberately distinct from [`Tool::read_only`]: a side-effect-free tool
/// can still belong only to the post-approval execution phase. A missing
/// declaration continues the ordinary permissions/sandbox path.
pub trait PlanModeClassifier: Capability {
    fn plan_mode_safe(&self) -> bool;
}

/// Whether the tool declares itself safe to run while planning.
pub fn plan_mode_safe(tool: &dyn Tool) -> bool {
    tool.capabilities()
        .get::<dyn PlanModeClassifier>()
        .is_some_and(|p| p.plan_mode_safe())
}

/// A target that is logically read-only but must first mutate host state to
/// become executable (Go `ReadOnlyExecutionHostMutation`). Strict read-only
/// agents reject these even when the eventual remote operation is trusted
/// read-only.
pub trait ReadOnlyHostMutation: Capability {
    fn read_only_execution_host_mutation(&self) -> bool;
}

/// Whether the tool needs a host mutation before it can run.
pub fn read_only_executes_host_mutation(tool: &dyn Tool) -> bool {
    tool.capabilities()
        .get::<dyn ReadOnlyHostMutation>()
        .is_some_and(|m| m.read_only_execution_host_mutation())
}

/// Whether this call must run on its own. A tool without [`SequentialTool`]
/// is ordered by read-only alone.
pub fn runs_sequentially(tool: &dyn Tool, ctx: &CallContext, args: &Value) -> bool {
    tool.capabilities()
        .get::<dyn SequentialTool>()
        .is_some_and(|s| s.sequential(ctx, args))
}

/// Whether this tool's writes can be read off its arguments. `read_only ==
/// false` alone does not say so — bash is the standing counter-example.
pub fn writes_named_paths(tool: &dyn Tool) -> bool {
    tool.capabilities()
        .get::<dyn PathWriter>()
        .is_some_and(|w| w.writes_named_paths())
}

/// The change a call would make, for display and preimage capture. Read-only
/// tools preview nothing: there is no write to approve.
pub fn preview_change(
    tool: Option<&Arc<dyn Tool>>,
    ctx: &CallContext,
    args: &Value,
) -> Option<PreviewChange> {
    let tool = tool?;
    if tool.read_only() {
        return None;
    }
    let previewer = tool.capabilities().get::<dyn Previewer>()?;
    previewer.preview(ctx, args).ok()
}

/// Per-call context values Go carried in `context.Context`. Each binding is
/// optional; a missing binding means the caller runs headless or outside the
/// run loop, and tools fail closed exactly as they do against Go's absent
/// context value.
#[derive(Default)]
pub struct CallContext {
    inner: Option<Arc<dyn Any + Send + Sync>>,
    nested: bool,
}

impl CallContext {
    /// An empty context: no bindings, not nested.
    pub fn new() -> Self {
        Self::default()
    }

    /// Attaches one typed binding (invoker, elicitor, provenance sink, ...).
    /// Attachments replace each other; compose the values you need into one
    /// struct at the call site.
    pub fn with_value(mut self, value: Arc<dyn Any + Send + Sync>) -> Self {
        self.inner = Some(value);
        self
    }

    /// The attached binding, if any.
    pub fn value(&self) -> Option<&Arc<dyn Any + Send + Sync>> {
        self.inner.as_ref()
    }

    /// Marks the call as made by another tool rather than by the model.
    pub fn mark_nested(mut self) -> Self {
        self.nested = true;
        self
    }

    /// Whether this call was made by another tool.
    pub fn is_nested(&self) -> bool {
        self.nested
    }
}

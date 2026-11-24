//! The tool ABI: `Tool`, its optional capability traits, the per-run
//! `Registry`, and the typed refusal/blocked/error identities a host and its
//! tools exchange.
//!
//! Parity target: `internal/contract/tool` (docs/contracts/tools.md §1.1-§1.16).
//!
//! Go expresses an optional capability as a second interface a tool type
//! asserts to; here a tool additionally implements the capability traits and
//! exposes them through [`Tool::capabilities`], probed by downcast
//! (`CapabilitySet::get::<dyn Trait>()`). The frozen semantics — batch
//! parallelism reads only `read_only`, preview resolution walks the same
//! argument path as `execute`, contextual availability fails closed — live
//! in the trait docs; the shape is this crate's choice. Context values Go
//! carried in `context.Context` become a typed per-call [`CallContext`].

#![forbid(unsafe_code)]

pub mod abi;
pub mod arguments;
pub mod ask;
pub mod builtin;
pub mod invoke;
pub mod provenance;
pub mod refusal;
pub mod registry;
pub mod roots;
pub mod schema;
pub mod shell;
pub mod withdrawn;

pub use abi::{
    CallContext, CapabilitySet, ContextualReasoner, ContextualTool, DecisionBarrier,
    DecisionCallBarrier, PathWriter, PlanModeClassifier, PreviewChange, Previewer,
    ReadOnlyHostMutation, SequentialTool, Tool, ToolResult, ToolSchema, is_decision_barrier,
    is_decision_barrier_call, plan_mode_safe, preview_change, read_only_executes_host_mutation,
    runs_sequentially, writes_named_paths,
};
pub use arguments::{UniqueJsonError, parse_unique, unwrap_arguments};
pub use ask::{AskAnswer, AskOutcome, Asker, asker_from};
pub use invoke::{Invoker, NESTED_CALL_FAILED, NestedCallError, is_nested_call_of, nested_call_id};
pub use provenance::{
    Provenance, ProvenanceDeclarer, ProvenanceKind, ProvenanceSink, host_of, provenance_header,
    provenance_label, provenance_of,
};
pub use refusal::{
    BlockedError, CANCELLED_BY_USER, Refusal, Remedy, RemedyOp, blocked, blocked_message,
    remedy_string,
};
pub use registry::{
    APPROVAL_SCOPE_OPEN_PROJECTS, ApprovalScoper, CODE_MCP_TOOL_DISABLED, CODE_MCP_TOOL_HELD,
    CODE_TOOL_DISABLED, CapabilityAliased, ContractEntry, DisableConflict, FEATURE_DEFERRED_MCP,
    FEATURE_GOAL_MODE, FEATURE_PLAN_STEPS, FEATURE_TURN_EXIT, HeldMcp, HeldMcpClass, HostRequired,
    MCP_NAME_PREFIX, McpBinding, McpMetadata, Registry, ResolveCall, approval_scope_of,
    feature_label, held_mcp_refusal_for, is_connection_name, mcp_binding_aliases, mcp_binding_for,
    portable_mcp_part, split_mcp_name,
};
pub use schema::canonicalize_schema;
pub use shell::{
    DetailedExecutor, DetailedResult, MutationRisk, NO_MATCHES, OUTPUT_TAIL_MAX_BYTES,
    ShellExecution, ShellKind, ShellName, ShellPhase, ShellState, ShellVersion, Verification,
    duration_ms, is_shell_tool,
};
pub use withdrawn::{WithdrawnTool, is_withdrawn};

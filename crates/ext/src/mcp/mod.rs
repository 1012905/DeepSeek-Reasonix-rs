//! MCP client wire protocol and lifecycle.
//!
//! Parity target: `internal/ext/plugin` (MCP contract §0-3). This slice owns
//! the JSON-RPC wire, era negotiation, tool naming, and lifecycle state;
//! subprocess and network transports arrive as a later slice.

pub mod lifecycle;
pub mod naming;
pub mod types;
pub mod wire;

pub use lifecycle::{
    DEFAULT_CALL_TIMEOUT_MS, DEFAULT_START_CONCURRENCY, DEFAULT_START_TIMEOUT_MS, Host,
    LifecycleError, McpProcessMode, Removal, START_PHASES, Spec, StartPhase, StartPlan,
    StartPlanEntry, StartPolicy, StartupFailure, TransportKind, build_start_plan,
    canonical_mcp_runtime_transport, check_deferred_generation, effective_concurrency,
    new_startup_failure, parse_transport_type, spawn_key,
};
pub use naming::{
    MAX_MODEL_TOOL_NAME, NamingError, bounded_tool_name, mcp_connect_permission_name, mcp_roots,
    mcp_roots_from_absolute, model_tool_name, normalize_name, short_name_hash, tool_name,
    tool_prefix, validate_mcp_tool_names,
};
pub use types::{ContentBlock, McpRoot, McpTool, ToolCallResult};
pub use wire::{
    InboundMessage, RpcError, RpcRequest, RpcResponse, ServerReply, ServerResult,
    decode_inbound_message, decode_rpc_response, format_mcp_progress, is_notification_id,
    progress_token_key, rpc_error_from_response, server_request_reply,
};

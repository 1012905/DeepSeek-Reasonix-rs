//! MCP client wire protocol and lifecycle.
//!
//! Parity target: `internal/ext/plugin` (MCP contract §0-3). This slice owns
//! the JSON-RPC wire, era negotiation, tool naming, and lifecycle state;
//! subprocess and network transports arrive as a later slice.

pub mod cache;
pub mod http;
mod http_connector;
pub mod http_headers;
pub mod http_transport;
pub mod lifecycle;
pub mod naming;
pub mod reconnect;
pub mod seam;
pub mod session;
pub mod sse;
pub mod stdio;
pub mod stdio_resolve;
pub mod types;
pub mod version;
pub mod wire;

pub use cache::{
    CachedSchema, CachedTool, legacy_schema_cache_key, load_cached_schema, load_cached_schema_any,
    load_cached_schema_for_spec, save_cached_schema, schema_cache_key, slug,
};
pub use http::{
    EventStream, HttpClient, HttpClientConfig, HttpError, HttpReply, SseEvent, same_origin,
};
pub use http_headers::{
    HeaderAnnotationError, ParamHeader, encode_header_value, header_scalar, modern_request_headers,
    param_headers_for_args, tool_param_headers,
};
pub use http_transport::{StreamableHttpConfig, StreamableHttpTransport};
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
pub use reconnect::{CLOSE_WAIT_BUDGET_MS, DEFAULT_DIAL_BUDGET_MS, ReconnectingTransport};
pub use seam::{Dialer, OwnedTransport, Transport, TransportError};
pub use session::{
    CODE_SESSION_EXPIRED, FreshSessionProbe, ProgressSink, ProgressTransport, ProtocolVersioned,
    RedialGate, Session, SessionError, SessionExpiredProbe, declares_tool_list_changed,
    format_timeout, raw_tool_name, resolve_call_timeout,
};
pub use sse::{LegacySseConfig, LegacySseTransport, SSE_REPLY_QUEUE_BOUND};
pub use types::{ContentBlock, McpRoot, McpTool, ToolCallResult};
pub use version::{
    CODE_HEADER_MISMATCH, CODE_MISSING_CLIENT_CAPABILITY, CODE_UNSUPPORTED_PROTOCOL_VERSION,
    DISCOVER_METHOD, ERA_CODES, Era, EraPlan, MAX_INPUT_REQUESTS, MAX_INPUT_ROUNDS,
    MAX_REQUEST_STATE_SZ, META_CLIENT_CAPABILITIES, META_CLIENT_INFO, META_PROTOCOL_VERSION,
    MODERN_PROBE_WAIT_MS, MODERN_PROTOCOL_VERSION, MODERN_VERSIONS, OLDEST_PROTOCOL_VERSION,
    PROTOCOL_VERSION, ProbeReply, SUPPORTED_PROTOCOL_VERSIONS, TASKS_EXTENSION, VersionError,
    check_input_bounds, classify_probe_result, client_capabilities, client_meta, connect_era,
    discover_result, initialize_params, negotiated_version, pick_era, with_meta,
};
pub use wire::{
    InboundMessage, RpcError, RpcRequest, RpcResponse, ServerReply, ServerResult,
    decode_inbound_message, decode_rpc_response, format_mcp_progress, is_notification_id,
    progress_token_key, rpc_error_from_response, server_request_reply,
};

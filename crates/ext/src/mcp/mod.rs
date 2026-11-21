//! MCP client wire protocol and lifecycle.
//!
//! Parity target: `internal/ext/plugin` (MCP contract §0-3). This slice owns
//! the JSON-RPC wire, era negotiation, tool naming, and lifecycle state;
//! subprocess and network transports arrive as a later slice.

pub mod naming;
pub mod types;

pub use naming::{
    MAX_MODEL_TOOL_NAME, NamingError, bounded_tool_name, mcp_connect_permission_name, mcp_roots,
    mcp_roots_from_absolute, model_tool_name, normalize_name, short_name_hash, tool_name,
    tool_prefix, validate_mcp_tool_names,
};
pub use types::{ContentBlock, McpRoot, McpTool, ToolCallResult};

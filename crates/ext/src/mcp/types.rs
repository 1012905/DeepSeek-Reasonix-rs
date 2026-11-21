//! Wire-adjacent MCP shapes: tool definitions, call results, content blocks.
//!
//! Only the shapes needed by protocol negotiation and tool naming live here;
//! actual tool execution is a later slice.

use alloc::string::String;
use alloc::vec::Vec;

use serde::Serialize;
use serde_json::Value;

/// A tool as returned by `tools/list`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct McpTool {
    pub name: String,
    pub title: Option<String>,
    pub description: String,
    pub input_schema: Option<Value>,
    pub output_schema: Option<Value>,
    /// Raw annotation object; never interpreted as instructions.
    pub annotations: Option<Value>,
}

impl McpTool {
    /// Reads the two hints that drive safety classification. A malformed object
    /// declares neither; destructive wins over a conflicting read-only hint.
    pub fn hints(&self) -> (bool, bool) {
        let read_only = self
            .annotations
            .as_ref()
            .and_then(|a| a.get("readOnlyHint"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let destructive = self
            .annotations
            .as_ref()
            .and_then(|a| a.get("destructiveHint"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        (read_only, destructive)
    }
}

/// A content block in a `tools/call` result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContentBlock {
    Text { text: String },
    Image { data: String, mime_type: String },
    Resource { uri: String, text: Option<String> },
}

/// Model-visible projection of a server's roots entry. Field order and the
/// omitted empty name mirror Go's `mcpRoot` struct (`uri`, `name,omitempty`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct McpRoot {
    pub uri: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub name: String,
}

/// Result of a `tools/call`: content blocks plus the optional isError flag.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ToolCallResult {
    pub content: Vec<ContentBlock>,
    pub is_error: bool,
}

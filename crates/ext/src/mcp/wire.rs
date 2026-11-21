//! JSON-RPC 2.0 envelopes and inbound frame handling.
//!
//! Oracle: `internal/ext/plugin/plugin.go:1526-1540`, `protocol_errors.go`,
//! `protocol_inbound.go`. Field order is frozen by Go's struct order and
//! `encoding/json`; the absent `id`/`params` are omitted, and a response
//! carries both `result` and `error` (null on the absent side).

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::types::McpRoot;

fn default_jsonrpc() -> String {
    "2.0".to_string()
}

/// Typed JSON-RPC error: `{code, message, data?}`. `Display` reports the code
/// and byte length only — message text is deliberately never rendered
/// (Go `rpcError.Error`, REASONIX error-identity rule).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RpcError {
    pub code: i64,
    #[serde(default)]
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl RpcError {
    pub fn new(code: i64, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }

    pub fn with_data(code: i64, message: impl Into<String>, data: Value) -> Self {
        Self {
            code,
            message: message.into(),
            data: Some(data),
        }
    }

    /// Message text is deliberately excluded, matching Go's diagnostic policy.
    pub fn diagnostic(&self) -> String {
        alloc::format!(
            "rpc error {} (message omitted; {} bytes)",
            self.code,
            self.message.len()
        )
    }
}

impl fmt::Display for RpcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.diagnostic())
    }
}

impl std::error::Error for RpcError {}

/// Outgoing request. Field order: `jsonrpc`, `id`, `method`, `params`.
/// `id` is omitted for notifications and when zero (Go `id,omitempty`; id 0
/// is unused); `params` is omitted when absent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RpcRequest {
    #[serde(default = "default_jsonrpc")]
    pub jsonrpc: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<i64>,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

impl RpcRequest {
    pub fn new(id: i64, method: impl Into<String>, params: Option<Value>) -> Self {
        Self {
            jsonrpc: default_jsonrpc(),
            id: Some(id),
            method: method.into(),
            params,
        }
    }

    /// Notification: no id field on the wire.
    pub fn notification(method: impl Into<String>, params: Option<Value>) -> Self {
        Self {
            jsonrpc: default_jsonrpc(),
            id: None,
            method: method.into(),
            params,
        }
    }

    /// NDJSON frame bytes, one trailing newline like Go's encoder path.
    pub fn to_wire(&self) -> Vec<u8> {
        let mut bytes = serde_json::to_vec(self).unwrap_or_default();
        bytes.push(b'\n');
        bytes
    }
}

/// Incoming response. Go's response struct has no `omitempty`: both `result`
/// and `error` are present on the wire, null on the absent side.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RpcResponse {
    #[serde(default = "default_jsonrpc")]
    pub jsonrpc: String,
    #[serde(default)]
    pub id: Value,
    #[serde(default)]
    pub result: Value,
    #[serde(default)]
    pub error: Option<RpcError>,
}

impl RpcResponse {
    pub fn ok(id: Value, result: Value) -> Self {
        Self {
            jsonrpc: default_jsonrpc(),
            id,
            result,
            error: None,
        }
    }

    pub fn fail(id: Value, error: RpcError) -> Self {
        Self {
            jsonrpc: default_jsonrpc(),
            id,
            result: Value::Null,
            error: Some(error),
        }
    }
}

/// Any inbound frame: a response, a request, or a notification. Go's decoder
/// accepts any JSON object; absent fields keep their zero values.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InboundMessage {
    #[serde(default)]
    pub jsonrpc: String,
    #[serde(default)]
    pub id: Option<Value>,
    #[serde(default)]
    pub method: String,
    #[serde(default)]
    pub params: Option<Value>,
}

/// Decodes an inbound frame; anything that is not a JSON object is rejected.
pub fn decode_inbound_message(payload: &[u8]) -> Option<InboundMessage> {
    serde_json::from_slice(payload).ok()
}

/// Empty id or literal `null` marks a notification.
pub fn is_notification_id(id: &Value) -> bool {
    match id {
        Value::Null => true,
        Value::String(s) => s.is_empty(),
        _ => false,
    }
}

/// Normalizes a progress token (string or number) to a map key.
pub fn progress_token_key(token: &Value) -> String {
    match token {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}

/// Formats a progress notification: `label (p/t)\n`, `label (p)\n`, `label\n`.
pub fn format_mcp_progress(message: &str, progress: Option<f64>, total: Option<f64>) -> String {
    let trimmed = message.trim();
    let label = if trimmed.is_empty() {
        "MCP progress"
    } else {
        trimmed
    };
    match (progress, total) {
        (Some(p), Some(t)) => alloc::format!("{label} ({p}/{t})\n"),
        (Some(p), None) => alloc::format!("{label} ({p})\n"),
        (None, _) => alloc::format!("{label}\n"),
    }
}

/// Typed payload of a server-request reply. Serialized manually so field
/// order inside `roots` entries follows Go's `mcpRoot` struct (`uri`, `name`),
/// never a sorted map.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServerResult {
    /// `ping` answers an empty object.
    Ping,
    /// `roots/list` answers the workspace roots.
    Roots(Vec<McpRoot>),
}

impl Serialize for ServerResult {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        match self {
            Self::Ping => serializer.serialize_map(Some(0))?.end(),
            Self::Roots(roots) => {
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry("roots", roots)?;
                map.end()
            }
        }
    }
}

/// Typed reply to a server-originated request (`serverRequestReply`): fields
/// `jsonrpc,id,result,error` with `result`/`error` omitted when absent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ServerReply {
    pub jsonrpc: String,
    pub id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<ServerResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

impl ServerReply {
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
}

/// Answers `ping` and `roots/list`; any other method is -32601.
pub fn server_request_reply(id: Value, method: &str, roots: Vec<McpRoot>) -> ServerReply {
    match method {
        "ping" => ServerReply {
            jsonrpc: default_jsonrpc(),
            id,
            result: Some(ServerResult::Ping),
            error: None,
        },
        "roots/list" => ServerReply {
            jsonrpc: default_jsonrpc(),
            id,
            result: Some(ServerResult::Roots(roots)),
            error: None,
        },
        _ => ServerReply {
            jsonrpc: default_jsonrpc(),
            id,
            result: None,
            error: Some(RpcError::new(-32601, "Method not found")),
        },
    }
}

/// Extracts a typed RPC error from a response, if present.
pub fn rpc_error_from_response(response: &RpcResponse) -> Option<RpcError> {
    response.error.clone()
}

/// Parses a response envelope from bytes. Encoding freezes field order;
/// decoding accepts either order, as Go's decoder does.
pub fn decode_rpc_response(payload: &[u8]) -> Option<RpcResponse> {
    let response: RpcResponse = serde_json::from_slice(payload).ok()?;
    if response.jsonrpc != "2.0" {
        return None;
    }
    Some(response)
}

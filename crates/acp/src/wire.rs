//! Inbound wire DTOs: `session/new` … `session/delete` and the shared content,
//! model, mode, and config-option shapes (protocol.go:199-476).
//!
//! The reference does no reflective required-field validation: missing strings
//! decode to `""` and handlers check them by hand. These structs mirror that
//! tolerance; protocol-level validation helpers live in [`crate::lifecycle`].

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::errors::{RpcError, RpcErrorCode};

// ---- Content blocks ---------------------------------------------------------

/// One piece of a prompt: text, image, audio, or embedded resource
/// (protocol.go:454-460).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ContentBlock {
    #[serde(rename = "type")]
    pub block_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource: Option<ResourceContents>,
    #[serde(rename = "mimeType", skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<String>,
}

/// The embedded resource of a resource content block (protocol.go:463-468).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ResourceContents {
    pub uri: String,
    #[serde(rename = "mimeType", skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blob: Option<String>,
}

// ---- MCP server specs -------------------------------------------------------

/// One official ACP `{name, value}` entry; reused for env and headers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct EnvVariable {
    pub name: String,
    pub value: String,
}

fn deserialize_name_value_map(
    value: &Value,
    field: &str,
) -> Result<BTreeMap<String, String>, String> {
    match value {
        Value::Null | Value::Object(_) if value.as_object().is_some_and(|o| o.is_empty()) => {
            Ok(BTreeMap::new())
        }
        Value::Array(items) => {
            let mut out = BTreeMap::new();
            for (i, item) in items.iter().enumerate() {
                let entry: EnvVariable = serde_json::from_value(item.clone())
                    .map_err(|_| format!("{field} must be an array of {{name,value}} objects"))?;
                if entry.name.trim().is_empty() {
                    return Err(format!("{field}[{i}].name is required"));
                }
                out.insert(entry.name, entry.value);
            }
            Ok(out)
        }
        Value::Object(map) => map
            .iter()
            .map(|(k, v)| {
                let v = v
                    .as_str()
                    .ok_or_else(|| format!("{field} must be an array of {{name,value}} objects"))?;
                Ok((k.clone(), v.to_owned()))
            })
            .collect(),
        Value::Null => Ok(BTreeMap::new()),
        _ => Err(format!(
            "{field} must be an array of {{name,value}} objects"
        )),
    }
}

/// MCP server environment variables, accepting the official
/// `[{name,value}]` array and the legacy `{k: v}` object (protocol.go:218,
/// 234-241).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
pub struct McpEnv(pub BTreeMap<String, String>);

impl<'de> Deserialize<'de> for McpEnv {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        let map = deserialize_name_value_map(&value, "env").map_err(serde::de::Error::custom)?;
        Ok(McpEnv(map))
    }
}

/// MCP server HTTP headers, same dual shape as [`McpEnv`] (protocol.go:224,
/// 243-250).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
pub struct McpHeaders(pub BTreeMap<String, String>);

impl<'de> Deserialize<'de> for McpHeaders {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        let map =
            deserialize_name_value_map(&value, "headers").map_err(serde::de::Error::custom)?;
        Ok(McpHeaders(map))
    }
}

/// One MCP server the client asks the agent to connect (protocol.go:206-214).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct McpServerSpec {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub args: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env: Option<McpEnv>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub headers: Option<McpHeaders>,
}

/// Transports the spec accepts after lowercasing/trim; an empty type defaults
/// to stdio (service.go:1501-1521).
const HTTP_TRANSPORTS: [&str; 4] = ["http", "streamable-http", "streamable_http", "sse"];

/// Validates one MCP spec, returning the kernel error message the service wraps
/// with its method prefix (service.go:1501-1521).
pub fn validate_mcp_server(spec: &McpServerSpec) -> Result<(), String> {
    let name = spec.name.trim();
    if name.is_empty() {
        return Err("MCP server name is required".to_owned());
    }
    let raw_type = spec.r#type.as_deref().unwrap_or("");
    let normalized = raw_type.trim().to_ascii_lowercase();
    let transport = if normalized.is_empty() {
        "stdio"
    } else {
        normalized.as_str()
    };
    match transport {
        "stdio" => {
            if spec.command.as_deref().unwrap_or("").trim().is_empty() {
                return Err(format!("MCP server {name:?} command is required"));
            }
        }
        t if HTTP_TRANSPORTS.contains(&t) => {
            if spec.url.as_deref().unwrap_or("").trim().is_empty() {
                return Err(format!("MCP server {name:?} url is required"));
            }
        }
        _ => {
            return Err(format!(
                "MCP server {name:?} uses unsupported transport {raw_type:?}"
            ));
        }
    }
    Ok(())
}

/// Loose absolute-path check covering POSIX roots, Windows drive letters, and
/// UNC paths; the session runtime cleans the resolved path itself.
pub fn is_absolute_path(cwd: &str) -> bool {
    let trimmed = cwd.trim();
    if trimmed.starts_with('/') || trimmed.starts_with("\\\\") {
        return true;
    }
    let bytes = trimmed.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

/// Rejects a non-empty relative cwd with the per-method invalid-params message
/// (service.go:1153-1155). An empty cwd inherits the process working directory.
pub fn require_absolute_cwd(method: &str, cwd: &str) -> Result<(), RpcError> {
    if cwd.trim().is_empty() || is_absolute_path(cwd) {
        Ok(())
    } else {
        Err(RpcError::invalid_params(format!(
            "{method}: cwd must be an absolute path"
        )))
    }
}

// ---- Vendor inbox / reload / goal DTOs ---------------------------------------

/// Enqueue a follow-up or steer intent into the durable inbox
/// (protocol.go:507-513).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SessionInboxEnqueueParams {
    #[serde(rename = "sessionId")]
    pub session_id: String,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub intent: Option<String>,
    #[serde(rename = "idempotencyKey", skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
}

/// Identifies one inbox item (protocol.go:515-519).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SessionInboxItemParams {
    #[serde(rename = "sessionId")]
    pub session_id: String,
    #[serde(rename = "itemId")]
    pub item_id: String,
}

/// Rewrites an inbox item body (protocol.go:521-526).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SessionInboxUpdateParams {
    #[serde(rename = "sessionId")]
    pub session_id: String,
    #[serde(rename = "itemId")]
    pub item_id: String,
    pub text: String,
}

/// Reorders an inbox item; `toIndex` is 0-based (protocol.go:528-533).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SessionInboxMoveParams {
    #[serde(rename = "sessionId")]
    pub session_id: String,
    #[serde(rename = "itemId")]
    pub item_id: String,
    #[serde(rename = "toIndex")]
    pub to_index: i32,
}

/// Toggles inbox pause (protocol.go:535-539).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SessionInboxPauseParams {
    #[serde(rename = "sessionId")]
    pub session_id: String,
    pub paused: bool,
}

/// Enqueue receipt (inbox.go:133-139).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SessionInboxEnqueueResult {
    #[serde(rename = "itemId")]
    pub item_id: String,
    pub disposition: String,
    pub position: i32,
    pub paused: bool,
    pub idempotent: bool,
}

/// One inbox item read back (inbox.go:169).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct SessionInboxGetResult {
    pub meta: Value,
    pub envelope: Value,
}

/// Ack for delete / move / retry / refresh (inbox.go:189-262).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SessionInboxOkResult {
    pub ok: bool,
}

/// Pause toggle ack (inbox.go:232).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SessionInboxPausedResult {
    pub paused: bool,
}

/// Addresses one live session for the reload method (protocol.go:541-544).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SessionReloadExtensionsParams {
    #[serde(rename = "sessionId")]
    pub session_id: String,
}

/// Reports whether the reload ran immediately or was coalesced behind an
/// in-flight turn (protocol.go:546-551).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SessionReloadExtensionsResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub queued: Option<bool>,
}

/// Goal resume / pause params (session_goal.go:28-31).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SessionGoalParams {
    #[serde(rename = "sessionId")]
    pub session_id: String,
}

/// Goal snapshot right after the change; the goal body is opaque at this
/// boundary (session_goal.go:33-37).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct SessionGoalResult {
    pub goal: Value,
}

/// `-32603` turn failure admitted but later failing; `data.code = turn_failed`
/// (protocol.go:37-44, session.go:407-417). The 2048-byte message clip is the
/// runtime's job.
pub fn turn_failed_error(message: impl Into<String>) -> RpcError {
    RpcError::new(RpcErrorCode::Internal, message)
        .with_data(serde_json::json!({"code": crate::errors::DATA_CODE_TURN_FAILED}))
}

// ---- Model / mode / config state reused by session results ------------------

/// One selectable legacy model (protocol.go:312-316).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ModelInfo {
    #[serde(rename = "modelId")]
    pub model_id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Legacy model selector state (protocol.go:320-323).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SessionModelState {
    #[serde(rename = "availableModels")]
    pub available_models: Vec<ModelInfo>,
    #[serde(rename = "currentModelId")]
    pub current_model_id: String,
}

/// One operating mode (protocol.go:290-294).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SessionMode {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Current mode and full mode list (protocol.go:297-300).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SessionModeState {
    #[serde(rename = "currentModeId")]
    pub current_mode_id: String,
    #[serde(rename = "availableModes")]
    pub available_modes: Vec<SessionMode>,
}

/// One selectable value for a config option (protocol.go:389-393).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SessionConfigSelectOption {
    pub value: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// One single-value session selector (protocol.go:378-386).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SessionConfigOption {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    pub r#type: String,
    #[serde(rename = "currentValue")]
    pub current_value: String,
    pub options: Vec<SessionConfigSelectOption>,
}

// ---- session/new ------------------------------------------------------------

/// Params opening a session rooted at cwd (protocol.go:199-203).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SessionNewParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(
        rename = "additionalDirectories",
        skip_serializing_if = "Option::is_none"
    )]
    pub additional_directories: Option<Vec<String>>,
    #[serde(rename = "mcpServers", skip_serializing_if = "Option::is_none")]
    pub mcp_servers: Option<Vec<McpServerSpec>>,
}

/// Result returning the opaque session id (protocol.go:280-285).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SessionNewResult {
    #[serde(rename = "sessionId")]
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub models: Option<SessionModelState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modes: Option<SessionModeState>,
    #[serde(rename = "configOptions", skip_serializing_if = "Option::is_none")]
    pub config_options: Option<Vec<SessionConfigOption>>,
}

// ---- session/load, session/resume -------------------------------------------

/// Shared params for load/resume (protocol.go:331-336, 349-354).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SessionOpenParams {
    #[serde(rename = "sessionId")]
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(
        rename = "additionalDirectories",
        skip_serializing_if = "Option::is_none"
    )]
    pub additional_directories: Option<Vec<String>>,
    #[serde(rename = "mcpServers", skip_serializing_if = "Option::is_none")]
    pub mcp_servers: Option<Vec<McpServerSpec>>,
}

/// Shared load/resume result: refreshed model/mode/config state, no sessionId
/// (protocol.go:340-344, 357-361).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SessionOpenResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub models: Option<SessionModelState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modes: Option<SessionModeState>,
    #[serde(rename = "configOptions", skip_serializing_if = "Option::is_none")]
    pub config_options: Option<Vec<SessionConfigOption>>,
}

// ---- Controls ----------------------------------------------------------------

/// Switch the operating mode (protocol.go:303-306).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SessionSetModeParams {
    #[serde(rename = "sessionId")]
    pub session_id: String,
    #[serde(rename = "modeId")]
    pub mode_id: String,
}

/// Legacy model switch (protocol.go:398-401).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SetSessionModelParams {
    #[serde(rename = "sessionId")]
    pub session_id: String,
    #[serde(rename = "modelId")]
    pub model_id: String,
}

/// Change one advertised config option (protocol.go:366-370).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SetSessionConfigOptionParams {
    #[serde(rename = "sessionId")]
    pub session_id: String,
    #[serde(rename = "configId")]
    pub config_id: String,
    pub value: String,
}

/// Full refreshed config state (protocol.go:373-375).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SetSessionConfigOptionResult {
    #[serde(rename = "configOptions")]
    pub config_options: Vec<SessionConfigOption>,
}

// ---- session/list ------------------------------------------------------------

/// List filter params (protocol.go:409-412).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SessionListParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

/// One list item (protocol.go:422-428).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct SessionInfo {
    #[serde(rename = "sessionId")]
    pub session_id: String,
    pub cwd: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(rename = "updatedAt", skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    #[serde(rename = "_meta", skip_serializing_if = "Option::is_none")]
    pub meta: Option<BTreeMap<String, Value>>,
}

/// First and only list page; `nextCursor` is never emitted in-process
/// (protocol.go:416-419).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct SessionListResult {
    pub sessions: Vec<SessionInfo>,
    #[serde(rename = "nextCursor", skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

// ---- close / delete ----------------------------------------------------------

/// Params addressing one session for close/delete (protocol.go:433-435,
/// 443-445).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SessionIdParams {
    #[serde(rename = "sessionId")]
    pub session_id: String,
}

/// Empty close ack.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SessionCloseResult {}

/// Empty delete ack.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SessionDeleteResult {}

// ---- prompt / steer -----------------------------------------------------------

/// Send a turn's prompt to a session (protocol.go:473-476).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SessionPromptParams {
    #[serde(rename = "sessionId")]
    pub session_id: String,
    pub prompt: Vec<ContentBlock>,
}

/// Vendor steer params (protocol.go:481-483).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SessionSteerParams {
    #[serde(rename = "sessionId")]
    pub session_id: String,
    pub prompt: Vec<ContentBlock>,
}

/// Steer admission ack (protocol.go:486-489).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SessionSteerResult {
    #[serde(rename = "itemId", skip_serializing_if = "Option::is_none")]
    pub item_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disposition: Option<String>,
}

/// Steer applied to the active prompt.
pub const STEER_DISPOSITION_ACCEPTED: &str = "steer_accepted";
/// Steer queued as a passive follow-up.
pub const STEER_DISPOSITION_QUEUED: &str = "queued_followup";

/// Why a turn ended (protocol.go:560-565). The reference only ever emits
/// `end_turn` / `cancelled`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum StopReason {
    #[default]
    #[serde(rename = "end_turn")]
    EndTurn,
    #[serde(rename = "cancelled")]
    Cancelled,
}

/// `session/prompt` result (protocol.go:569-572).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct SessionPromptResult {
    #[serde(rename = "stopReason")]
    pub stop_reason: StopReason,
    #[serde(rename = "_meta", skip_serializing_if = "Option::is_none")]
    pub meta: Option<BTreeMap<String, Value>>,
}

/// `session/cancel` notification params (protocol.go:739-741).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SessionCancelParams {
    #[serde(rename = "sessionId")]
    pub session_id: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcp_env_accepts_official_array_and_legacy_map() {
        let array: McpEnv = serde_json::from_value(serde_json::json!([
            {"name": "HOME", "value": "/tmp"},
            {"name": "EMPTY", "value": ""}
        ]))
        .unwrap();
        assert_eq!(array.0["HOME"], "/tmp");
        assert_eq!(array.0["EMPTY"], "");
        assert_eq!(array.0.len(), 2);

        let spec: McpServerSpec = serde_json::from_value(serde_json::json!({
            "name": "fs", "command": "mcp-fs",
            "env": [{"name": "HOME", "value": "/tmp"}]
        }))
        .unwrap();
        assert_eq!(spec.env.unwrap().0["HOME"], "/tmp");

        let legacy: McpServerSpec = serde_json::from_value(serde_json::json!({
            "name": "remote", "type": "http", "url": "https://x.test",
            "headers": {"Authorization": "Bearer token"}
        }))
        .unwrap();
        assert_eq!(legacy.headers.unwrap().0["Authorization"], "Bearer token");
    }

    #[test]
    fn mcp_env_rejects_blank_name_and_neither_shape() {
        let blank = serde_json::from_value::<McpServerSpec>(serde_json::json!({
            "name": "s", "env": [{"name": "  ", "value": "v"}]
        }));
        assert!(
            blank
                .unwrap_err()
                .to_string()
                .contains("env[0].name is required")
        );

        let neither = serde_json::from_value::<McpServerSpec>(serde_json::json!({
            "name": "s", "env": 42
        }));
        assert!(
            neither
                .unwrap_err()
                .to_string()
                .contains("env must be an array of {name,value} objects")
        );

        let headers = serde_json::from_value::<McpServerSpec>(serde_json::json!({
            "name": "s", "headers": 42
        }));
        assert!(
            headers
                .unwrap_err()
                .to_string()
                .contains("headers must be an array of {name,value} objects")
        );
    }

    #[test]
    fn mcp_spec_validation_follows_the_kernel_messages() {
        let unnamed = McpServerSpec::default();
        assert_eq!(
            validate_mcp_server(&unnamed).unwrap_err(),
            "MCP server name is required"
        );

        let stdio_no_cmd = McpServerSpec {
            name: "s".to_owned(),
            r#type: None,
            command: None,
            ..Default::default()
        };
        assert_eq!(
            validate_mcp_server(&stdio_no_cmd).unwrap_err(),
            "MCP server \"s\" command is required"
        );

        let http_no_url = McpServerSpec {
            name: "r".to_owned(),
            r#type: Some("http".to_owned()),
            url: None,
            ..Default::default()
        };
        assert_eq!(
            validate_mcp_server(&http_no_url).unwrap_err(),
            "MCP server \"r\" url is required"
        );

        let bad_transport = McpServerSpec {
            name: "x".to_owned(),
            r#type: Some("ftp".to_owned()),
            ..Default::default()
        };
        assert_eq!(
            validate_mcp_server(&bad_transport).unwrap_err(),
            "MCP server \"x\" uses unsupported transport \"ftp\""
        );

        let ok = McpServerSpec {
            name: "fs".to_owned(),
            command: Some("mcp-fs".to_owned()),
            ..Default::default()
        };
        assert!(validate_mcp_server(&ok).is_ok());
    }

    #[test]
    fn cwd_must_be_absolute_or_empty() {
        assert!(require_absolute_cwd("session/new", "").is_ok());
        assert!(require_absolute_cwd("session/new", "/proj").is_ok());
        #[cfg(windows)]
        assert!(require_absolute_cwd("session/new", "C:\\proj").is_ok());
        let rel = require_absolute_cwd("session/new", "relative/proj").unwrap_err();
        assert_eq!(rel.message, "session/new: cwd must be an absolute path");
    }

    #[test]
    fn session_new_minimal_json_decodes_and_round_trips() {
        let params: SessionNewParams = serde_json::from_value(serde_json::json!({})).unwrap();
        assert!(params.cwd.is_none());
        assert!(params.mcp_servers.is_none());

        let result = SessionNewResult {
            session_id: "123e4567-e89b-42d3-a562-426614174000".to_owned(),
            models: None,
            modes: None,
            config_options: None,
        };
        let value = serde_json::to_value(&result).unwrap();
        assert_eq!(value["sessionId"], result.session_id);
        assert!(value.get("models").is_none());
        assert!(value.get("modes").is_none());
        assert!(value.get("configOptions").is_none());
    }

    #[test]
    fn vendor_inbox_reload_goal_dtos_round_trip() {
        let enqueue: SessionInboxEnqueueParams = serde_json::from_value(serde_json::json!({
            "sessionId": "s1", "text": "follow up", "intent": "steer",
            "idempotencyKey": "k1"
        }))
        .unwrap();
        assert_eq!(enqueue.intent.as_deref(), Some("steer"));
        let receipt = SessionInboxEnqueueResult {
            item_id: "i1".to_owned(),
            disposition: STEER_DISPOSITION_ACCEPTED.to_owned(),
            position: 0,
            paused: false,
            idempotent: false,
        };
        let v = serde_json::to_value(&receipt).unwrap();
        assert_eq!(v["itemId"], "i1");
        assert_eq!(v["disposition"], "steer_accepted");

        let get = SessionInboxGetResult {
            meta: serde_json::json!({"id": "i1"}),
            envelope: serde_json::json!({"text": "x"}),
        };
        let back: SessionInboxGetResult =
            serde_json::from_value(serde_json::to_value(&get).unwrap()).unwrap();
        assert_eq!(back.meta, get.meta);

        let ok = SessionInboxOkResult { ok: true };
        assert_eq!(
            serde_json::to_value(ok).unwrap(),
            serde_json::json!({"ok": true})
        );

        let reload = SessionReloadExtensionsResult { queued: Some(true) };
        let v = serde_json::to_value(reload).unwrap();
        assert_eq!(v["queued"], true);
        let plain = SessionReloadExtensionsResult::default();
        assert!(serde_json::to_value(plain).unwrap().get("queued").is_none());

        let goal = SessionGoalResult {
            goal: serde_json::json!({"status": "running"}),
        };
        assert_eq!(
            serde_json::to_value(&goal).unwrap()["goal"]["status"],
            "running"
        );

        let move_p: SessionInboxMoveParams = serde_json::from_value(serde_json::json!({
            "sessionId": "s1", "itemId": "i1", "toIndex": 2
        }))
        .unwrap();
        assert_eq!(move_p.to_index, 2);
    }

    #[test]
    fn load_resume_result_carries_no_session_id() {
        let value = serde_json::to_value(SessionOpenResult::default()).unwrap();
        assert!(value.get("sessionId").is_none());
        assert!(value.get("models").is_none());
    }

    #[test]
    fn list_result_omits_next_cursor_when_unset() {
        let result = SessionListResult {
            sessions: vec![],
            next_cursor: None,
        };
        let value = serde_json::to_value(&result).unwrap();
        assert!(value.get("nextCursor").is_none());
    }

    #[test]
    fn stop_reason_wire_values_are_end_turn_or_cancelled() {
        assert_eq!(
            serde_json::to_value(StopReason::EndTurn).unwrap(),
            serde_json::json!("end_turn")
        );
        assert_eq!(
            serde_json::to_value(StopReason::Cancelled).unwrap(),
            serde_json::json!("cancelled")
        );
    }
}

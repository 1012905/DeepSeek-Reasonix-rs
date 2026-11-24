//! MCP / extension control: reconnect, disconnect, submitForm
//! (session_mcp_control.go:12-148).
//!
//! The adapter freezes the params/result DTOs, the three-state reconnect
//! machine, and the ACP-layer validation boundary. Real controller calls run
//! behind the injectable [`McpControl`] trait; extension-side field validation
//! stays with the extension hub, so this layer must not add schema checks that
//! would move a `-32603` failure onto `-32602`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::errors::{RpcError, RpcErrorCode};
use crate::methods::{
    SESSION_EXTENSION_SUBMIT_FORM, SESSION_MCP_DISCONNECT, SESSION_MCP_RECONNECT,
};

/// Control schema version advertised on every result (session_mcp_control.go:13).
pub const CONTROL_SCHEMA_VERSION: i32 = 1;

/// Reconnect approval-owed business code (mcp_admin.go:217-229).
pub const MCP_APPROVAL_OWED: &str = "mcp.approval_owed";

// ---- Params / results ---------------------------------------------------------

/// Shared reconnect/disconnect address: session + server name
/// (session_mcp_control.go:29-32).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SessionMcpNameParams {
    #[serde(rename = "sessionId")]
    pub session_id: String,
    pub name: String,
}

/// Reconnect outcome state on the wire (session_mcp_control.go:36-45, 96-116).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReconnectState {
    /// Connected; tools re-registered.
    Ready,
    /// Project-declared but not approved; the server must never be started.
    ApprovalOwed,
    /// Any other connection failure; the unknown-name case lands here too.
    Failed,
}

impl ReconnectState {
    /// Wire vocabulary.
    pub fn wire(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::ApprovalOwed => "approval_owed",
            Self::Failed => "failed",
        }
    }
}

/// Controller-side reconnect outcome, mapped onto the wire result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReconnectOutcome {
    Ready { tools: i64 },
    ApprovalOwed { launch: String },
    Failed { error: String },
}

/// Reconnect result (session_mcp_control.go:36-45).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ReasonixMcpReconnect {
    #[serde(rename = "schemaVersion")]
    pub schema_version: i32,
    #[serde(rename = "sessionId")]
    pub session_id: String,
    pub name: String,
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub launch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl ReasonixMcpReconnect {
    /// Builds the result from a controller outcome; conditional fields ride
    /// only on their owning state.
    pub fn from_outcome(session_id: &str, name: &str, outcome: ReconnectOutcome) -> Self {
        match outcome {
            ReconnectOutcome::Ready { tools } => Self {
                schema_version: CONTROL_SCHEMA_VERSION,
                session_id: session_id.to_owned(),
                name: name.to_owned(),
                state: ReconnectState::Ready.wire().to_owned(),
                tools: Some(tools),
                code: None,
                launch: None,
                error: None,
            },
            ReconnectOutcome::ApprovalOwed { launch } => Self {
                schema_version: CONTROL_SCHEMA_VERSION,
                session_id: session_id.to_owned(),
                name: name.to_owned(),
                state: ReconnectState::ApprovalOwed.wire().to_owned(),
                tools: None,
                code: Some(MCP_APPROVAL_OWED.to_owned()),
                launch: Some(launch),
                error: None,
            },
            ReconnectOutcome::Failed { error } => Self {
                schema_version: CONTROL_SCHEMA_VERSION,
                session_id: session_id.to_owned(),
                name: name.to_owned(),
                state: ReconnectState::Failed.wire().to_owned(),
                tools: None,
                code: None,
                launch: None,
                error: Some(error),
            },
        }
    }
}

/// Disconnect result (session_mcp_control.go:48-53).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ReasonixMcpDisconnect {
    #[serde(rename = "schemaVersion")]
    pub schema_version: i32,
    #[serde(rename = "sessionId")]
    pub session_id: String,
    pub name: String,
    pub disconnected: bool,
}

/// submitForm params (session_mcp_control.go:57-62).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct SessionExtensionFormParams {
    #[serde(rename = "sessionId")]
    pub session_id: String,
    #[serde(rename = "pluginId")]
    pub plugin_id: String,
    #[serde(rename = "surfaceId")]
    pub surface_id: String,
    pub values: BTreeMap<String, Value>,
}

/// submitForm result (session_mcp_control.go:65-69).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ReasonixExtensionFormResult {
    #[serde(rename = "schemaVersion")]
    pub schema_version: i32,
    #[serde(rename = "sessionId")]
    pub session_id: String,
    pub accepted: bool,
}

// ---- Validation boundary -------------------------------------------------------

/// `-32602` blank server name (`"<method>: name is required"`).
pub fn name_required(method: &str) -> RpcError {
    RpcError::invalid_params(format!("{method}: name is required"))
}

/// `-32602` unknown session, reused across the three control methods.
pub fn unknown_session(method: &str, session_id: &str) -> RpcError {
    RpcError::invalid_params(format!("{method}: unknown session {session_id}"))
}

/// `-32603` controller / extension rejection (`"<method>: <err>"`).
pub fn control_failure(method: &str, err: &str) -> RpcError {
    RpcError::new(RpcErrorCode::Internal, format!("{method}: {err}"))
}

/// Blank plugin/surface identity check at the ACP layer; anything past this
/// boundary that fails belongs to the extension hub and maps to `-32603`.
pub fn validate_form_identity(params: &SessionExtensionFormParams) -> Result<(), RpcError> {
    if params.plugin_id.trim().is_empty() || params.surface_id.trim().is_empty() {
        return Err(crate::extensions::submit_form_identity_required(
            SESSION_EXTENSION_SUBMIT_FORM,
        ));
    }
    Ok(())
}

// ---- Injectable controller seam -------------------------------------------------

/// Controller-side MCP/extension operations the runtime supplies. Tests inject
/// an in-memory double that records calls and returns fixed outcomes.
pub trait McpControl {
    /// Reconnect one configured server; returns its outcome state.
    fn reconnect(&self, name: &str) -> ReconnectOutcome;
    /// Disconnect a live session-local server; reports whether one was live.
    fn disconnect(&self, name: &str) -> bool;
    /// Hand the form values to the owning extension surface.
    fn submit_form(
        &self,
        plugin_id: &str,
        surface_id: &str,
        values: &BTreeMap<String, Value>,
    ) -> Result<(), String>;
}

/// Method constants re-exported for the registry assembly.
pub const CONTROL_REQUEST_METHODS: [&str; 3] = [
    SESSION_MCP_RECONNECT,
    SESSION_MCP_DISCONNECT,
    SESSION_EXTENSION_SUBMIT_FORM,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconnect_result_carries_state_conditional_fields() {
        let ready =
            ReasonixMcpReconnect::from_outcome("s1", "fs", ReconnectOutcome::Ready { tools: 4 });
        assert_eq!(ready.state, "ready");
        assert_eq!(ready.tools, Some(4));
        assert!(ready.error.is_none());

        let owed = ReasonixMcpReconnect::from_outcome(
            "s1",
            "fs",
            ReconnectOutcome::ApprovalOwed {
                launch: "npx fs".to_owned(),
            },
        );
        assert_eq!(owed.state, "approval_owed");
        assert_eq!(owed.code.as_deref(), Some(MCP_APPROVAL_OWED));
        assert_eq!(owed.launch.as_deref(), Some("npx fs"));
        assert!(owed.tools.is_none());

        let failed = ReasonixMcpReconnect::from_outcome(
            "s1",
            "fs",
            ReconnectOutcome::Failed {
                error: "boom".to_owned(),
            },
        );
        assert_eq!(failed.state, "failed");
        assert_eq!(failed.error.as_deref(), Some("boom"));
    }

    #[test]
    fn form_identity_validation_boundary() {
        let ok = SessionExtensionFormParams {
            session_id: "s1".to_owned(),
            plugin_id: "p".to_owned(),
            surface_id: "s".to_owned(),
            values: BTreeMap::new(),
        };
        assert!(validate_form_identity(&ok).is_ok());

        let blank = SessionExtensionFormParams {
            session_id: "s1".to_owned(),
            plugin_id: "  ".to_owned(),
            surface_id: "s".to_owned(),
            values: BTreeMap::new(),
        };
        let err = validate_form_identity(&blank).unwrap_err();
        assert_eq!(err.code, RpcErrorCode::InvalidParams);
        assert_eq!(
            err.message,
            "_reasonix.io/session/extension/submitForm: pluginId and surfaceId are required"
        );
    }

    #[test]
    fn control_error_messages_are_verbatim() {
        let name = name_required(SESSION_MCP_RECONNECT);
        assert_eq!(name.code, RpcErrorCode::InvalidParams);
        assert_eq!(
            name.message,
            "_reasonix.io/session/mcp/reconnect: name is required"
        );

        let failed = control_failure(SESSION_EXTENSION_SUBMIT_FORM, "no hub");
        assert_eq!(failed.code, RpcErrorCode::Internal);
        assert_eq!(
            failed.message,
            "_reasonix.io/session/extension/submitForm: no hub"
        );
    }
}

//! JSON-RPC error identity for the ACP wire (protocol.go:27-39,
//! cancel_request.go:11-17, session_goal.go:23-26, tool_approval_ceiling.go:9-13).
//!
//! Callers key on the typed code, never on matching the message sentence. The
//! numeric identities below are frozen by the reference; messages are built
//! per call site and ride alongside the code.

use serde::{Deserialize, Serialize};

/// Standard and vendor JSON-RPC error codes used on the ACP wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[non_exhaustive]
pub enum RpcErrorCode {
    /// -32700: frame does not parse as JSON.
    Parse,
    /// -32600: frame is neither request, notification, nor response.
    InvalidRequest,
    /// -32601: unknown request method.
    MethodNotFound,
    /// -32602: bad params, unknown configId, unknown session.
    InvalidParams,
    /// -32603: internal failure.
    Internal,
    /// -32000: ACP schema "Authentication required".
    AuthRequired,
    /// -32800: request cancelled by `$/cancel_request`.
    RequestCancelled,
    /// -32010: goal resume with no resumable Goal.
    GoalNotResumable,
    /// -32011: goal pause with no running Goal.
    GoalNotRunning,
    /// -32020: durable inbox refused the call.
    InboxRefused,
    /// -32021: inbox work may already have run.
    InboxMayHaveRun,
    /// -32022: session transcript lease held by another holder.
    SessionLeaseHeld,
    /// -32023: lease ownership unprovable; the client must not be told to close
    /// a window.
    SessionLeaseUnprovable,
}

impl RpcErrorCode {
    /// Frozen numeric identity on the wire.
    pub const fn code(self) -> i32 {
        match self {
            Self::Parse => -32700,
            Self::InvalidRequest => -32600,
            Self::MethodNotFound => -32601,
            Self::InvalidParams => -32602,
            Self::Internal => -32603,
            Self::AuthRequired => -32000,
            Self::RequestCancelled => -32800,
            Self::GoalNotResumable => -32010,
            Self::GoalNotRunning => -32011,
            Self::InboxRefused => -32020,
            Self::InboxMayHaveRun => -32021,
            Self::SessionLeaseHeld => -32022,
            Self::SessionLeaseUnprovable => -32023,
        }
    }
}

/// `data.code` for a turn that failed after admission (protocol.go:37-39).
pub const DATA_CODE_TURN_FAILED: &str = "turn_failed";

/// Structured `data` carried by an error object; omitted when [`RpcError::data`]
/// is `None`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ErrorData {
    /// Stable machine-readable reason; human detail rides the message.
    pub code: String,
    /// Optional ceiling/holder detail the client keys on.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ceiling: Option<String>,
}

/// A JSON-RPC error ready for the wire.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct RpcError {
    /// Typed code identity.
    pub code: RpcErrorCode,
    /// Human-readable message, verbatim per call site.
    pub message: String,
    /// Optional structured data; serialized only when present.
    pub data: Option<serde_json::Value>,
}

impl RpcError {
    pub fn new(code: RpcErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }

    pub fn with_data(mut self, data: serde_json::Value) -> Self {
        self.data = Some(data);
        self
    }

    /// `-32700 "parse error"`, answered with a null id (server.go:207-209).
    pub fn parse_error() -> Self {
        Self::new(RpcErrorCode::Parse, "parse error")
    }

    /// `-32600 "invalid request"` (server.go:228).
    pub fn invalid_request() -> Self {
        Self::new(RpcErrorCode::InvalidRequest, "invalid request")
    }

    /// `-32601 "method not found: <method>"` (server.go:236).
    pub fn method_not_found(method: &str) -> Self {
        Self::new(
            RpcErrorCode::MethodNotFound,
            format!("method not found: {method}"),
        )
    }

    /// `-32602` invalid-params failure with a call-site message.
    pub fn invalid_params(message: impl Into<String>) -> Self {
        Self::new(RpcErrorCode::InvalidParams, message)
    }

    /// `-32603` internal failure with a call-site message.
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(RpcErrorCode::Internal, message)
    }

    /// `-32000 "Authentication required"` (contract §1).
    pub fn auth_required() -> Self {
        Self::new(RpcErrorCode::AuthRequired, "Authentication required")
    }

    /// Cancelled rewrite (server.go:249-251): a request the peer cancelled is
    /// answered `-32800`, its original message is preserved, and `data` is
    /// dropped.
    pub fn rewrite_cancelled(self) -> Self {
        Self {
            code: RpcErrorCode::RequestCancelled,
            message: self.message,
            data: None,
        }
    }
}

/// Wire shape of the JSON-RPC error object: `{code, message, data?}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RpcErrorBody {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

impl From<&RpcError> for RpcErrorBody {
    fn from(err: &RpcError) -> Self {
        Self {
            code: err.code.code(),
            message: err.message.clone(),
            data: err.data.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_identities_match_the_frozen_table() {
        let cases = [
            (RpcErrorCode::Parse, -32700),
            (RpcErrorCode::InvalidRequest, -32600),
            (RpcErrorCode::MethodNotFound, -32601),
            (RpcErrorCode::InvalidParams, -32602),
            (RpcErrorCode::Internal, -32603),
            (RpcErrorCode::AuthRequired, -32000),
            (RpcErrorCode::RequestCancelled, -32800),
            (RpcErrorCode::GoalNotResumable, -32010),
            (RpcErrorCode::GoalNotRunning, -32011),
            (RpcErrorCode::InboxRefused, -32020),
            (RpcErrorCode::InboxMayHaveRun, -32021),
            (RpcErrorCode::SessionLeaseHeld, -32022),
            (RpcErrorCode::SessionLeaseUnprovable, -32023),
        ];
        for (code, want) in cases {
            assert_eq!(code.code(), want, "{code:?}");
        }
    }

    #[test]
    fn canned_messages_are_verbatim() {
        assert_eq!(RpcError::parse_error().message, "parse error");
        assert_eq!(RpcError::invalid_request().message, "invalid request");
        assert_eq!(
            RpcError::method_not_found("session/bogus").message,
            "method not found: session/bogus"
        );
        assert_eq!(RpcError::auth_required().message, "Authentication required");
    }

    #[test]
    fn cancelled_rewrite_keeps_message_and_drops_data() {
        let err = RpcError::internal("boom").with_data(serde_json::json!({"code": "x"}));
        let cancelled = err.rewrite_cancelled();
        assert_eq!(cancelled.code, RpcErrorCode::RequestCancelled);
        assert_eq!(cancelled.code.code(), -32800);
        assert_eq!(cancelled.message, "boom");
        assert!(cancelled.data.is_none());
    }

    #[test]
    fn error_body_serializes_data_only_when_present() {
        let err = RpcError::invalid_params("authenticate: unknown methodId x");
        let body = RpcErrorBody::from(&err);
        let v = serde_json::to_value(&body).unwrap();
        assert_eq!(v["code"], -32602);
        assert_eq!(v["message"], "authenticate: unknown methodId x");
        assert!(v.get("data").is_none());

        let with_data =
            err.with_data(serde_json::json!({"code": "acp.tool_approval.above_ceiling"}));
        let body = RpcErrorBody::from(&with_data);
        let v = serde_json::to_value(&body).unwrap();
        assert_eq!(v["data"]["code"], "acp.tool_approval.above_ceiling");
    }
}

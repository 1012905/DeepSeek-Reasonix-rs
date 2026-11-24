//! Vendor extension method registry and opt-in gating (service.go:106-117;
//! status.go:677-688; mcp_status.go:80-85; inbox.go:40-78).
//!
//! Opt-in gates notifications only. Every vendor request handler is registered
//! unconditionally: an unadvertised capability does not produce a dedicated
//! rejection, and an unrecognized method falls through to `-32601`
//! (matrix B §5.3, conflict C8).

use crate::caps::ClientCapabilities;
use crate::errors::{RpcError, RpcErrorCode};
use crate::methods::{
    SESSION_EXTENSION_SUBMIT_FORM, SESSION_GOAL_PAUSE, SESSION_GOAL_RESUME, SESSION_INBOX_DELETE,
    SESSION_INBOX_ENQUEUE, SESSION_INBOX_GET, SESSION_INBOX_LIST, SESSION_INBOX_MOVE,
    SESSION_INBOX_REFRESH, SESSION_INBOX_RETRY, SESSION_INBOX_SET_PAUSED, SESSION_INBOX_UPDATE,
    SESSION_INFO, SESSION_MCP_DISCONNECT, SESSION_MCP_RECONNECT, SESSION_MCP_SERVERS,
    SESSION_MCP_STATUS, SESSION_MCP_STATUS_UPDATE, SESSION_RELOAD_EXTENSIONS, SESSION_STATUS,
    SESSION_STATUS_UPDATE, SESSION_STEER,
};

/// Vendor request methods the agent accepts from the client, in registration
/// order. Requests are not gated by initialize opt-in.
pub const VENDOR_REQUEST_METHODS: &[&str] = &[
    SESSION_STEER,
    SESSION_INBOX_ENQUEUE,
    SESSION_INBOX_LIST,
    SESSION_INBOX_GET,
    SESSION_INBOX_UPDATE,
    SESSION_INBOX_DELETE,
    SESSION_INBOX_MOVE,
    SESSION_INBOX_SET_PAUSED,
    SESSION_INBOX_RETRY,
    SESSION_INBOX_REFRESH,
    SESSION_RELOAD_EXTENSIONS,
    SESSION_GOAL_RESUME,
    SESSION_GOAL_PAUSE,
    SESSION_STATUS,
    SESSION_MCP_STATUS,
    SESSION_INFO,
    SESSION_MCP_SERVERS,
    SESSION_MCP_RECONNECT,
    SESSION_MCP_DISCONNECT,
    SESSION_EXTENSION_SUBMIT_FORM,
];

/// Vendor notification methods the agent may push to the client.
pub const VENDOR_NOTIFICATION_METHODS: &[&str] = &[
    "_reasonix.io/extension_surface",
    SESSION_STATUS_UPDATE,
    SESSION_MCP_STATUS_UPDATE,
];

/// True when `method` is a registered vendor request.
pub fn is_vendor_request(method: &str) -> bool {
    VENDOR_REQUEST_METHODS.contains(&method)
}

/// True when `method` is a vendor notification gated by opt-in.
pub fn is_vendor_notification(method: &str) -> bool {
    VENDOR_NOTIFICATION_METHODS.contains(&method)
}

/// Decides whether a vendor notification may be published. Requests never
/// reach this gate.
///
/// * `_reasonix.io/extension_surface` — `extensionSurface` opt-in; when
///   suppressed, the flattened text fallback chunk still goes out.
/// * `status_update` / `mcpStatus_update` — `statusUpdates` opt-in.
pub fn notification_enabled(method: &str, caps: &ClientCapabilities) -> bool {
    match method {
        "_reasonix.io/extension_surface" => caps.extension_surface_supported(),
        SESSION_STATUS_UPDATE | SESSION_MCP_STATUS_UPDATE => caps.status_updates_supported(),
        _ => false,
    }
}

// ---- Inbox refusal code table ------------------------------------------------

/// Stable inbox / steer failure code mapped to its frozen RPC code
/// (inbox.go:40-78).
pub fn inbox_refusal_rpc_code(data_code: &str) -> RpcErrorCode {
    match data_code {
        "steer.already_applied"
        | "inbox.item_too_large"
        | "inbox.capacity_items"
        | "inbox.capacity_bytes"
        | "inbox.invalid_state"
        | "inbox.foreign_run"
        | "inbox.paused"
        | "inbox.unrecovered"
        | "inbox.not_found"
        | "inbox.idempotency_conflict"
        | "inbox.schema_readonly"
        | "inbox.closed" => RpcErrorCode::InboxRefused,
        "inbox.retry_may_repeat" => RpcErrorCode::InboxMayHaveRun,
        "inbox.empty" => RpcErrorCode::InvalidParams,
        _ => RpcErrorCode::Internal,
    }
}

// ---- Vendor handler error templates ------------------------------------------

/// `-32602` unknown session for a vendor method (`"<method>: unknown session <id>"`).
pub fn unknown_session(method: &str, session_id: &str) -> RpcError {
    RpcError::invalid_params(format!("{method}: unknown session {session_id}"))
}

/// `-32602` steer with an empty prompt (`"<steer method>: empty prompt"`).
pub fn steer_empty_prompt(method: &str) -> RpcError {
    RpcError::invalid_params(format!("{method}: empty prompt"))
}

/// `-32010` goal resume with no resumable goal.
pub fn goal_not_resumable(method: &str) -> RpcError {
    RpcError::new(
        RpcErrorCode::GoalNotResumable,
        format!("{method}: session has no resumable goal"),
    )
}

/// `-32011` goal pause with no running goal.
pub fn goal_not_running(method: &str) -> RpcError {
    RpcError::new(
        RpcErrorCode::GoalNotRunning,
        format!("{method}: session has no running goal"),
    )
}

/// `-32602` submitForm missing its plugin/surface identity.
pub fn submit_form_identity_required(method: &str) -> RpcError {
    RpcError::invalid_params(format!("{method}: pluginId and surfaceId are required"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::caps::ClientCapabilities;

    #[test]
    fn registry_covers_all_vendor_methods() {
        assert_eq!(VENDOR_REQUEST_METHODS.len(), 20);
        assert_eq!(VENDOR_NOTIFICATION_METHODS.len(), 3);
        assert!(is_vendor_request(SESSION_STEER));
        assert!(is_vendor_notification(SESSION_STATUS_UPDATE));
        assert!(!is_vendor_request("session/steer"));
    }

    #[test]
    fn notification_gate_reads_initialize_opt_in() {
        let opted: ClientCapabilities = serde_json::from_value(serde_json::json!({
            "_meta": {"reasonix.io": {
                "extensionSurface": {"supported": true},
                "statusUpdates": {"supported": true}
            }}
        }))
        .unwrap();
        assert!(notification_enabled(
            "_reasonix.io/extension_surface",
            &opted
        ));
        assert!(notification_enabled(SESSION_STATUS_UPDATE, &opted));
        assert!(notification_enabled(SESSION_MCP_STATUS_UPDATE, &opted));

        let bare = ClientCapabilities::default();
        assert!(!notification_enabled(
            "_reasonix.io/extension_surface",
            &bare
        ));
        assert!(!notification_enabled(SESSION_STATUS_UPDATE, &bare));
    }

    #[test]
    fn inbox_refusal_map_maps_fourteen_codes() {
        let refused = [
            "steer.already_applied",
            "inbox.item_too_large",
            "inbox.capacity_items",
            "inbox.capacity_bytes",
            "inbox.invalid_state",
            "inbox.foreign_run",
            "inbox.paused",
            "inbox.unrecovered",
            "inbox.not_found",
            "inbox.idempotency_conflict",
            "inbox.schema_readonly",
            "inbox.closed",
        ];
        for code in refused {
            assert_eq!(
                inbox_refusal_rpc_code(code),
                RpcErrorCode::InboxRefused,
                "{code}"
            );
        }
        assert_eq!(
            inbox_refusal_rpc_code("inbox.retry_may_repeat"),
            RpcErrorCode::InboxMayHaveRun
        );
        assert_eq!(
            inbox_refusal_rpc_code("inbox.empty"),
            RpcErrorCode::InvalidParams
        );
        assert_eq!(
            inbox_refusal_rpc_code("inbox.unknown_fault"),
            RpcErrorCode::Internal
        );
    }

    #[test]
    fn vendor_error_messages_are_verbatim() {
        let err = unknown_session(SESSION_STEER, "s7");
        assert_eq!(err.code, RpcErrorCode::InvalidParams);
        assert_eq!(
            err.message,
            "_reasonix.io/session/steer: unknown session s7"
        );

        assert_eq!(
            goal_not_resumable(SESSION_GOAL_RESUME).message,
            "_reasonix.io/session/goal/resume: session has no resumable goal"
        );
        assert_eq!(
            goal_not_running(SESSION_GOAL_PAUSE).message,
            "_reasonix.io/session/goal/pause: session has no running goal"
        );
    }
}

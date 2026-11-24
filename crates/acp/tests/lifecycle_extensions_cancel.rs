//! ACP-010 lifecycle/lease/controls, ACP-011 vendor registry and opt-in,
//! ACP-012 event mapping and flightSet cancel.

use reasonix_acp::cancel::{FlightSet, cancel_request_id, should_rewrite_cancelled};
use reasonix_acp::dispatch::{AgentEventKind, EventDispatch, map_event};
use reasonix_acp::errors::RpcErrorCode;
use reasonix_acp::extensions::{
    VENDOR_NOTIFICATION_METHODS, VENDOR_REQUEST_METHODS, inbox_refusal_rpc_code,
    is_vendor_notification, is_vendor_request, notification_enabled, submit_form_identity_required,
};
use reasonix_acp::lifecycle::{
    CODE_LEASE_UNPROVABLE, CODE_SESSION_IN_USE, LeaseRefusal, LifecycleEvent, SessionState,
    lease_error, next_state, normalize_config_id, resolve_mode_id, validate_session_id,
};
use reasonix_acp::methods::SESSION_STEER;
use reasonix_acp::methods::{
    SESSION_EXTENSION_SUBMIT_FORM, SESSION_MCP_STATUS_UPDATE, SESSION_STATUS_UPDATE,
};
use reasonix_acp::permission::above_ceiling_error;
use serde_json::Value;

// ---- ACP-010 -----------------------------------------------------------------

#[test]
fn session_id_validation_messages_and_charset() {
    assert!(validate_session_id("session/new", "abc-DEF_123.x").is_ok());

    let blank = validate_session_id("session/new", "  ").unwrap_err();
    assert_eq!(blank.code, RpcErrorCode::InvalidParams);
    assert_eq!(blank.message, "session/new: missing sessionId");

    for bad in [".", "..", "has space", "sl/ash"] {
        let err = validate_session_id("session/close", bad).unwrap_err();
        assert_eq!(err.message, "session/close: invalid sessionId");
    }
}

#[test]
fn lifecycle_state_machine_transitions() {
    use LifecycleEvent::*;
    use SessionState::*;
    assert_eq!(next_state(New, Open).unwrap(), Active);
    assert_eq!(next_state(Active, Close).unwrap(), Closed);
    assert_eq!(next_state(Closed, Delete).unwrap(), Deleted);
    assert!(next_state(Closed, Close).is_err());
    assert!(next_state(Deleted, Delete).is_err());
    assert!(next_state(New, Close).is_err());
}

#[test]
fn lease_refusal_code_table_and_data() {
    let held = lease_error(
        "session/load",
        LeaseRefusal::Held {
            holder: String::new(),
        },
    );
    assert_eq!(held.code, RpcErrorCode::SessionLeaseHeld);
    assert_eq!(held.code.code(), -32022);
    assert_eq!(held.data.as_ref().unwrap()["code"], CODE_SESSION_IN_USE);
    assert!(held.message.contains("close the other Reasonix window"));

    let unprovable = lease_error(
        "session/resume",
        LeaseRefusal::Unprovable {
            detail: "probe timed out".to_owned(),
        },
    );
    assert_eq!(unprovable.code, RpcErrorCode::SessionLeaseUnprovable);
    assert_eq!(unprovable.code.code(), -32023);
    assert_eq!(
        unprovable.data.as_ref().unwrap()["code"],
        CODE_LEASE_UNPROVABLE
    );
    assert_eq!(unprovable.message, "session/resume: probe timed out");

    let other = lease_error(
        "session/new",
        LeaseRefusal::Other {
            detail: "io failure".to_owned(),
        },
    );
    assert_eq!(other.code, RpcErrorCode::Internal);
    assert_eq!(other.message, "session/new: session lease: io failure");
    assert!(other.data.is_none());
}

#[test]
fn legacy_mode_and_config_aliases_normalize() {
    let default = resolve_mode_id("default").unwrap();
    assert_eq!(default.mode_id, "normal");
    assert_eq!(default.implied_posture, Some("ask"));
    let auto = resolve_mode_id("auto").unwrap();
    assert_eq!(auto.implied_posture, Some("yolo"));
    assert!(resolve_mode_id("bogus").is_none());

    assert_eq!(normalize_config_id("models"), "model");
    assert_eq!(normalize_config_id("thought_level"), "effort");
    assert_eq!(normalize_config_id("token_mode"), "work_mode");
    assert_eq!(normalize_config_id("approval_mode"), "tool_approval");
}

// ---- ACP-011 -----------------------------------------------------------------

#[test]
fn vendor_registry_lists_requests_and_notifications() {
    assert_eq!(VENDOR_REQUEST_METHODS.len(), 20);
    assert_eq!(VENDOR_NOTIFICATION_METHODS.len(), 3);
    assert!(is_vendor_request(SESSION_STEER));
    assert!(is_vendor_notification(SESSION_STATUS_UPDATE));
    // A bare unprefixed method is unknown, not a vendor request.
    assert!(!is_vendor_request("session/steer"));
}

#[test]
fn opt_in_gates_notifications_only_requests_unconditional() {
    let opted: reasonix_acp::caps::ClientCapabilities = serde_json::from_value(serde_json::json!({
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

    let bare = reasonix_acp::caps::ClientCapabilities::default();
    assert!(!notification_enabled(
        "_reasonix.io/extension_surface",
        &bare
    ));
    assert!(!notification_enabled(SESSION_STATUS_UPDATE, &bare));
    assert!(!notification_enabled(SESSION_MCP_STATUS_UPDATE, &bare));
}

#[test]
fn inbox_refusal_code_map_covers_fourteen_entries() {
    for code in [
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
    ] {
        assert_eq!(inbox_refusal_rpc_code(code), RpcErrorCode::InboxRefused);
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
        inbox_refusal_rpc_code("inbox.unknown"),
        RpcErrorCode::Internal
    );
}

#[test]
fn vendor_error_templates_are_verbatim() {
    let err = submit_form_identity_required(SESSION_EXTENSION_SUBMIT_FORM);
    assert_eq!(err.code, RpcErrorCode::InvalidParams);
    assert_eq!(
        err.message,
        "_reasonix.io/session/extension/submitForm: pluginId and surfaceId are required"
    );
}

// ---- ACP-012 -----------------------------------------------------------------

#[test]
fn event_mapping_covers_mapped_and_dropped_kinds() {
    assert_eq!(
        map_event(AgentEventKind::Reasoning),
        EventDispatch::ThoughtChunk
    );
    assert_eq!(map_event(AgentEventKind::Text), EventDispatch::MessageChunk);
    assert_eq!(
        map_event(AgentEventKind::ToolDispatch),
        EventDispatch::ToolCallPending
    );
    assert_eq!(
        map_event(AgentEventKind::ToolResult),
        EventDispatch::ToolCallUpdate
    );
    assert_eq!(
        map_event(AgentEventKind::Notice),
        EventDispatch::WarningChunk
    );
    assert_eq!(
        map_event(AgentEventKind::CompactionDone),
        EventDispatch::CompactionChunk
    );
    assert_eq!(
        map_event(AgentEventKind::ApprovalRequest),
        EventDispatch::PermissionRequest
    );
    assert_eq!(
        map_event(AgentEventKind::AskRequest),
        EventDispatch::AskRoundTrip
    );
    assert_eq!(
        map_event(AgentEventKind::ExtensionSurface),
        EventDispatch::ExtensionDualEmission
    );
    assert_eq!(
        map_event(AgentEventKind::McpSurfaceReady),
        EventDispatch::PublishMcpStatus
    );
    assert_eq!(
        map_event(AgentEventKind::StreamAttempt),
        EventDispatch::BookkeepingOnly
    );

    for kind in [
        AgentEventKind::TurnStarted,
        AgentEventKind::Message,
        AgentEventKind::Usage,
        AgentEventKind::Phase,
        AgentEventKind::TurnDone,
        AgentEventKind::CompactionStarted,
        AgentEventKind::ToolProgress,
        AgentEventKind::Retrying,
        AgentEventKind::Steer,
        AgentEventKind::GuardianAssessment,
        AgentEventKind::ContextMaintenance,
        AgentEventKind::TodoProgress,
        AgentEventKind::WorkspaceLease,
    ] {
        assert_eq!(map_event(kind), EventDispatch::NoUpdate, "{kind:?}");
    }
}

#[test]
fn flight_set_begin_cancel_end_guards() {
    let mut set = FlightSet::default();
    let key = set.begin(&Value::from(7));
    assert_eq!(set.len(), 1);

    set.cancel(Some(&serde_json::json!({"requestId": 7})));
    assert!(set.is_cancelled(&key));

    // Unknown, answered, and malformed cancels are ignored.
    set.cancel(Some(&serde_json::json!({"requestId": "nope"})));
    set.cancel(Some(&serde_json::json!({"requestId": ""})));
    set.cancel(Some(&serde_json::json!({"requestId": {"nested": 1}})));
    set.cancel(None);
    assert_eq!(set.len(), 1);

    set.end(&key);
    assert!(set.is_empty());
    set.cancel(Some(&serde_json::json!({"requestId": 7})));
    assert!(!set.is_cancelled("7"));
}

#[test]
fn cancel_before_registration_does_not_precancel() {
    let mut set = FlightSet::default();
    set.cancel(Some(&serde_json::json!({"requestId": 9})));
    let key = set.begin(&Value::from(9));
    assert!(!set.is_cancelled(&key));
    set.end(&key);
}

#[test]
fn cancel_request_id_parsing_is_tolerant_and_silent_on_bad_body() {
    assert_eq!(
        cancel_request_id(Some(&serde_json::json!({"requestId": "  9 "}))),
        Some("9".to_owned())
    );
    assert_eq!(
        cancel_request_id(Some(&serde_json::json!({"requestId": 42}))),
        Some("42".to_owned())
    );
    assert_eq!(
        cancel_request_id(Some(&serde_json::json!({"requestId": null}))),
        None
    );
    assert_eq!(cancel_request_id(Some(&serde_json::json!({}))), None);
    assert_eq!(cancel_request_id(None), None);
}

#[test]
fn rewrite_requires_both_cancelled_flag_and_context_done() {
    assert!(should_rewrite_cancelled(true, true));
    assert!(!should_rewrite_cancelled(true, false));
    assert!(!should_rewrite_cancelled(false, true));
    assert!(!should_rewrite_cancelled(false, false));
}

#[test]
fn rewritten_error_is_minus_32800_with_null_data() {
    let err =
        reasonix_acp::errors::RpcError::internal("boom").with_data(serde_json::json!({"x": 1}));
    let rewritten = err.rewrite_cancelled();
    assert_eq!(rewritten.code, RpcErrorCode::RequestCancelled);
    assert_eq!(rewritten.code.code(), -32800);
    assert_eq!(rewritten.message, "boom");
    assert!(rewritten.data.is_none());
}

#[test]
fn above_ceiling_is_invalid_params_not_permission_denied() {
    let err = above_ceiling_error("session/set_config_option", "yolo", "readOnly");
    assert_eq!(err.code, RpcErrorCode::InvalidParams);
    assert_eq!(err.data.unwrap()["code"], "acp.tool_approval.above_ceiling");
}

//! ACP-002: error code numeric identity, verbatim messages, cancelled rewrite.

use reasonix_acp::errors::{RpcError, RpcErrorCode};
use reasonix_acp::jsonrpc::error_frame;
use serde_json::Value;

#[test]
fn every_code_has_frozen_numeric_identity() {
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
    for (code, expected) in cases {
        assert_eq!(code.code(), expected, "{code:?}");
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
fn cancelled_rewrite_sets_code_nils_data_keeps_message() {
    let original = RpcError::internal("boom").with_data(serde_json::json!({"code": "upstream"}));
    let rewritten = original.rewrite_cancelled();
    assert_eq!(rewritten.code, RpcErrorCode::RequestCancelled);
    assert_eq!(rewritten.code.code(), -32800);
    assert_eq!(rewritten.message, "boom");
    assert!(rewritten.data.is_none());
}

#[test]
fn error_frame_wire_shape_and_field_order() {
    let err = RpcError::method_not_found("bogus");
    let frame = error_frame(Some(&Value::from(1)), &err);
    let text = String::from_utf8(frame).unwrap();
    assert_eq!(
        text,
        r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32601,"message":"method not found: bogus"}}"#
            .to_owned()
            + "\n"
    );

    let parse = RpcError::parse_error();
    let frame = error_frame(None, &parse);
    let body: serde_json::Value = serde_json::from_slice(&frame[..frame.len() - 1]).unwrap();
    assert!(body.get("id").is_none());
    assert_eq!(body["error"]["code"], -32700);
    assert_eq!(body["error"]["message"], "parse error");
    assert!(body["error"].get("data").is_none());
}

#[test]
fn data_rides_only_when_present() {
    let err = RpcError::invalid_params("bad").with_data(serde_json::json!({"code": "x"}));
    let body = serde_json::to_value(reasonix_acp::errors::RpcErrorBody::from(&err)).unwrap();
    assert_eq!(body["code"], -32602);
    assert_eq!(body["data"]["code"], "x");

    let plain = RpcError::invalid_params("bad");
    let body = serde_json::to_value(reasonix_acp::errors::RpcErrorBody::from(&plain)).unwrap();
    assert!(body.get("data").is_none());
}

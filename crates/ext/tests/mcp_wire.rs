//! rpcRequest/rpcResponse wire oracles (MCP-007).
//!
//! Byte shapes confirmed against Go `encoding/json` for
//! `internal/ext/plugin/plugin.go:1526-1540`, `protocol_errors.go`, and
//! `protocol_inbound.go`.

use reasonix_ext::mcp::{
    InboundMessage, McpRoot, RpcError, RpcRequest, RpcResponse, decode_inbound_message,
    decode_rpc_response, format_mcp_progress, is_notification_id, progress_token_key,
    rpc_error_from_response, server_request_reply,
};
use serde_json::{json, to_string};

#[test]
fn rpc_request_byte_shape_with_id_and_params() {
    let req = RpcRequest::new(7, "tools/list", Some(json!({"x": 1})));
    assert_eq!(
        to_string(&req).unwrap(),
        r#"{"jsonrpc":"2.0","id":7,"method":"tools/list","params":{"x":1}}"#
    );
}

#[test]
fn rpc_request_notification_omits_id() {
    let req = RpcRequest::notification("notifications/initialized", Some(json!({})));
    assert_eq!(
        to_string(&req).unwrap(),
        r#"{"jsonrpc":"2.0","method":"notifications/initialized","params":{}}"#
    );

    let no_params = RpcRequest::new(3, "ping", None);
    assert_eq!(
        to_string(&no_params).unwrap(),
        r#"{"jsonrpc":"2.0","id":3,"method":"ping"}"#
    );
}

#[test]
fn rpc_request_wire_frame_appends_one_newline() {
    // Go's json.Encoder.Encode terminates every NDJSON frame with \n.
    let mut frame = RpcRequest::new(1, "ping", None).to_wire();
    assert_eq!(frame.pop(), Some(b'\n'));
    assert_eq!(
        frame,
        br#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#.to_vec()
    );
}

#[test]
fn rpc_response_emits_both_result_and_error_like_go() {
    // Go's response struct has no omitempty: absent side is null.
    let ok = RpcResponse::ok(json!(7), json!({"ok": true}));
    assert_eq!(
        to_string(&ok).unwrap(),
        r#"{"jsonrpc":"2.0","id":7,"result":{"ok":true},"error":null}"#
    );

    let fail = RpcResponse::fail(json!(8), RpcError::new(-32601, "Method not found"));
    assert_eq!(
        to_string(&fail).unwrap(),
        r#"{"jsonrpc":"2.0","id":8,"result":null,"error":{"code":-32601,"message":"Method not found"}}"#
    );
}

#[test]
fn rpc_error_byte_shape_and_data_omission() {
    let err = RpcError::new(-32000, "boom");
    assert_eq!(
        to_string(&err).unwrap(),
        r#"{"code":-32000,"message":"boom"}"#
    );

    let with_data = RpcError::with_data(
        -32022,
        "Unsupported protocol version",
        json!({"supported": ["2026-07-28"]}),
    );
    assert_eq!(
        to_string(&with_data).unwrap(),
        r#"{"code":-32022,"message":"Unsupported protocol version","data":{"supported":["2026-07-28"]}}"#
    );
}

#[test]
fn rpc_error_diagnostic_never_renders_message_text() {
    let err = RpcError::new(-32000, "unstructured neutralprobe");
    assert_eq!(
        err.diagnostic(),
        "rpc error -32000 (message omitted; 25 bytes)"
    );
    assert_eq!(err.to_string(), err.diagnostic());
    assert!(!err.to_string().contains("neutralprobe"));
}

#[test]
fn decode_rpc_response_accepts_go_emitted_envelopes() {
    let resp =
        decode_rpc_response(br#"{"jsonrpc":"2.0","id":7,"result":{"ok":true},"error":null}"#)
            .unwrap();
    assert_eq!(resp.id, json!(7));
    assert_eq!(resp.result, json!({"ok": true}));
    assert_eq!(rpc_error_from_response(&resp), None);

    let resp_err = decode_rpc_response(
        br#"{"jsonrpc":"2.0","id":8,"result":null,"error":{"code":-32001,"message":"Session not found"}}"#,
    )
    .unwrap();
    assert_eq!(
        rpc_error_from_response(&resp_err),
        Some(RpcError::new(-32001, "Session not found"))
    );
    assert_eq!(resp_err.result, serde_json::Value::Null);

    // Decoding is order-agnostic even though encoding is frozen.
    let reordered = decode_rpc_response(br#"{"id":9,"error":null,"result":{},"jsonrpc":"2.0"}"#);
    assert!(reordered.is_some());

    assert!(decode_rpc_response(br#"{"jsonrpc":"1.0","id":1}"#).is_none());
    assert!(decode_rpc_response(b"garbage").is_none());
}

#[test]
fn decode_inbound_message_handles_requests_notifications_and_null_ids() {
    let req = decode_inbound_message(
        br#"{"jsonrpc":"2.0","id":"e1","method":"elicitation/create","params":{"mode":"form"}}"#,
    )
    .unwrap();
    assert_eq!(req.jsonrpc, "2.0");
    assert_eq!(req.id, Some(json!("e1")));
    assert_eq!(req.method, "elicitation/create");
    assert_eq!(req.params, Some(json!({"mode": "form"})));

    let note = decode_inbound_message(
        br#"{"jsonrpc":"2.0","method":"notifications/progress","params":{}}"#,
    )
    .unwrap();
    assert_eq!(note.id, None);
    assert_eq!(note.method, "notifications/progress");

    let null_id =
        decode_inbound_message(br#"{"jsonrpc":"2.0","id":null,"method":"ping"}"#).unwrap();
    assert_eq!(null_id.id, None);

    // Go decodes any valid JSON object, absent fields keeping zero values.
    let bare = decode_inbound_message(br#"{"id":1}"#).unwrap();
    assert_eq!(bare.jsonrpc, "");
    assert_eq!(bare.method, "");

    assert!(decode_inbound_message(b"not json").is_none());
    assert!(decode_inbound_message(b"[1,2]").is_none());
}

#[test]
fn notification_id_predicate_covers_empty_and_null() {
    assert!(is_notification_id(&json!(null)));
    assert!(is_notification_id(&json!("")));
    assert!(!is_notification_id(&json!("e1")));
    assert!(!is_notification_id(&json!(0)));
    assert!(!is_notification_id(&json!(1.5)));
}

#[test]
fn progress_token_key_accepts_strings_and_numbers_only() {
    assert_eq!(progress_token_key(&json!("t1")), "t1");
    assert_eq!(progress_token_key(&json!(42)), "42");
    assert_eq!(progress_token_key(&json!(1.5)), "1.5");
    assert_eq!(progress_token_key(&json!(true)), "");
    assert_eq!(progress_token_key(&json!(null)), "");
}

#[test]
fn format_mcp_progress_byte_exact_table() {
    assert_eq!(
        format_mcp_progress("step", Some(1.0), Some(2.0)),
        "step (1/2)\n"
    );
    assert_eq!(format_mcp_progress("step", Some(3.0), None), "step (3)\n");
    assert_eq!(format_mcp_progress("step", None, None), "step\n");
    assert_eq!(format_mcp_progress("  hello  ", None, None), "hello\n");
    assert_eq!(format_mcp_progress("", None, None), "MCP progress\n");
    assert_eq!(
        format_mcp_progress("   ", Some(2.0), Some(4.0)),
        "MCP progress (2/4)\n"
    );
}

#[test]
fn server_request_replies_ping_roots_and_method_not_found() {
    // Field order jsonrpc,id,result,error; the absent side is omitted.
    let ping = server_request_reply(json!("srv-1"), "ping", vec![]);
    assert_eq!(
        ping.to_json(),
        r#"{"jsonrpc":"2.0","id":"srv-1","result":{}}"#
    );

    let roots = server_request_reply(
        json!(7),
        "roots/list",
        vec![McpRoot {
            uri: "file:///work".into(),
            name: "work".into(),
        }],
    );
    assert_eq!(
        roots.to_json(),
        r#"{"jsonrpc":"2.0","id":7,"result":{"roots":[{"uri":"file:///work","name":"work"}]}}"#
    );

    let unknown = server_request_reply(json!(3), "unknown", vec![]);
    assert_eq!(
        unknown.to_json(),
        r#"{"jsonrpc":"2.0","id":3,"error":{"code":-32601,"message":"Method not found"}}"#
    );
}

#[test]
fn inbound_message_serializes_like_go_struct_without_omitempty() {
    // Go's inboundMessage struct emits all four fields, nulls included.
    let msg = InboundMessage {
        jsonrpc: "2.0".into(),
        id: Some(json!("server-roots")),
        method: "roots/list".into(),
        params: None,
    };
    assert_eq!(
        to_string(&msg).unwrap(),
        r#"{"jsonrpc":"2.0","id":"server-roots","method":"roots/list","params":null}"#
    );
}

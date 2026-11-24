//! Real-behavior Streamable HTTP transport tests (MCP-010).
//!
//! Every test drives a real loopback peer over hand-written HTTP/1.1; there is
//! no in-memory transport double. Coverage: JSON single reply, SSE stream
//! with interleaved notification and a server request answered by a second
//! POST, session capture/echo, protocol-version header timing, typed status
//! failures with RPC extraction, 404 session expiry, cross-origin redirect
//! refusal, the 16 MiB body bound, notifications, DELETE close, and the
//! reserved GET SSE stream.

mod support;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};
use std::time::Duration;

use reasonix_ext::mcp::{
    Era, StreamableHttpConfig, StreamableHttpTransport, Transport, TransportError,
};
use serde_json::{Value, json};
use support::http_peer::{HttpPeer, PeerConn, PeerRequest};

fn connect_transport(url: &str, headers: BTreeMap<String, String>) -> StreamableHttpTransport {
    StreamableHttpTransport::connect(StreamableHttpConfig {
        url: url.to_string(),
        headers,
        era: Era::legacy(),
        call_timeout: Some(Duration::from_secs(10)),
    })
    .expect("connect transport")
}

fn body_json(body: &[u8]) -> Value {
    serde_json::from_slice(body).unwrap_or(Value::Null)
}

fn rpc_response(id: &Value, result: Value) -> Vec<u8> {
    serde_json::to_vec(&json!({"jsonrpc": "2.0", "id": id, "result": result}))
        .expect("response json")
}

#[test]
fn post_json_single_response() {
    let peer = HttpPeer::new(|req: &PeerRequest, conn: &mut PeerConn| {
        let body = body_json(&req.body);
        let id = body.get("id").cloned().unwrap_or(Value::Null);
        conn.respond(200, &[], &rpc_response(&id, json!({"pong": true})));
    });
    let transport = connect_transport(peer.base_url(), BTreeMap::new());

    let result = transport.call("ping", None).expect("json reply");
    assert_eq!(result["pong"], json!(true));

    let requests = peer.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "POST");
    assert_eq!(requests[0].header("content-type"), Some("application/json"));
    assert!(
        requests[0]
            .header("accept")
            .unwrap_or_default()
            .contains("text/event-stream")
    );
}

#[test]
fn sse_stream_skips_notification_and_replies_server_request() {
    let peer = HttpPeer::new(|req: &PeerRequest, conn: &mut PeerConn| {
        let body = body_json(&req.body);
        // A server-request reply carries no method: answer it 202 and record it.
        if body.get("method").is_none() {
            conn.respond(202, &[], b"");
            return;
        }
        let id = body.get("id").cloned().unwrap_or(Value::Null);
        let mut sse = conn.start_sse();
        sse.send_event(
            "message",
            r#"{"jsonrpc":"2.0","method":"notifications/message","params":{"text":"hi"}}"#,
        );
        sse.send_event("message", r#"{"jsonrpc":"2.0","id":7001,"method":"ping"}"#);
        sse.send_event(
            "message",
            &serde_json::to_string(&json!({"jsonrpc": "2.0", "id": id, "result": {"echo": "ok"}}))
                .expect("final event"),
        );
    });
    let transport = connect_transport(peer.base_url(), BTreeMap::new());

    let result = transport.call("tools/call", None).expect("sse reply");
    assert_eq!(result["echo"], json!("ok"));

    let replies: Vec<_> = peer
        .requests()
        .into_iter()
        .filter(|req| body_json(&req.body).get("method").is_none())
        .collect();
    assert_eq!(
        replies.len(),
        1,
        "server request must be answered by a second POST"
    );
    let reply_body = body_json(&replies[0].body);
    assert_eq!(reply_body["id"], json!(7001));
    assert_eq!(reply_body["result"], json!({}));
}

#[test]
fn session_id_captured_and_echoed() {
    let peer = HttpPeer::new(|req: &PeerRequest, conn: &mut PeerConn| {
        let body = body_json(&req.body);
        let method = body
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if method == "initialize" {
            conn.respond(
                200,
                &[("Mcp-Session-Id", "sess-xyz")],
                &rpc_response(&body["id"], json!({"protocolVersion": "2025-11-25"})),
            );
            return;
        }
        if req.header("mcp-session-id") != Some("sess-xyz") {
            conn.respond(
                400,
                &[],
                br#"{"jsonrpc":"2.0","error":{"code":-32002,"message":"missing session"}}"#,
            );
            return;
        }
        conn.respond(200, &[], &rpc_response(&body["id"], json!({"ok": true})));
    });
    let transport = connect_transport(peer.base_url(), BTreeMap::new());

    transport.call("initialize", None).expect("initialize");
    assert_eq!(transport.session_id(), Some("sess-xyz".to_string()));

    transport.call("tools/list", None).expect("session echoed");
    let requests = peer.requests();
    assert_eq!(requests[1].header("mcp-session-id"), Some("sess-xyz"));
}

#[test]
fn protocol_version_header_only_after_negotiation() {
    let peer = HttpPeer::new(|req: &PeerRequest, conn: &mut PeerConn| {
        let body = body_json(&req.body);
        conn.respond(200, &[], &rpc_response(&body["id"], json!({})));
    });
    let transport = connect_transport(peer.base_url(), BTreeMap::new());

    transport.call("ping", None).expect("first call");
    assert!(peer.requests()[0].header("mcp-protocol-version").is_none());

    transport.set_protocol_version("2026-07-28");
    transport.call("ping", None).expect("second call");
    assert_eq!(
        peer.requests()[1].header("mcp-protocol-version"),
        Some("2026-07-28")
    );
}

#[test]
fn non_2xx_failure_keeps_status_and_extracts_rpc_error() {
    let peer = HttpPeer::new(|_req: &PeerRequest, conn: &mut PeerConn| {
        conn.respond(
            400,
            &[],
            br#"{"jsonrpc":"2.0","id":1,"error":{"code":-32000,"message":"boom"}}"#,
        );
    });
    let transport = connect_transport(peer.base_url(), BTreeMap::new());

    let err = transport.call("ping", None).expect_err("status failure");
    let message = format!("{err}");
    match &err {
        TransportError::Rpc(rpc) => {
            assert_eq!(rpc.code, -32000);
            assert!(!rpc.diagnostic().contains("boom"));
        }
        other => panic!("expected typed RPC error, got {other:?}"),
    }
    assert!(!message.contains("boom"));
}

#[test]
fn expired_session_on_404_with_held_session() {
    let peer = HttpPeer::new(|req: &PeerRequest, conn: &mut PeerConn| {
        let body = body_json(&req.body);
        let method = body
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if method == "initialize" {
            conn.respond(
                200,
                &[("Mcp-Session-Id", "sess-xyz")],
                &rpc_response(&body["id"], json!({})),
            );
            return;
        }
        conn.respond(404, &[], b"not found");
    });
    let transport = connect_transport(peer.base_url(), BTreeMap::new());

    transport.call("initialize", None).expect("initialize");
    let err = transport.call("ping", None).expect_err("expired session");
    assert!(matches!(err, TransportError::Io(_)));
    assert!(format!("{err}").to_lowercase().contains("session"));
    assert_eq!(transport.session_id(), None);

    transport.close();
    let deleted: Vec<_> = peer
        .requests()
        .into_iter()
        .filter(|req| req.method == "DELETE")
        .collect();
    assert!(
        deleted
            .iter()
            .all(|req| req.header("mcp-session-id").is_none())
    );
}

#[test]
fn cross_origin_redirect_is_refused_and_target_not_hit() {
    let hits = Arc::new(AtomicI32::new(0));
    let target = {
        let hits = hits.clone();
        HttpPeer::new(move |_req: &PeerRequest, conn: &mut PeerConn| {
            hits.fetch_add(1, Ordering::SeqCst);
            conn.respond(200, &[], b"{}");
        })
    };
    let source = HttpPeer::new({
        let target_url = target.base_url().to_string();
        move |_req: &PeerRequest, conn: &mut PeerConn| {
            conn.respond(307, &[("Location", &format!("{target_url}/mcp"))], b"");
        }
    });
    let transport = connect_transport(source.base_url(), BTreeMap::new());

    let err = transport
        .call("ping", None)
        .expect_err("cross-origin refused");
    assert!(format!("{err}").to_lowercase().contains("cross-origin"));
    assert_eq!(hits.load(Ordering::SeqCst), 0);
}

#[test]
fn body_over_16_mib_is_rejected() {
    let limit: usize = 16 * 1024 * 1024;
    let peer = HttpPeer::new(move |_req: &PeerRequest, conn: &mut PeerConn| {
        conn.respond(200, &[], &vec![b'x'; limit + 1]);
    });
    let transport = connect_transport(peer.base_url(), BTreeMap::new());

    let err = transport.call("ping", None).expect_err("body bound");
    assert!(format!("{err}").contains("16 MiB"));
}

#[test]
fn notification_posts_accepted_without_id() {
    let peer = HttpPeer::new(|_req: &PeerRequest, conn: &mut PeerConn| {
        conn.respond(202, &[], b"");
    });
    let transport = connect_transport(peer.base_url(), BTreeMap::new());

    let params = json!({});
    transport
        .notify("notifications/initialized", Some(&params))
        .expect("notification accepted");

    let requests = peer.requests();
    assert_eq!(requests.len(), 1);
    let body = body_json(&requests[0].body);
    assert!(body.get("id").is_none());
    assert_eq!(body["method"], json!("notifications/initialized"));
}

#[test]
fn close_sends_delete_echoing_session() {
    let peer = HttpPeer::new(|req: &PeerRequest, conn: &mut PeerConn| {
        let body = body_json(&req.body);
        if body.get("method").and_then(Value::as_str) == Some("initialize") {
            conn.respond(
                200,
                &[("Mcp-Session-Id", "sess-xyz")],
                &rpc_response(&body["id"], json!({})),
            );
            return;
        }
        conn.respond(202, &[], b"");
    });
    let transport = connect_transport(peer.base_url(), BTreeMap::new());
    transport.call("initialize", None).expect("initialize");

    transport.close();
    let deleted: Vec<_> = peer
        .requests()
        .into_iter()
        .filter(|req| req.method == "DELETE")
        .collect();
    assert!(!deleted.is_empty());
    assert_eq!(deleted[0].header("mcp-session-id"), Some("sess-xyz"));
}

#[test]
fn get_event_stream_opens_long_lived_sse_get() {
    use reasonix_ext::mcp::HttpClient;
    use reasonix_ext::mcp::HttpClientConfig;

    let peer = HttpPeer::new(|_req: &PeerRequest, conn: &mut PeerConn| {
        let mut sse = conn.start_sse();
        sse.send_event("endpoint", "/mcp");
        sse.send_event(
            "message",
            r#"{"jsonrpc":"2.0","id":1,"result":{"ok":true}}"#,
        );
    });
    let client = HttpClient::new(HttpClientConfig {
        url: peer.base_url().to_string(),
        headers: BTreeMap::new(),
        call_timeout: Duration::from_secs(10),
    })
    .expect("http client");

    let mut stream = client
        .get_event_stream(&[], Duration::from_secs(10))
        .expect("sse stream");
    let endpoint = stream.next_event().expect("endpoint event").expect("event");
    assert_eq!(endpoint.event, "endpoint");
    assert_eq!(endpoint.data, "/mcp");
    let message = stream.next_event().expect("message event").expect("event");
    assert_eq!(message.event, "message");
    assert!(message.data.contains("\"ok\":true"));
    stream.close();

    assert!(peer.requests().iter().any(|req| req.method == "GET"));
}

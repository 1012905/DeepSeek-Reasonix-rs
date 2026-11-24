//! Real-behavior legacy HTTP+SSE transport tests (MCP-012, contract §6).
//!
//! Every test drives a real loopback peer over hand-written HTTP/1.1: one
//! long-lived GET stream announces the POST endpoint, JSON-RPC calls go to
//! that endpoint, and replies travel back on the GET stream. Coverage:
//! endpoint-announced POST routing with request body shape, cross-origin
//! endpoint refusal with zero target hits, non-SSE content-type rejection,
//! stream-end-before-endpoint, concurrent call demux by id, read-error latch
//! with fast-fail after death, the bounded worker answering server requests on
//! a second POST, and idempotent close releasing pending calls.

mod support;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};
use std::time::Duration;

use reasonix_ext::mcp::{
    Era, LegacySseConfig, LegacySseTransport, OwnedTransport, Transport, TransportError,
};
use serde_json::{Value, json};
use support::http_peer::{HttpPeer, PeerConn, PeerRequest, SseBus};

fn connect(url: &str, headers: BTreeMap<String, String>) -> LegacySseTransport {
    LegacySseTransport::connect(LegacySseConfig {
        url: url.to_string(),
        headers,
        era: Era::legacy(),
        call_timeout: Some(Duration::from_secs(10)),
        workspace_root: String::new(),
    })
    .expect("connect sse transport")
}

fn body_json(body: &[u8]) -> Value {
    serde_json::from_slice(body).unwrap_or(Value::Null)
}

fn response_frame(id: &Value, result: Value) -> String {
    serde_json::to_string(&json!({"jsonrpc": "2.0", "id": id, "result": result}))
        .expect("response frame json")
}

#[test]
fn posts_to_announced_endpoint_and_replies_on_get_stream() {
    let bus = Arc::new(SseBus::new());
    let peer = {
        let bus = bus.clone();
        HttpPeer::new(move |req: &PeerRequest, conn: &mut PeerConn| {
            if req.method == "GET" {
                let mut sse = conn.start_sse();
                sse.send_event("endpoint", "/messages?session=test");
                while let Some(frame) = bus.drain_next() {
                    sse.send_event(&frame.event, &frame.data);
                }
                return;
            }
            let body = body_json(&req.body);
            let id = body.get("id").cloned().unwrap_or(Value::Null);
            bus.push("message", &response_frame(&id, json!({"pong": true})));
            conn.respond(202, &[], b"");
        })
    };

    let transport = connect(peer.base_url(), BTreeMap::new());
    let result = transport.call("ping", None).expect("sse reply");
    assert_eq!(result["pong"], json!(true));

    let posts: Vec<_> = peer
        .requests()
        .into_iter()
        .filter(|request| request.method == "POST")
        .collect();
    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0].path, "/messages?session=test");
    assert_eq!(posts[0].header("content-type"), Some("application/json"));
    let body = body_json(&posts[0].body);
    assert_eq!(body["jsonrpc"], json!("2.0"));
    assert_eq!(body["method"], json!("ping"));
    assert!(body["id"].is_number());
}

#[test]
fn cross_origin_endpoint_is_refused_and_target_not_hit() {
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
            let mut sse = conn.start_sse();
            sse.send_event("endpoint", &format!("{target_url}/messages"));
        }
    });

    let transport = connect(source.base_url(), BTreeMap::new());
    let err = transport
        .call("initialize", None)
        .expect_err("cross-origin endpoint refused");
    assert!(format!("{err}").contains("cross-origin endpoint"));
    assert_eq!(hits.load(Ordering::SeqCst), 0);
}

#[test]
fn non_sse_content_type_is_rejected() {
    let peer = HttpPeer::new(|_req: &PeerRequest, conn: &mut PeerConn| {
        conn.respond(200, &[], br#"{"ok":true}"#);
    });

    let transport = connect(peer.base_url(), BTreeMap::new());
    let err = transport
        .call("ping", None)
        .expect_err("content type rejected");
    assert!(format!("{err}").contains("unexpected content type"));

    // The failure latches: a later call fails fast instead of reopening.
    let err = transport.call("ping", None).expect_err("latched failure");
    assert!(!format!("{err}").is_empty());
}

#[test]
fn stream_ends_before_endpoint_announced() {
    let peer = HttpPeer::new(|_req: &PeerRequest, conn: &mut PeerConn| {
        let mut sse = conn.start_sse();
        sse.comment("warming up");
        // No `endpoint` frame; the handler returns and the stream closes.
    });

    let transport = connect(peer.base_url(), BTreeMap::new());
    let err = transport.call("ping", None).expect_err("endpoint missing");
    assert!(format!("{err}").contains("ended before announcing an endpoint"));
}

#[test]
fn concurrent_calls_demux_by_id() {
    let bus = Arc::new(SseBus::new());
    let peer = {
        let bus = bus.clone();
        HttpPeer::new(move |req: &PeerRequest, conn: &mut PeerConn| {
            if req.method == "GET" {
                let mut sse = conn.start_sse();
                sse.send_event("endpoint", "/messages");
                while let Some(frame) = bus.drain_next() {
                    sse.send_event(&frame.event, &frame.data);
                }
                return;
            }
            let body = body_json(&req.body);
            let id = body.get("id").cloned().unwrap_or(Value::Null);
            let echo = body
                .get("params")
                .and_then(|params| params.get("echo"))
                .cloned()
                .unwrap_or(Value::Null);
            bus.push("message", &response_frame(&id, json!({"echo": echo})));
            conn.respond(202, &[], b"");
        })
    };

    let transport = Arc::new(connect(peer.base_url(), BTreeMap::new()));
    let mut threads = Vec::new();
    for index in 0..2 {
        let transport = transport.clone();
        threads.push(std::thread::spawn(move || {
            transport.call("ping", Some(&json!({"echo": index})))
        }));
    }
    let results: Vec<_> = threads
        .into_iter()
        .map(|thread| thread.join().unwrap())
        .collect();
    let echoes: Vec<i64> = results
        .iter()
        .map(|result| result.as_ref().unwrap()["echo"].as_i64().unwrap())
        .collect();
    assert!(echoes.contains(&0));
    assert!(echoes.contains(&1));
}

#[test]
fn read_error_latches_pending_and_later_calls_fail_fast() {
    let peer = HttpPeer::new(|req: &PeerRequest, conn: &mut PeerConn| {
        if req.method == "GET" {
            let mut sse = conn.start_sse();
            sse.send_event("endpoint", "/messages");
            // Hold the stream briefly, then close it out from under the
            // in-flight call.
            std::thread::sleep(Duration::from_millis(300));
            return;
        }
        // The POST is accepted but the GET stream never carries a reply.
        conn.respond(202, &[], b"");
    });

    let transport = Arc::new(connect(peer.base_url(), BTreeMap::new()));
    let blocked = {
        let transport = transport.clone();
        std::thread::spawn(move || transport.call("ping", None))
    };
    support::wait_until("post arrived", Duration::from_secs(2), || {
        peer.requests()
            .iter()
            .any(|request| request.method == "POST")
    });

    let err = blocked
        .join()
        .unwrap()
        .expect_err("pending released on read error");
    assert!(!format!("{err}").is_empty());

    let err = transport
        .call("ping", None)
        .expect_err("latched fast failure");
    assert!(matches!(err, TransportError::Gone | TransportError::Io(_)));
}

#[test]
fn server_request_is_answered_by_worker_second_post() {
    let bus = Arc::new(SseBus::new());
    let peer = {
        let bus = bus.clone();
        HttpPeer::new(move |req: &PeerRequest, conn: &mut PeerConn| {
            if req.method == "GET" {
                let mut sse = conn.start_sse();
                sse.send_event("endpoint", "/messages");
                while let Some(frame) = bus.drain_next() {
                    sse.send_event(&frame.event, &frame.data);
                }
                return;
            }
            let body = body_json(&req.body);
            if body.get("method").is_none() {
                // This is the worker's reply to a server request; ack it.
                conn.respond(202, &[], b"");
                return;
            }
            let id = body.get("id").cloned().unwrap_or(Value::Null);
            bus.push("message", &response_frame(&id, json!({"ok": true})));
            conn.respond(202, &[], b"");
        })
    };

    let transport = connect(peer.base_url(), BTreeMap::new());
    bus.push(
        "message",
        r#"{"jsonrpc":"2.0","id":"srv-1","method":"ping"}"#,
    );

    support::wait_until("reply post recorded", Duration::from_secs(2), || {
        peer.requests().iter().any(|request| {
            let body = body_json(&request.body);
            body["id"] == json!("srv-1") && body.get("method").is_none()
        })
    });

    let replies: Vec<_> = peer
        .requests()
        .into_iter()
        .filter(|request| body_json(&request.body)["id"] == json!("srv-1"))
        .collect();
    assert_eq!(replies.len(), 1);
    let reply_body = body_json(&replies[0].body);
    assert_eq!(reply_body["result"], json!({}));

    // An ordinary call still routes over the same stream after the reply.
    let result = transport.call("ping", None).expect("call after reply");
    assert_eq!(result["ok"], json!(true));
}

#[test]
fn close_is_idempotent_and_releases_pending() {
    let bus = Arc::new(SseBus::new());
    let peer = {
        let bus = bus.clone();
        HttpPeer::new(move |req: &PeerRequest, conn: &mut PeerConn| {
            if req.method == "GET" {
                let mut sse = conn.start_sse();
                sse.send_event("endpoint", "/messages");
                while let Some(frame) = bus.drain_next() {
                    sse.send_event(&frame.event, &frame.data);
                }
                return;
            }
            // Never answers on the GET stream.
            conn.respond(202, &[], b"");
        })
    };

    let transport = Arc::new(connect(peer.base_url(), BTreeMap::new()));
    let blocked = {
        let transport = transport.clone();
        std::thread::spawn(move || transport.call("ping", None))
    };
    support::wait_until("post arrived", Duration::from_secs(2), || {
        peer.requests()
            .iter()
            .any(|request| request.method == "POST")
    });

    transport.close();
    transport.close();

    let outcome = blocked.join().unwrap();
    assert!(outcome.is_err());
}

#[test]
fn deadline_exceeded_sends_cancelled_notification() {
    let bus = Arc::new(SseBus::new());
    let peer = {
        let bus = bus.clone();
        HttpPeer::new(move |req: &PeerRequest, conn: &mut PeerConn| {
            if req.method == "GET" {
                let mut sse = conn.start_sse();
                sse.send_event("endpoint", "/messages");
                while let Some(frame) = bus.drain_next() {
                    sse.send_event(&frame.event, &frame.data);
                }
                return;
            }
            conn.respond(202, &[], b"");
        })
    };

    let transport = LegacySseTransport::connect(LegacySseConfig {
        url: peer.base_url().to_string(),
        headers: BTreeMap::new(),
        era: Era::legacy(),
        call_timeout: Some(Duration::from_secs(1)),
        workspace_root: String::new(),
    })
    .expect("connect sse transport");

    let err = transport
        .call("ping", None)
        .expect_err("call deadline exceeded");
    assert!(matches!(err, TransportError::NoReply));

    support::wait_until("cancel notification posted", Duration::from_secs(2), || {
        peer.requests()
            .iter()
            .any(|request| body_json(&request.body)["method"] == json!("notifications/cancelled"))
    });
}

//! Transport-level oracles against a local tokio TCP listener speaking canned
//! HTTP+SSE (Go: `TestStreamRetriesThenSucceeds` minus the retry ladder,
//! `TestStreamInsufficientBalance`, `TestStreamAuthError`,
//! `TestStreamSendsCustomHeaders`, stall tests). No live network.

use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use reasonix_provider::ChatCompletionRequest;
use reasonix_provider::config::{ClientConfig, build_client};
use reasonix_provider::error::ProviderError;
use reasonix_provider::stream::Chunk;

/// HTTP/1.1 response with chunked transfer framing around the given frames.
fn chunked_sse(frames: &[&str]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(
        b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n",
    );
    for frame in frames {
        let line = format!("data: {frame}\n\n");
        out.extend_from_slice(format!("{:x}\r\n", line.len()).into_bytes().as_slice());
        out.extend_from_slice(line.as_bytes());
        out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(b"0\r\n\r\n");
    out
}

/// HTTP/1.1 response with a fixed Content-Length body.
fn full(status: &str, content_type: &str, body: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}

/// One-shot HTTP server on 127.0.0.1; answers every connection it accepts
/// once, then closes.
async fn canned_server(response: Vec<u8>) -> (String, tokio::sync::oneshot::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().unwrap();
    let (tx, rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.expect("accept");
        let mut buf = vec![0u8; 65536];
        let n = sock.read(&mut buf).await.expect("read request");
        let _ = tx.send(String::from_utf8_lossy(&buf[..n]).into_owned());
        sock.write_all(&response).await.ok();
        sock.shutdown().await.ok();
    });
    (format!("http://{addr}"), rx)
}

fn client_for(base: &str) -> reasonix_provider::Client {
    let mut cfg = ClientConfig::new("relay", base, "m");
    cfg.api_key = "k".to_owned();
    build_client(&cfg).expect("build client")
}

fn req() -> ChatCompletionRequest {
    ChatCompletionRequest {
        messages: vec![reasonix_contract::provider::Message {
            role: reasonix_contract::provider::Role::USER,
            content: "hi".to_owned(),
            ..Default::default()
        }],
        ..ChatCompletionRequest::default()
    }
}

// Go TestStreamRetriesThenSucceeds's stream half: a 200 SSE stream decodes
// end to end through the real transport.
#[tokio::test]
async fn streams_a_canned_sse_response() {
    let response = chunked_sse(&[
        r#"{"choices":[{"delta":{"content":"hi there"}}]}"#,
        r#"{"choices":[],"usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}}"#,
        "[DONE]",
    ]);
    let (base, inspector) = canned_server(response).await;
    let c = client_for(&base);
    let mut stream = c.stream(&req()).await.expect("stream");
    let mut text = String::new();
    let mut usage_prompt = 0i64;
    let mut saw_done = false;
    while let Some(chunk) = stream.next().await {
        match chunk.expect("no stream error") {
            Chunk::Text(t) => text.push_str(&t),
            Chunk::Usage(u) => usage_prompt = u.prompt_tokens,
            Chunk::Done => saw_done = true,
            _ => {}
        }
    }
    assert_eq!(text, "hi there");
    assert_eq!(usage_prompt, 3);
    assert!(saw_done);
    let request = inspector.await.expect("server saw request");
    assert!(request.contains("POST /chat/completions"), "{request}");
    assert!(
        request
            .to_ascii_lowercase()
            .contains("authorization: bearer k"),
        "{request}"
    );
    assert!(
        request
            .to_ascii_lowercase()
            .contains("accept: text/event-stream"),
        "{request}"
    );
    assert!(
        request
            .to_ascii_lowercase()
            .contains("content-type: application/json"),
        "{request}"
    );
    // stream_options always on for the streaming path.
    assert!(
        request.contains(r#""stream_options":{"include_usage":true}"#),
        "{request}"
    );
}

// Go TestStreamInsufficientBalance: a 402 is a typed API error carrying the
// status, and (without the deferred retry ladder) makes exactly one request.
#[tokio::test]
async fn insufficient_balance_is_a_typed_api_error() {
    let response = full(
        "402 Payment Required",
        "application/json",
        r#"{"error":"Insufficient Balance"}"#,
    );
    let (base, _inspector) = canned_server(response).await;
    let c = client_for(&base);
    let err = c.stream(&req()).await.expect_err("402 must fail");
    match &err {
        ProviderError::Api { status, body, .. } => {
            assert_eq!(*status, 402);
            assert!(body.contains("Insufficient Balance"), "{body}");
        }
        other => panic!("want Api(402), got {other:?}"),
    }
}

// Go TestStreamAuthError: a 401 is an actionable AuthError naming the key
// env var and never dumping the raw body.
#[tokio::test]
async fn auth_error_names_the_key_env_and_hides_the_body() {
    let response = full(
        "401 Unauthorized",
        "application/json",
        r#"{"error":{"message":"Authentication Fails, Your api key: ****ae54 is invalid"}}"#,
    );
    let (base, inspector) = canned_server(response).await;
    let mut cfg = ClientConfig::new("deepseek", &base, "deepseek-v4");
    cfg.api_key = "bad".to_owned();
    cfg.key_env = "DEEPSEEK_API_KEY".to_owned();
    let c = build_client(&cfg).unwrap();
    let err = c.stream(&req()).await.expect_err("401 must fail");
    let ProviderError::Auth {
        key_env,
        status,
        has_key,
        body,
        ..
    } = &err
    else {
        panic!("want AuthError, got {err:?}")
    };
    assert_eq!(key_env, "DEEPSEEK_API_KEY");
    assert_eq!(*status, 401);
    assert!(*has_key);
    assert!(body.contains("ae54"), "body kept for display: {body}");
    let msg = err.to_string();
    assert!(msg.contains("DEEPSEEK_API_KEY"), "{msg}");
    assert!(
        !msg.contains("ae54"),
        "message must not dump the raw body: {msg}"
    );
    let request = inspector.await.expect("server saw request");
    assert!(
        request
            .to_ascii_lowercase()
            .contains("authorization: bearer bad"),
        "{request}"
    );
}

// Go TestStreamSendsCustomHeaders: reserved headers are preserved, custom
// ones ride through, empty/reserved config headers are dropped.
#[tokio::test]
async fn custom_headers_ride_and_reserved_ones_are_preserved() {
    let response = chunked_sse(&[r#"{"choices":[{"delta":{"content":"ok"}}]}"#, "[DONE]"]);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (tx, rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        let mut buf = vec![0u8; 65536];
        let n = sock.read(&mut buf).await.unwrap();
        let _ = tx.send(String::from_utf8_lossy(&buf[..n]).into_owned());
        sock.write_all(&response).await.ok();
        sock.shutdown().await.ok();
    });
    let mut cfg = ClientConfig::new("custom", &format!("http://{addr}"), "model-a");
    cfg.api_key = "real-key".to_owned();
    cfg.headers
        .insert("HTTP-Referer".into(), "https://app.example".into());
    cfg.headers.insert("X-Title".into(), "Reasonix".into());
    cfg.headers
        .insert("Authorization".into(), "Bearer wrong".into());
    cfg.headers
        .insert("Accept".into(), "application/json".into());
    cfg.headers.insert("   ".into(), "dropped".into());
    let c = build_client(&cfg).unwrap();
    let mut stream = c.stream(&req()).await.expect("stream");
    while let Some(chunk) = stream.next().await {
        chunk.expect("no error");
    }
    let request = rx.await.unwrap().to_ascii_lowercase();
    assert!(
        request.contains("authorization: bearer real-key"),
        "{request}"
    );
    assert!(
        request.contains("http-referer: https://app.example"),
        "{request}"
    );
    assert!(request.contains("x-title: reasonix"), "{request}");
    assert!(request.contains("accept: text/event-stream"), "{request}");
    assert!(
        !request.contains("dropped"),
        "empty header name leaked: {request}"
    );
}

// Go idle watchdog (stall_test.go/idle_timeout_test.go behavior): a stream
// that sends headers then goes silent past idle_timeout_seconds is an
// idle_timeout interruption the caller may replay.
#[tokio::test]
async fn stalled_stream_is_an_idle_timeout_interrupt() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        let mut buf = vec![0u8; 65536];
        let _ = sock.read(&mut buf).await;
        sock.write_all(
            b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n",
        )
        .await
        .ok();
        // Headers sent; the body then goes silent for the rest of the test.
        tokio::time::sleep(Duration::from_secs(30)).await;
    });
    let mut cfg = ClientConfig::new("relay", &format!("http://{addr}"), "m");
    cfg.idle_timeout_seconds = 1;
    let c = build_client(&cfg).unwrap();
    let mut stream = c.stream(&req()).await.expect("headers ok");
    let err = stream.next().await.expect("an item").expect_err("stall");
    assert_eq!(
        err.stream_interrupt_reason(),
        reasonix_provider::error::STREAM_INTERRUPT_IDLE_TIMEOUT
    );
    assert!(err.to_string().contains("stream stalled"), "{err}");
}

// Go h2_reset_test/ReadCut behavior: a body cut after headers is a
// premature_eof or connection_reset interruption.
#[tokio::test]
async fn mid_stream_cut_is_a_typed_interrupt() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        let mut buf = vec![0u8; 65536];
        let _ = sock.read(&mut buf).await;
        // A partial chunk then a hard close — the body never completes.
        sock.write_all(
            b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\nff\r\ndata: {\"cho",
        )
        .await
        .ok();
        sock.shutdown().await.ok();
    });
    let c = client_for(&format!("http://{addr}"));
    let mut stream = c.stream(&req()).await.expect("headers ok");
    let err = stream.next().await.expect("an item").expect_err("cut");
    assert!(
        err.is_stream_interrupted(),
        "a body cut must be a replayable interruption: {err:?}"
    );
}

// The non-streaming path: stream:false on the wire, one-shot JSON decoded.
#[tokio::test]
async fn non_streaming_completion_round_trip() {
    let body = r#"{"choices":[{"message":{"content":"all done"},"finish_reason":"stop"}],"usage":{"prompt_tokens":7,"completion_tokens":3,"total_tokens":10}}"#;
    let response = full("200 OK", "application/json", body);
    let (base, inspector) = canned_server(response).await;
    let c = client_for(&base);
    let out = c.complete(&req()).await.expect("complete");
    assert_eq!(out.text, "all done");
    assert_eq!(out.finish_reason, "stop");
    assert_eq!(out.usage.prompt_tokens, 7);
    let request = inspector.await.unwrap();
    assert!(request.contains(r#""stream":false"#), "{request}");
    assert!(!request.contains("stream_options"), "{request}");
    assert!(
        request
            .to_ascii_lowercase()
            .contains("accept: application/json"),
        "{request}"
    );
}

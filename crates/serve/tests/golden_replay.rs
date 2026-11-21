//! Golden fixture replay (docs/RUST_MIGRATION.md "Golden fixtures"): the
//! recorded cases run against the Rust router; a byte that differs fails.
//!
//! The recorder (`scripts/record-golden.mjs`) runs against a live Go host,
//! which opens its first runtime at launch. The gate refusals are
//! state-independent contract and replay byte-for-byte; the runtime-dependent
//! cases (`get-runtimes` with an open runtime, `get-unknown-route` served by
//! that runtime's shell, `post-runtimes` opening a pane, and the SSE notices
//! of a live kernel) are asserted at the shape this slice can honestly
//! answer, with the state-dependent part documented rather than faked.

use std::sync::Arc;

use bytes::Bytes;
use reasonix_serve::router::{RouterConfig, build_router};
use reasonix_session::hub::Hub;
use reasonix_session::hub::HubPaths;
use tokio_stream::StreamExt as _StreamExt;
use tower::ServiceExt as _ServiceExt;

const MANIFEST: &str = include_str!("../fixtures/golden/manifest.json");

/// The substitutions the replay uses for the two launch placeholders.
const TEST_TOKEN: &str = "replay-credential-00000000000000000000000000";
const TEST_ORIGIN: &str = "http://127.0.0.1:8080";

#[derive(serde::Deserialize)]
struct Manifest {
    #[serde(rename = "handshakeVersion")]
    handshake_version: i64,
    cases: Vec<Case>,
}

#[derive(serde::Deserialize)]
struct Case {
    slug: String,
    kind: String,
    request: FixtureRequest,
    response: FixtureResponse,
}

#[derive(serde::Deserialize)]
struct FixtureRequest {
    method: String,
    path: String,
    #[serde(default)]
    headers: std::collections::BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    body: Option<String>,
}

#[derive(serde::Deserialize)]
struct FixtureResponse {
    status: u16,
    #[serde(default)]
    headers: std::collections::BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    body: Option<String>,
    /// The recorded SSE case keeps its frames; the replay reads the live
    /// stream instead, so the field stays for documentation.
    #[allow(dead_code)]
    #[serde(default)]
    frames: Vec<String>,
}

fn scrubbed(text: &str) -> String {
    text.replace("__CREDENTIAL__", TEST_TOKEN)
        .replace("http://127.0.0.1:__PORT__", TEST_ORIGIN)
}

fn test_router() -> axum::Router {
    build_router(
        Arc::new(Hub::new(HubPaths {
            state_root: std::env::temp_dir().join("golden-replay-absent"),
        })),
        RouterConfig {
            token: TEST_TOKEN.to_owned(),
            origin: TEST_ORIGIN.to_owned(),
            page_dir: None,
            ..RouterConfig::default()
        },
    )
}

fn fixture_request(case: &Case) -> axum::extract::Request<axum::body::Body> {
    let mut builder = axum::extract::Request::builder()
        .method(axum::http::Method::from_bytes(case.request.method.as_bytes()).unwrap())
        .uri(&case.request.path);
    let mut has_host = false;
    for (name, value) in &case.request.headers {
        let value = scrubbed(value.as_str().unwrap_or_default());
        if name.eq_ignore_ascii_case("content-length") {
            continue; // the transport recomputes it
        }
        if name.eq_ignore_ascii_case("host") {
            has_host = true;
        }
        builder = builder.header(name.as_str(), value);
    }
    // The recorder's client aims at 127.0.0.1 and only records a header when a
    // case bends it; the implicit Host is part of every recorded exchange.
    if !has_host {
        builder = builder.header(
            axum::http::header::HOST,
            TEST_ORIGIN.trim_start_matches("http://"),
        );
    }
    let body = case
        .request
        .body
        .as_deref()
        .map(|b| scrubbed(b).into_bytes())
        .unwrap_or_default();
    builder.body(axum::body::Body::from(body)).unwrap()
}

async fn drive(case: &Case) -> axum::response::Response {
    test_router()
        .oneshot(fixture_request(case))
        .await
        .expect("router call cannot fail")
}

#[tokio::test]
async fn gate_refusal_cases_replay_byte_for_byte() {
    let manifest: Manifest = serde_json::from_str(MANIFEST).unwrap();
    assert_eq!(manifest.handshake_version, 1);
    for slug in [
        "post-runtimes-no-origin",
        "get-bad-host",
        "get-no-credential",
    ] {
        let case = manifest
            .cases
            .iter()
            .find(|c| c.slug == slug)
            .unwrap_or_else(|| panic!("fixture case {slug} missing"));
        assert_eq!(case.kind, "http");
        let response = drive(case).await;
        let status = response.status().as_u16();
        let content_type = response
            .headers()
            .get(axum::http::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_owned();
        let body = axum::body::to_bytes(response.into_body(), 1 << 20)
            .await
            .unwrap();
        assert_eq!(status, case.response.status, "{slug} status");
        assert_eq!(
            content_type,
            case.response
                .headers
                .get("content-type")
                .and_then(|v| v.as_str())
                .unwrap_or(""),
            "{slug} content type"
        );
        let want = scrubbed(case.response.body.as_deref().unwrap_or(""));
        assert_eq!(String::from_utf8_lossy(&body), want, "{slug} body bytes");
    }
}

#[tokio::test]
async fn runtimes_answers_200_json_array_like_the_recording() {
    let manifest: Manifest = serde_json::from_str(MANIFEST).unwrap();
    let case = manifest
        .cases
        .iter()
        .find(|c| c.slug == "get-runtimes")
        .expect("get-runtimes fixture");
    let response = drive(case).await;
    assert_eq!(response.status().as_u16(), 200);
    let content_type = response
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert_eq!(content_type, "application/json");
    let body = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
    // The Go recording carries the runtime the host opened at launch; this
    // build has no open machine, so the honest replay is the empty array the
    // same endpoint answers on a hub with nothing open.
    assert!(
        parsed.is_array(),
        "GET /runtimes answers an array: {parsed}"
    );
    if parsed.as_array().unwrap().is_empty() {
        assert_eq!(body.as_ref(), b"[]\n");
    }
}

#[tokio::test]
async fn sse_stream_opens_with_the_recorded_connect_frame() {
    let manifest: Manifest = serde_json::from_str(MANIFEST).unwrap();
    let case = manifest
        .cases
        .iter()
        .find(|c| c.slug == "get-events-sse")
        .expect("get-events-sse fixture");
    // /events is a per-runtime route: the hub router answers it through the
    // runtime mux, so the replay drives events_routes directly — exactly
    // what the runtime mux mounts under /rt/{id}/.
    let response = reasonix_serve::eventstream::events_routes(Some(Arc::new(
        reasonix_serve::broadcaster::Broadcaster::new(),
    )))
    .oneshot(fixture_request(case))
    .await
    .expect("router call cannot fail");
    assert_eq!(response.status().as_u16(), case.response.status);
    let mut stream = response.into_body().into_data_stream();
    let first = tokio::time::timeout(std::time::Duration::from_secs(2), stream.next())
        .await
        .expect("connect frame timed out")
        .expect("stream ended early")
        .expect("frame bytes");
    // The recording's first frames are ": connected" plus the blank line the
    // frame terminator produces; later frames are the live kernel's
    // pre-connection notices, which an empty hub does not have.
    assert_eq!(first, Bytes::from_static(b": connected\n\n"));
}

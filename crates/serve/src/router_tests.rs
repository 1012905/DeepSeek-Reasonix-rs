use super::*;
use crate::gate::{GateInput, RefusalCode};
use crate::test_support::tempdir;
use reasonix_session::hub::HubPaths;

fn router(cfg: RouterConfig) -> Router {
    let tmp = tempdir();
    build_router(
        Arc::new(Hub::new(HubPaths {
            state_root: tmp.path().to_path_buf(),
        })),
        cfg,
    )
}

fn request_get(path: &str, origin: Option<&str>) -> Request<Body> {
    let mut builder = axum::extract::Request::builder().uri(path);
    if let Some(origin) = origin {
        builder = builder.header(axum::http::header::ORIGIN, origin);
    }
    builder.body(Body::empty()).unwrap()
}

async fn oneshot(router: Router, request: Request<Body>) -> Response {
    use tower::ServiceExt as _ServiceExt;
    router
        .oneshot(request)
        .await
        .expect("router call cannot fail")
}

async fn envelope_of(response: Response) -> (StatusCode, serde_json::Value) {
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .unwrap();
    let json = serde_json::from_slice(&body)
        .unwrap_or_else(|e| panic!("non-JSON body for status {status}: {e}; body {body:?}"));
    (status, json)
}

#[tokio::test]
async fn runtimes_answers_an_empty_array() {
    let response = oneshot(
        router(RouterConfig::default()),
        request_get("/runtimes", None),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .unwrap();
    assert_eq!(body.as_ref(), b"[]\n");
}

#[tokio::test]
async fn wrong_methods_answer_the_envelope() {
    let response = oneshot(
        router(RouterConfig {
            token: "tkn".to_owned(),
            ..RouterConfig::default()
        }),
        axum::extract::Request::builder()
            .method(Method::DELETE)
            .uri("/runtimes")
            .header(
                axum::http::header::COOKIE,
                format!("{}=tkn", crate::gate::TOKEN_COOKIE),
            )
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    let (status, json) = envelope_of(response).await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(json["code"], crate::refuse::METHOD_NOT_ALLOWED);
}

#[tokio::test]
async fn open_runtime_answers_the_unsupported_build_refusal() {
    let response = oneshot(
        router(RouterConfig {
            token: "tkn".to_owned(),
            ..RouterConfig::default()
        }),
        axum::extract::Request::builder()
            .method(Method::POST)
            .uri("/runtimes")
            .header(CONTENT_TYPE, "application/json")
            .header(
                axum::http::header::COOKIE,
                format!("{}=tkn", crate::gate::TOKEN_COOKIE),
            )
            .body(Body::from(br#"{"root":"/w"}"#.to_vec()))
            .unwrap(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        response.headers().get(CONTENT_TYPE),
        Some(&HeaderValue::from_static("text/plain; charset=utf-8"))
    );
    let body = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&body),
        "runtime open is not part of this build\n"
    );
}

#[tokio::test]
async fn open_runtime_rejects_an_unknown_body_field_as_bad_body() {
    let response = oneshot(
        router(RouterConfig {
            token: "tkn".to_owned(),
            ..RouterConfig::default()
        }),
        axum::extract::Request::builder()
            .method(Method::POST)
            .uri("/runtimes")
            .header(CONTENT_TYPE, "application/json")
            .header(
                axum::http::header::COOKIE,
                format!("{}=tkn", crate::gate::TOKEN_COOKIE),
            )
            .body(Body::from(br#"{"root":"/w","extra":1}"#.to_vec()))
            .unwrap(),
    )
    .await;
    let (status, json) = envelope_of(response).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["code"], crate::refuse::BAD_BODY);
}

// OPT-004: the cap is on the stream, not the declared length — a body
// over the 64KiB budget refuses as one kind however it announces itself.
#[tokio::test]
async fn oversized_body_refuses_as_bad_body() {
    let oversized = format!("{{\"root\":\"{}\"}}", "w".repeat(70 * 1024));
    let r = router(RouterConfig {
        token: "tkn".to_owned(),
        ..RouterConfig::default()
    });
    for builder in [
        // Lying Content-Length: the header says small, the stream is big.
        axum::extract::Request::builder()
            .method(Method::POST)
            .uri("/runtimes")
            .header(CONTENT_TYPE, "application/json")
            .header(axum::http::header::CONTENT_LENGTH, "16")
            .header(
                axum::http::header::COOKIE,
                format!("{}=tkn", crate::gate::TOKEN_COOKIE),
            )
            .body(Body::from(oversized.clone().into_bytes()))
            .unwrap(),
        // No Content-Length (chunked) with the same overrun.
        axum::extract::Request::builder()
            .method(Method::POST)
            .uri("/runtimes")
            .header(CONTENT_TYPE, "application/json")
            .header(axum::http::header::TRANSFER_ENCODING, "chunked")
            .header(
                axum::http::header::COOKIE,
                format!("{}=tkn", crate::gate::TOKEN_COOKIE),
            )
            .body(Body::from(oversized.into_bytes()))
            .unwrap(),
    ] {
        let response = oneshot(r.clone(), builder).await;
        let (status, json) = envelope_of(response).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(json["code"], crate::refuse::BAD_BODY);
    }
}

#[tokio::test]
async fn close_runtime_of_an_unknown_id_is_a_typed_404() {
    let response = oneshot(
        router(RouterConfig {
            token: "tkn".to_owned(),
            ..RouterConfig::default()
        }),
        axum::extract::Request::builder()
            .method(Method::POST)
            .uri("/runtimes/r9/close")
            .header(CONTENT_TYPE, "application/json")
            .header(
                axum::http::header::COOKIE,
                format!("{}=tkn", crate::gate::TOKEN_COOKIE),
            )
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    let (status, json) = envelope_of(response).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(json["code"], crate::refuse::NOT_FOUND);
    assert_eq!(json["params"]["kind"], "runtime");
    assert_eq!(json["params"]["name"], "r9");
}

#[tokio::test]
async fn unknown_runtime_prefix_is_a_typed_404_but_the_bare_id_falls_through() {
    let r = router(RouterConfig::default());
    let (status, json) = envelope_of(oneshot(r.clone(), request_get("/rt/r9/", None)).await).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(json["params"]["kind"], "runtime");
    let (status, json) =
        envelope_of(oneshot(r.clone(), request_get("/rt/r9/tree", None)).await).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(json["code"], crate::refuse::NOT_FOUND);
    // Without the trailing slash the request reaches routeDefault.
    let (status, json) = envelope_of(oneshot(r.clone(), request_get("/rt/r9", None)).await).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(json["code"], NO_RUNTIME_OPEN);
}

#[tokio::test]
async fn tree_answers_an_array_and_sessions_falls_to_route_default() {
    let r = router(RouterConfig::default());
    let response = oneshot(r.clone(), request_get("/tree", None)).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .unwrap();
    assert_eq!(body.as_ref(), b"[]\n");
    // The hub mux does not register /sessions (http.md §1.2); the request
    // reaches routeDefault, which answers 503 with no runtime open.
    let response = oneshot(r, request_get("/sessions", None)).await;
    let (status, json) = envelope_of(response).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(json["code"], NO_RUNTIME_OPEN);
}

#[test]
fn tree_snapshot_runs_on_the_blocking_pool() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .max_blocking_threads(1)
        .build()
        .expect("tokio runtime");
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    runtime.spawn_blocking(move || {
        started_tx.send(()).expect("signal occupied worker");
        release_rx.recv().expect("release occupied worker");
    });
    started_rx.recv().expect("blocking worker occupied");

    let mut response = Box::pin(oneshot(
        router(RouterConfig::default()),
        request_get("/tree", None),
    ));
    let waiting = runtime.block_on(async {
        tokio::time::timeout(std::time::Duration::from_millis(200), &mut response)
            .await
            .is_err()
    });
    release_tx.send(()).expect("release blocking worker");
    assert!(waiting, "tree snapshot bypassed the occupied blocking pool");

    let response = runtime.block_on(response);
    assert_eq!(response.status(), StatusCode::OK);
    let body = runtime.block_on(async {
        axum::body::to_bytes(response.into_body(), 1 << 20)
            .await
            .unwrap()
    });
    assert_eq!(body.as_ref(), b"[]\n");
}

#[tokio::test]
async fn runtime_sessions_handler_publishes_foreign_rows() {
    let tmp = tempdir();
    let workspace = tmp.path().join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let dir = reasonix_session::paths::session_dir_for(tmp.path(), &workspace.to_string_lossy());
    let id = "0123456789abcdef0123456789abcdef";
    let foreign = crate::test_support::write_v4_fixture(
        &dir,
        id,
        vec![serde_json::json!({
            "kind": "message/complete",
            "payload": {"message": {"id": "m1", "role": "user", "content": "legacy preview"}}
        })],
    );
    crate::test_support::write_v4_fixture(&dir, "1123456789abcdef0123456789abcdef", Vec::new());
    let hub = Arc::new(Hub::new(HubPaths {
        state_root: tmp.path().to_path_buf(),
    }));
    let app = Router::new()
        .route("/sessions", get(runtime_sessions))
        .with_state((hub, workspace.to_string_lossy().into_owned()));

    let (status, json) = envelope_of(oneshot(app, request_get("/sessions", None)).await).await;
    assert_eq!(status, StatusCode::OK);
    let rows = json.as_array().expect("session rows array");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["name"], format!("v4-{id}"));
    assert_eq!(rows[0]["path"], foreign.to_string_lossy().into_owned());
    assert_eq!(rows[0]["turns"], 1);
}

#[tokio::test]
async fn route_default_answers_503_and_the_events_fallback_follows() {
    let r = router(RouterConfig::default());
    for path in ["/", "/definitely-not-a-route", "/events", "/events/replay"] {
        let (status, json) = envelope_of(oneshot(r.clone(), request_get(path, None)).await).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{path}");
        assert_eq!(json["code"], NO_RUNTIME_OPEN, "{path}");
    }
}

#[tokio::test]
async fn csrf_guard_rejects_a_non_json_post() {
    let r = router(RouterConfig::default());
    let response = oneshot(
        r,
        axum::extract::Request::builder()
            .method(Method::POST)
            .uri("/runtimes")
            .header(CONTENT_TYPE, "text/plain")
            .body(Body::from("x"))
            .unwrap(),
    )
    .await;
    let (status, json) = envelope_of(response).await;
    assert_eq!(status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
    assert_eq!(json["code"], crate::refuse::BAD_CONTENT_TYPE);
}

#[tokio::test]
async fn mutation_gate_demands_the_launch_token_without_one() {
    let r = router(RouterConfig {
        token: "tkn".to_owned(),
        ..RouterConfig::default()
    });
    let (status, _json) =
        envelope_of(oneshot(r.clone(), request_get("/runtimes", None)).await).await;
    assert_eq!(status, StatusCode::OK, "a read needs no token");
    let response = oneshot(
        r,
        axum::extract::Request::builder()
            .method(Method::POST)
            .uri("/runtimes")
            .header(CONTENT_TYPE, "application/json")
            .body(Body::from(br#"{"root":"/w"}"#.to_vec()))
            .unwrap(),
    )
    .await;
    let (status, json) = envelope_of(response).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(json["code"], crate::refuse::AUTH_LAUNCH_TOKEN_REQUIRED);
}

#[tokio::test]
async fn mutation_gate_passes_a_bearer_token() {
    let r = router(RouterConfig {
        token: "tkn".to_owned(),
        ..RouterConfig::default()
    });
    let response = oneshot(
        r,
        axum::extract::Request::builder()
            .method(Method::POST)
            .uri("/runtimes")
            .header(CONTENT_TYPE, "application/json")
            .header(axum::http::header::AUTHORIZATION, "Bearer tkn")
            .body(Body::from(br#"{"root":"/w"}"#.to_vec()))
            .unwrap(),
    )
    .await;
    // Past the gates, the open machine's refusal is the answer.
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn mutation_gate_passes_the_loopback_gate_mark() {
    // build_router with an origin mounts the gate; a passing request
    // carries HostGated and the mutation goes through without a token.
    let r = router(RouterConfig {
        token: "tkn".to_owned(),
        origin: "http://127.0.0.1:8080".to_owned(),
        ..RouterConfig::default()
    });
    let request = axum::extract::Request::builder()
        .method(Method::POST)
        .uri("/runtimes")
        .header(axum::http::header::HOST, "127.0.0.1:8080")
        .header(axum::http::header::ORIGIN, "http://127.0.0.1:8080")
        .header(
            axum::http::header::COOKIE,
            format!("{}=tkn", crate::gate::TOKEN_COOKIE),
        )
        .header(CONTENT_TYPE, "application/json")
        .body(Body::from(br#"{"root":"/w"}"#.to_vec()))
        .unwrap();
    let response = oneshot(r, request).await;
    assert_eq!(
        response.status(),
        StatusCode::INTERNAL_SERVER_ERROR,
        "gates passed; the build refusal is the answer"
    );
}

#[tokio::test]
async fn loopback_gate_rejects_a_wrong_host_before_anything_else() {
    let r = router(RouterConfig {
        token: "tkn".to_owned(),
        origin: "http://127.0.0.1:8080".to_owned(),
        ..RouterConfig::default()
    });
    let response = oneshot(
        r,
        axum::extract::Request::builder()
            .uri("/runtimes")
            .header(axum::http::header::HOST, "example.com:80")
            .header(axum::http::header::ORIGIN, "http://127.0.0.1:8080")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    let (status, json) = envelope_of(response).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(json["code"], crate::refuse::LOOPBACK_HOST_REJECTED);
}

#[tokio::test]
async fn host_guard_rejects_a_rebound_host_when_auth_is_off() {
    // No loopback gate mounted: the host guard answers by itself.
    let r = router(RouterConfig::default());
    let response = oneshot(
        r,
        axum::extract::Request::builder()
            .uri("/runtimes")
            .header(axum::http::header::HOST, "example.com:80")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    let (status, json) = envelope_of(response).await;
    assert_eq!(status, StatusCode::MISDIRECTED_REQUEST);
    assert_eq!(json["code"], crate::refuse::HOST_REJECTED);
}

#[tokio::test]
async fn watch_tree_writes_trigger_after_a_2xx_only() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let fired = Arc::new(AtomicUsize::new(0));
    let counter = fired.clone();
    let r = router(RouterConfig {
        token: "tkn".to_owned(),
        on_tree_changed: Some(Arc::new(move || {
            counter.fetch_add(1, Ordering::SeqCst);
        })),
        ..RouterConfig::default()
    });
    // A POST /runtimes is a tree write, but the refusal is not a 2xx.
    let request = axum::extract::Request::builder()
        .method(Method::POST)
        .uri("/runtimes")
        .header(CONTENT_TYPE, "application/json")
        .header(
            axum::http::header::COOKIE,
            format!("{}=tkn", crate::gate::TOKEN_COOKIE),
        )
        .body(Body::from(br#"{"root":"/w"}"#.to_vec()))
        .unwrap();
    let _ = oneshot(r, request).await;
    assert_eq!(
        fired.load(Ordering::SeqCst),
        0,
        "a refusal triggers nothing"
    );
}

#[test]
fn tree_write_matches_the_go_predicate() {
    assert!(tree_write("/tree/sessions/remove"));
    assert!(!tree_write("/tree/sessions/export"));
    assert!(tree_write("/runtimes"));
    assert!(tree_write("/runtimes/r1/close"));
    assert!(!tree_write("/runtime"));
    assert!(tree_write("/rt/r1/new"));
    assert!(tree_write("/rt/r1/resume"));
    assert!(!tree_write("/rt/r1"));
    assert!(tree_write("/workspace"));
    assert!(tree_write("/delete-session"));
    assert!(tree_write("/summarize"));
    assert!(!tree_write("/submit"));
    // "/tree" with no trailing slash is not the /tree/ prefix; Go's
    // HasPrefix("/tree/") says no, and the export exception is exact.
    assert!(!tree_write("/tree"));
    assert!(tree_write("/tree/sessions/export/"));
}

#[tokio::test]
async fn route_default_serves_the_shell_when_a_page_is_configured() {
    let page = tempdir();
    std::fs::write(page.path().join("index.html"), b"<html>shell</html>").unwrap();
    let r = router(RouterConfig {
        page_dir: Some(page.path().to_path_buf()),
        ..RouterConfig::default()
    });
    let response = oneshot(r, request_get("/", None)).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .unwrap();
    assert_eq!(body.as_ref(), b"<html>shell</html>");
}

#[tokio::test]
async fn a_panicking_handler_answers_500_text() {
    // The catch layer wraps every handler the way net/http's recover does;
    // drive it directly on a router with a panicking route.
    let app = Router::new()
        .route(
            "/boom",
            axum::routing::get(|| async {
                panic!("injected for the catch layer test");
                #[allow(unreachable_code)]
                axum::response::IntoResponse::into_response("unreachable")
            }),
        )
        .layer(CatchPanicLayer);
    let response = oneshot(
        app,
        axum::extract::Request::builder()
            .uri("/boom")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        response.headers().get(CONTENT_TYPE),
        Some(&HeaderValue::from_static("text/plain; charset=utf-8"))
    );
}

#[tokio::test]
async fn auth_token_bootstrap_trades_the_body_token_for_a_cookie() {
    let r = router(RouterConfig {
        token: "tkn".to_owned(),
        ..RouterConfig::default()
    });
    let request = axum::extract::Request::builder()
        .method(Method::POST)
        .uri("/auth/token")
        .header(CONTENT_TYPE, "application/json")
        .body(Body::from(br#"{"token":"tkn"}"#.to_vec()))
        .unwrap();
    let response = oneshot(r.clone(), request).await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let cookie = response
        .headers()
        .get(axum::http::header::SET_COOKIE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        cookie.starts_with("reasonix_token=tkn; Path=/; Max-Age=2592000; HttpOnly; SameSite=Lax"),
        "cookie shape {cookie}"
    );
    // A wrong token is the generic deny, and a second JSON value is a
    // bad body — the body is where the credential lives, so its parsing
    // is strict.
    for (body, want) in [
        (
            &br#"{"token":"wrong"}"#.to_vec()[..],
            StatusCode::UNAUTHORIZED,
        ),
        (
            &br#"{"token":"tkn"}{}"#.to_vec()[..],
            StatusCode::BAD_REQUEST,
        ),
    ] {
        let request = axum::extract::Request::builder()
            .method(Method::POST)
            .uri("/auth/token")
            .header(CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_vec()))
            .unwrap();
        let response = oneshot(r.clone(), request).await;
        assert_eq!(response.status(), want);
    }
    // GET is not accepted.
    let response = oneshot(r, request_get("/auth/token", None)).await;
    let (status, json) = envelope_of(response).await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(json["code"], crate::refuse::METHOD_NOT_ALLOWED);
}

#[tokio::test]
async fn token_mode_denies_without_the_cookie_and_redirects_a_query_token() {
    let tmp = tempdir();
    let r = build_router_with_auth(
        Arc::new(Hub::new(HubPaths {
            state_root: tmp.path().to_path_buf(),
        })),
        RouterConfig::default(),
        Arc::new(crate::auth::AuthGate::token_mode("tkn").unwrap()),
    );
    // API paths deny with the generic 401 text.
    let response = oneshot(r.clone(), request_get("/runtimes", None)).await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let body = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .unwrap();
    assert_eq!(body.as_ref(), b"Unauthorized\n");
    // The shell bootstrap paths pass without a credential...
    let response = oneshot(r.clone(), request_get("/", None)).await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let response = oneshot(r.clone(), request_get("/sessions/r1", None)).await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    // ...and a valid query token trades itself for a cookie and a
    // redirect with the parameter stripped.
    let response = oneshot(r.clone(), request_get("/tree?token=tkn", None)).await;
    assert_eq!(response.status(), StatusCode::FOUND);
    let cookie = response
        .headers()
        .get(axum::http::header::SET_COOKIE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(cookie.starts_with("reasonix_token=tkn;"), "cookie {cookie}");
    assert_eq!(
        response
            .headers()
            .get(axum::http::header::LOCATION)
            .and_then(|v| v.to_str().ok()),
        Some("/tree")
    );
    // A wrong query token denies.
    let response = oneshot(r, request_get("/tree?token=nope", None)).await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[test]
fn gate_input_shape_is_held_by_the_contract() {
    // The frozen decision surface: Allow or one of the four codes.
    let gate = LoopbackGate::new("http://127.0.0.1:8080", "tkn");
    let decision = gate.check_http(GateInput {
        method: "GET",
        host: "127.0.0.1:8080",
        origin: None,
        token_cookie: Some("tkn"),
    });
    assert_eq!(decision, GateDecision::Allow);
    // The origin check runs before the credential check, outermost first.
    let decision = gate.check_http(GateInput {
        method: "POST",
        host: "127.0.0.1:8080",
        origin: None,
        token_cookie: None,
    });
    assert_eq!(
        decision,
        GateDecision::Reject(RefusalCode::LoopbackOriginRejected)
    );
    let decision = gate.check_http(GateInput {
        method: "POST",
        host: "127.0.0.1:8080",
        origin: Some("http://127.0.0.1:8080"),
        token_cookie: None,
    });
    assert_eq!(
        decision,
        GateDecision::Reject(RefusalCode::LoopbackUnauthorized)
    );
}

//! The hub router (Go `hub.go` `Handler` + the read-side handlers of this
//! slice): the middleware stack, the hub-level routes, and the routeDefault
//! fallback. Per-runtime routes (`/events`, `/events/replay`) live in
//! `eventstream::session_routes` and are mounted by the host under a runtime
//! prefix — the hub router never registers them, so with zero runtimes those
//! paths fall to routeDefault's 503 exactly as they do in Go.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use reasonix_session::hub::Hub;
use reasonix_session::model::{OpenRuntimeRequest, SessionRow};
use serde::Serialize;
use tower::{Layer, Service};

use axum::Router;
use axum::body::Body;
use axum::extract::{Path, Request, State};
use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderValue, Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::Response;
use axum::routing::{any, get, post};

use crate::auth::{
    AuthGate, HostGated, csrf_guard_middleware, host_guard_middleware, mutation_gate_middleware,
};
use crate::gate::{GateDecision, LoopbackGate};
use crate::refuse::{NO_RUNTIME_OPEN, Reason, RefuseError, not_found, refuse_response, write_err};

/// What the host decides once for the whole hub (docs/contract.md S3: the
/// host wires `build_router(hub, cfg)`; the loopback gate mounts when an
/// origin is named).
#[derive(Clone, Default)]
pub struct RouterConfig {
    /// The launch credential (auth none mode). Empty mints one.
    pub token: String,
    /// The loopback origin (`http://127.0.0.1:<port>`); empty disables the
    /// loopback gate — the serve-CLI shape, not the desktop one.
    pub origin: String,
    /// The built page; `None` serves the kernel alone.
    pub page_dir: Option<PathBuf>,
    /// What a tree write triggers after a 2xx (Go `Hub.treeChanged`). The
    /// tree-change watcher is a later wave; this is where it lands.
    pub on_tree_changed: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl std::fmt::Debug for RouterConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The token never prints.
        f.debug_struct("RouterConfig")
            .field("token", &"***")
            .field("origin", &self.origin)
            .field("page_dir", &self.page_dir)
            .field("on_tree_changed", &self.on_tree_changed.is_some())
            .finish()
    }
}

/// Shared across the middleware stack (axum state).
#[derive(Clone)]
pub(crate) struct RouterState {
    pub(crate) hub: Arc<Hub>,
    pub(crate) auth: Arc<AuthGate>,
    pub(crate) page: Option<PathBuf>,
    pub(crate) on_tree_changed: Option<Arc<dyn Fn() + Send + Sync>>,
}

/// Assembles the hub router with the Go middleware order, outermost first:
/// loopback gate -> log -> auth -> hostGuard -> withPage -> csrfGuard ->
/// mutationGate -> watchTreeWrites -> routes, with the panic catch (OPT-003)
/// wrapped around everything net/http would have recovered.
pub fn build_router(hub: Arc<Hub>, cfg: RouterConfig) -> Router {
    let token = cfg.token.trim().to_owned();
    let auth = Arc::new(match AuthGate::none(&token) {
        Ok(gate) => gate,
        // A serve that cannot mint a credential refuses everything, like an
        // invalid auth mode: there is no usable boundary to lower.
        Err(err) => {
            tracing::error!("serve: token generation failed: {err}; refusing every request");
            AuthGate::invalid()
        }
    });
    build_router_with_auth(hub, cfg, auth)
}

/// The same assembly over a caller-built gate — token mode is exercised this
/// way in tests; the desktop host runs auth none (HOST-018 zeroes the user's
/// serve auth), so `build_router` always mints the none-mode gate.
pub(crate) fn build_router_with_auth(
    hub: Arc<Hub>,
    cfg: RouterConfig,
    auth: Arc<AuthGate>,
) -> Router {
    let state = RouterState {
        hub,
        auth: auth.clone(),
        page: cfg.page_dir.clone(),
        on_tree_changed: cfg.on_tree_changed.clone(),
    };

    let routes = Router::new()
        .route(
            "/runtimes",
            get(list_runtimes)
                .post(open_runtime)
                .fallback(method_not_allowed_response),
        )
        .route(
            "/runtimes/{id}/close",
            post(close_runtime).fallback(method_not_allowed_response),
        )
        .route("/tree", get(get_tree).fallback(method_not_allowed_response))
        // Go's pattern "/rt/{id}/" matches the bare prefix and anything
        // under it. "/rt/{id}" without the slash falls to routeDefault,
        // as there. The id is read off the path because the two shapes
        // carry different parameter counts.
        .route("/rt/{id}/", any(route_runtime))
        .route("/rt/{id}/{*rest}", any(route_runtime))
        .fallback(route_default)
        .layer(middleware::from_fn_with_state(
            state.clone(),
            watch_tree_writes_middleware,
        ))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            mutation_gate_middleware,
        ))
        .layer(middleware::from_fn(csrf_guard_middleware))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            crate::page::with_page_middleware,
        ))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            host_guard_middleware,
        ))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            crate::auth::auth_middleware,
        ))
        .layer(middleware::from_fn(log_middleware));

    let app = if cfg.origin.is_empty() {
        routes
    } else {
        let gate = Arc::new(LoopbackGate::new(&cfg.origin, auth.token()));
        routes.layer(middleware::from_fn_with_state(
            gate,
            loopback_gate_middleware,
        ))
    };
    app.layer(CatchPanicLayer).with_state(state)
}

/// The loopback gate as a layer (Go `NewLoopbackGate`): checks outermost
/// first, marks what passes, and answers each refusal with its own code so a
/// test can tell a rebinding defense that held from a credential check that
/// happened to answer first.
async fn loopback_gate_middleware(
    State(gate): State<Arc<LoopbackGate>>,
    request: Request,
    next: Next,
) -> Response {
    match gate.check_request(request.method(), request.headers()) {
        GateDecision::Allow => {
            let (mut parts, body) = request.into_parts();
            parts.extensions.insert(HostGated);
            next.run(Request::from_parts(parts, body)).await
        }
        GateDecision::Reject(code) => refuse_response(
            StatusCode::FORBIDDEN,
            &Reason::new(code.as_str(), code.message()),
        ),
    }
}

/// Go `logMiddleware` (requestlog.go:27-45): one INFO line per request.
/// No route of this slice is a `routine()` timer, so nothing logs at Debug.
async fn log_middleware(request: Request, next: Next) -> Response {
    let start = Instant::now();
    let method = request.method().clone();
    let path = request.uri().path().to_owned();
    let response = next.run(request).await;
    tracing::info!(
        method = %method,
        path = %path,
        status = response.status().as_u16(),
        duration_ms = start.elapsed().as_millis() as u64,
        "serve: request"
    );
    response
}

async fn method_not_allowed_response() -> Response {
    crate::refuse::method_not_allowed()
}

/// Go `writeJSON`: `application/json` plus the encoder's trailing newline.
pub(crate) fn write_json<T: Serialize + ?Sized>(value: &T) -> Response {
    let mut body = match serde_json::to_vec(value) {
        Ok(body) => body,
        Err(err) => {
            tracing::warn!("serve: writeJSON encode failed: {err}");
            return refuse_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &Reason::new("internal.failed", "something went wrong on this side"),
            );
        }
    };
    body.push(b'\n');
    let mut response = Response::new(Body::from(bytes::Bytes::from(body)));
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    response
}

// ---------------------------------------------------------------------------
// Hub routes (hub.go:542-553).
// ---------------------------------------------------------------------------

/// Go `listRuntimes`: the open runtimes in the order they were opened; the
/// empty hub answers an empty array.
async fn list_runtimes(State(state): State<RouterState>) -> Response {
    write_json(&state.hub.runtimes())
}

/// Go `openRuntime`: the catch-all refusal (writeErr 500, http.Error shape)
/// is what a pane the kernel could not build deserves — 409 dressed a missing
/// transcript as a held one. The decode runs under the same 64KiB cap and
/// unknown-field rejection as every other body.
async fn open_runtime(State(state): State<RouterState>, request: Request) -> Response {
    let body = request.into_body();
    let req: OpenRuntimeRequest = match crate::refuse::decode_body(body).await {
        Ok(req) => req,
        Err(_) => return crate::refuse::bad_body(),
    };
    match state.hub.open_runtime(req) {
        Ok(summary) => write_json(&summary),
        Err(refusal) => write_err(
            StatusCode::INTERNAL_SERVER_ERROR,
            &RefuseError::Plain(refusal.to_string()),
        ),
    }
}

/// Go `closeRuntime`: an unknown id is a 404 `request.not_found` naming the
/// kind. The close machine itself is a later wave; a runtime the host adopted
/// answers the honest catch-all rather than a fake success.
async fn close_runtime(State(state): State<RouterState>, Path(id): Path<String>) -> Response {
    if state.hub.runtimes().iter().any(|rt| rt.id == id) {
        return write_err(
            StatusCode::INTERNAL_SERVER_ERROR,
            &RefuseError::Plain("runtime close is not part of this build".to_owned()),
        );
    }
    not_found("runtime", &id)
}

/// Go `h.tree`: the whole sidebar in one snapshot.
async fn get_tree(State(state): State<RouterState>) -> Response {
    write_json(&state.hub.tree())
}

/// Go `s.sessions` (sessions_list.go, routes.go:91) as a per-runtime route
/// (mounted by `sessions_routes`, never by the hub mux): the runtime's own
/// workspace names the transcript directory. An empty or unreadable
/// directory answers `[]`, never an error.
pub(crate) async fn runtime_sessions(State((hub, root)): State<(Arc<Hub>, String)>) -> Response {
    // The transcript walk is blocking file I/O (docs/kernel-rs/AGENTS.md
    // 10.3) — it does not belong on the async workers.
    let rows: Vec<SessionRow> =
        tokio::task::spawn_blocking(move || hub.sessions(std::path::Path::new(&root)))
            .await
            .unwrap_or_default();
    write_json(&rows)
}

/// Go `routeRuntime` (hub.go:622-629): an unknown id is a 404 naming the
/// kind; a known one is served by its own mux, frozen at publish the way
/// Go's `http.StripPrefix("/rt/"+ID, Server.routes())` freezes it — the
/// runtime routes stay unprefixed.
async fn route_runtime(State(state): State<RouterState>, request: Request) -> Response {
    let path = request.uri().path().trim_start_matches("/rt/");
    let id = path.split('/').next().unwrap_or("");
    let Some(runtime) = state.hub.runtimes().into_iter().find(|rt| rt.id == id) else {
        return not_found("runtime", id);
    };
    let router = runtime_routes(state.hub.clone(), &runtime);
    // Strip the runtime prefix (Go StripPrefix): "/rt/r1/events" reaches the
    // runtime mux as "/events". The bare "/rt/r1" never gets here — the
    // hub-level routes only match the trailing-slash shapes.
    let (mut parts, body) = request.into_parts();
    let consumed = "/rt/".len() + runtime.id.len();
    let mut rest = parts.uri.path().get(consumed..).unwrap_or("/").to_owned();
    if rest.is_empty() {
        rest.push('/');
    }
    let path_and_query = match parts.uri.query() {
        Some(query) => format!("{rest}?{query}"),
        None => rest,
    };
    parts.uri = axum::http::Uri::builder()
        .path_and_query(path_and_query)
        .build()
        .expect("a stripped path re-parses");
    serve_mounted(router, Request::from_parts(parts, body)).await
}

/// One runtime's own mux (Go `Server.routes()`' subset this build answers),
/// the handler `publish` freezes for `/rt/{id}/`: the event stream family,
/// the runtime's session list, and the turn-command verbs. Built per
/// dispatch from the hub's runtime table — the table is the state publish
/// froze; the router is its view.
pub(crate) fn runtime_routes(
    hub: Arc<Hub>,
    runtime: &reasonix_session::model::RuntimeSummary,
) -> Router {
    let runtime_id = reasonix_session::model::RuntimeId(runtime.id.clone());
    let ctrl = hub.turn_controller(runtime_id.clone());
    let bc = hub.broadcaster(runtime_id);
    crate::eventstream::events_routes(bc)
        .merge(sessions_routes(hub, runtime.root.clone()))
        .merge(crate::commands::command_routes(ctrl))
}

/// Go `s.sessions` (sessions_list.go): the runtime's own workspace names
/// the transcript directory, unlike the hub-level tree route.
fn sessions_routes(hub: Arc<Hub>, root: String) -> Router {
    Router::new()
        .route(
            "/sessions",
            get(runtime_sessions).fallback(method_not_allowed_response),
        )
        .with_state((hub, root))
}

/// Serves `request` through a runtime's mounted router (Go: rt.handler or
/// routeDefault's `local.Server.routes().ServeHTTP(w, r)`). The nested
/// router is bare — every hub-level middleware has already run — and its
/// error type is infallible.
async fn serve_mounted(router: Router, request: Request) -> Response {
    use tower::Service as _Service;
    let mut svc: Router = router;
    std::future::poll_fn(|cx| _Service::<Request>::poll_ready(&mut svc, cx))
        .await
        .unwrap_or_else(|e| match e {});
    match _Service::<Request>::call(&mut svc, request).await {
        Ok(response) => response,
        Err(e) => match e {},
    }
}

/// Go `routeDefault` (hub.go:635-667): serves the first local runtime
/// unprefixed for a browser opened straight at the port or an older
/// frontend. The page branch is only reachable with no runtime open: GET /
/// with a page while authenticated serves the shell — only past the gate,
/// since "/" is public in token mode so a bare shell can bootstrap.
async fn route_default(State(state): State<RouterState>, request: Request) -> Response {
    if let Some(runtime) = state.hub.runtimes().into_iter().next() {
        let router = runtime_routes(state.hub.clone(), &runtime);
        return serve_mounted(router, request).await;
    }
    let page = (request.method() == Method::GET
        && request.uri().path() == "/"
        && state.auth.authenticated(request.headers()))
    .then_some(())
    .and(state.page.clone());
    if let Some(page) = page {
        // Blocking read off the async workers (docs/kernel-rs/AGENTS.md 10.3).
        let bytes = tokio::task::spawn_blocking(move || std::fs::read(page.join("index.html")))
            .await
            .unwrap_or_else(|_| Err(std::io::Error::other("page read task ended")));
        return match bytes {
            Ok(bytes) => {
                let mut response = Response::new(Body::from(bytes::Bytes::from(bytes)));
                *response.status_mut() = StatusCode::OK;
                response.headers_mut().insert(
                    CONTENT_TYPE,
                    HeaderValue::from_static("text/html; charset=utf-8"),
                );
                response
            }
            Err(_) => crate::refuse::http_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal Server Error",
            ),
        };
    }
    no_runtime_open()
}

fn no_runtime_open() -> Response {
    refuse_response(
        StatusCode::SERVICE_UNAVAILABLE,
        &Reason::new(NO_RUNTIME_OPEN, "no runtime is open"),
    )
}

// ---------------------------------------------------------------------------
// watchTreeWrites (hub_sync.go).
// ---------------------------------------------------------------------------

/// Go `treeWrite` (hub_sync.go:71-90): whether a path mutates the workspace
/// tree. `/tree/...` except the export, the runtimes family, and the
/// session-path-changing routes of a runtime, prefixed or not.
pub fn tree_write(path: &str) -> bool {
    if path == "/tree/sessions/export" {
        return false;
    }
    if path.starts_with("/tree/") || path == "/runtimes" || path.starts_with("/runtimes/") {
        return true;
    }
    let path = match path.strip_prefix("/rt/") {
        Some(tail) => match tail.split_once('/') {
            Some((_id, rest)) => format!("/{rest}"),
            None => return false,
        },
        None => path.to_owned(),
    };
    matches!(
        path.as_str(),
        "/new" | "/resume" | "/fork" | "/rewind" | "/summarize" | "/delete-session" | "/workspace"
    )
}

/// Go `watchTreeWrites`: after a 2xx to a tree write, the hub's tree changed
/// and the list is read again.
async fn watch_tree_writes_middleware(
    State(state): State<RouterState>,
    request: Request,
    next: Next,
) -> Response {
    let method = request.method().clone();
    let path = request.uri().path().to_owned();
    if method == Method::GET || method == Method::HEAD || !tree_write(&path) {
        return next.run(request).await;
    }
    let response = next.run(request).await;
    if response.status().is_success() {
        // Go counts a response that never wrote a header as 200; axum always
        // has a status, so the 2xx check alone matches.
        if let Some(on_tree_changed) = state.on_tree_changed.as_ref() {
            on_tree_changed();
        }
    }
    response
}

// ---------------------------------------------------------------------------
// OPT-003: the panic catch (net/http recovers handler panics per request;
// axum needs a layer for the same guarantee). ~The plan's ten lines grew a
// little because a panic mid-await must be caught at each poll.
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct CatchPanicLayer;

impl<S> Layer<S> for CatchPanicLayer {
    type Service = CatchPanicService<S>;
    fn layer(&self, inner: S) -> Self::Service {
        CatchPanicService { inner }
    }
}

#[derive(Clone)]
struct CatchPanicService<S> {
    inner: S,
}

impl<S> Service<Request<Body>> for CatchPanicService<S>
where
    S: Service<Request<Body>, Response = Response, Error = std::convert::Infallible>
        + Clone
        + Send
        + 'static,
    S::Future: Send + 'static,
{
    type Response = Response;
    type Error = std::convert::Infallible;
    type Future =
        std::pin::Pin<Box<dyn std::future::Future<Output = Result<Response, Self::Error>> + Send>>;

    fn poll_ready(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, request: Request<Body>) -> Self::Future {
        let fut = self.inner.call(request);
        Box::pin(async move {
            let mut fut = Box::pin(fut);
            std::future::poll_fn(|cx| {
                match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    fut.as_mut().poll(cx)
                })) {
                    Ok(poll) => poll,
                    Err(panic) => std::task::Poll::Ready(Ok(panic_response(panic))),
                }
            })
            .await
        })
    }
}

fn panic_response(panic: Box<dyn std::any::Any + Send>) -> Response {
    let message = panic
        .downcast_ref::<&str>()
        .map(|s| (*s).to_owned())
        .or_else(|| panic.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "panic".to_owned());
    tracing::error!(panic = %message, "serve: handler panicked");
    crate::refuse::http_error(StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error")
}

#[cfg(test)]
mod tests {
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
        let (status, json) =
            envelope_of(oneshot(r.clone(), request_get("/rt/r9/", None)).await).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(json["params"]["kind"], "runtime");
        let (status, json) =
            envelope_of(oneshot(r.clone(), request_get("/rt/r9/tree", None)).await).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(json["code"], crate::refuse::NOT_FOUND);
        // Without the trailing slash the request reaches routeDefault.
        let (status, json) =
            envelope_of(oneshot(r.clone(), request_get("/rt/r9", None)).await).await;
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

    #[tokio::test]
    async fn route_default_answers_503_and_the_events_fallback_follows() {
        let r = router(RouterConfig::default());
        for path in ["/", "/definitely-not-a-route", "/events", "/events/replay"] {
            let (status, json) =
                envelope_of(oneshot(r.clone(), request_get(path, None)).await).await;
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
            cookie
                .starts_with("reasonix_token=tkn; Path=/; Max-Age=2592000; HttpOnly; SameSite=Lax"),
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
}

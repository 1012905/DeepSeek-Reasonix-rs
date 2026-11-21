//! The turn-command routes of one runtime (Go serve.go's command endpoints
//! over `Server.routes()`, http.md §7): submit admits a turn against the
//! published runtime's controller, cancel aborts the in-flight one, and the
//! verbs whose machines are not in this build answer the typed refusal —
//! never a faked 204 (docs/contract.md §3's trim principle).

use std::sync::Arc;

use serde::Deserialize;

use axum::body::Body;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use reasonix_session::turns::TurnController;

use crate::refuse::{Reason, refuse_response};

/// The per-runtime command verbs (Go routes.go:29, 46-47, 63), mounted under
/// a runtime's prefix by the hub router's dispatch. The controller is `None`
/// for a host-adopted runtime whose write side is the host's own.
pub(crate) fn command_routes(ctrl: Option<Arc<TurnController>>) -> axum::Router {
    axum::Router::new()
        .route("/submit", axum::routing::post(runtime_submit).fallback(wrong_method))
        .route("/cancel", axum::routing::post(runtime_cancel).fallback(wrong_method))
        .route("/new", axum::routing::post(runtime_new).fallback(wrong_method))
        .route("/approve", axum::routing::post(runtime_approve).fallback(wrong_method))
        .with_state(ctrl)
}

async fn wrong_method() -> impl IntoResponse {
    crate::refuse::method_not_allowed()
}

/// POST /submit's body (serve.go:421): the composer line plus the
/// turn-shaping fields. Go decodes with a plain `json.Decoder` here —
/// unknown fields ride through (the strict 64KiB/unknown-fields decode is
/// per-route, and submit is not one of its callers) and any decode failure
/// answers `missing_field` with the empty-input case.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct SubmitBody {
    input: String,
    /// The structured-output request for this turn (`""` or `json_object`).
    format: String,
    /// Skill chips: `"/name args"` entries carried beside the line.
    invocations: Vec<serde_json::Value>,
    /// Parsed for shape parity; the notice-instead-of-turn machine behind it
    /// is unported, and the field alone changes nothing here.
    #[serde(rename = "refuseUnknownSlash")]
    #[allow(dead_code)]
    refuse_unknown_slash: bool,
    /// Parsed for shape parity; `!` commands are refused at the transport
    /// boundary before this flag could matter (see `runtime_submit`).
    #[serde(rename = "localShell")]
    #[allow(dead_code)]
    local_shell: bool,
}

/// POST /approve's body (serve.go:507-520).
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ApproveBody {
    id: String,
    #[allow(dead_code)]
    allow: bool,
    #[allow(dead_code)]
    session: bool,
    #[allow(dead_code)]
    persist: bool,
}

/// Reads the first JSON value out of the body, tolerating unknown fields and
/// trailing bytes exactly as Go's single `Decode` does. `None` is any decode
/// failure — the command routes fold every one into the missing-field shape.
fn parse_body<T: serde::de::DeserializeOwned>(raw: &[u8]) -> Option<T> {
    let mut de = serde_json::Deserializer::from_slice(raw);
    serde_ignored::deserialize(&mut de, |_| {}).ok()
}

/// The route exists, the machine does not, and a fake success would lie
/// about work nobody did. The catch-all `http.Error` shape is what the
/// sibling refusals (open, close) answer with.
fn not_in_this_build(message: &str) -> Response {
    crate::refuse::http_error(StatusCode::INTERNAL_SERVER_ERROR, message)
}

/// Go `s.submit` (serve.go:417-500): 202 Accepted — output arrives on the
/// event stream; a turn in flight answers 409 `busy.session_running`; a `!`
/// line off a socket answers 403 `shell.unavailable_over_http`. Admission is
/// synchronous and the turn is spawned, so the response never waits on the
/// model and a racing submit is refused by the controller's own swap.
pub(crate) async fn runtime_submit(
    State(ctrl): State<Option<Arc<TurnController>>>,
    request: axum::extract::Request<Body>,
) -> Response {
    let Some(ctrl) = ctrl else {
        return not_in_this_build("turn control is not part of this build");
    };
    // Go reads the first JSON value straight off the stream; the bounded
    // read stands in for that (the composer's lines are small — the strict
    // decode's cap bounds this route too).
    let raw = match axum::body::to_bytes(request.into_body(), crate::refuse::BODY_LIMIT).await {
        Ok(raw) => raw,
        Err(_) => return crate::refuse::missing_field("input"),
    };
    let body: SubmitBody = match parse_body(&raw) {
        Some(body) => body,
        None => return crate::refuse::missing_field("input"),
    };
    if body.input.is_empty() && body.invocations.is_empty() {
        return crate::refuse::missing_field("input");
    }
    match body.format.trim() {
        "" => {}
        // The binding is the agent's response-format machine (Go
        // `WithResponseFormat`); unported here, and prose in place of the
        // JSON the client asked for is a faked success.
        "json_object" => {
            return not_in_this_build("structured output is not part of this build");
        }
        _ => return crate::refuse::bad_value("format", &["json_object"]),
    }
    if !body.invocations.is_empty() {
        // Go submits the chip line through the durable inbox so the chips
        // travel beside the text; neither machine is here, and dropping
        // chips silently is not an admission.
        return not_in_this_build("chip submission is not part of this build");
    }
    let trimmed = body.input.trim_start();
    if trimmed.starts_with('!') {
        // Every request this build serves arrives off a socket — the
        // in-process mark does not exist (Go `refuseNetworkShell`), so a `!`
        // line is refused whatever localShell says.
        return refuse_response(
            StatusCode::FORBIDDEN,
            &Reason::new(
                crate::refuse::SHELL_UNAVAILABLE,
                "shell commands are unavailable over HTTP",
            ),
        );
    }
    if trimmed.starts_with('/') {
        // Go intercepts three settings verbs at this handler and hands the
        // rest to the controller's slash machine; neither is ported, and the
        // model must not read a management verb as a turn.
        return not_in_this_build("slash commands are not part of this build");
    }
    if !ctrl.try_admit() {
        return crate::refuse::session_busy();
    }
    let input = body.input;
    tokio::spawn(async move {
        ctrl.run_admitted(&input).await;
    });
    StatusCode::ACCEPTED.into_response()
}

/// Go `s.cancel` (serve.go:502-505): 204, the in-flight turn aborts at its
/// next await point. Cancelling with no turn running is a no-op, as there.
pub(crate) async fn runtime_cancel(State(ctrl): State<Option<Arc<TurnController>>>) -> Response {
    if let Some(ctrl) = ctrl {
        ctrl.cancel();
    }
    StatusCode::NO_CONTENT.into_response()
}

/// Go `s.newSession` (serve.go:522-538): the controller's session machine
/// and the lease rebind are one entry point there; not ported, and a fake
/// 204 would claim a fresh transcript nothing wrote.
pub(crate) async fn runtime_new(State(_ctrl): State<Option<Arc<TurnController>>>) -> Response {
    not_in_this_build("session reset is not part of this build")
}

/// Go `s.approve` (serve.go:507-520): the body shape is checked first (a
/// decode failure or an empty id answers `missing_field`), then the
/// orchestration would run. Approval orchestration — the Asker seam and the
/// approval bridge — is not ported, so the typed refusal is the answer: a
/// 204 here would fake a decision nothing recorded.
pub(crate) async fn runtime_approve(
    State(_ctrl): State<Option<Arc<TurnController>>>,
    request: axum::extract::Request<Body>,
) -> Response {
    let raw = match axum::body::to_bytes(request.into_body(), crate::refuse::BODY_LIMIT).await {
        Ok(raw) => raw,
        Err(_) => return crate::refuse::missing_field("id"),
    };
    let body: ApproveBody = match parse_body(&raw) {
        Some(body) => body,
        None => return crate::refuse::missing_field("id"),
    };
    if body.id.is_empty() {
        return crate::refuse::missing_field("id");
    }
    not_in_this_build("approval orchestration is not part of this build")
}

#[cfg(test)]
mod tests {
    use super::*;
    use reasonix_agent::config::AgentConfig;
    use reasonix_agent::sink::Sink;
    use reasonix_contract::eventwire::event::Event;
    use reasonix_provider::config::{ClientConfig, build_client};
    use reasonix_session::turns::TurnControllerParts;
    use reasonix_tools::Registry;

    /// A controller over a client pointed at a closed port: admission and
    /// the shape refusals never reach a provider, and a spawned turn fails
    /// fast and harmlessly.
    fn dead_controller() -> Arc<TurnController> {
        TurnController::new(TurnControllerParts {
            config: AgentConfig::default(),
            client: build_client(&ClientConfig::new("t", "http://127.0.0.1:1", "m"))
                .expect("client builds"),
            registry: Arc::new(Registry::new()),
            sink: Arc::new(BlackHole),
            workspace_root: ".".to_owned(),
            session_dir: String::new(),
            label: "t".to_owned(),
        })
    }

    struct BlackHole;

    impl Sink for BlackHole {
        fn emit(&self, _e: Event) {}
    }

    fn command_router(ctrl: Option<Arc<TurnController>>) -> axum::Router {
        axum::Router::new()
            .route("/submit", axum::routing::post(runtime_submit))
            .route("/cancel", axum::routing::post(runtime_cancel))
            .route("/new", axum::routing::post(runtime_new))
            .route("/approve", axum::routing::post(runtime_approve))
            .with_state(ctrl)
    }

    async fn drive(ctrl: Option<Arc<TurnController>>, path: &str, raw: &[u8]) -> Response {
        let request = axum::extract::Request::builder()
            .method(axum::http::Method::POST)
            .uri(path)
            .header(axum::http::header::CONTENT_TYPE, "application/json")
            .body(Body::from(raw.to_vec()))
            .unwrap();
        use tower::ServiceExt as _;
        command_router(ctrl)
            .oneshot(request)
            .await
            .expect("router call")
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

    async fn text_of(response: Response) -> String {
        let body = axum::body::to_bytes(response.into_body(), 1 << 20)
            .await
            .unwrap();
        String::from_utf8_lossy(&body).into_owned()
    }

    #[tokio::test]
    async fn submit_answers_202_and_carries_no_body() {
        let response = drive(Some(dead_controller()), "/submit", br#"{"input":"hello"}"#).await;
        assert_eq!(response.status(), StatusCode::ACCEPTED);
        assert!(text_of(response).await.trim().is_empty(), "202 carries no body");
    }

    #[tokio::test]
    async fn submit_refusals_match_the_go_shapes() {
        let ctrl = dead_controller();
        // Empty input and no chips: 400 missing input.
        let (status, json) = envelope_of(drive(Some(ctrl.clone()), "/submit", br#"{}"#).await).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(json["code"], crate::refuse::MISSING_FIELD);
        assert_eq!(json["params"]["field"], "input");
        // A decode failure folds into the same shape (Go: one check).
        let (status, _) = envelope_of(drive(Some(ctrl.clone()), "/submit", b"not json").await).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        // Unknown format value: 400 request.bad_value naming the allowed one.
        let (status, json) = envelope_of(
            drive(Some(ctrl.clone()), "/submit", br#"{"input":"x","format":"yaml"}"#).await,
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(json["code"], crate::refuse::BAD_VALUE);
        assert_eq!(json["params"]["field"], "format");
        // The structured-output binding is unported; prose instead of the
        // requested JSON would fake the success.
        let response = drive(
            Some(ctrl.clone()),
            "/submit",
            br#"{"input":"x","format":"json_object"}"#,
        )
        .await;
        assert_eq!(text_of(response).await, "structured output is not part of this build\n");
        // Chips: the inbox machine is unported — typed refusal, not a 202.
        let response = drive(
            Some(ctrl.clone()),
            "/submit",
            br#"{"input":"x","invocations":[{"name":"s"}]}"#,
        )
        .await;
        assert_eq!(text_of(response).await, "chip submission is not part of this build\n");
        // A `!` line off a socket: 403, the frozen Go code.
        let (status, json) =
            envelope_of(drive(Some(ctrl.clone()), "/submit", br#"{"input":"!ls"}"#).await).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(json["code"], crate::refuse::SHELL_UNAVAILABLE);
        // A slash line: the dispatch machine is unported — typed refusal.
        let response = drive(Some(ctrl.clone()), "/submit", br#"{"input":"/new"}"#).await;
        assert_eq!(text_of(response).await, "slash commands are not part of this build\n");
        // Unknown fields ride through, as Go's plain Decode does here.
        let response = drive(
            Some(ctrl.clone()),
            "/submit",
            br#"{"input":"x","refuseUnknownSlash":true}"#,
        )
        .await;
        assert_eq!(response.status(), StatusCode::ACCEPTED);
    }

    #[tokio::test]
    async fn a_second_submit_while_admission_holds_answers_409() {
        let ctrl = dead_controller();
        // The route does the same swap; holding it here is what a turn in
        // flight looks like to the next submit.
        assert!(ctrl.try_admit(), "first admission holds");
        let (status, json) =
            envelope_of(drive(Some(ctrl.clone()), "/submit", br#"{"input":"second"}"#).await)
                .await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(json["code"], crate::refuse::SESSION_BUSY);
        // Release through the turn body itself: the provider fails fast on
        // the closed port and running clears (the completion lands nowhere).
        ctrl.run_admitted("release").await;
    }

    #[tokio::test]
    async fn cancel_answers_204_and_new_and_approve_refuse_typed() {
        let ctrl = dead_controller();
        let response = drive(Some(ctrl.clone()), "/cancel", b"").await;
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        let response = drive(Some(ctrl.clone()), "/new", b"").await;
        assert_eq!(text_of(response).await, "session reset is not part of this build\n");
        // An approve without an id is a 400 before the machine refusal.
        let (status, json) =
            envelope_of(drive(Some(ctrl.clone()), "/approve", br#"{"allow":true}"#).await).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(json["params"]["field"], "id");
        let response =
            drive(Some(ctrl.clone()), "/approve", br#"{"id":"a1","allow":true}"#).await;
        assert_eq!(text_of(response).await, "approval orchestration is not part of this build\n");
    }

    #[tokio::test]
    async fn without_a_controller_the_write_side_refuses_typed() {
        // A host-adopted runtime carries no write side here; the refusal is
        // typed, and cancel keeps its 204 (a no-op there).
        let response = drive(None, "/submit", br#"{"input":"x"}"#).await;
        assert_eq!(text_of(response).await, "turn control is not part of this build\n");
        let response = drive(None, "/cancel", b"").await;
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
    }
}

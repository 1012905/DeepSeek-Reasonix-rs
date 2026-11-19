//! The refusal envelope (Go `fail.go` + `refuse_common.go`): every refusal is
//! a `Reason` — a dotted code, an English fallback sentence, and the params a
//! sentence needs — so the kernel decides what happened and a frontend decides
//! how to say it. OPT-017: the dotted-code table, the `RefuseError` mapping and
//! `write_err` live here and nowhere else.

use bytes::Bytes;
use serde::Serialize;
use serde_json::Value;
use serde_json::value::Map as JsonMap;

use axum::body::Body;
use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::Response;

/// The recurring dotted codes (refuse_common.go:15-35). Codes are wire-stable:
/// renaming one is a breaking change, the same as renaming a route.
pub const BAD_BODY: &str = "request.bad_body";
pub const MISSING_FIELD: &str = "request.missing_field";
pub const NOT_FOUND: &str = "request.not_found";
pub const UNKNOWN_PROJECT: &str = "project.unknown";
pub const SESSION_IN_USE: &str = "busy.session_in_use";
pub const BAD_VALUE: &str = "request.bad_value";
pub const MODEL_DECISION_ONLY: &str = "model.decision_only";
pub const MODEL_NOT_DECISION: &str = "model.not_decision_source";
pub const SESSION_BUSY: &str = "busy.session_running";
pub const SESSION_BAD_NAME: &str = "session.bad_name";
pub const SESSION_BAD_PATH: &str = "session.bad_path";
pub const SESSION_PATH_UNREADABLE: &str = "session.path_unreadable";
pub const SESSION_UNKNOWN: &str = "session.unknown";
pub const SESSION_REPLAY_LIMIT: &str = "session.replay_limit";
pub const SESSION_OUTSIDE: &str = "session.outside_dir";
pub const SESSION_ACTIVE: &str = "busy.session_active";
pub const SWITCH_MODEL: &str = "busy.switch_model";
pub const SESSION_OPEN_FAILED: &str = "session.open_failed";
pub const SESSION_TITLE_SAVE_FAILED: &str = "session.title_save_failed";

/// Codes this slice's routes and gates answer with; each is contract-stable
/// like the block above.
pub const BAD_CONTENT_TYPE: &str = "request.bad_content_type";
pub const METHOD_NOT_ALLOWED: &str = "request.method_not_allowed";
pub const AUTH_LAUNCH_TOKEN_REQUIRED: &str = "auth.launch_token_required";
pub const HOST_REJECTED: &str = "serve.host_rejected";
pub const LOOPBACK_HOST_REJECTED: &str = "loopback.host_rejected";
pub const LOOPBACK_ORIGIN_REJECTED: &str = "loopback.origin_rejected";
pub const LOOPBACK_UNAUTHORIZED: &str = "loopback.unauthorized";
pub const LOOPBACK_MISCONFIGURED: &str = "loopback.misconfigured";
pub const NO_RUNTIME_OPEN: &str = "hub.no_runtime_open";
pub const PAGE_NOT_BUILT: &str = "page.not_built";
pub const STREAM_UNSUPPORTED: &str = "stream.unsupported";

/// Memory read granularity (fail.go `memoryReadReason`).
pub const MEMORY_DIRECTORY_UNREADABLE: &str = "memory.directory_unreadable";
pub const MEMORY_FACT_UNREADABLE: &str = "memory.fact_unreadable";

/// `decodeBody` caps bodies at 64KiB (refuse_common.go:94) with real
/// truncation semantics: the stream is read up to the cap and anything past
/// it refuses, whatever Content-Length claimed.
pub const BODY_LIMIT: usize = 64 << 10;

/// A machine-readable refusal: the kernel decides what happened, a frontend
/// decides how to say it. `error` is the English fallback for logs and for
/// clients with no wording for this code — never the translation.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Reason {
    pub code: &'static str,
    pub error: String,
    #[serde(default, skip_serializing_if = "JsonMap::is_empty")]
    pub params: JsonMap<String, Value>,
}

impl Reason {
    pub fn new(code: &'static str, error: impl Into<String>) -> Self {
        Self {
            code,
            error: error.into(),
            params: JsonMap::new(),
        }
    }

    pub fn with_param(mut self, key: &str, value: impl Into<Value>) -> Self {
        self.params.insert(key.to_owned(), value.into());
        self
    }
}

/// One error reaching the boundary (fail.go `coded` plus the writeErr fallback
/// categories). `write_err` maps the categories in Go's order.
#[derive(Debug, Clone)]
pub enum RefuseError {
    /// A refusal that carries its own status and code (Go `coded`).
    Coded { status: StatusCode, reason: Reason },
    /// A memory read failure keeps its granularity (fail.go `memoryReadReason`):
    /// a directory that could not be listed is not one unreadable fact file.
    MemoryRead { code: &'static str },
    /// A config file the kernel could not read keeps its code wherever it
    /// surfaces (409 `config.unparsed`, fail.go `unparsedConfigReason`).
    UnparsedConfig {
        path: String,
        line: i64,
        excerpt: String,
        repair: String,
        recovered: String,
    },
    /// A model misplaced between the decision and conversation jobs
    /// (409 `model.decision_only` | `model.not_decision_source`).
    AnswersMismatch { decision: bool, model_ref: String },
    /// Anything else: `http.Error` text with the caller's fallback status.
    Plain(String),
}

impl std::fmt::Display for RefuseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RefuseError::Coded { reason, .. } => f.write_str(&reason.error),
            RefuseError::MemoryRead { .. } | RefuseError::UnparsedConfig { .. } => {
                f.write_str("the referenced state could not be read")
            }
            RefuseError::AnswersMismatch { .. } => f.write_str("model answers mismatch"),
            RefuseError::Plain(message) => f.write_str(message),
        }
    }
}

impl RefuseError {
    /// Go `refusal`: an error that `write_err` renders as a Reason.
    pub fn refusal(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self::Coded {
            status,
            reason: Reason::new(code, message),
        }
    }
}

fn json_params(entries: &[(&str, Value)]) -> JsonMap<String, Value> {
    entries
        .iter()
        .map(|(k, v)| ((*k).to_owned(), v.clone()))
        .collect()
}

/// Renders the envelope exactly as Go `refuse` writes it: JSON with the
/// encoder's trailing newline, `application/json; charset=utf-8`.
pub fn refuse_response(status: StatusCode, reason: &Reason) -> Response {
    let mut body = serde_json::to_vec(reason).unwrap_or_default();
    body.push(b'\n');
    let mut headers = HeaderMap::new();
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static("application/json; charset=utf-8"),
    );
    raw_response(status, headers, Bytes::from(body))
}

/// Go `http.Error`: plain text, the message plus a newline, and `nosniff` —
/// the fallback shape anything with no code keeps.
pub fn http_error(status: StatusCode, message: &str) -> Response {
    let mut body = message.to_owned().into_bytes();
    body.push(b'\n');
    let mut headers = HeaderMap::new();
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    headers.insert(
        axum::http::header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    raw_response(status, headers, Bytes::from(body))
}

fn raw_response(status: StatusCode, headers: HeaderMap, body: Bytes) -> Response {
    let mut response = Response::new(Body::from(body));
    *response.status_mut() = status;
    *response.headers_mut() = headers;
    response
}

/// Go `writeErr` (fail.go:88-107): a coded refusal keeps its own status and
/// code; a memory read failure or an unparsable config keeps theirs wherever
/// they surface; anything else keeps the old `http.Error` shape with the
/// caller's fallback status.
pub fn write_err(fallback: StatusCode, err: &RefuseError) -> Response {
    match err {
        RefuseError::MemoryRead { code } => refuse_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &Reason::new(code, err.to_string()),
        ),
        RefuseError::Coded { status, reason } => refuse_response(*status, reason),
        RefuseError::UnparsedConfig {
            path,
            line,
            excerpt,
            repair,
            recovered,
        } => refuse_response(
            StatusCode::CONFLICT,
            &Reason::new("config.unparsed", err.to_string())
                .with_param("path", path.clone())
                .with_param("line", *line)
                .with_param("excerpt", excerpt.clone())
                .with_param("repair", repair.clone())
                .with_param("recovered", recovered.clone()),
        ),
        RefuseError::AnswersMismatch {
            decision,
            model_ref,
        } => {
            let code = if *decision {
                MODEL_DECISION_ONLY
            } else {
                MODEL_NOT_DECISION
            };
            refuse_response(
                StatusCode::CONFLICT,
                &Reason::new(code, err.to_string()).with_param("model", model_ref.clone()),
            )
        }
        RefuseError::Plain(message) => http_error(fallback, message),
    }
}

// The shared refusal helpers (refuse_common.go). Each is one kind of failure
// with one code, so a frontend can say most failures without a code per route.

/// Go `badBody`: the parse error itself is never shown — it names types and
/// offsets, which tells a reader nothing and a translator less.
pub fn bad_body() -> Response {
    refuse_response(
        StatusCode::BAD_REQUEST,
        &Reason::new(BAD_BODY, "the request body could not be read"),
    )
}

pub fn missing_field(field: &str) -> Response {
    refuse_response(
        StatusCode::BAD_REQUEST,
        &Reason::new(MISSING_FIELD, format!("missing {field}")).with_param("field", field),
    )
}

/// Go `notFound`: only for things that could exist and do not. A name that is
/// merely invalid is a 400, never this.
pub fn not_found(kind: &str, name: &str) -> Response {
    refuse_response(
        StatusCode::NOT_FOUND,
        &Reason::new(NOT_FOUND, format!("no {kind} named {name}"))
            .with_param("kind", kind)
            .with_param("name", name),
    )
}

pub fn unknown_project(root: &str) -> Response {
    refuse_response(
        StatusCode::BAD_REQUEST,
        &Reason::new(UNKNOWN_PROJECT, "project is not open here").with_param("root", root),
    )
}

pub fn session_in_use(detail: &str) -> Response {
    refuse_response(
        StatusCode::CONFLICT,
        &Reason::new(SESSION_IN_USE, detail).with_param("detail", detail),
    )
}

pub fn bad_value(field: &str, allowed: &[&str]) -> Response {
    refuse_response(
        StatusCode::BAD_REQUEST,
        &Reason::new(
            BAD_VALUE,
            format!("{} is not one of {}", field, allowed.join(", ")),
        )
        .with_param("field", field)
        .with_param("allowed", allowed),
    )
}

pub fn session_busy() -> Response {
    refuse_response(
        StatusCode::CONFLICT,
        &Reason::new(SESSION_BUSY, "a turn owns this session until it lands"),
    )
}

pub fn method_not_allowed() -> Response {
    refuse_response(
        StatusCode::METHOD_NOT_ALLOWED,
        &Reason::new(METHOD_NOT_ALLOWED, "that method is not accepted here"),
    )
}

pub fn bad_content_type() -> Response {
    refuse_response(
        StatusCode::UNSUPPORTED_MEDIA_TYPE,
        &Reason::new(BAD_CONTENT_TYPE, "the body must be application/json"),
    )
}

/// Reads a JSON body under the 64KiB cap (OPT-004: the stream itself is
/// bounded, so a lying Content-Length or a chunked overrun is caught) and
/// rejects unknown fields the way Go `decodeBody`'s DisallowUnknownFields
/// does — a client sending a field this build does not have is not saying
/// what it thinks it is saying. Trailing bytes after the first value are
/// tolerated, as Go's single `Decode` tolerates them.
pub async fn decode_body<T: serde::de::DeserializeOwned>(body: Body) -> Result<T, RefuseError> {
    let raw = axum::body::to_bytes(body, BODY_LIMIT)
        .await
        .map_err(|_| RefuseError::Plain("request body could not be read".to_owned()))?;
    decode_json(&raw)
}

pub fn decode_json<T: serde::de::DeserializeOwned>(raw: &[u8]) -> Result<T, RefuseError> {
    let mut unknown = false;
    let mut de = serde_json::Deserializer::from_slice(raw);
    let value = serde_ignored::deserialize(&mut de, |_| unknown = true)
        .map_err(|_| RefuseError::Plain("request body did not parse".to_owned()))?;
    if unknown {
        return Err(RefuseError::Plain(
            "unknown fields in request body".to_owned(),
        ));
    }
    Ok(value)
}

#[allow(dead_code)]
fn _params_shape_check(entries: &[(&str, Value)]) -> JsonMap<String, Value> {
    json_params(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;

    async fn body_of(response: Response) -> (StatusCode, String) {
        let status = response.status();
        let body = to_bytes(response.into_body(), 1 << 20).await.unwrap();
        (status, String::from_utf8(body.to_vec()).unwrap())
    }

    #[tokio::test]
    async fn envelope_serializes_code_error_params_in_go_order() {
        let reason = Reason::new(NOT_FOUND, "no runtime named r9")
            .with_param("kind", "runtime")
            .with_param("name", "r9");
        let response = refuse_response(StatusCode::NOT_FOUND, &reason);
        let (status, body) = body_of(response).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert!(body.ends_with('\n'), "Go's encoder appends a newline");
        let json: serde_json::Value = serde_json::from_str(&body).unwrap();
        let keys: Vec<&str> = json
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(keys, ["code", "error", "params"]);
        assert_eq!(json["code"], NOT_FOUND);
        assert_eq!(json["params"]["kind"], "runtime");
        assert_eq!(json["params"]["name"], "r9");
    }

    #[tokio::test]
    async fn reason_without_params_omits_the_field() {
        let response = refuse_response(
            StatusCode::CONFLICT,
            &Reason::new(SESSION_BUSY, "a turn owns this session until it lands"),
        );
        let (_, body) = body_of(response).await;
        let json: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert!(json.get("params").is_none());
    }

    #[tokio::test]
    async fn write_err_keeps_a_coded_refusal_status_and_code() {
        let err = RefuseError::refusal(
            StatusCode::CONFLICT,
            SWITCH_MODEL,
            "cannot switch model while a turn is running",
        );
        let (status, body) = body_of(write_err(StatusCode::INTERNAL_SERVER_ERROR, &err)).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert!(body.contains(SWITCH_MODEL));
    }

    #[tokio::test]
    async fn write_err_falls_back_to_http_error_text() {
        let err = RefuseError::Plain("runtime open is not part of this build".to_owned());
        let response = write_err(StatusCode::INTERNAL_SERVER_ERROR, &err);
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(
            response.headers().get(CONTENT_TYPE),
            Some(&HeaderValue::from_static("text/plain; charset=utf-8"))
        );
        let (_, body) = body_of(response).await;
        assert_eq!(body, "runtime open is not part of this build\n");
    }

    #[test]
    fn decode_json_rejects_unknown_fields_and_bad_json_alike() {
        #[derive(serde::Deserialize)]
        #[allow(dead_code)]
        struct Req {
            root: String,
        }
        let ok: Req = decode_json(br#"{"root":"/w"}"#).unwrap();
        assert_eq!(ok.root, "/w");
        assert!(decode_json::<Req>(br#"{"root":"/w","extra":1}"#).is_err());
        assert!(decode_json::<Req>(b"not json").is_err());
        assert!(decode_json::<Req>(b"").is_err());
    }

    #[test]
    fn decode_json_tolerates_trailing_bytes_like_go_single_decode() {
        #[derive(serde::Deserialize)]
        #[allow(dead_code)]
        struct Req {
            root: String,
        }
        let ok: Req = decode_json(br#"{"root":"/w"}trailing"#).unwrap();
        assert_eq!(ok.root, "/w");
    }
}

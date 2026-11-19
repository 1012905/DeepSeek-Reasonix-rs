//! The auth gate (Go `auth.go`, `launch_token.go`, `host_guard.go`): the
//! none/token/invalid modes, the mutation launch-token requirement, the CSRF
//! content-type guard, and the DNS-rebound Host allowlist. Password mode is a
//! later-wave machine and is refused at construction (deny-everything), never
//! faked.

use std::collections::HashSet;
use std::sync::Mutex;

use axum::body::Body;
use axum::extract::Request;
use axum::extract::State;
use axum::http::header::{AUTHORIZATION, CACHE_CONTROL, CONTENT_TYPE, HOST, LOCATION, SET_COOKIE};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;

use crate::gate::{self, TOKEN_COOKIE, constant_time_eq};
use crate::refuse::{
    AUTH_LAUNCH_TOKEN_REQUIRED, BAD_CONTENT_TYPE, HOST_REJECTED, Reason, refuse_response,
};
use crate::router::RouterState;

/// Auth modes for the serve frontend (Go `authMode`). `Invalid` is an
/// unrecognized configuration: every request is denied rather than guessed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthMode {
    /// Explicit opt-out; mutations still need the launch token.
    None,
    /// Pre-shared token in cookie, bearer header, or the legacy query string.
    Token,
    /// Unrecognized mode: deny everything (auth.go `authInvalid`).
    Invalid,
}

impl AuthMode {
    /// The mode name the gate reports (auth.go `Mode()`).
    pub fn as_str(self) -> &'static str {
        match self {
            AuthMode::None => "none",
            AuthMode::Token => "token",
            AuthMode::Invalid => "invalid",
        }
    }
}

/// How long a token cookie lives (auth.go `sessionDuration`), in seconds.
const SESSION_MAX_AGE: i64 = 30 * 24 * 60 * 60;
/// 256-bit random token (auth.go `tokenByteLen`).
const TOKEN_BYTE_LEN: usize = 32;
/// `POST /auth/token` bodies are capped at 8KiB (auth.go handleTokenBootstrap).
const TOKEN_BOOTSTRAP_BODY_LIMIT: usize = 8 << 10;

/// Request extension marking a caller some boundary already authenticated
/// (Go `withHostGated`): the loopback gate, or — later waves — the in-process
/// transport or a paired device. `proven_operator` reads it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct HostGated;

/// The authentication middleware and its runtime state (Go `authGate`).
#[derive(Debug)]
pub struct AuthGate {
    mode: AuthMode,
    token: String,
    hosts: HostAllowlist,
}

impl AuthGate {
    /// Auth off: mutations still need the launch token (Go `newAuthGate`,
    /// authNone arm). An empty configured token is replaced by a generated
    /// one; minting can fail only if the OS CSPRNG does, and the caller
    /// decides the fallback (`build_router` degrades to deny-everything).
    pub fn none(configured: &str) -> Result<Self, getrandom::Error> {
        Ok(Self {
            mode: AuthMode::None,
            token: configured_token(configured)?,
            hosts: HostAllowlist::default(),
        })
    }

    /// Token mode: cookie, bearer, or legacy query parameter.
    pub fn token_mode(configured: &str) -> Result<Self, getrandom::Error> {
        Ok(Self {
            mode: AuthMode::Token,
            token: configured_token(configured)?,
            hosts: HostAllowlist::default(),
        })
    }

    /// An unrecognized mode denies everything (auth.go `authInvalid`).
    pub fn invalid() -> Self {
        Self {
            mode: AuthMode::Invalid,
            token: String::new(),
            hosts: HostAllowlist::default(),
        }
    }

    /// Password mode is not a machine of this build (docs/contract.md §3);
    /// the gate refuses everything rather than answering for a login page
    /// that does not exist.
    pub fn password_unavailable() -> Self {
        tracing::warn!("serve auth: password mode is not migrated; refusing every request");
        Self::invalid()
    }

    pub fn mode(&self) -> AuthMode {
        self.mode
    }

    /// Go `Token()`: the shared credential — the token mutations need when
    /// authentication is off, the credential token mode checks.
    pub fn token(&self) -> &str {
        &self.token
    }

    pub fn hosts(&self) -> &HostAllowlist {
        &self.hosts
    }

    /// Go `presentsLaunchToken`: the cookie `/auth/token` issues, or an
    /// `Authorization: Bearer` header, compared constant-time.
    pub fn presents_launch_token(&self, headers: &HeaderMap) -> bool {
        if self.token.is_empty() {
            return false;
        }
        if let Some(cookie) = gate::cookie_value(headers, TOKEN_COOKIE)
            && launch_token_equal(cookie, &self.token)
        {
            return true;
        }
        let auth = headers
            .get(AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .trim();
        auth.split_once(' ').is_some_and(|(scheme, value)| {
            scheme.eq_ignore_ascii_case("Bearer") && launch_token_equal(value, &self.token)
        })
    }

    /// Go `authenticated`: whether a request already carries the credential
    /// this gate accepts, for a handler reached through a path the gate
    /// leaves public. Token mode reads the cookie only, never the bearer.
    pub fn authenticated(&self, headers: &HeaderMap) -> bool {
        match self.mode {
            AuthMode::None => true,
            AuthMode::Invalid => false,
            AuthMode::Token => gate::cookie_value(headers, TOKEN_COOKIE)
                .is_some_and(|c| constant_time_eq(c.as_bytes(), self.token.as_bytes())),
        }
    }
}

fn configured_token(configured: &str) -> Result<String, getrandom::Error> {
    let token = configured.trim().to_owned();
    if token.is_empty() {
        generate_token()
    } else {
        Ok(token)
    }
}

/// Go `launchTokenEqual`: trimmed, non-empty, constant-time.
pub fn launch_token_equal(got: &str, want: &str) -> bool {
    let got = got.trim();
    !got.is_empty() && constant_time_eq(got.as_bytes(), want.as_bytes())
}

/// Go `generateToken`: 32 random bytes, base64url without padding. Fallible:
/// a CSPRNG failure is the caller's to answer, never a panic in library code.
pub fn generate_token() -> Result<String, getrandom::Error> {
    let mut bytes = [0u8; TOKEN_BYTE_LEN];
    getrandom::fill(&mut bytes)?;
    Ok(base64url_raw(&bytes))
}

fn base64url_raw(data: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[(n >> 18 & 63) as usize] as char);
        out.push(TABLE[(n >> 12 & 63) as usize] as char);
        if chunk.len() > 1 {
            out.push(TABLE[(n >> 6 & 63) as usize] as char);
        }
        if chunk.len() > 2 {
            out.push(TABLE[(n & 63) as usize] as char);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// The middleware pieces. Request order (outermost first): log -> auth ->
// hostGuard -> withPage -> csrfGuard -> mutationGate -> watchTreeWrites ->
// routes.
// ---------------------------------------------------------------------------

/// Go `authGate.middleware`: the mode gate. `/auth/token` is answered here —
/// before the CSRF guard sees it — because the shell trades its token for a
/// cookie without any other route existing yet.
pub(crate) async fn auth_middleware(
    State(state): State<RouterState>,
    request: Request,
    next: Next,
) -> Response {
    let gate = &state.auth;
    if gate.mode == AuthMode::Invalid {
        return deny();
    }
    if gate.mode == AuthMode::None {
        if request.uri().path() == "/auth/token" {
            return handle_token_bootstrap(request, gate).await;
        }
        return next.run(request).await;
    }
    // Token mode.
    if request.uri().path() == "/auth/token" {
        return handle_token_bootstrap(request, gate).await;
    }
    let has_query_token = request
        .uri()
        .query()
        .and_then(|q| query_param(q, "token"))
        .is_some();
    if !has_query_token && token_bootstrap_public_path(request.method(), request.uri().path()) {
        return next.run(request).await;
    }
    check_token(request, gate, next).await
}

/// Go `checkToken`: cookie/bearer first, then the legacy query parameter,
/// then deny.
async fn check_token(request: Request, gate: &AuthGate, next: Next) -> Response {
    // 1. Cookie or Authorization bearer (the fast path).
    if gate.presents_launch_token(request.headers()) {
        return next.run(request).await;
    }
    // 2. Legacy query parameter: on match it trades the URL token for a
    // persistent cookie and a redirect with the parameter stripped, so future
    // requests (including SSE) authenticate without the token in the URL.
    let query_token = request
        .uri()
        .query()
        .and_then(|q| query_param(q, "token"))
        .unwrap_or_default();
    if !query_token.is_empty() && constant_time_eq(query_token.as_bytes(), gate.token.as_bytes()) {
        let target = safe_redirect_target(
            &request
                .uri()
                .path_and_query()
                .map(|pq| pq.as_str().to_owned())
                .unwrap_or_else(|| "/".to_owned()),
        )
        .replace('\\', "/");
        let cleaned = strip_query_param(&target, "token");
        let mut response = redirect_response(StatusCode::FOUND, cleaned);
        set_cookie(&mut response, &gate.token);
        return response;
    }
    // 3. Not authenticated.
    deny()
}

/// Go `tokenBootstrapPublicPath`: the inert shell's entry points, public in
/// token mode so it can trade the fragment for a cookie before any API call.
/// Exactly one non-empty segment under `/sessions/`; API-like paths stay out.
pub fn token_bootstrap_public_path(method: &Method, path: &str) -> bool {
    if method != Method::GET && method != Method::HEAD {
        return false;
    }
    if path == "/" || path == "/assets/logo-wordmark.svg" {
        return true;
    }
    match path.strip_prefix("/sessions/") {
        Some(id) => !id.is_empty() && !id.contains('/'),
        None => false,
    }
}

/// Go `handleTokenBootstrap`: the token travels in a bounded JSON body rather
/// than the URL, keeping it out of request lines, access logs, browser
/// history, and referrers. Unknown fields pass (no DisallowUnknownFields
/// here); a second JSON value does not.
async fn handle_token_bootstrap(request: Request, gate: &AuthGate) -> Response {
    if request.method() != Method::POST {
        return crate::refuse::method_not_allowed();
    }
    if !content_type_is_json(request.headers()) {
        return crate::refuse::bad_content_type();
    }
    let body = match axum::body::to_bytes(request.into_body(), TOKEN_BOOTSTRAP_BODY_LIMIT).await {
        Ok(bytes) => bytes,
        Err(_) => return crate::refuse::bad_body(),
    };
    #[derive(Deserialize)]
    struct BootstrapBody {
        #[serde(default)]
        token: String,
    }
    let mut de = serde_json::Deserializer::from_slice(&body);
    let parsed = BootstrapBody::deserialize(&mut de);
    if parsed.is_err() || de.end().is_err() {
        return crate::refuse::bad_body();
    }
    if !constant_time_eq(parsed.unwrap().token.as_bytes(), gate.token.as_bytes()) {
        return deny();
    }
    let mut response = Response::builder()
        .status(StatusCode::NO_CONTENT)
        .header(CACHE_CONTROL, "no-store")
        .body(Body::empty())
        .expect("static response cannot fail to build");
    set_cookie(&mut response, &gate.token);
    response
}

fn content_type_is_json(headers: &HeaderMap) -> bool {
    content_type_base(headers).is_some_and(|ct| ct.eq_ignore_ascii_case("application/json"))
}

/// The content type up to the first `;`, trimmed (Go `strings.IndexByte(';')`).
fn content_type_base(headers: &HeaderMap) -> Option<&str> {
    let ct = headers.get(CONTENT_TYPE)?.to_str().ok()?;
    let base = ct.split(';').next().unwrap_or("");
    Some(base.trim())
}

fn set_cookie(response: &mut Response, token: &str) {
    let value = format!(
        "{TOKEN_COOKIE}={token}; Path=/; Max-Age={SESSION_MAX_AGE}; HttpOnly; SameSite=Lax"
    );
    if let Ok(header) = HeaderValue::from_str(&value) {
        response.headers_mut().append(SET_COOKIE, header);
    }
}

fn redirect_response(status: StatusCode, target: String) -> Response {
    (status, [(LOCATION, target)], Body::empty()).into_response()
}

/// Drops one query parameter from a path?query string, keeping the rest of
/// the query as the client sent it.
fn strip_query_param(path_and_query: &str, key: &str) -> String {
    match path_and_query.split_once('?') {
        None => path_and_query.to_owned(),
        Some((path, query)) => {
            let kept: Vec<&str> = query
                .split('&')
                .filter(|pair| pair.split_once('=').map_or(*pair, |(k, _)| k) != key)
                .collect();
            if kept.is_empty() {
                path.to_owned()
            } else {
                format!("{path}?{}", kept.join("&"))
            }
        }
    }
}

/// Go `deny`: 401 plain text, deliberately generic so it leaks nothing about
/// which auth mode is active.
pub fn deny() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        [(CONTENT_TYPE, "text/plain; charset=utf-8")],
        "Unauthorized\n",
    )
        .into_response()
}

/// Go `query_param` over the raw query string with `+` and `%XX` decoding
/// (Go `url.Query().Get`). A pair with an invalid escape is dropped, as Go's
/// ParseQuery drops it.
pub(crate) fn query_param(query: &str, key: &str) -> Option<String> {
    for pair in query.split('&') {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        let (Some(k), Some(v)) = (query_unescape(k), query_unescape(v)) else {
            continue;
        };
        if k == key {
            return Some(v);
        }
    }
    None
}

fn query_unescape(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' => {
                let hex = bytes.get(i + 1..i + 3)?;
                let hi = (hex[0] as char).to_digit(16)?;
                let lo = (hex[1] as char).to_digit(16)?;
                out.push((hi * 16 + lo) as u8);
                i += 3;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

/// Go `safeRedirectTarget`: same-origin paths only — no `//`, no absolute
/// URLs, no backslashes — so a redirect never becomes an open redirect.
pub fn safe_redirect_target(raw: &str) -> String {
    let mut raw = raw.trim().replace('\\', "/");
    if let Some(i) = raw.find('#') {
        raw.truncate(i);
    }
    if raw.is_empty() {
        return "/".to_owned();
    }
    if raw != "/" && (raw.len() <= 1 || !raw.starts_with('/') || raw.as_bytes()[1] == b'/') {
        return "/".to_owned();
    }
    // An absolute URL (a scheme prefix) never redirects; a relative path may
    // still carry a full URL inside a query value.
    if let Some(colon) = raw.find(':') {
        let before = &raw[..colon];
        if !before.is_empty()
            && !before.contains('/')
            && before
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
        {
            return "/".to_owned();
        }
    }
    raw
}

// ---------------------------------------------------------------------------
// hostGuard, csrfGuard, mutationGate.
// ---------------------------------------------------------------------------

/// Go `provenOperator`: a request some boundary already authenticated — here,
/// the loopback gate's mark (the in-process transport and paired devices are
/// later-wave machines).
pub(crate) fn proven_operator(request: &Request) -> bool {
    request.extensions().get::<HostGated>().is_some()
}

/// Go `authGate.hostGuard`: while authentication is off, the Host header must
/// be one this listener was bound to — a DNS-rebound page reaches loopback
/// under its own name, and nothing else stops it reading an open serve. A
/// request carrying the launch token, or one that passed a gate that pins
/// Host itself, is not a rebound page and passes.
pub(crate) async fn host_guard_middleware(
    State(state): State<RouterState>,
    request: Request,
    next: Next,
) -> Response {
    let gate = &state.auth;
    if gate.mode == AuthMode::None {
        let host = request
            .headers()
            .get(HOST)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if !gate.hosts.admits(host)
            && !proven_operator(&request)
            && !gate.presents_launch_token(request.headers())
        {
            return refuse_response(
                StatusCode::MISDIRECTED_REQUEST,
                &Reason::new(HOST_REJECTED, "this serve does not answer for that host"),
            );
        }
    }
    next.run(request).await
}

/// Go `csrfGuard` (serve.go:372-387): POST must carry `application/json`,
/// except the binary upload routes taking a file as the body. There is no
/// CORS: same-origin policy is the protection, and a dev frontend proxies
/// through its own origin rather than being allowed in.
pub(crate) async fn csrf_guard_middleware(request: Request, next: Next) -> Response {
    if request.method() == Method::POST {
        let base = content_type_base(request.headers()).unwrap_or("");
        let binary_upload = binary_upload_path(request.uri().path())
            && base.eq_ignore_ascii_case("application/octet-stream");
        if !base.eq_ignore_ascii_case("application/json") && !binary_upload {
            return refuse_response(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                &Reason::new(BAD_CONTENT_TYPE, "the body must be application/json"),
            );
        }
    }
    next.run(request).await
}

/// Go `binaryUploadPath`: the routes that take a file as the body rather than
/// as JSON — the attachment upload and the wallpaper picture.
pub fn binary_upload_path(path: &str) -> bool {
    path.ends_with("/attachments/upload") || path.ends_with("/appearance/wallpaper")
}

/// Go `authGate.mutationGate` (launch_token.go:57-65): an auth-disabled
/// serve's mutations are held to the launch token. The METHOD decides rather
/// than a list of routes — any mutation can widen what the agent may do.
pub(crate) async fn mutation_gate_middleware(
    State(state): State<RouterState>,
    request: Request,
    next: Next,
) -> Response {
    let gate = &state.auth;
    if gate.mode == AuthMode::None
        && !safe_method(request.method())
        && !proven_operator(&request)
        && !gate.presents_launch_token(request.headers())
    {
        return refuse_response(
            StatusCode::FORBIDDEN,
            &Reason::new(
                AUTH_LAUNCH_TOKEN_REQUIRED,
                "approvals require the launch token",
            ),
        );
    }
    next.run(request).await
}

/// Go `safeMethod`.
pub fn safe_method(method: &Method) -> bool {
    method == Method::GET || method == Method::HEAD || method == Method::OPTIONS
}

// ---------------------------------------------------------------------------
// Host allowlist (host_guard.go).
// ---------------------------------------------------------------------------

/// Which Host headers an auth-disabled serve answers (Go `hostAllowlist`).
#[derive(Debug, Default)]
pub struct HostAllowlist {
    names: Mutex<HashSet<String>>,
}

impl HostAllowlist {
    /// Go `ListenOn`: records the address serve is bound to. A wildcard bind
    /// answers for this machine's interface addresses as IP literals — std has
    /// no interface enumeration, so a wildcard bind records nothing beyond a
    /// warning and the loopback hosts every desktop bind already admits.
    pub fn listen_on(&self, addr: &str) {
        let host = host_only_name(addr);
        match host.parse::<std::net::IpAddr>() {
            Ok(ip) if ip.is_unspecified() => {
                tracing::warn!(
                    "serve: wildcard bind interface enumeration is not implemented; \
                     only loopback Host headers pass the host guard"
                );
            }
            _ => self.allow_inner(&host),
        }
    }

    /// Go `Allow`: a name serve answers besides its bound address, such as
    /// the host of `--public-url` a proxy forwards.
    pub fn allow(&self, host: &str) {
        self.allow_inner(&host_only_name(host));
    }

    fn allow_inner(&self, host: &str) {
        if host.is_empty() {
            return;
        }
        self.names
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(host.to_owned());
    }

    /// Go `admits`: an empty or loopback host always passes; anything else
    /// must be on the recorded list.
    pub fn admits(&self, hostport: &str) -> bool {
        let host = host_only_name(hostport);
        if host.is_empty() || is_loopback_host(&host) {
            return true;
        }
        self.names
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(&host)
    }
}

/// Go `hostOnlyName`: the host of a hostport, lowercased, brackets trimmed.
pub fn host_only_name(hostport: &str) -> String {
    let host = hostport.trim();
    let host = if let Some(rest) = host.strip_prefix('[') {
        match rest.split_once(']') {
            Some((inner, _)) => inner,
            None => host,
        }
    } else if host.matches(':').count() > 1 {
        // Unbracketed IPv6 literal without a port: SplitHostPort errors, and
        // Go keeps the whole string.
        host
    } else {
        match host.rsplit_once(':') {
            Some((h, _)) => h,
            None => host,
        }
    };
    host.trim_matches(|c| c == '[' || c == ']')
        .to_ascii_lowercase()
}

/// Go `isLoopbackHost` (auth.go): localhost by any case, or a loopback IP.
pub fn is_loopback_host(hostport: &str) -> bool {
    let host = hostport.trim();
    if host.is_empty() {
        return false;
    }
    let host = host_only_name(host);
    if host == "localhost" {
        return true;
    }
    host.parse::<std::net::IpAddr>()
        .is_ok_and(|ip| ip.is_loopback())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_bootstrap_public_paths_are_exactly_the_shell_entries() {
        assert!(token_bootstrap_public_path(&Method::GET, "/"));
        assert!(token_bootstrap_public_path(
            &Method::HEAD,
            "/assets/logo-wordmark.svg"
        ));
        assert!(token_bootstrap_public_path(&Method::GET, "/sessions/r1"));
        assert!(!token_bootstrap_public_path(
            &Method::GET,
            "/sessions/r1/extra"
        ));
        assert!(!token_bootstrap_public_path(&Method::GET, "/sessions/"));
        assert!(!token_bootstrap_public_path(&Method::GET, "/sessions"));
        assert!(!token_bootstrap_public_path(&Method::POST, "/"));
        assert!(!token_bootstrap_public_path(&Method::GET, "/runtimes"));
    }

    #[test]
    fn launch_token_equal_trims_and_rejects_empty() {
        assert!(launch_token_equal(" tkn ", "tkn"));
        assert!(!launch_token_equal("  ", "tkn"));
        assert!(!launch_token_equal("tkn", "other"));
    }

    #[test]
    fn generate_token_is_43_unpadded_base64url_chars() {
        let token = generate_token().unwrap();
        assert_eq!(token.len(), 43, "32 bytes -> 43 raw base64url chars");
        assert!(
            token
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        );
        assert_ne!(generate_token().unwrap(), token);
    }

    #[test]
    fn host_allowlist_admits_loopback_and_recorded_names() {
        let hosts = HostAllowlist::default();
        assert!(hosts.admits("127.0.0.1:9000"));
        assert!(hosts.admits("localhost:80"));
        assert!(hosts.admits("[::1]:80"));
        assert!(!hosts.admits("example.com:80"));
        hosts.listen_on("10.0.0.5:8080");
        assert!(hosts.admits("10.0.0.5:8080"));
        hosts.allow("Proxy.Example.com");
        assert!(hosts.admits("proxy.example.com:443"));
        assert!(hosts.admits(""));
        assert!(!hosts.admits("0.0.0.0:8080"));
    }

    #[test]
    fn host_only_name_strips_ports_and_brackets() {
        assert_eq!(host_only_name("Example.com:80"), "example.com");
        assert_eq!(host_only_name("[::1]:8080"), "::1");
        assert_eq!(host_only_name("::1"), "::1");
        assert_eq!(host_only_name("  Host  "), "host");
    }

    #[test]
    fn query_params_decode_plus_and_percent() {
        assert_eq!(
            query_param("a=1&token=hello+world", "token"),
            Some("hello world".to_owned())
        );
        assert_eq!(query_param("token=%2Fx", "token"), Some("/x".to_owned()));
        assert_eq!(query_param("token", "token"), Some(String::new()));
        assert_eq!(query_param("other=1", "token"), None);
        // A pair with an invalid escape is dropped, as Go's ParseQuery does.
        assert_eq!(query_param("token=%zz", "token"), None);
    }

    #[test]
    fn safe_redirect_target_sanitizes() {
        assert_eq!(safe_redirect_target("/x?y=1"), "/x?y=1");
        assert_eq!(safe_redirect_target("//evil.com/x"), "/");
        // Backslashes normalize to slashes, so this is a plain path.
        assert_eq!(safe_redirect_target(r"\evil"), "/evil");
        assert_eq!(safe_redirect_target("http://evil.com"), "/");
        assert_eq!(safe_redirect_target(""), "/");
        assert_eq!(safe_redirect_target("/a#frag"), "/a");
        // A full URL inside a query value stays put.
        assert_eq!(safe_redirect_target("/x?url=http://y"), "/x?url=http://y");
    }

    #[test]
    fn csrf_binary_exceptions_are_the_two_upload_suffixes() {
        assert!(binary_upload_path("/attachments/upload"));
        assert!(binary_upload_path("/rt/r1/attachments/upload"));
        assert!(binary_upload_path("/appearance/wallpaper"));
        assert!(!binary_upload_path("/attachments"));
        assert!(!binary_upload_path("/appearance/wallpaper/scene"));
    }

    #[test]
    fn strip_query_param_only_drops_the_named_key() {
        assert_eq!(
            strip_query_param("/x?a=1&token=t&b=2", "token"),
            "/x?a=1&b=2"
        );
        assert_eq!(strip_query_param("/x?token=t", "token"), "/x");
        assert_eq!(strip_query_param("/x", "token"), "/x");
    }
}

//! The password-mode machine (Go `auth.go` password arms,
//! docs/contracts/http.md §2.3): bcrypt verification, the HMAC-signed
//! `reasonix_session` cookie, the PBKDF2 session key, the login page, and the
//! rate-limited submit. `/login` is answered inside `auth_middleware`, before
//! the CSRF guard — the form posts `application/x-www-form-urlencoded`, not
//! JSON.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::body::Body;
use axum::extract::Request;
use axum::http::header::{CONTENT_TYPE, SET_COOKIE};
use axum::http::{HeaderValue, Method, StatusCode};
use axum::response::Response;
use bytes::Bytes;
use hmac::{Hmac, Mac};
use sha2::Sha256;

use crate::auth::AuthGate;
use crate::gate::{constant_time_eq, cookie_value};
use crate::refuse;

/// The password-session cookie (auth.go `cookieSession`).
pub const SESSION_COOKIE: &str = "reasonix_session";
/// Where login returns to (auth.go `cookieRedirect`); 5 minutes (auth.go:393).
pub const REDIRECT_COOKIE: &str = "reasonix_redirect";
/// How long the post-login redirect target is remembered, seconds.
pub const REDIRECT_COOKIE_MAX_AGE: i64 = 300;
/// bcrypt cost factor (auth.go `bcryptCost`).
pub const BCRYPT_COST: u32 = 12;
/// Deterministic session-key rounds (auth.go `pbkdf2Iter`).
pub const PBKDF2_ITERATIONS: u32 = 4096;
/// The fixed PBKDF2 salt (auth.go `sessionKeyForPasswordHash`).
pub const PBKDF2_SALT: &[u8] = b"reasonix serve session key";
/// How long a password session lasts (auth.go `sessionDuration`), seconds.
pub const SESSION_DURATION_SECS: i64 = 30 * 24 * 60 * 60;
/// Login form body cap (auth.go `maxLoginFormBytes`).
pub const LOGIN_FORM_LIMIT: usize = 4 << 10;
/// Login attempts per IP per window (auth.go `rateLimitMax`).
pub const RATE_LIMIT_MAX: u32 = 5;
/// Login rate-limit window (auth.go `rateLimitWin`).
pub const RATE_LIMIT_WINDOW: Duration = Duration::from_secs(60);

type HmacSha256 = Hmac<Sha256>;

/// The embedded login page (auth.go `//go:embed login.html`).
static LOGIN_HTML: &[u8] = include_bytes!("login.html");

/// Errors a fresh hash can hit (Go `GenerateFromPassword`).
#[derive(Debug, thiserror::Error)]
pub enum HashPasswordError {
    #[error("password exceeds bcrypt's 72-byte input limit")]
    PasswordTooLong,
    #[error("random source failed: {0}")]
    Random(#[from] getrandom::Error),
    #[error("bcrypt failed: {0}")]
    Bcrypt(#[from] bcrypt::BcryptError),
}

/// Go `HashPassword`: bcrypt at the production cost, `$2a$` like Go's
/// x/crypto emits — the Rust crate's native spelling is `$2b$`, and both
/// verifiers read either prefix.
pub fn hash_password(password: &[u8]) -> Result<String, HashPasswordError> {
    if password.len() > 72 {
        return Err(HashPasswordError::PasswordTooLong);
    }
    let mut salt = [0u8; 16];
    getrandom::fill(&mut salt)?;
    Ok(bcrypt::hash_with_salt(password, BCRYPT_COST, salt)?
        .format_for_version(bcrypt::Version::TwoA))
}

/// Go `sessionKeyForPasswordHash`: PBKDF2-HMAC-SHA256 over the password hash —
/// deterministic, so sessions survive a restart while the hash stands — or 32
/// random bytes when no hash is configured.
pub fn session_key_for_password_hash(password_hash: &str) -> Result<[u8; 32], getrandom::Error> {
    if password_hash.is_empty() {
        let mut key = [0u8; 32];
        getrandom::fill(&mut key)?;
        return Ok(key);
    }
    Ok(pbkdf2_one_block(
        password_hash.as_bytes(),
        PBKDF2_SALT,
        PBKDF2_ITERATIONS,
    ))
}

/// PBKDF2 with dkLen equal to the hash size: one block, RFC 2898 §5.2 —
/// matching Go `pbkdf2.Key(sha256.New, ..., 32)` byte for byte.
fn pbkdf2_one_block(password: &[u8], salt: &[u8], rounds: u32) -> [u8; 32] {
    let mut mac = HmacSha256::new_from_slice(password).expect("HMAC accepts any key length");
    mac.update(salt);
    mac.update(&[0, 0, 0, 1]);
    let mut block: [u8; 32] = mac.finalize().into_bytes().into();
    let mut u = block;
    for _ in 1..rounds {
        let mut mac = HmacSha256::new_from_slice(password).expect("HMAC accepts any key length");
        mac.update(&u);
        u = mac.finalize().into_bytes().into();
        for (b, ui) in block.iter_mut().zip(u.iter()) {
            *b ^= ui;
        }
    }
    block
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(data);
    mac.finalize().into_bytes().into()
}

/// Go `signSession`: `base64url(expiry_base10|random_16_bytes).hex(hmac)`.
pub fn sign_session(sess_key: &[u8; 32]) -> Result<String, getrandom::Error> {
    let expiry = now_unix() + SESSION_DURATION_SECS;
    let mut nonce = [0u8; 16];
    getrandom::fill(&mut nonce)?;
    let payload = format!("{expiry}|{}", crate::auth::base64url_raw(&nonce));
    let signature = hex_encode(&hmac_sha256(sess_key, payload.as_bytes()));
    Ok(format!("{payload}.{signature}"))
}

/// Go `verifySession`: the HMAC comparison is constant-time, then the expiry
/// must still be in the future.
pub fn verify_session(sess_key: &[u8; 32], token: &str) -> bool {
    let Some((payload, sig_hex)) = token.rsplit_once('.') else {
        return false;
    };
    let Some(sig) = hex_decode(sig_hex) else {
        return false;
    };
    if !constant_time_eq(&sig, &hmac_sha256(sess_key, payload.as_bytes())) {
        return false;
    }
    let Some((expiry, _nonce)) = payload.split_once('|') else {
        return false;
    };
    expiry.parse::<i64>().is_ok_and(|e| now_unix() < e)
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// Go `encoding/hex` for the signature: lowercase out, either case in.
fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(char::from_digit((b >> 4) as u32, 16).unwrap_or('0'));
        out.push(char::from_digit((b & 15) as u32, 16).unwrap_or('0'));
    }
    out
}

fn hex_decode(s: &str) -> Option<Vec<u8>> {
    let bytes = s.as_bytes();
    if !bytes.len().is_multiple_of(2) {
        return None;
    }
    bytes
        .chunks(2)
        .map(|pair| Some(hex_nibble(pair[0])? << 4 | hex_nibble(pair[1])?))
        .collect()
}

fn hex_nibble(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Rate limiter (auth.go rateLimit).
// ---------------------------------------------------------------------------

/// Per-IP login attempt windows (auth.go `rateLimit`). Go sweeps the map from
/// a 2-minute ticker; the same sweep runs here inside allow() once the map
/// grows past a bound, so an unattended server still cannot grow it forever.
#[derive(Debug, Default)]
pub struct RateLimiter {
    attempts: Mutex<HashMap<String, RateWindow>>,
}

#[derive(Debug, Clone, Copy)]
struct RateWindow {
    count: u32,
    start: Instant,
}

/// The map size that triggers the opportunistic expired-window sweep.
const RATE_MAP_SWEEP: usize = 1024;

impl RateLimiter {
    /// Go `allow`: the first attempt opens the window; past `RATE_LIMIT_MAX`
    /// inside one window the IP waits for it to expire.
    pub fn allow(&self, ip: &str) -> bool {
        let mut attempts = self.attempts.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        if attempts.len() >= RATE_MAP_SWEEP {
            attempts.retain(|_, w| now.duration_since(w.start) <= RATE_LIMIT_WINDOW);
        }
        let window = attempts.entry(ip.to_owned()).or_insert(RateWindow {
            count: 0,
            start: now,
        });
        if now.duration_since(window.start) > RATE_LIMIT_WINDOW {
            *window = RateWindow {
                count: 1,
                start: now,
            };
            return true;
        }
        window.count += 1;
        window.count <= RATE_LIMIT_MAX
    }

    /// Rewinds one IP's window for tests (Go's test writes the field directly).
    #[cfg(test)]
    pub(crate) fn expire_window(&self, ip: &str) {
        let mut attempts = self.attempts.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(w) = attempts.get_mut(ip) {
            w.start -= RATE_LIMIT_WINDOW + RATE_LIMIT_WINDOW;
        }
    }
}

// ---------------------------------------------------------------------------
// The login routes (auth.go handleLogin / loginPage / loginSubmit).
// ---------------------------------------------------------------------------

/// Go `handleLogin`: GET serves the page, POST submits, anything else is 405.
pub(crate) async fn handle_login(request: Request, gate: &AuthGate) -> Response {
    let method = request.method().clone();
    if method == Method::GET {
        return login_page();
    }
    if method == Method::POST {
        return login_submit(request, gate).await;
    }
    refuse::method_not_allowed()
}

/// Go `loginPage`: 200 with the embedded HTML.
fn login_page() -> Response {
    html_response(StatusCode::OK, Bytes::from_static(LOGIN_HTML))
}

/// Go `loginPageWithError`: the page with `<!--ERROR-->` replaced once by the
/// escaped error block. Go writes 401 here and its tests assert it; http.md
/// §2.3's "(all 200 HTML)" misstates its own citation (auth.go:608-614).
fn login_page_with_error(message: &str) -> Response {
    let block = format!(r#"<div class="error">{}</div>"#, html_escape(message));
    let html = String::from_utf8_lossy(LOGIN_HTML).replacen("<!--ERROR-->", &block, 1);
    html_response(StatusCode::UNAUTHORIZED, Bytes::from(html))
}

fn html_response(status: StatusCode, body: Bytes) -> Response {
    let mut response = Response::new(Body::from(body));
    *response.status_mut() = status;
    response.headers_mut().insert(
        CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    response
}

/// Go `htmlEscape`, `&` first so the entities are not double-escaped.
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&#34;")
        .replace('\'', "&#39;")
}

/// Go's rate-limited login answer: plain text, not a Reason (http.md §4.2),
/// without the `nosniff` header Go's http.Error would have added.
fn too_many_attempts() -> Response {
    let mut response = Response::new(Body::from(Bytes::from_static(
        b"Too many attempts. Please wait a minute.\n",
    )));
    *response.status_mut() = StatusCode::TOO_MANY_REQUESTS;
    response.headers_mut().insert(
        CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    response
}

/// Go `loginSubmit`: rate limit first, then the capped form, then bcrypt;
/// failures re-render the page with their message, success issues the session
/// cookie and redirects to the sanitized target.
async fn login_submit(request: Request, gate: &AuthGate) -> Response {
    let ip = gate.client_ip(&request);
    if !gate.rate_limiter().allow(&ip) {
        tracing::warn!("serve/auth: rate-limited login attempt, ip {ip}");
        return too_many_attempts();
    }
    let secure = gate.is_tls(&request);
    let dest = crate::auth::safe_redirect_target(
        cookie_value(request.headers(), REDIRECT_COOKIE).unwrap_or("/"),
    );
    let pairs = match read_login_form(request).await {
        Ok(pairs) => pairs,
        Err(()) => return refuse::bad_body(),
    };
    if form_password(&pairs).is_empty() {
        return login_page_with_error("Password is required.");
    }
    if gate.password_hash().is_empty() {
        tracing::error!("serve/auth: cannot verify password — no password_hash configured");
        return login_page_with_error("Server not configured for password authentication.");
    }
    let ok = bcrypt::verify(form_password(&pairs), gate.password_hash()).unwrap_or(false);
    if !ok {
        return login_page_with_error("Invalid password.");
    }
    let session = match gate.sign_session() {
        Ok(session) => session,
        Err(err) => {
            tracing::error!("serve/auth: session signing failed: {err}");
            return refuse::http_error(StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error");
        }
    };
    let mut response = crate::auth::redirect_response(StatusCode::FOUND, dest);
    set_auth_cookie(&mut response, REDIRECT_COOKIE, "", 0, secure);
    set_auth_cookie(
        &mut response,
        SESSION_COOKIE,
        &session,
        SESSION_DURATION_SECS,
        secure,
    );
    response
}

/// Go `setAuthCookie`: `Path=/`, `HttpOnly`, `SameSite=Lax`, `Secure` when
/// the request is TLS (directly or through a trusted proxy). `max_age <= 0`
/// spells the Go `Max-Age=0` delete.
pub(crate) fn set_auth_cookie(
    response: &mut Response,
    name: &str,
    value: &str,
    max_age: i64,
    secure: bool,
) {
    let mut cookie = format!("{name}={value}; Path=/; Max-Age={max_age}; HttpOnly; SameSite=Lax");
    if secure {
        cookie.push_str("; Secure");
    }
    if let Ok(header) = HeaderValue::from_str(&cookie) {
        response.headers_mut().append(SET_COOKIE, header);
    }
}

// ---------------------------------------------------------------------------
// The form (auth.go ParseForm + FormValue over url.ParseQuery).
// ---------------------------------------------------------------------------

/// One decoded form: key/value byte pairs in FormValue's order.
type FormPairs = Vec<(Vec<u8>, Vec<u8>)>;

/// Go `r.ParseForm`: the urlencoded body is read only when the content type
/// says so, body pairs are kept ahead of query pairs (FormValue's order), and
/// a pair that fails to decode fails the whole parse (bad_body upstream).
async fn read_login_form(request: Request) -> Result<FormPairs, ()> {
    let (parts, body) = request.into_parts();
    let mut pairs = match crate::auth::content_type_base(&parts.headers) {
        Some(ct) if ct.eq_ignore_ascii_case("application/x-www-form-urlencoded") => {
            let bytes = axum::body::to_bytes(body, LOGIN_FORM_LIMIT)
                .await
                .map_err(|_| ())?;
            parse_urlencoded(bytes.as_ref())?
        }
        _ => Vec::new(),
    };
    pairs.extend(parse_urlencoded(
        parts.uri.query().unwrap_or("").as_bytes(),
    )?);
    Ok(pairs)
}

/// Go `url.ParseQuery`: `+` is a space, `%XX` decodes to its byte, an invalid
/// escape drops the pair and fails the parse, an empty key is skipped, and a
/// `;` in a segment is the invalid-separator error.
fn parse_urlencoded(raw: &[u8]) -> Result<FormPairs, ()> {
    let mut pairs = Vec::new();
    let mut failed = false;
    for segment in raw.split(|&b| b == b'&') {
        if segment.contains(&b';') {
            failed = true;
            continue;
        }
        let (key, value) = match segment.iter().position(|&b| b == b'=') {
            Some(i) => (&segment[..i], &segment[i + 1..]),
            None => (segment, &[][..]),
        };
        if key.is_empty() {
            continue;
        }
        let (Some(k), Some(v)) = (form_unescape(key), form_unescape(value)) else {
            failed = true;
            continue;
        };
        pairs.push((k, v));
    }
    if failed {
        return Err(());
    }
    Ok(pairs)
}

/// Go `url.QueryUnescape` for form components: `+` becomes a space and `%XX`
/// decodes to the raw byte, so any password bytes survive to the compare.
fn form_unescape(raw: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        match raw[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' => {
                let hex = raw.get(i + 1..i + 3)?;
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
    Some(out)
}

/// Go `FormValue`: the first `password` pair — body pairs sit ahead of the
/// query's.
fn form_password(pairs: &[(Vec<u8>, Vec<u8>)]) -> &[u8] {
    pairs
        .iter()
        .find(|(k, _)| k == b"password")
        .map(|(_, v)| v.as_slice())
        .unwrap_or(&[])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::router::{RouterConfig, build_router_with_auth};
    use crate::test_support::tempdir;
    use axum::extract::ConnectInfo;
    use axum::http::HeaderMap;
    use axum::http::header::ACCEPT;
    use reasonix_session::hub::{Hub, HubPaths};
    use std::net::{IpAddr, SocketAddr};
    use std::sync::Arc;
    use tower::ServiceExt as _ServiceExt;

    // ── Vectors generated with `go run` against auth.go's own machines ──

    /// pbkdf2.Key(sha256.New, "test-password", PBKDF2_SALT, 4096, 32).
    const GO_PBKDF2_TEST_PASSWORD: &str =
        "f42fd06ff1b6d178c6429160dcb477db1cca2455be7be57758805040603c41b3";
    /// pbkdf2.Key(sha256.New, "$2a$12$abcdefghijklmnopqrstuv", PBKDF2_SALT, 4096, 32).
    const GO_PBKDF2_HASH_LIKE: &str =
        "17c21c02abeafd89a58e8758c058b55625719a1fc49827f4d70a0289db4cd47d";
    /// x/crypto/bcrypt GenerateFromPassword("correct", 12) — a $2a$ hash.
    const GO_BCRYPT_CORRECT: &str = "$2a$12$TSrt3XrF1WdX7RvtOr4i/e.3rlJ3t23gHxtrlWlx2YkN6DbxuXkjO";
    /// HMAC-SHA256 over GO_HMAC_PAYLOAD with the key Go derived from the
    /// stored bcrypt hash of "correct" (the session machine's exact math).
    const GO_HMAC_KEY: &str = "49cec10fd0e8517eff2069d0c1301f915fdb7fe5f025515d5052c66673fa2724";
    const GO_HMAC_PAYLOAD: &str = "4102444800|dGhpcyBpcyBhIHRlc3Qgbm9uY2Uh";
    const GO_HMAC_SIG: &str = "6d86556d5104832133fe271c9ed57d8e4a4de3fa932af6045e09172029b82fc3";

    /// bcrypt cost 4 with a fixed salt: verifies read the cost from the hash,
    /// so route-level oracles stay fast and only the cost-12 oracle above
    /// pays the production price once.
    fn cheap_hash(password: &str) -> String {
        bcrypt::hash_with_salt(password, 4, [7u8; 16])
            .unwrap()
            .format_for_version(bcrypt::Version::TwoA)
    }

    fn password_router(hash: &str) -> axum::Router {
        let tmp = tempdir();
        build_router_with_auth(
            Arc::new(Hub::new(HubPaths {
                state_root: tmp.path().to_path_buf(),
            })),
            RouterConfig::default(),
            Arc::new(AuthGate::password(hash, false).unwrap()),
        )
    }

    fn password_router_behind_proxy(hash: &str) -> axum::Router {
        let tmp = tempdir();
        build_router_with_auth(
            Arc::new(Hub::new(HubPaths {
                state_root: tmp.path().to_path_buf(),
            })),
            RouterConfig::default(),
            Arc::new(AuthGate::password(hash, true).unwrap()),
        )
    }

    fn get(path: &'static str, accept: Option<&'static str>, cookie: Option<String>) -> Request {
        let mut builder = axum::extract::Request::builder()
            .method(Method::GET)
            .uri(path);
        if let Some(accept) = accept {
            builder = builder.header(ACCEPT, accept);
        }
        if let Some(cookie) = cookie {
            builder = builder.header(axum::http::header::COOKIE, cookie);
        }
        builder.body(Body::empty()).unwrap()
    }

    fn login_post(body: &str, extras: &[(&'static str, &str)]) -> Request {
        let mut builder = axum::extract::Request::builder()
            .method(Method::POST)
            .uri("/login")
            .header(CONTENT_TYPE, "application/x-www-form-urlencoded");
        for (name, value) in extras {
            builder = builder.header(*name, *value);
        }
        builder.body(Body::from(body.to_owned())).unwrap()
    }

    async fn oneshot(router: axum::Router, request: Request) -> Response {
        router
            .oneshot(request)
            .await
            .expect("router call cannot fail")
    }

    /// (status, body, Set-Cookie headers, Location header).
    async fn parts_of(response: Response) -> (StatusCode, String, Vec<String>, Option<String>) {
        let (parts, body) = response.into_parts();
        let bytes = axum::body::to_bytes(body, 1 << 20).await.unwrap();
        let cookies = parts
            .headers
            .get_all(SET_COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .map(str::to_owned)
            .collect();
        let location = parts
            .headers
            .get(axum::http::header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        (
            parts.status,
            String::from_utf8_lossy(&bytes).into_owned(),
            cookies,
            location,
        )
    }

    // ── Crypto vectors ──

    #[test]
    fn pbkdf2_matches_the_go_derivation() {
        let key = session_key_for_password_hash("test-password").unwrap();
        assert_eq!(hex_encode(&key), GO_PBKDF2_TEST_PASSWORD);
        let key = pbkdf2_one_block(
            b"$2a$12$abcdefghijklmnopqrstuv",
            PBKDF2_SALT,
            PBKDF2_ITERATIONS,
        );
        assert_eq!(hex_encode(&key), GO_PBKDF2_HASH_LIKE);
    }

    #[test]
    fn hmac_matches_the_go_signature() {
        let key = hex_decode(GO_HMAC_KEY).unwrap();
        assert_eq!(
            hex_encode(&hmac_sha256(&key, GO_HMAC_PAYLOAD.as_bytes())),
            GO_HMAC_SIG
        );
    }

    #[test]
    fn bcrypt_verifies_a_go_generated_hash() {
        assert!(bcrypt::verify(b"correct", GO_BCRYPT_CORRECT).unwrap());
        assert!(!bcrypt::verify(b"wrong", GO_BCRYPT_CORRECT).unwrap());
    }

    #[test]
    fn hash_password_matches_the_go_format() {
        let first = hash_password(b"test-password").unwrap();
        assert!(first.starts_with("$2a$12$"), "{first}");
        assert!(bcrypt::verify(b"test-password", &first).unwrap());
        // Same password, random salt: the hashes differ.
        let second = hash_password(b"test-password").unwrap();
        assert_ne!(first, second);
    }

    #[test]
    fn hash_password_refuses_a_password_past_bcrypts_limit() {
        let long = vec![b'a'; 73];
        assert!(matches!(
            hash_password(&long),
            Err(HashPasswordError::PasswordTooLong)
        ));
    }

    #[test]
    fn hex_round_trips_both_cases() {
        assert_eq!(hex_encode(&[0x00, 0x0f, 0xff, 0xab]), "000fffab");
        assert_eq!(
            hex_decode("000fFFab").unwrap(),
            vec![0x00, 0x0f, 0xff, 0xab]
        );
        assert_eq!(hex_decode("abc"), None);
        assert_eq!(hex_decode("zz"), None);
        assert_eq!(hex_decode(""), Some(Vec::new()));
    }

    // ── Session sign/verify (auth_test.go password-mode block) ──

    #[test]
    fn sign_and_verify_session_round_trips() {
        let gate = AuthGate::password(&cheap_hash("test"), false).unwrap();
        let tok = gate.sign_session().unwrap();
        let key = gate.session_key_for_tests();
        assert!(verify_session(&key, &tok));
    }

    #[test]
    fn session_verifies_across_restart_with_same_password_hash() {
        let hash = cheap_hash("test");
        let gate1 = AuthGate::password(&hash, false).unwrap();
        let tok = gate1.sign_session().unwrap();
        let gate2 = AuthGate::password(&hash, false).unwrap();
        let key = gate2.session_key_for_tests();
        assert!(
            verify_session(&key, &tok),
            "a session signed with a persisted password hash should verify after restart"
        );
    }

    #[test]
    fn session_rejects_a_different_password_hash() {
        let gate1 = AuthGate::password(&cheap_hash("first"), false).unwrap();
        let tok = gate1.sign_session().unwrap();
        let gate2 = AuthGate::password(&cheap_hash("second"), false).unwrap();
        let key = gate2.session_key_for_tests();
        assert!(!verify_session(&key, &tok));
    }

    #[test]
    fn session_key_is_random_without_a_hash() {
        let gate1 = AuthGate::password("", false).unwrap();
        let gate2 = AuthGate::password("", false).unwrap();
        assert_ne!(gate1.session_key_for_tests(), gate2.session_key_for_tests());
    }

    #[test]
    fn tampered_session_does_not_verify() {
        let gate = AuthGate::password(&cheap_hash("test"), false).unwrap();
        let key = gate.session_key_for_tests();
        let tok = gate.sign_session().unwrap();
        let dot = tok.find('.').unwrap();
        let mut bytes = tok.clone().into_bytes();
        bytes[dot - 1] ^= 1;
        assert!(!verify_session(&key, &String::from_utf8(bytes).unwrap()));
    }

    #[test]
    fn wrong_signature_does_not_verify() {
        let gate = AuthGate::password(&cheap_hash("test"), false).unwrap();
        let key = gate.session_key_for_tests();
        let tok = gate.sign_session().unwrap();
        let dot = tok.rfind('.').unwrap();
        let target = if tok.as_bytes()[dot + 1] == b'0' {
            'f'
        } else {
            '0'
        };
        let flipped = format!("{}{}{}", &tok[..=dot], target, &tok[dot + 2..]);
        assert!(!verify_session(&key, &flipped));
    }

    #[test]
    fn expired_session_does_not_verify() {
        let gate = AuthGate::password(&cheap_hash("test"), false).unwrap();
        let key = gate.session_key_for_tests();
        let tok = gate.sign_session().unwrap();
        let pipe = tok.find('|').unwrap();
        let expired = format!("1000000000{}", &tok[pipe..]);
        assert!(!verify_session(&key, &expired));
    }

    #[test]
    fn malformed_sessions_do_not_verify() {
        let gate = AuthGate::password(&cheap_hash("test"), false).unwrap();
        let key = gate.session_key_for_tests();
        for bad in [
            "",
            "no-dot",
            "no-pipe.signature",
            ".just-signature",
            "payload.",
            "payload.badhex",
        ] {
            assert!(!verify_session(&key, bad), "{bad:?} should not verify");
        }
    }

    // ── Rate limiter (auth_test.go rate-limiter block) ──

    #[test]
    fn rate_limiter_allows_five_then_blocks() {
        let rl = RateLimiter::default();
        let ip = "192.0.2.1";
        for i in 1..=RATE_LIMIT_MAX {
            assert!(rl.allow(ip), "attempt {i} should be allowed");
        }
        assert!(!rl.allow(ip), "6th attempt should be blocked");
    }

    #[test]
    fn rate_limiter_resets_after_window() {
        let rl = RateLimiter::default();
        let ip = "192.0.2.2";
        for _ in 0..RATE_LIMIT_MAX {
            rl.allow(ip);
        }
        assert!(!rl.allow(ip));
        rl.expire_window(ip);
        assert!(rl.allow(ip), "should be allowed after the window expires");
    }

    #[test]
    fn rate_limiter_is_per_ip() {
        let rl = RateLimiter::default();
        for _ in 0..RATE_LIMIT_MAX {
            rl.allow("192.0.2.1");
        }
        assert!(rl.allow("192.0.2.2"), "different IP should not be limited");
    }

    // ── Escaping and the form (auth_test.go plus ParseQuery behavior) ──

    #[test]
    fn html_escape_matches_go() {
        assert_eq!(
            html_escape("<b>\"x\"&'y'</b>"),
            "&lt;b&gt;&#34;x&#34;&amp;&#39;y&#39;&lt;/b&gt;"
        );
    }

    #[test]
    fn parse_urlencoded_matches_parse_query() {
        assert_eq!(
            parse_urlencoded(b"password=ab%2Fcd+ef"),
            Ok(vec![(b"password".to_vec(), b"ab/cd ef".to_vec())])
        );
        // Invalid escapes fail the parse, empty keys are skipped.
        assert_eq!(parse_urlencoded(b"password=%zz"), Err(()));
        assert_eq!(
            parse_urlencoded(b"=x&password=y"),
            Ok(vec![(b"password".to_vec(), b"y".to_vec())])
        );
        assert_eq!(
            parse_urlencoded(b"password"),
            Ok(vec![(b"password".to_vec(), Vec::new())])
        );
        // A semicolon is the invalid-separator error (auth_test.go edge).
        assert_eq!(parse_urlencoded(b"password=a;session=x"), Err(()));
        // Raw non-UTF-8 bytes survive for the bcrypt compare.
        assert_eq!(
            parse_urlencoded(b"password=%ff"),
            Ok(vec![(b"password".to_vec(), vec![0xff])])
        );
    }

    #[tokio::test]
    async fn body_password_wins_over_query_password() {
        let request = axum::extract::Request::builder()
            .method(Method::POST)
            .uri("/login?password=querypass")
            .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::from("password=bodypass"))
            .unwrap();
        let pairs = read_login_form(request).await.unwrap();
        assert_eq!(form_password(&pairs), b"bodypass");
    }

    // ── clientIP / isTLS / Secure policy (auth_test.go blocks) ──

    #[test]
    fn client_ip_uses_remote_addr_then_xff_only_behind_proxy() {
        let gate = AuthGate::password(&cheap_hash("x"), false).unwrap();
        let request = get("/", None, None);
        // Without a reported peer (no ConnectInfo) the key is shared.
        assert_eq!(gate.client_ip(&request), "");

        let mut request = get("/", None, None);
        request.extensions_mut().insert(ConnectInfo(SocketAddr::new(
            IpAddr::from([192, 0, 2, 42]),
            12345,
        )));
        assert_eq!(gate.client_ip(&request), "192.0.2.42");

        let mut request = get("/", None, None);
        request.extensions_mut().insert(ConnectInfo(SocketAddr::new(
            IpAddr::from([192, 0, 2, 1]),
            12345,
        )));
        request.headers_mut().insert(
            crate::auth::X_FORWARDED_FOR,
            HeaderValue::from_static("10.0.0.1, 10.0.0.2"),
        );
        assert_eq!(
            gate.client_ip(&request),
            "192.0.2.1",
            "X-Forwarded-For is ignored without behind_proxy"
        );

        let proxy = AuthGate::password(&cheap_hash("x"), true).unwrap();
        let mut request = get("/", None, None);
        request.extensions_mut().insert(ConnectInfo(SocketAddr::new(
            IpAddr::from([10, 0, 0, 99]),
            12345,
        )));
        request.headers_mut().insert(
            crate::auth::X_FORWARDED_FOR,
            HeaderValue::from_static("192.0.2.42, 10.0.0.1"),
        );
        assert_eq!(proxy.client_ip(&request), "192.0.2.42");
    }

    #[test]
    fn is_tls_trusts_forwarded_proto_only_behind_proxy() {
        let gate = AuthGate::password(&cheap_hash("x"), false).unwrap();
        let request = get("/", None, None);
        assert!(!gate.is_tls(&request));
        let mut request = get("/", None, None);
        request.headers_mut().insert(
            crate::auth::X_FORWARDED_PROTO,
            HeaderValue::from_static("https"),
        );
        assert!(!gate.is_tls(&request));
        let proxy = AuthGate::password(&cheap_hash("x"), true).unwrap();
        assert!(proxy.is_tls(&request));
    }

    // ── The login routes over the router (auth_test.go password-mode block) ──

    #[tokio::test]
    async fn login_page_is_served_byte_exact() {
        let r = password_router(&cheap_hash("test"));
        for path in ["/login", "/login/"] {
            let response = oneshot(r.clone(), get(path, None, None)).await;
            let (status, body, _, _) = parts_of(response).await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(body.as_bytes(), LOGIN_HTML);
        }
    }

    #[tokio::test]
    async fn login_refuses_other_methods_with_the_envelope() {
        let r = password_router(&cheap_hash("test"));
        let request = axum::extract::Request::builder()
            .method(Method::PUT)
            .uri("/login")
            .body(Body::empty())
            .unwrap();
        let response = oneshot(r, request).await;
        assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
        let (_, body, _, _) = parts_of(response).await;
        let json: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(json["code"], crate::refuse::METHOD_NOT_ALLOWED);
    }

    #[tokio::test]
    async fn browser_without_session_is_parked_on_login() {
        let r = password_router(&cheap_hash("test"));
        let response = oneshot(r, get("/", Some("text/html,application/xhtml+xml"), None)).await;
        let (status, _, cookies, location) = parts_of(response).await;
        assert_eq!(status, StatusCode::FOUND);
        assert_eq!(location.as_deref(), Some("/login"));
        let redirect = cookies
            .iter()
            .find(|c| c.starts_with("reasonix_redirect="))
            .expect("redirect cookie set");
        assert!(redirect.contains("Max-Age=300"), "{redirect}");
        assert!(redirect.contains("HttpOnly"), "{redirect}");
        assert!(redirect.contains("SameSite=Lax"), "{redirect}");
        assert!(!redirect.contains("Secure"), "{redirect}");
    }

    #[tokio::test]
    async fn api_without_session_is_denied() {
        let r = password_router(&cheap_hash("test"));
        let response = oneshot(r, get("/runtimes", None, None)).await;
        let (status, body, _, _) = parts_of(response).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(body, "Unauthorized\n");
    }

    #[tokio::test]
    async fn mutation_without_session_stays_gated() {
        let r = password_router(&cheap_hash("test"));
        let request = axum::extract::Request::builder()
            .method(Method::POST)
            .uri("/runtimes")
            .header(CONTENT_TYPE, "application/json")
            .body(Body::from(br#"{"root":"/w"}"#.to_vec()))
            .unwrap();
        let response = oneshot(r, request).await;
        let (status, body, _, _) = parts_of(response).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    }

    #[tokio::test]
    async fn valid_login_sets_the_session_and_grants_access() {
        let r = password_router(&cheap_hash("correct"));
        let response = oneshot(r.clone(), login_post("password=correct", &[])).await;
        let (status, _, cookies, location) = parts_of(response).await;
        assert_eq!(status, StatusCode::FOUND);
        assert_eq!(location.as_deref(), Some("/"));
        let session = cookies
            .iter()
            .find(|c| c.starts_with("reasonix_session="))
            .expect("session cookie set");
        assert!(
            session.contains("Path=/; Max-Age=2592000; HttpOnly; SameSite=Lax"),
            "{session}"
        );
        assert!(!session.contains("Secure"), "{session}");
        // The redirect cookie is cleared, not just overwritten.
        assert!(
            cookies
                .iter()
                .any(|c| c.starts_with("reasonix_redirect=; Path=/; Max-Age=0;")),
            "redirect cookie cleared: {cookies:?}"
        );
        // The session cookie authenticates the API.
        let response = oneshot(
            r,
            get(
                "/runtimes",
                None,
                Some(session.split(';').next().unwrap().to_owned()),
            ),
        )
        .await;
        let (status, body, _, _) = parts_of(response).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, "[]\n");
    }

    #[tokio::test]
    async fn wrong_password_renders_the_error_page() {
        let r = password_router(&cheap_hash("correct"));
        let response = oneshot(r, login_post("password=wrong", &[])).await;
        let (status, body, cookies, _) = parts_of(response).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert!(body.contains("Invalid password."), "{body}");
        assert!(
            !cookies.iter().any(|c| c.starts_with("reasonix_session=")),
            "no session cookie on a wrong password"
        );
    }

    #[tokio::test]
    async fn empty_password_is_required() {
        let r = password_router(&cheap_hash("correct"));
        let response = oneshot(r, login_post("", &[])).await;
        let (status, body, _, _) = parts_of(response).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert!(body.contains("Password is required."), "{body}");
    }

    #[tokio::test]
    async fn password_gate_without_a_hash_answers_not_configured() {
        let r = password_router("");
        let response = oneshot(r, login_post("password=anything", &[])).await;
        let (status, body, _, _) = parts_of(response).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert!(
            body.contains("Server not configured for password authentication."),
            "{body}"
        );
    }

    #[tokio::test]
    async fn login_redirect_cookie_is_sanitized() {
        let r = password_router(&cheap_hash("correct"));
        let response = oneshot(
            r,
            login_post(
                "password=correct",
                &[("Cookie", "reasonix_redirect=//evil.example/path")],
            ),
        )
        .await;
        let (status, _, _, location) = parts_of(response).await;
        assert_eq!(status, StatusCode::FOUND);
        assert_eq!(location.as_deref(), Some("/"));
    }

    #[tokio::test]
    async fn an_oversized_login_form_is_a_bad_body() {
        let r = password_router(&cheap_hash("correct"));
        let pad = "a".repeat(1 << 16);
        let response = oneshot(r, login_post(&format!("password=correct&pad={pad}"), &[])).await;
        let (status, body, _, _) = parts_of(response).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let json: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(json["code"], crate::refuse::BAD_BODY);
    }

    #[tokio::test]
    async fn rate_limit_trips_on_the_sixth_attempt() {
        let r = password_router(&cheap_hash("correct"));
        for i in 0..5 {
            let response = oneshot(r.clone(), login_post("password=wrong", &[])).await;
            assert_eq!(
                response.status(),
                StatusCode::UNAUTHORIZED,
                "attempt {}",
                i + 1
            );
        }
        let response = oneshot(r, login_post("password=wrong", &[])).await;
        let (status, body, _, _) = parts_of(response).await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(body, "Too many attempts. Please wait a minute.\n");
    }

    #[tokio::test]
    async fn trusted_proxy_https_marks_the_session_cookie_secure() {
        let r = password_router_behind_proxy(&cheap_hash("correct"));
        let response = oneshot(
            r,
            login_post("password=correct", &[("x-forwarded-proto", "https")]),
        )
        .await;
        let (_, _, cookies, _) = parts_of(response).await;
        let session = cookies
            .iter()
            .find(|c| c.starts_with("reasonix_session="))
            .expect("session cookie set");
        assert!(session.contains("; Secure"), "{session}");

        let r = password_router_behind_proxy(&cheap_hash("correct"));
        let response = oneshot(r, login_post("password=correct", &[])).await;
        let (_, _, cookies, _) = parts_of(response).await;
        let session = cookies
            .iter()
            .find(|c| c.starts_with("reasonix_session="))
            .expect("session cookie set");
        assert!(
            !session.contains("Secure"),
            "plain HTTP stays usable: {session}"
        );
    }

    #[tokio::test]
    async fn token_mode_serves_the_login_page_too() {
        // Go answers /login inside the middleware for token mode as well.
        let tmp = tempdir();
        let r = build_router_with_auth(
            Arc::new(Hub::new(HubPaths {
                state_root: tmp.path().to_path_buf(),
            })),
            RouterConfig::default(),
            Arc::new(AuthGate::token_mode("tkn").unwrap()),
        );
        let response = oneshot(r.clone(), get("/login", None, None)).await;
        let (status, body, _, _) = parts_of(response).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body.as_bytes(), LOGIN_HTML);
        // Token mode carries no password hash: a non-empty password answers
        // the not-configured page.
        let response = oneshot(r, login_post("password=whatever", &[])).await;
        let (status, body, _, _) = parts_of(response).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert!(
            body.contains("Server not configured for password authentication."),
            "{body}"
        );
    }

    #[test]
    fn password_mode_reports_its_name_and_requires_the_session() {
        let gate = AuthGate::password(&cheap_hash("test"), false).unwrap();
        assert_eq!(gate.mode().as_str(), "password");
        assert!(!gate.authenticated(&HeaderMap::new()));
        let tok = gate.sign_session().unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::COOKIE,
            HeaderValue::from_str(&format!("{}={tok}", SESSION_COOKIE)).unwrap(),
        );
        assert!(gate.authenticated(&headers));
        // The launch-token machinery stays empty in password mode, as in Go.
        assert!(gate.token().is_empty());
        assert!(!gate.presents_launch_token(&headers));
    }
}

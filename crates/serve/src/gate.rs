//! The loopback gate (Go `loopback.go`): the boundary a desktop host puts in
//! front of everything. A request must be addressed to this listener, must
//! name this listener as its origin when it changes anything, and must carry
//! the host's credential — checked outermost first, so a caller that reached
//! the socket under another name learns nothing about the credential.

use std::io;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpListener};

use axum::http::{HeaderMap, Method};

/// Where the token gate looks for a caller's credential. Exported for a host
/// that must authenticate to another machine's kernel on a page's behalf.
pub const TOKEN_COOKIE: &str = "reasonix_token";

/// The four refusals the gate can answer with, one per invariant (the shared
/// constants live in `refuse`; this enum is what `GateDecision` carries).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefusalCode {
    LoopbackHostRejected,
    LoopbackOriginRejected,
    LoopbackUnauthorized,
    LoopbackMisconfigured,
}

impl RefusalCode {
    pub fn as_str(self) -> &'static str {
        match self {
            RefusalCode::LoopbackHostRejected => crate::refuse::LOOPBACK_HOST_REJECTED,
            RefusalCode::LoopbackOriginRejected => crate::refuse::LOOPBACK_ORIGIN_REJECTED,
            RefusalCode::LoopbackUnauthorized => crate::refuse::LOOPBACK_UNAUTHORIZED,
            RefusalCode::LoopbackMisconfigured => crate::refuse::LOOPBACK_MISCONFIGURED,
        }
    }

    /// The English fallback sentence each refusal carries (loopback.go).
    pub fn message(self) -> &'static str {
        match self {
            RefusalCode::LoopbackHostRejected => "this listener does not answer for that host",
            RefusalCode::LoopbackOriginRejected => "that origin may not reach this listener",
            RefusalCode::LoopbackUnauthorized => "this listener needs the host's credential",
            RefusalCode::LoopbackMisconfigured => {
                "this gate was built without a loopback origin and a token"
            }
        }
    }
}

/// The gate's answer (docs/contract.md §4.2: `LoopbackGate::check_http`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateDecision {
    Allow,
    Reject(RefusalCode),
}

/// What one request presents to the gate. Headers are spelled out rather than
/// taken as an `http::Request` so the gate stays testable without a transport.
#[derive(Debug, Clone, Copy)]
pub struct GateInput<'a> {
    pub method: &'a str,
    /// The `Host` header authority (host:port, as the client spelled it).
    pub host: &'a str,
    /// The `Origin` header, when the client sent one.
    pub origin: Option<&'a str>,
    /// The value of the `reasonix_token` cookie, when presented.
    pub token_cookie: Option<&'a str>,
}

/// The whole gate policy. Both halves belong to the host that opened the
/// listener: no configuration file contributes to either, so nothing a user
/// can edit turns this boundary off.
#[derive(Debug, Clone)]
pub struct LoopbackGate {
    valid: bool,
    authority: String,
    origin: String,
    token: String,
}

impl LoopbackGate {
    /// Go `NewLoopbackGate`: a gate built without both halves of its policy
    /// refuses everything with `loopback.misconfigured`.
    pub fn new(origin: &str, token: &str) -> Self {
        let authority = match loopback_authority(origin) {
            Some(authority) if !token.is_empty() => authority,
            _ => {
                return Self {
                    valid: false,
                    authority: String::new(),
                    origin: String::new(),
                    token: String::new(),
                };
            }
        };
        Self {
            valid: true,
            origin: format!("http://{authority}"),
            authority,
            token: token.to_owned(),
        }
    }

    /// The authority the gate answers for (host:port), for diagnostics.
    pub fn authority(&self) -> &str {
        &self.authority
    }

    /// The origin the gate answers for (Go `NewLoopbackGate`'s derived value).
    pub fn origin(&self) -> &str {
        &self.origin
    }

    /// Checks one request, outermost first: Host, then Origin (reads may
    /// arrive without one), then the credential.
    pub fn check_http(&self, input: GateInput<'_>) -> GateDecision {
        if !self.valid {
            return GateDecision::Reject(RefusalCode::LoopbackMisconfigured);
        }
        if !input.host.eq_ignore_ascii_case(&self.authority) {
            return GateDecision::Reject(RefusalCode::LoopbackHostRejected);
        }
        let sent = input.origin.unwrap_or("");
        if sent.is_empty() {
            if !safe_method(input.method) {
                return GateDecision::Reject(RefusalCode::LoopbackOriginRejected);
            }
        } else if sent != self.origin {
            return GateDecision::Reject(RefusalCode::LoopbackOriginRejected);
        }
        match input.token_cookie {
            Some(cookie) if constant_time_eq(cookie.as_bytes(), self.token.as_bytes()) => {
                GateDecision::Allow
            }
            _ => GateDecision::Reject(RefusalCode::LoopbackUnauthorized),
        }
    }

    /// `check_http` over the headers a transport already parsed.
    pub fn check_request(&self, method: &Method, headers: &HeaderMap) -> GateDecision {
        let host = headers
            .get(axum::http::header::HOST)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        let origin = headers
            .get(axum::http::header::ORIGIN)
            .and_then(|v| v.to_str().ok());
        let cookie = cookie_value(headers, TOKEN_COOKIE);
        self.check_http(GateInput {
            method: method.as_str(),
            host,
            origin,
            token_cookie: cookie,
        })
    }
}

fn safe_method(method: &str) -> bool {
    method.eq_ignore_ascii_case("GET") || method.eq_ignore_ascii_case("HEAD")
}

/// Go `loopbackAuthority`: the host:port an origin answers for, and whether it
/// is one this gate may guard — an IP literal on the loopback interface only.
/// A name would leave a resolver deciding what the gate protects; no path and
/// no query. A fragment does not invalidate (Go `url.Parse` leaves Path and
/// RawQuery empty for one).
fn loopback_authority(origin: &str) -> Option<String> {
    let (scheme, rest) = origin.split_once("://")?;
    if !scheme.eq_ignore_ascii_case("http") {
        return None;
    }
    let rest = rest.split('#').next().unwrap_or("");
    // A path or a query on the origin is not an origin this gate may guard.
    if rest.contains('/') || rest.contains('?') {
        return None;
    }
    // Go's url.Parse keeps userinfo out of u.Host; a credential in the origin
    // is not part of what the gate answers for.
    let host = rest.rsplit('@').next().unwrap_or(rest);
    if host.is_empty() {
        return None;
    }
    let addr: SocketAddr = host.parse().ok()?;
    if !addr.ip().is_loopback() || addr.port() == 0 {
        return None;
    }
    Some(host.to_owned())
}

/// Go `ListenLoopback`: the only listener this gate is meant to guard,
/// `tcp4 127.0.0.1:0` spelled out so no caller can widen the socket to a
/// wildcard address or a name a resolver gets to answer for.
pub fn listen_loopback() -> io::Result<TcpListener> {
    TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
}

/// Go `LoopbackOrigin`: the origin a `listen_loopback` listener answers for.
pub fn loopback_origin(listener: &TcpListener) -> io::Result<String> {
    Ok(format!("http://{}", listener.local_addr()?))
}

/// Bridges a caller-bound std listener into the async accept loop: the socket
/// flips to non-blocking, which `from_std` requires.
pub fn to_tokio_listener(listener: std::net::TcpListener) -> io::Result<tokio::net::TcpListener> {
    listener.set_nonblocking(true)?;
    tokio::net::TcpListener::from_std(listener)
}

/// Constant-time equality (Go `subtle.ConstantTimeCompare`'s contract as the
/// tech choice spells it): the work depends on the lengths only, never on
/// where the first differing byte sits.
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    let mut diff = (a.len() ^ b.len()) as u64;
    for i in 0..a.len().max(b.len()) {
        let ab = a.get(i).copied().unwrap_or(0) as u64;
        let bb = b.get(i).copied().unwrap_or(0) as u64;
        diff |= ab ^ bb;
    }
    diff == 0
}

/// Reads one cookie out of a `Cookie` header (Go `r.Cookie`): the first
/// match wins, values lose surrounding quotes.
pub(crate) fn cookie_value<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    for value in headers.get_all(axum::http::header::COOKIE) {
        let value = value.to_str().ok()?;
        for pair in value.split(';') {
            let pair = pair.trim_start();
            let (key, val) = pair.split_once('=')?;
            if key.trim() == name {
                let val = val.trim();
                let unquoted = val
                    .strip_prefix('"')
                    .and_then(|v| v.strip_suffix('"'))
                    .unwrap_or(val);
                return Some(unquoted);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn gate() -> LoopbackGate {
        LoopbackGate::new("http://127.0.0.1:8080", "tkn")
    }

    fn allow(method: &str, host: &str, origin: Option<&str>, cookie: Option<&str>) -> GateDecision {
        gate().check_http(GateInput {
            method,
            host,
            origin,
            token_cookie: cookie,
        })
    }

    #[test]
    fn constant_time_eq_holds_across_lengths_and_contents() {
        assert!(constant_time_eq(b"", b""));
        assert!(constant_time_eq(b"secret", b"secret"));
        assert!(!constant_time_eq(b"secret", b"secretx"));
        assert!(!constant_time_eq(b"secret", b"secret2"));
        assert!(!constant_time_eq(b"secret", b""));
        // A difference at the last byte is as fatal as one at the first.
        assert!(!constant_time_eq(b"abcdef", b"abcdeg"));
    }

    #[test]
    fn gate_rejects_a_host_that_is_not_the_listener() {
        assert_eq!(
            allow("GET", "example.com:8080", None, Some("tkn")),
            GateDecision::Reject(RefusalCode::LoopbackHostRejected)
        );
        // Host comparison folds case, as Go's EqualFold does.
        assert_eq!(
            allow("GET", "127.0.0.1:8080", None, Some("tkn")),
            GateDecision::Allow
        );
        assert_eq!(
            allow("GET", "LOCALHOST:8080", None, Some("tkn")),
            GateDecision::Reject(RefusalCode::LoopbackHostRejected)
        );
    }

    #[test]
    fn gate_lets_a_read_arrive_without_an_origin_but_not_a_mutation() {
        assert_eq!(
            allow("GET", "127.0.0.1:8080", None, Some("tkn")),
            GateDecision::Allow
        );
        assert_eq!(
            allow("HEAD", "127.0.0.1:8080", None, Some("tkn")),
            GateDecision::Allow
        );
        assert_eq!(
            allow("POST", "127.0.0.1:8080", None, Some("tkn")),
            GateDecision::Reject(RefusalCode::LoopbackOriginRejected)
        );
        assert_eq!(
            allow(
                "POST",
                "127.0.0.1:8080",
                Some("http://127.0.0.1:8080"),
                Some("tkn")
            ),
            GateDecision::Allow
        );
        assert_eq!(
            allow(
                "POST",
                "127.0.0.1:8080",
                Some("http://127.0.0.1:9999"),
                Some("tkn")
            ),
            GateDecision::Reject(RefusalCode::LoopbackOriginRejected)
        );
        // Origin comparison is exact, not folded.
        assert_eq!(
            allow(
                "POST",
                "127.0.0.1:8080",
                Some("http://127.0.0.1:8080/"),
                Some("tkn")
            ),
            GateDecision::Reject(RefusalCode::LoopbackOriginRejected)
        );
    }

    #[test]
    fn gate_refuses_a_missing_or_wrong_credential_last() {
        assert_eq!(
            allow("GET", "127.0.0.1:8080", None, None),
            GateDecision::Reject(RefusalCode::LoopbackUnauthorized)
        );
        assert_eq!(
            allow("GET", "127.0.0.1:8080", None, Some("wrong")),
            GateDecision::Reject(RefusalCode::LoopbackUnauthorized)
        );
        assert_eq!(
            allow("GET", "example.com:80", None, None),
            GateDecision::Reject(RefusalCode::LoopbackHostRejected)
        );
    }

    #[test]
    fn misconfigured_gate_refuses_everything() {
        for origin in [
            "",
            "https://127.0.0.1:8080",
            "http://example.com:80",
            "http://127.0.0.1:0",
            "http://127.0.0.1:8080/x",
            "http://127.0.0.1:8080?q=1",
            "http://localhost:8080",
            "http://127.0.0.1",
            "127.0.0.1:8080",
        ] {
            let bad = LoopbackGate::new(origin, "tkn");
            assert_eq!(
                bad.check_http(GateInput {
                    method: "GET",
                    host: "127.0.0.1:8080",
                    origin: None,
                    token_cookie: Some("tkn"),
                }),
                GateDecision::Reject(RefusalCode::LoopbackMisconfigured),
                "origin {origin:?} must misconfigure the gate"
            );
        }
        assert_eq!(
            LoopbackGate::new("http://127.0.0.1:8080", "").check_http(GateInput {
                method: "GET",
                host: "127.0.0.1:8080",
                origin: None,
                token_cookie: None,
            }),
            GateDecision::Reject(RefusalCode::LoopbackMisconfigured)
        );
    }

    #[test]
    fn gate_accepts_ipv6_loopback_and_strips_userinfo() {
        let v6 = LoopbackGate::new("http://[::1]:8080", "tkn");
        assert_eq!(v6.authority(), "[::1]:8080");
        assert_eq!(v6.origin(), "http://[::1]:8080");
        let with_user = LoopbackGate::new("http://u@127.0.0.1:8080", "tkn");
        assert_eq!(with_user.authority(), "127.0.0.1:8080");
    }

    #[test]
    fn cookie_reader_takes_the_first_named_pair() {
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::COOKIE,
            HeaderValue::from_static(r#"other=1; reasonix_token="tkn"; more=2"#),
        );
        assert_eq!(cookie_value(&headers, TOKEN_COOKIE), Some("tkn"));
        headers.insert(
            axum::http::header::COOKIE,
            HeaderValue::from_static("reasonix_token=plain"),
        );
        assert_eq!(cookie_value(&headers, TOKEN_COOKIE), Some("plain"));
        assert_eq!(cookie_value(&headers, "absent"), None);
    }

    #[test]
    fn listen_loopback_binds_tcp4_loopback_only() {
        let listener = listen_loopback().unwrap();
        let addr = listener.local_addr().unwrap();
        assert!(addr.is_ipv4());
        assert_eq!(addr.ip(), Ipv4Addr::LOCALHOST);
        assert_ne!(addr.port(), 0);
        let origin = loopback_origin(&listener).unwrap();
        assert!(origin.starts_with("http://127.0.0.1:"));
    }
}

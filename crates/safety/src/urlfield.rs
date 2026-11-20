//! Minimal `net/url` reader for the egress and redirect decisions: scheme,
//! userinfo, host, port, and a display form (Go 1.26.11 subset, mirroring the
//! parser in `reasonix-core::secrets::url`, which is private to that crate).
//!
//! Only the fields the gates read are kept. Anything the reference parser
//! rejects is rejected here too where reachable; hosts this reader cannot
//! validate stay fail-closed because allowlist matching never admits them.

use std::net::Ipv6Addr;

/// Parse failures a caller answers by refusing; the variant is for tests.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum UrlParseError {
    #[error("net/url: invalid control character in URL")]
    ControlChar,
    #[error("missing protocol scheme")]
    MissingScheme,
    #[error("first path segment in URL cannot contain colon")]
    FirstSegmentColon,
    #[error("net/url: invalid userinfo")]
    InvalidUserinfo,
    #[error("invalid IP-literal")]
    InvalidIpLiteral,
    #[error("missing ']' in host")]
    MissingBracket,
    #[error("invalid port {0:?} after host")]
    InvalidPort(String),
    #[error("invalid character in host name")]
    InvalidHost,
}

/// The URL fields the egress and redirect decisions read (Go `*url.URL`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct UrlFields {
    /// Lowercased at parse, the way Go's Parse does.
    pub scheme: String,
    /// Raw userinfo before the last `@`, when present.
    pub userinfo: Option<String>,
    /// host[:port] as spelled, brackets kept (Go URL.Host).
    pub host: String,
    /// host with brackets and any valid port removed (Go URL.Hostname).
    pub hostname: String,
    /// The digits after the last colon, or "" (Go URL.Port).
    pub port: String,
    /// Raw path?query#fragment after the authority, for the display form.
    pub tail: String,
    /// Whether an authority (`//...`) was present at all.
    pub has_authority: bool,
    /// `scheme:opaque` form (no leading `/` after the scheme).
    pub opaque: bool,
}

impl UrlFields {
    pub fn parse(raw: &str) -> Result<UrlFields, UrlParseError> {
        if raw.bytes().any(|b| b < 0x20 || b == 0x7f) {
            return Err(UrlParseError::ControlChar);
        }
        let frag_idx = raw.find('#').unwrap_or(raw.len());
        let head = &raw[..frag_idx];
        // The tail keeps query and fragment raw, so display matches what the
        // caller was handed (Go URL.String() reassembles the same bytes).
        let query_idx = head.find('?').unwrap_or(head.len());
        let head_noq = &head[..query_idx];

        let (scheme, rest_start) = get_scheme(head_noq)?;
        let rest = &head_noq[rest_start..];
        if !rest.starts_with('/') && scheme.is_empty() {
            let segment = match rest.find('/') {
                Some(i) => &rest[..i],
                None => rest,
            };
            if segment.contains(':') {
                return Err(UrlParseError::FirstSegmentColon);
            }
        }

        let mut out = UrlFields {
            scheme,
            ..UrlFields::default()
        };
        let authority_rule =
            (!rest.starts_with("///") || !out.scheme.is_empty()) && rest.starts_with("//");
        if authority_rule {
            let after = &rest[2..];
            let authority = match after.find('/') {
                Some(i) => &after[..i],
                None => after,
            };
            let (userinfo, host) = parse_authority(&out.scheme, authority)?;
            out.userinfo = userinfo;
            out.host = host.clone();
            (out.hostname, out.port) = split_host_port(&host);
            out.has_authority = true;
            let tail_start = rest_start + 2 + authority.len();
            out.tail = raw[tail_start..].to_string();
        } else {
            out.opaque = !out.scheme.is_empty() && !rest.starts_with('/');
            out.tail = raw[rest_start..].to_string();
        }
        Ok(out)
    }

    /// Go URL.Hostname: host with brackets and a valid optional port removed.
    pub fn hostname(&self) -> &str {
        &self.hostname
    }

    /// Go URL.Port: the digits after the last colon, or "" when absent or
    /// invalid.
    pub fn port(&self) -> &str {
        &self.port
    }

    /// Go URL.String() for the forms these gates receive.
    pub fn to_url_string(&self) -> String {
        if self.has_authority {
            let mut s = String::new();
            s.push_str(&self.scheme);
            s.push_str("://");
            if let Some(user) = &self.userinfo {
                s.push_str(user);
                s.push('@');
            }
            s.push_str(&self.host);
            s.push_str(&self.tail);
            s
        } else if self.scheme.is_empty() {
            self.tail.clone()
        } else {
            format!("{}:{}", self.scheme, self.tail)
        }
    }
}

fn parse_authority(
    scheme: &str,
    authority: &str,
) -> Result<(Option<String>, String), UrlParseError> {
    let (userinfo, host) = match authority.rfind('@') {
        Some(i) => (Some(&authority[..i]), &authority[i + 1..]),
        None => (None, authority),
    };
    let host = parse_host(scheme, host)?;
    match userinfo {
        None => Ok((None, host)),
        Some(user) => {
            if !valid_userinfo(user) {
                return Err(UrlParseError::InvalidUserinfo);
            }
            Ok((Some(user.to_string()), host))
        }
    }
}

/// Go net/url parseHost: bracketed IP-literals validated against IPv6, and
/// strict colons for http/https hosts.
fn parse_host(scheme: &str, host: &str) -> Result<String, UrlParseError> {
    if host.contains('[') {
        if !host.starts_with('[') {
            return Err(UrlParseError::InvalidIpLiteral);
        }
        let close = host.find(']').ok_or(UrlParseError::MissingBracket)?;
        let colon_port = &host[close + 1..];
        if !valid_optional_port(colon_port) {
            return Err(UrlParseError::InvalidPort(colon_port.to_string()));
        }
        let inner = &host[1..close];
        // RFC 6874 zones (%25...) are out of scope for these gates; a zone
        // spelling is refused rather than matched against an allowlist. An
        // IPv4-mapped spelling is refused too, matching the pinned reference
        // parser (reasonix-core::secrets::url, Go 1.26.11).
        match inner.parse::<Ipv6Addr>() {
            Ok(v6) if v6.to_ipv4_mapped().is_none() => {}
            _ => return Err(UrlParseError::InvalidIpLiteral),
        }
        return Ok(host.to_string());
    }
    if let Some(first) = host.find(':') {
        let mut i = first;
        let last = host.rfind(':').expect("first colon exists");
        if last != i && scheme != "http" && scheme != "https" {
            i = last;
        }
        let colon_port = &host[i..];
        if !valid_optional_port(colon_port) {
            return Err(UrlParseError::InvalidPort(colon_port.to_string()));
        }
    }
    if !host.is_empty() && !valid_host_chars(host) {
        return Err(UrlParseError::InvalidHost);
    }
    Ok(host.to_string())
}

/// The byte set Go's host validation admits for non-IP hosts; whitespace and
/// delimiters stay out so a host can never smuggle a separator past a
/// string comparison.
fn valid_host_chars(host: &str) -> bool {
    host.bytes().all(|b| {
        b.is_ascii_alphanumeric()
            || matches!(
                b,
                b'-' | b'.'
                    | b'_'
                    | b'~'
                    | b'%'
                    | b'!'
                    | b'$'
                    | b'&'
                    | b'\''
                    | b'('
                    | b')'
                    | b'*'
                    | b'+'
                    | b','
                    | b';'
                    | b'='
                    | b':'
            )
    })
}

/// Go net/url validUserinfo: RFC 3986 userinfo plus `@` (issues 3439/22655).
fn valid_userinfo(s: &str) -> bool {
    s.chars().all(|r| {
        r.is_ascii_alphanumeric()
            || matches!(
                r,
                '-' | '.'
                    | '_'
                    | ':'
                    | '~'
                    | '!'
                    | '$'
                    | '&'
                    | '\''
                    | '('
                    | ')'
                    | '*'
                    | '+'
                    | ','
                    | ';'
                    | '='
                    | '%'
                    | '@'
            )
    })
}

/// Go net/url validOptionalPort: empty or `:` followed by digits only.
pub(crate) fn valid_optional_port(port: &str) -> bool {
    if port.is_empty() {
        return true;
    }
    let bytes = port.as_bytes();
    bytes[0] == b':' && bytes[1..].iter().all(|b| b.is_ascii_digit())
}

/// Go net/url splitHostPort: last colon only when it opens a valid optional
/// port; brackets removed from the host half.
pub(crate) fn split_host_port(host_port: &str) -> (String, String) {
    let mut host = host_port;
    let mut port = String::new();
    if let Some(colon) = host.rfind(':')
        && valid_optional_port(&host[colon..])
    {
        host = &host[..colon];
        port = host_port[colon + 1..].to_string();
    }
    if host.starts_with('[') && host.ends_with(']') {
        host = &host[1..host.len() - 1];
    }
    (host.to_string(), port)
}

/// Go net.JoinHostPort: brackets exactly when the host contains a colon.
pub(crate) fn join_host_port(host: &str, port: &str) -> String {
    if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

/// Go net/url getScheme: a colon at position 0 is an error; schemes must
/// start with a letter.
fn get_scheme(raw: &str) -> Result<(String, usize), UrlParseError> {
    let bytes = raw.as_bytes();
    for (i, &c) in bytes.iter().enumerate() {
        match c {
            b'a'..=b'z' | b'A'..=b'Z' => {}
            b'0'..=b'9' | b'+' | b'-' | b'.' => {
                if i == 0 {
                    return Ok((String::new(), 0));
                }
            }
            b':' => {
                if i == 0 {
                    return Err(UrlParseError::MissingScheme);
                }
                return Ok((raw[..i].to_ascii_lowercase(), i + 1));
            }
            _ => return Ok((String::new(), 0)),
        }
    }
    Ok((String::new(), 0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_of_redirect_guard_shapes() {
        let u = UrlFields::parse("https://user:pw@dl.reasonix.io/x").unwrap();
        assert_eq!(u.scheme, "https");
        assert_eq!(u.userinfo.as_deref(), Some("user:pw"));
        assert_eq!(u.host, "dl.reasonix.io");
        assert_eq!(u.tail, "/x");

        let u = UrlFields::parse("https://dl.reasonix.io:8443/x").unwrap();
        assert_eq!(u.hostname(), "dl.reasonix.io");
        assert_eq!(u.port(), "8443");

        let u = UrlFields::parse("https://[::1]:8080/x").unwrap();
        assert_eq!(u.hostname(), "::1");
        assert_eq!(u.port(), "8080");

        let u = UrlFields::parse("https:///x").unwrap();
        assert_eq!(u.host, "");
        assert_eq!(u.tail, "/x");

        let u = UrlFields::parse("not a url").unwrap();
        assert_eq!(u.host, "");
        assert!(!u.has_authority);
    }

    #[test]
    fn parse_rejects_what_go_rejects() {
        assert_eq!(
            UrlFields::parse(":x").unwrap_err(),
            UrlParseError::MissingScheme
        );
        assert_eq!(
            UrlFields::parse("1foo:bar").unwrap_err(),
            UrlParseError::FirstSegmentColon
        );
        assert_eq!(
            UrlFields::parse("http://[::1/x").unwrap_err(),
            UrlParseError::MissingBracket
        );
        assert_eq!(
            UrlFields::parse("http://[1.2.3.4]/x").unwrap_err(),
            UrlParseError::InvalidIpLiteral
        );
        assert_eq!(
            UrlFields::parse("http://[::ffff:1.2.3.4]/x").unwrap_err(),
            UrlParseError::InvalidIpLiteral
        );
        assert_eq!(
            UrlFields::parse("http://h:1:2/").unwrap_err(),
            UrlParseError::InvalidPort(":1:2".to_string())
        );
        assert!(UrlFields::parse("https://ho st/x").is_err());
    }

    #[test]
    fn display_round_trips_the_spelling() {
        for raw in [
            "https://dl.reasonix.io/studio/x.tar.gz",
            "https://user:pw@dl.reasonix.io/x",
            "https://www.example.com/v1?token=x",
            "https:///x",
            "mailto:foo@bar",
            "https://host/mcp?",
        ] {
            assert_eq!(UrlFields::parse(raw).unwrap().to_url_string(), raw);
        }
    }

    #[test]
    fn join_host_port_brackets_only_coloned_hosts() {
        assert_eq!(join_host_port("127.0.0.1", "7890"), "127.0.0.1:7890");
        assert_eq!(join_host_port("::1", "1080"), "[::1]:1080");
    }

    #[test]
    fn split_host_port_follows_go_edges() {
        assert_eq!(
            split_host_port("[::1]:1080"),
            ("::1".to_string(), "1080".to_string())
        );
        assert_eq!(
            split_host_port("host:"),
            ("host".to_string(), String::new())
        );
        assert_eq!(
            split_host_port("h:bad"),
            ("h:bad".to_string(), String::new())
        );
    }
}

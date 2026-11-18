//! Minimal port of the Go standard library net/url subset used by the
//! secrets endpoint projection (oracle: Go 1.26.11 src/net/url/url.go,
//! captured at .go-url-12611.txt in the repository root during M2).
//!
//! Only what the projection needs is implemented: Parse, String, query
//! parsing/encoding and the escaping helpers. Behavior follows Go 1.26.11
//! for the reference module (the reference go.mod pins go 1.26.0),
//! including the urlstrictcolons=1 default that applies to go1.26 modules
//! (strict colons for http/https hosts), the urlmaxqueryparams 10000
//! limit, and the absence of the Go 1.27 OmitHost rule for paths starting
//! with an authority-less double slash.
//!
//! Documented divergences (both unreachable in the reference test
//! corpus): Go strings are raw byte slices while Rust strings must be
//! valid UTF-8. unescape therefore decodes lossily, so an input whose
//! %XX escapes decode to invalid UTF-8 and whose path or host is then
//! modified re-serializes the replacement character as %EF%BF%BD instead
//! of the original bytes. Values that round-trip unmodified stay
//! byte-exact through raw_path/raw_query.

use std::collections::BTreeMap;
use std::fmt;

/// Encoding mode bits, mirroring the Go encoding constants (the values
/// are the table column bits from gen_encoding_table.go).
pub(crate) type Encoding = u8;
pub(crate) const ENC_PATH: Encoding = 1 << 0;
pub(crate) const ENC_PATH_SEGMENT: Encoding = 1 << 1;
pub(crate) const ENC_HOST: Encoding = 1 << 2;
pub(crate) const ENC_ZONE: Encoding = 1 << 3;
pub(crate) const ENC_USER_PASSWORD: Encoding = 1 << 4;
pub(crate) const ENC_QUERY_COMPONENT: Encoding = 1 << 5;
pub(crate) const ENC_FRAGMENT: Encoding = 1 << 6;

const UPPERHEX: &[u8; 16] = b"0123456789ABCDEF";

/// Query parameter map. Go uses map[string][]string with random iteration;
/// the BTreeMap keeps Encode deterministic (Encode sorts keys anyway, so
/// output only differs when two distinct keys project to the same key,
/// where Go itself was nondeterministic).
pub(crate) type Values = BTreeMap<String, Vec<String>>;

const DEFAULT_MAX_PARAMS: usize = 10000;

/// Errors mirror the Go net/url error values; the projection only checks
/// for failure, so the Display texts exist for debugging and tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum UrlError {
    Escape(String),
    InvalidHost(String),
    InvalidPort(String),
    InvalidIpLiteral,
    MissingBracket,
    InvalidHostAddr,
    MissingScheme,
    FirstSegmentColon,
    InvalidUserinfo,
    ControlChar,
    SemicolonSeparator,
    TooManyParams,
}

impl fmt::Display for UrlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UrlError::Escape(s) => write!(f, "invalid URL escape {s:?}"),
            UrlError::InvalidHost(s) => write!(f, "invalid character {s:?} in host name"),
            UrlError::InvalidPort(s) => write!(f, "invalid port {s:?} after host"),
            UrlError::InvalidIpLiteral => write!(f, "invalid IP-literal"),
            UrlError::MissingBracket => write!(f, "missing ']' in host"),
            UrlError::InvalidHostAddr => write!(f, "invalid host"),
            UrlError::MissingScheme => write!(f, "missing protocol scheme"),
            UrlError::FirstSegmentColon => {
                write!(f, "first path segment in URL cannot contain colon")
            }
            UrlError::InvalidUserinfo => write!(f, "net/url: invalid userinfo"),
            UrlError::ControlChar => write!(f, "net/url: invalid control character in URL"),
            UrlError::SemicolonSeparator => write!(f, "invalid semicolon separator in query"),
            UrlError::TooManyParams => write!(f, "number of URL query parameters exceeded limit"),
        }
    }
}

/// shouldEscape reports whether the byte c must be escaped when appearing
/// in the given mode. Mirrors the Go encoding_table.go column logic,
/// which was verified against the generator for all 256 bytes and all 7
/// modes during M2.
fn should_escape(c: u8, mode: Encoding) -> bool {
    // RFC 3986 unreserved characters are never escaped.
    if c.is_ascii_alphanumeric() {
        return false;
    }

    if mode == ENC_HOST || mode == ENC_ZONE {
        // Section 3.2.2 of RFC 3986 allows sub-delims plus ": [ ] < > ""
        // inside the host; Go additionally keeps them for zones.
        if matches!(
            c,
            b'!' | b'$'
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
                | b'['
                | b']'
                | b'<'
                | b'>'
                | b'"'
        ) {
            return false;
        }
    }

    match c {
        // RFC 3986 unreserved marks.
        b'-' | b'_' | b'.' | b'~' => false,
        // Reserved characters, per mode.
        b'$' | b'&' | b'+' | b',' | b'/' | b':' | b';' | b'=' | b'?' | b'@' => match mode {
            ENC_PATH => c == b'?',
            ENC_PATH_SEGMENT => matches!(c, b'/' | b';' | b',' | b'?'),
            ENC_USER_PASSWORD => matches!(c, b'@' | b'/' | b'?' | b':'),
            ENC_QUERY_COMPONENT => true,
            ENC_FRAGMENT => false,
            _ => true,
        },
        _ => {
            if mode == ENC_FRAGMENT && matches!(c, b'!' | b'(' | b')' | b'*') {
                // RFC 3986 leaves these fragment characters alone.
                return false;
            }
            true
        }
    }
}

fn escape(s: &str, mode: Encoding) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    for &b in bytes {
        if b == b' ' && mode == ENC_QUERY_COMPONENT {
            out.push(b'+');
        } else if should_escape(b, mode) {
            out.push(b'%');
            out.push(UPPERHEX[usize::from(b >> 4)]);
            out.push(UPPERHEX[usize::from(b & 0x0f)]);
        } else {
            out.push(b);
        }
    }
    // Only ASCII bytes are ever escaped, so multi-byte UTF-8 sequences pass
    // through intact and the output stays valid UTF-8.
    String::from_utf8(out).expect("escape output is valid UTF-8")
}

fn unescape(s: &str, mode: Encoding) -> Result<String, UrlError> {
    let bytes = s.as_bytes();
    let mut n = 0usize;
    let mut has_plus = false;
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'%' => {
                n += 1;
                if i + 2 >= bytes.len() || !is_hex(bytes[i + 1]) || !is_hex(bytes[i + 2]) {
                    return Err(UrlError::Escape(err_payload(bytes, i, 3)));
                }
                // In the host component %-encoding can only be used for
                // non-ASCII bytes, except %25 for IPv6 zones (RFC 6874).
                if mode == ENC_HOST && unhex(bytes[i + 1]) < 8 && &bytes[i..i + 3] != b"%25" {
                    return Err(UrlError::Escape(err_payload(bytes, i, 3)));
                }
                if mode == ENC_ZONE {
                    let v = (unhex(bytes[i + 1]) << 4) | unhex(bytes[i + 2]);
                    if &bytes[i..i + 3] != b"%25" && v != b' ' && should_escape(v, ENC_HOST) {
                        return Err(UrlError::Escape(err_payload(bytes, i, 3)));
                    }
                }
                i += 3;
            }
            b'+' => {
                has_plus = mode == ENC_QUERY_COMPONENT;
                i += 1;
            }
            b => {
                if (mode == ENC_HOST || mode == ENC_ZONE) && b < 0x80 && should_escape(b, mode) {
                    return Err(UrlError::InvalidHost(err_payload(bytes, i, 1)));
                }
                i += 1;
            }
        }
    }

    if n == 0 && !has_plus {
        return Ok(s.to_string());
    }

    let plus_out = if mode == ENC_QUERY_COMPONENT {
        b' '
    } else {
        b'+'
    };
    let mut out = Vec::with_capacity(bytes.len() - 2 * n);
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'%' => {
                out.push((unhex(bytes[i + 1]) << 4) | unhex(bytes[i + 2]));
                i += 3;
            }
            b'+' => {
                out.push(plus_out);
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    // See the module docs: Go hands back raw bytes, Rust strings must be
    // valid UTF-8, so escapes that decode to invalid UTF-8 become U+FFFD.
    Ok(String::from_utf8_lossy(&out).into_owned())
}

/// Go slices the error payload out of the byte string; a char-boundary
/// slice would panic mid-rune, so the payload is taken byte-safely.
fn err_payload(bytes: &[u8], start: usize, max: usize) -> String {
    let end = (start + max).min(bytes.len());
    String::from_utf8_lossy(&bytes[start..end]).into_owned()
}

fn is_hex(c: u8) -> bool {
    c.is_ascii_hexdigit()
}

/// Go's arithmetic unhex from encoding_table.go.
fn unhex(c: u8) -> u8 {
    9 * (c >> 6) + (c & 0x0f)
}

/// Userinfo holds a username/password pair, mirroring url.Userinfo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Userinfo {
    username: String,
    password: String,
    password_set: bool,
}

impl Userinfo {
    pub(crate) fn user(username: String) -> Self {
        Userinfo {
            username,
            password: String::new(),
            password_set: false,
        }
    }

    pub(crate) fn user_password(username: String, password: String) -> Self {
        Userinfo {
            username,
            password,
            password_set: true,
        }
    }

    fn to_go_string(&self) -> String {
        let mut s = escape(&self.username, ENC_USER_PASSWORD);
        if self.password_set {
            s.push(':');
            s.push_str(&escape(&self.password, ENC_USER_PASSWORD));
        }
        s
    }
}

/// Url is the decoded form of a net/url.URL, restricted to the fields the
/// endpoint projection reads and mutates.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Url {
    pub(crate) scheme: String,
    pub(crate) opaque: String,
    pub(crate) user: Option<Userinfo>,
    pub(crate) host: String,
    pub(crate) path: String,
    pub(crate) fragment: String,
    pub(crate) raw_query: String,
    pub(crate) raw_path: String,
    pub(crate) raw_fragment: String,
    pub(crate) force_query: bool,
    pub(crate) omit_host: bool,
}

impl Url {
    fn set_path(&mut self, p: &str) -> Result<(), UrlError> {
        let path = unescape(p, ENC_PATH)?;
        if escape(&path, ENC_PATH) == p {
            // Default encoding is fine.
            self.raw_path = String::new();
        } else {
            self.raw_path = p.to_string();
        }
        self.path = path;
        Ok(())
    }

    /// setFragment is like setPath but for Fragment/RawFragment.
    fn set_fragment(&mut self, f: &str) -> Result<(), UrlError> {
        let frag = unescape(f, ENC_FRAGMENT)?;
        if escape(&frag, ENC_FRAGMENT) == f {
            self.raw_fragment = String::new();
        } else {
            self.raw_fragment = f.to_string();
        }
        self.fragment = frag;
        Ok(())
    }

    pub(crate) fn escaped_path(&self) -> String {
        if !self.raw_path.is_empty()
            && valid_encoded(&self.raw_path, ENC_PATH)
            && let Ok(p) = unescape(&self.raw_path, ENC_PATH)
            && p == self.path
        {
            return self.raw_path.clone();
        }
        if self.path == "*" {
            // Do not escape (golang.org/issue/11202).
            return "*".to_string();
        }
        escape(&self.path, ENC_PATH)
    }

    pub(crate) fn escaped_fragment(&self) -> String {
        if !self.raw_fragment.is_empty()
            && valid_encoded(&self.raw_fragment, ENC_FRAGMENT)
            && let Ok(f) = unescape(&self.raw_fragment, ENC_FRAGMENT)
            && f == self.fragment
        {
            return self.raw_fragment.clone();
        }
        escape(&self.fragment, ENC_FRAGMENT)
    }

    /// Reassembles the URL into a URL string, mirroring Go 1.26.11
    /// URL.String (including the RFC 3986 4.2 dot-segment rule, but NOT
    /// the Go 1.27 "%2F"-in-OmitHost-path rule, which this version lacks).
    pub(crate) fn to_go_string(&self) -> String {
        let mut buf = String::new();
        if !self.scheme.is_empty() {
            buf.push_str(&self.scheme);
            buf.push(':');
        }
        if !self.opaque.is_empty() {
            buf.push_str(&self.opaque);
        } else {
            if !self.scheme.is_empty() || !self.host.is_empty() || self.user.is_some() {
                if self.omit_host && self.host.is_empty() && self.user.is_none() {
                    // Omit empty host.
                } else {
                    if !self.host.is_empty() || !self.path.is_empty() || self.user.is_some() {
                        buf.push_str("//");
                    }
                    if let Some(ui) = &self.user {
                        buf.push_str(&ui.to_go_string());
                        buf.push('@');
                    }
                    if !self.host.is_empty() {
                        buf.push_str(&escape(&self.host, ENC_HOST));
                    }
                }
            }
            let path = self.escaped_path();
            if !path.is_empty() && !path.starts_with('/') && !self.host.is_empty() {
                buf.push('/');
            }
            if buf.is_empty() {
                // RFC 3986 4.2: a first path segment containing a colon
                // must be preceded by a dot-segment, or it would be
                // mistaken for a scheme name.
                let segment = match path.find('/') {
                    Some(i) => &path[..i],
                    None => &path[..],
                };
                if segment.contains(':') {
                    buf.push_str("./");
                }
            }
            buf.push_str(&path);
        }
        if self.force_query || !self.raw_query.is_empty() {
            buf.push('?');
            buf.push_str(&self.raw_query);
        }
        if !self.fragment.is_empty() {
            buf.push('#');
            buf.push_str(&self.escaped_fragment());
        }
        buf
    }
}

/// validEncoded reports whether s is a valid encoded path or fragment
/// according to mode; it must not contain bytes that require escaping.
fn valid_encoded(s: &str, mode: Encoding) -> bool {
    s.bytes().all(|b| match b {
        // RFC 3986, Appendix A: pchar = unreserved / pct-encoded /
        // sub-delims / ":" / "@". shouldEscape is not quite compliant, so
        // the sub-delims are checked here and the rest delegated.
        b'!' | b'$' | b'&' | b'\'' | b'(' | b')' | b'*' | b'+' | b',' | b';' | b'=' | b':'
        | b'@' => true,
        // Not specified in RFC 3986 but left alone by modern browsers.
        b'[' | b']' => true,
        // Percent encoded; will decode.
        b'%' => true,
        _ => !should_escape(b, mode),
    })
}

/// validOptionalPort reports whether port is empty or matches /^:\d*$/.
fn valid_optional_port(port: &str) -> bool {
    if port.is_empty() {
        return true;
    }
    let bytes = port.as_bytes();
    if bytes[0] != b':' {
        return false;
    }
    bytes[1..].iter().all(|b| b.is_ascii_digit())
}

/// parse parses raw_url as a general reference (viaRequest=false in Go).
pub(crate) fn parse(raw_url: &str) -> Result<Url, UrlError> {
    // Cut off #frag.
    let (u, frag) = match raw_url.find('#') {
        Some(i) => (&raw_url[..i], &raw_url[i + 1..]),
        None => (raw_url, ""),
    };
    let mut url = parse_internal(u)?;
    if !frag.is_empty() {
        url.set_fragment(frag)?;
    }
    Ok(url)
}

fn parse_internal(raw_url: &str) -> Result<Url, UrlError> {
    if string_contains_ctl_byte(raw_url) {
        return Err(UrlError::ControlChar);
    }

    let mut url = Url::default();

    if raw_url == "*" {
        url.path = "*".to_string();
        return Ok(url);
    }

    // Split off a possible leading "http:", "mailto:", etc.; it cannot
    // contain escaped characters.
    let (scheme, mut rest) = get_scheme(raw_url)?;
    let scheme = scheme.to_ascii_lowercase();
    url.scheme = scheme.clone();

    if rest.ends_with('?') && rest.matches('?').count() == 1 {
        url.force_query = true;
        rest = &rest[..rest.len() - 1];
    } else if let Some(i) = rest.find('?') {
        url.raw_query = rest[i + 1..].to_string();
        rest = &rest[..i];
    }

    if !rest.starts_with('/') {
        if !scheme.is_empty() {
            // Rootless paths per RFC 3986 are considered opaque.
            url.opaque = rest.to_string();
            return Ok(url);
        }

        // Avoid confusion with malformed schemes, like
        // cache_object:foo/bar (golang.org/issue/16822): the first path
        // segment of a relative-path reference cannot contain a colon.
        let segment = match rest.find('/') {
            Some(i) => &rest[..i],
            None => rest,
        };
        if segment.contains(':') {
            return Err(UrlError::FirstSegmentColon);
        }
    }

    if (!rest.starts_with("///") || !scheme.is_empty()) && rest.starts_with("//") {
        let after = &rest[2..];
        let (authority, tail) = match after.find('/') {
            Some(i) => (&after[..i], &after[i..]),
            None => (after, ""),
        };
        let (user, host) = parse_authority(&scheme, authority)?;
        url.user = user;
        url.host = host;
        rest = tail;
    } else if !scheme.is_empty() && rest.starts_with('/') {
        // OmitHost is set when raw_url has an empty host (authority);
        // see golang.org/issue/46059.
        url.omit_host = true;
    }

    url.set_path(rest)?;
    Ok(url)
}

fn parse_authority(scheme: &str, authority: &str) -> Result<(Option<Userinfo>, String), UrlError> {
    let at = authority.rfind('@');
    let host = match at {
        Some(i) => parse_host(scheme, &authority[i + 1..])?,
        None => parse_host(scheme, authority)?,
    };
    let Some(at) = at else {
        return Ok((None, host));
    };
    let userinfo = &authority[..at];
    if !valid_userinfo(userinfo) {
        return Err(UrlError::InvalidUserinfo);
    }
    let user = if !userinfo.contains(':') {
        Userinfo::user(unescape(userinfo, ENC_USER_PASSWORD)?)
    } else {
        let colon = userinfo.find(':').expect("checked above");
        let username = unescape(&userinfo[..colon], ENC_USER_PASSWORD)?;
        let password = unescape(&userinfo[colon + 1..], ENC_USER_PASSWORD)?;
        Userinfo::user_password(username, password)
    };
    Ok((Some(user), host))
}

/// parseHost parses host as host[:port], the authority without user
/// information.
fn parse_host(scheme: &str, host: &str) -> Result<String, UrlError> {
    if let Some(open) = host.rfind('[') {
        if open > 0 {
            return Err(UrlError::InvalidIpLiteral);
        }
        // Parse an IP-Literal per RFC 3986 and RFC 6874, e.g. "[fe80::1]",
        // "[fe80::1%25en0]", "[fe80::1]:80".
        let close = host.rfind(']').ok_or(UrlError::MissingBracket)?;
        let colon_port = &host[close + 1..];
        if !valid_optional_port(colon_port) {
            return Err(UrlError::InvalidPort(colon_port.to_string()));
        }
        let unescaped_colon_port = unescape(colon_port, ENC_HOST)?;

        let hostname = &host[1..close];
        // RFC 6874 defines that %25 (%-encoded percent) introduces the
        // zone identifier, which may use basically any %-encoding it
        // likes; the host itself may only %-encode non-ASCII bytes.
        let unescaped_hostname = match hostname.find("%25") {
            Some(zone_idx) => {
                let host_part = unescape(&hostname[..zone_idx], ENC_HOST)?;
                let zone_part = unescape(&hostname[zone_idx..], ENC_ZONE)?;
                format!("{host_part}{zone_part}")
            }
            None => unescape(hostname, ENC_HOST)?,
        };

        // Per RFC 3986 only a host identified by a valid IPv6 address may
        // be enclosed in square brackets; this excludes any IPv4, but
        // notably not IPv4-mapped addresses.
        match parse_addr(&unescaped_hostname) {
            AddrKind::NotIp => return Err(UrlError::InvalidHostAddr),
            AddrKind::V4 => return Err(UrlError::InvalidIpLiteral),
            AddrKind::V6 => {}
        }
        return Ok(format!("[{unescaped_hostname}]{unescaped_colon_port}"));
    }

    if let Some(first_colon) = host.find(':') {
        // RFC 3986 does not allow colons in the host subcomponent, but
        // several databases permit comma-separated host lists with
        // optional ports, so strict colons are enforced only for http and
        // https (urlstrictcolons=1 is the go1.26 module default: keep the
        // FIRST colon, which then fails the port validation below).
        let mut i = first_colon;
        let last_colon = host.rfind(':').expect("first colon exists");
        if last_colon != i && scheme != "http" && scheme != "https" {
            i = last_colon;
        }
        let colon_port = &host[i..];
        if !valid_optional_port(colon_port) {
            return Err(UrlError::InvalidPort(colon_port.to_string()));
        }
    }

    unescape(host, ENC_HOST)
}

/// getScheme splits the leading scheme off raw_url. A colon at position 0
/// is an error; schemes must start with a letter.
fn get_scheme(raw_url: &str) -> Result<(String, &str), UrlError> {
    let bytes = raw_url.as_bytes();
    for (i, &c) in bytes.iter().enumerate() {
        match c {
            b'a'..=b'z' | b'A'..=b'Z' => {}
            b'0'..=b'9' | b'+' | b'-' | b'.' => {
                if i == 0 {
                    return Ok((String::new(), raw_url));
                }
            }
            b':' => {
                if i == 0 {
                    return Err(UrlError::MissingScheme);
                }
                return Ok((raw_url[..i].to_string(), &raw_url[i + 1..]));
            }
            _ => return Ok((String::new(), raw_url)),
        }
    }
    Ok((String::new(), raw_url))
}

/// validUserinfo reports whether s is a valid userinfo subcomponent per
/// RFC 3986 (Go deliberately also allows "@"; see issues 3439 and 22655).
fn valid_userinfo(s: &str) -> bool {
    s.chars().all(|r| {
        if r.is_ascii_alphanumeric() {
            return true;
        }
        matches!(
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

fn string_contains_ctl_byte(s: &str) -> bool {
    s.bytes().any(|b| b < b' ' || b == 0x7f)
}

enum AddrKind {
    NotIp,
    V4,
    V6,
}

/// netip.ParseAddr equivalent for the bracket IP-literal check. The zone
/// is split at the first percent sign and is only allowed (possibly
/// empty) for IPv6; IPv4 parsing rejects leading zeros exactly like
/// netip and Rust std.
fn parse_addr(s: &str) -> AddrKind {
    let (addr, zone) = match s.find('%') {
        Some(i) => (&s[..i], Some(&s[i + 1..])),
        None => (s, None),
    };
    if !addr.contains(':') {
        if let Some(zone) = zone
            && !zone.is_empty()
        {
            return AddrKind::NotIp;
        }
        return match addr.parse::<std::net::Ipv4Addr>() {
            Ok(_) => AddrKind::V4,
            Err(_) => AddrKind::NotIp,
        };
    }
    match addr.parse::<std::net::Ipv6Addr>() {
        Ok(v6) => {
            let o = v6.octets();
            if o[..10].iter().all(|&b| b == 0) && o[10] == 0xff && o[11] == 0xff {
                // IPv4-mapped IPv6 addresses report Is4() in netip.
                AddrKind::V4
            } else {
                AddrKind::V6
            }
        }
        Err(_) => AddrKind::NotIp,
    }
}

/// parseQuery mirrors Go ParseQuery: url.ParamsWithinMax limits the
/// parameter count to 10000 and ";" separators are rejected.
pub(crate) fn parse_query(query: &str) -> Result<Values, UrlError> {
    // urlmaxqueryparams: the go1.26 default caps parameter count at 10000.
    let param_count = query.bytes().filter(|&b| b == b'&').count() + 1;
    if param_count > DEFAULT_MAX_PARAMS {
        return Err(UrlError::TooManyParams);
    }
    let mut m = Values::new();
    let mut rest = query;
    while !rest.is_empty() {
        let (segment, next) = match rest.find('&') {
            Some(i) => (&rest[..i], &rest[i + 1..]),
            None => (rest, ""),
        };
        rest = next;
        if segment.contains(';') {
            // Settings with a non-URL-encoded semicolon are invalid.
            return Err(UrlError::SemicolonSeparator);
        }
        if segment.is_empty() {
            continue;
        }
        let (key, value) = match segment.find('=') {
            Some(i) => (&segment[..i], &segment[i + 1..]),
            None => (segment, ""),
        };
        let key = query_unescape(key)?;
        let value = query_unescape(value)?;
        m.entry(key).or_default().push(value);
    }
    Ok(m)
}

/// Encode mirrors Go url.Values.Encode: sorted keys, always "=",
/// query-component escaping.
pub(crate) fn encode_values(values: &Values) -> String {
    let mut buf = String::new();
    for (k, vs) in values {
        let key_escaped = query_escape(k);
        for v in vs {
            if !buf.is_empty() {
                buf.push('&');
            }
            buf.push_str(&key_escaped);
            buf.push('=');
            buf.push_str(&query_escape(v));
        }
    }
    buf
}

pub(crate) fn query_escape(s: &str) -> String {
    escape(s, ENC_QUERY_COMPONENT)
}

pub(crate) fn query_unescape(s: &str) -> Result<String, UrlError> {
    unescape(s, ENC_QUERY_COMPONENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_string_round_trip() {
        for raw in [
            "https://host/mcp",
            "https://host/mcp?a=1&b=2",
            "http://user:pass@example.com:8080/p/q#frag",
            "https://user%20name:pa%40ss@host/",
            "https://host/a%20b",
            "https://host/mcp?%74oken=x",
            "mailto:foo@bar",
            "https://host",
            "https://[::1]:8080/x",
            "foo://h:1:2/",
            "*",
            "https://user@example.com/",
        ] {
            let u = parse(raw).unwrap_or_else(|e| panic!("parse {raw:?} failed: {e}"));
            assert_eq!(u.to_go_string(), raw, "round trip of {raw}");
        }
    }

    #[test]
    fn force_query_preserves_trailing_question() {
        let u = parse("https://host/mcp?").unwrap();
        assert!(u.force_query);
        assert!(u.raw_query.is_empty());
        assert_eq!(u.to_go_string(), "https://host/mcp?");
    }

    #[test]
    fn first_segment_colon_rejected_without_scheme() {
        assert_eq!(parse("1foo:bar").unwrap_err(), UrlError::FirstSegmentColon);
    }

    #[test]
    fn control_characters_rejected() {
        assert_eq!(
            parse("https://host/\u{01}x").unwrap_err(),
            UrlError::ControlChar
        );
    }

    #[test]
    fn missing_scheme_rejected() {
        assert_eq!(parse(":x").unwrap_err(), UrlError::MissingScheme);
    }

    #[test]
    fn bracket_ipv6_host_and_port() {
        let u = parse("https://[::1]:8080/x").unwrap();
        assert_eq!(u.host, "[::1]:8080");
        assert_eq!(u.path, "/x");
    }

    #[test]
    fn bracket_host_missing_close_rejected() {
        assert_eq!(
            parse("http://[::1/x").unwrap_err(),
            UrlError::MissingBracket
        );
    }

    #[test]
    fn ipv4_in_brackets_rejected() {
        assert_eq!(
            parse("http://[1.2.3.4]/x").unwrap_err(),
            UrlError::InvalidIpLiteral
        );
        assert_eq!(
            parse("http://[::ffff:1.2.3.4]/x").unwrap_err(),
            UrlError::InvalidIpLiteral
        );
    }

    #[test]
    fn embedded_ipv4_without_mapping_allowed() {
        // ::1.2.3.4 is a plain IPv6 literal (not v4-mapped), so it parses.
        let u = parse("http://[::1.2.3.4]/x").unwrap();
        assert_eq!(u.host, "[::1.2.3.4]");
    }

    #[test]
    fn http_strict_colons_reject_extra_colons() {
        assert!(parse("http://h:1:2/").is_err());
        assert!(parse("https://h:1:2/").is_err());
        let u = parse("foo://h:1:2/").unwrap();
        assert_eq!(u.host, "h:1:2");
        assert_eq!(u.to_go_string(), "foo://h:1:2/");
    }

    #[test]
    fn semicolon_in_query_rejected_by_parse_query() {
        assert_eq!(
            parse_query("a=1;b=2").unwrap_err(),
            UrlError::SemicolonSeparator
        );
    }

    #[test]
    fn query_unescape_decodes_and_plus() {
        assert_eq!(query_unescape("a+b%20c").unwrap(), "a b c");
        assert_eq!(query_unescape("%74oken").unwrap(), "token");
        assert!(query_unescape("%zz").is_err());
    }

    #[test]
    fn query_escape_encodes_component() {
        assert_eq!(query_escape("a b<c"), "a+b%3Cc");
    }

    #[test]
    fn encode_values_sorts_keys_and_keeps_empty_values() {
        let mut v = Values::new();
        v.insert("b".to_string(), vec!["2".to_string()]);
        v.insert("a".to_string(), vec![String::new(), "1".to_string()]);
        assert_eq!(encode_values(&v), "a=&a=1&b=2");
    }

    #[test]
    fn escaped_path_preserves_raw_encoding() {
        let u = parse("https://host/%74oken/x").unwrap();
        assert_eq!(u.path, "/token/x");
        assert_eq!(u.raw_path, "/%74oken/x");
        assert_eq!(u.escaped_path(), "/%74oken/x");
        assert_eq!(u.to_go_string(), "https://host/%74oken/x");
    }

    #[test]
    fn dot_slash_prefix_for_leading_colon_segment() {
        let u = Url {
            path: "this:that".to_string(),
            ..Default::default()
        };
        assert_eq!(u.to_go_string(), "./this:that");
    }

    #[test]
    fn opaque_form_uses_scheme_colon() {
        let u = parse("mailto:foo@bar").unwrap();
        assert_eq!(u.scheme, "mailto");
        assert_eq!(u.opaque, "foo@bar");
        assert_eq!(u.to_go_string(), "mailto:foo@bar");
    }

    #[test]
    fn fragment_round_trip_keeps_raw_encoding() {
        // Verified against Go 1.26: escape(fragment) == raw keeps the
        // raw form out of RawFragment; only non-canonical encodings
        // (like %7E for the unreserved ~) round-trip raw.
        let u = parse("https://host/mcp#a%20b").unwrap();
        assert_eq!(u.fragment, "a b");
        assert_eq!(u.raw_fragment, "");
        assert_eq!(u.to_go_string(), "https://host/mcp#a%20b");
        let u = parse("https://host/mcp#a%7Eb").unwrap();
        assert_eq!(u.fragment, "a~b");
        assert_eq!(u.raw_fragment, "a%7Eb");
        assert_eq!(u.to_go_string(), "https://host/mcp#a%7Eb");
        let u = parse("https://host/mcp#frag").unwrap();
        assert_eq!(u.to_go_string(), "https://host/mcp#frag");
    }
}

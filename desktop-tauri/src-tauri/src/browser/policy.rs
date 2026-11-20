// What a page in the agent's browser may become, and what a typed address is.
// The kernel's policy runs before a command is sent; this is the window's
// answer for everything else a page can reach. The kernel's own origin is
// never a destination: its routes act on the session.
use url::Url;

#[derive(Debug, PartialEq, Eq)]
pub enum LocalKind {
    Local,
    Network,
    Invalid,
}

pub struct LocalPath {
    pub url: String,
    pub kind: LocalKind,
}

pub struct Typed {
    pub url: String,
    pub fallback: String,
    pub refusal: &'static str,
}

pub fn guest_navigation_allowed(raw: &str, kernel_origin: &str) -> bool {
    let Ok(url) = Url::parse(raw) else {
        return false;
    };
    if url.origin().ascii_serialization() == kernel_origin {
        return false;
    }
    matches!(url.scheme(), "http" | "https") || raw == "about:blank"
}

fn trim_typed(raw: &str) -> String {
    raw.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}')
        .to_string()
}

/// Only a form that cannot be a host claims a place on disk: a drive letter
/// and a separator, a leading "\\", one leading slash, or a file: URL.
pub fn local_path(raw: &str) -> Option<LocalPath> {
    let text = trim_typed(raw);
    let flat: String = text
        .chars()
        .filter(|c| !matches!(c, '\t' | '\n' | '\r'))
        .collect();
    if flat.len() >= 6 && flat[..6].eq_ignore_ascii_case("file:/") {
        return Some(file_form(&flat[5..]));
    }
    if text.starts_with("\\\\?\\") || text.starts_with("\\\\.\\") {
        return Some(device_form(&text[4..]));
    }
    if let Some((drive, rest)) = drive_split(&text) {
        return Some(LocalPath {
            url: format!("file:///{drive}:/{seg}", seg = segments(rest, true)),
            kind: LocalKind::Local,
        });
    }
    if let Some(rest) = text.strip_prefix("\\\\") {
        return unc_form(rest);
    }
    if text.starts_with('/') && !text.starts_with("//") {
        return Some(LocalPath {
            url: format!("file:///{}", segments(&text[1..], false)),
            kind: LocalKind::Local,
        });
    }
    None
}

fn drive_split(text: &str) -> Option<(char, &str)> {
    let body = text.strip_prefix('/').unwrap_or(text);
    let mut chars = body.chars();
    let letter = chars.next()?;
    if !letter.is_ascii_alphabetic() || chars.next() != Some(':') {
        return None;
    }
    let rest = &body[2..];
    if rest.starts_with('\\') || rest.starts_with('/') {
        Some((letter, &rest[1..]))
    } else {
        None
    }
}

fn unc_form(rest: &str) -> Option<LocalPath> {
    let cut = rest.find(['\\', '/']);
    let host = cut.map_or(rest, |at| &rest[..at]);
    if host.is_empty() {
        return None;
    }
    let share = cut.map_or("", |at| &rest[at + 1..]);
    Some(LocalPath {
        url: format!(
            "file://{}/{}",
            encode_component(host),
            segments(share, true)
        ),
        kind: LocalKind::Network,
    })
}

fn device_form(rest: &str) -> LocalPath {
    if rest.len() >= 4 && rest[..4].eq_ignore_ascii_case("unc\\") {
        return unc_form(&rest[4..]).unwrap_or(LocalPath {
            url: String::new(),
            kind: LocalKind::Network,
        });
    }
    if rest.len() >= 4 && rest[..4].eq_ignore_ascii_case("unc/") {
        return unc_form(&rest[4..]).unwrap_or(LocalPath {
            url: String::new(),
            kind: LocalKind::Network,
        });
    }
    if let Some((drive, tail)) = drive_split(rest) {
        return LocalPath {
            url: format!("file:///{drive}:/{seg}", seg = segments(tail, true)),
            kind: LocalKind::Local,
        };
    }
    LocalPath {
        url: String::new(),
        kind: LocalKind::Network,
    }
}

fn file_form(after_scheme: &str) -> LocalPath {
    if after_scheme.chars().any(|c| c.is_ascii_control()) {
        return LocalPath {
            url: String::new(),
            kind: LocalKind::Invalid,
        };
    }
    let mut rest = after_scheme.replace('\\', "/");
    let mut host = String::new();
    if let Some(tail) = rest.strip_prefix("//") {
        let cut = tail.find('/');
        host = cut.map_or(tail, |at| &tail[..at]).to_string();
        rest = cut.map_or("/", |at| &tail[at..]).to_string();
    }
    let tail_at = rest.find(['?', '#']);
    let (mut path, tail) = match tail_at {
        Some(at) => (rest[..at].to_string(), rest[at..].to_string()),
        None => (rest.clone(), String::new()),
    };
    let drive_host =
        host.len() == 2 && host.as_bytes()[0].is_ascii_alphabetic() && host.as_bytes()[1] == b':';
    if drive_host {
        path = format!("/{host}{path}");
    } else if !host.is_empty() && !host.eq_ignore_ascii_case("localhost") {
        return LocalPath {
            url: String::new(),
            kind: LocalKind::Network,
        };
    }
    if path.is_empty() {
        path = "/".into();
    }
    if opens_share(&path) {
        return LocalPath {
            url: String::new(),
            kind: LocalKind::Network,
        };
    }
    LocalPath {
        url: format!("file://{path}{tail}"),
        kind: LocalKind::Local,
    }
}

/// A path that opens a share once escapes and dot segments resolve, or that
/// climbs above its own root, is not a local file: Windows reads //host/... as
/// a share, and a drive path that escapes its letter names nothing on disk.
fn opens_share(path: &str) -> bool {
    let decoded = path
        .replace("%5c", "/")
        .replace("%5C", "/")
        .replace("%2f", "/")
        .replace("%2F", "/")
        .replace("%2e", ".")
        .replace("%2E", ".");
    let mut out: Vec<&str> = Vec::new();
    let mut escaped = false;
    for seg in decoded.split('/').skip(1) {
        match seg {
            "." | "" => {}
            ".." => {
                if out.len() <= 1 {
                    escaped = true;
                }
                out.pop();
            }
            _ => out.push(seg),
        }
    }
    escaped || (out.len() > 1 && out[0].is_empty())
}

fn segments(rest: &str, both_separators: bool) -> String {
    let parts: Vec<String> = rest
        .split(|c| c == '/' || (both_separators && c == '\\'))
        .filter(|p| !p.is_empty())
        .map(encode_component)
        .collect();
    let joined = parts.join("/");
    let trailing = rest
        .chars()
        .last()
        .is_some_and(|c| c == '/' || (both_separators && c == '\\'));
    if !parts.is_empty() && trailing {
        format!("{joined}/")
    } else {
        joined
    }
}

fn encode_component(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-'
            | b'_'
            | b'.'
            | b'!'
            | b'~'
            | b'*'
            | b'\''
            | b'('
            | b')' => out.push(byte as char),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// The whole answer for what the person typed: the address, its fallback, and
/// "" or why the window will not load it.
pub fn typed(raw: &str, kernel_origin: &str) -> Typed {
    if let Some(local) = local_path(raw) {
        match local.kind {
            LocalKind::Network => {
                return Typed {
                    url: String::new(),
                    fallback: String::new(),
                    refusal: "network_file",
                };
            }
            LocalKind::Invalid => {
                return Typed {
                    url: String::new(),
                    fallback: String::new(),
                    refusal: "scheme",
                };
            }
            LocalKind::Local => {
                return Typed {
                    url: local.url,
                    fallback: String::new(),
                    refusal: "",
                };
            }
        }
    }
    let (url, fallback) = typed_address(raw);
    let refusal = if url.starts_with("file:") || guest_navigation_allowed(&url, kernel_origin) {
        ""
    } else {
        "scheme"
    };
    Typed {
        url,
        fallback,
        refusal,
    }
}

fn has_scheme(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    if lower.starts_with("about:") {
        return true;
    }
    let Some(at) = lower.find("://") else {
        return false;
    };
    let scheme = &lower[..at];
    let mut chars = scheme.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-'))
}

pub fn typed_address(raw: &str) -> (String, String) {
    let text = trim_typed(raw);
    if text.is_empty() {
        return (String::new(), String::new());
    }
    if let Some(local) = local_path(&text) {
        return (local.url, String::new());
    }
    if has_scheme(&text) {
        return (text, String::new());
    }
    let host = Url::parse(&format!("http://{text}"))
        .ok()
        .and_then(|u| u.host_str().map(str::to_string));
    match host {
        None => (format!("https://{text}"), String::new()),
        Some(host) if private_host(&host) => (format!("http://{text}"), String::new()),
        Some(_) => (format!("https://{text}"), format!("http://{text}")),
    }
}

/// The address's structure, never its spelling: loopback, private, link-local
/// or shared IP literals, localhost, or a name with no dot.
pub fn private_host(host: &str) -> bool {
    let h = host.to_ascii_lowercase();
    if let Some(v6) = h.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
        if v6 == "::1" {
            return true;
        }
        let first = v6.split(':').next().unwrap_or("");
        if first.len() == 4 {
            let b = first.as_bytes();
            if b[0] == b'f'
                && matches!(b[1], b'c' | b'd')
                && b[2].is_ascii_hexdigit()
                && b[3].is_ascii_hexdigit()
            {
                return true;
            }
            if b[0] == b'f'
                && b[1] == b'e'
                && matches!(b[2], b'8' | b'9' | b'a' | b'b')
                && b[3].is_ascii_hexdigit()
            {
                return true;
            }
        }
        return false;
    }
    let parts: Vec<&str> = h.split('.').collect();
    if parts.len() == 4
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
    {
        let a: u32 = parts[0].parse().unwrap_or(0);
        let b: u32 = parts[1].parse().unwrap_or(0);
        return a == 10
            || a == 127
            || (a == 172 && (16..=31).contains(&b))
            || (a == 192 && b == 168)
            || (a == 169 && b == 254)
            || (a == 100 && (64..=127).contains(&b));
    }
    h == "localhost" || h.ends_with(".localhost") || !h.contains('.')
}

#[cfg(test)]
mod tests {
    use super::*;

    const KERNEL: &str = "http://127.0.0.1:5281";

    #[test]
    fn the_kernel_origin_is_never_a_destination() {
        assert!(!guest_navigation_allowed(
            "http://127.0.0.1:5281/status",
            KERNEL
        ));
        assert!(!guest_navigation_allowed("http://127.0.0.1:5281/", KERNEL));
        assert!(guest_navigation_allowed("https://example.com/", KERNEL));
        assert!(guest_navigation_allowed("about:blank", KERNEL));
        assert!(!guest_navigation_allowed("file:///C:/x", KERNEL));
        assert!(!guest_navigation_allowed("javascript:alert(1)", KERNEL));
        assert!(!guest_navigation_allowed("not a url", KERNEL));
    }

    #[test]
    fn drive_paths_and_posix_paths_are_local_files() {
        let p = local_path("C:\\Users\\me\\a.txt").expect("path");
        assert_eq!(p.url, "file:///C:/Users/me/a.txt");
        assert_eq!(p.kind, LocalKind::Local);
        assert_eq!(
            local_path("/tmp/a.txt").expect("path").url,
            "file:///tmp/a.txt"
        );
        assert_eq!(
            local_path("/w/报告.md").expect("path").url,
            "file:///w/%E6%8A%A5%E5%91%8A.md"
        );
        assert_eq!(local_path("C:\\w\\").expect("path").url, "file:///C:/w/");
    }

    #[test]
    fn hosts_are_not_paths() {
        assert!(local_path("D:foo").is_none());
        assert!(local_path("c:").is_none());
        assert!(local_path("host:8080").is_none());
        assert!(local_path("//host/x").is_none());
        assert!(local_path("example.com/a").is_none());
    }

    #[test]
    fn shares_are_refused_as_network_files() {
        let p = local_path("\\\\server\\share\\a.txt").expect("path");
        assert_eq!(p.kind, LocalKind::Network);
        assert_eq!(p.url, "file://server/share/a.txt");
        assert_eq!(typed("\\\\server\\share", KERNEL).refusal, "network_file");
        assert_eq!(
            local_path("\\\\?\\UNC\\server\\share").expect("path").kind,
            LocalKind::Network
        );
        assert_eq!(
            local_path("\\\\?\\C:\\w").expect("path").kind,
            LocalKind::Local
        );
        assert_eq!(
            local_path("\\\\.\\pipe\\x").expect("path").kind,
            LocalKind::Network
        );
    }

    #[test]
    fn file_urls_resolve_their_host_and_dot_segments() {
        assert_eq!(
            local_path("file:///C:/w/a").expect("path").url,
            "file:///C:/w/a"
        );
        assert_eq!(
            local_path("file://localhost/C:/w").expect("path").kind,
            LocalKind::Local
        );
        assert_eq!(
            local_path("file://other/C:/w").expect("path").kind,
            LocalKind::Network
        );
        assert_eq!(
            local_path("file:///C:/w/../../etc").expect("path").kind,
            LocalKind::Network
        );
        assert_eq!(
            local_path("file:///C:/w/%2e%2e/%2e%2e/etc")
                .expect("path")
                .kind,
            LocalKind::Network
        );
        assert_eq!(
            local_path("file:///C:/w/%5c..%5c..%5cetc")
                .expect("path")
                .kind,
            LocalKind::Network
        );
        assert_eq!(
            local_path("file:///C:/w\u{0}").expect("path").kind,
            LocalKind::Invalid
        );
        assert_eq!(typed("file:///C:/w\u{7f}", KERNEL).refusal, "scheme");
    }

    #[test]
    fn typed_addresses_choose_http_for_private_hosts() {
        assert_eq!(
            typed_address("localhost:3000"),
            ("http://localhost:3000".into(), String::new())
        );
        assert_eq!(
            typed_address("192.168.1.5"),
            ("http://192.168.1.5".into(), String::new())
        );
        assert_eq!(
            typed_address("intranet"),
            ("http://intranet".into(), String::new())
        );
        assert_eq!(
            typed_address("example.com"),
            ("https://example.com".into(), "http://example.com".into())
        );
        assert_eq!(
            typed_address("https://example.com"),
            ("https://example.com".into(), String::new())
        );
        assert_eq!(
            typed_address("  example.com  "),
            ("https://example.com".into(), "http://example.com".into())
        );
        assert_eq!(typed_address(""), (String::new(), String::new()));
    }

    #[test]
    fn private_hosts_read_structure() {
        for host in [
            "10.0.0.1",
            "127.0.0.1",
            "172.16.0.1",
            "172.31.255.255",
            "192.168.0.1",
            "169.254.1.1",
            "100.64.0.1",
            "localhost",
            "a.localhost",
            "intranet",
            "[::1]",
            "[fd00::1]",
            "[fe80::1]",
        ] {
            assert!(private_host(host), "{host}");
        }
        for host in [
            "8.8.8.8",
            "172.32.0.1",
            "100.128.0.1",
            "example.com",
            "[2001:db8::1]",
        ] {
            assert!(!private_host(host), "{host}");
        }
    }

    #[test]
    fn typed_refuses_the_kernel_origin_and_foreign_schemes() {
        assert_eq!(typed("http://127.0.0.1:5281/x", KERNEL).refusal, "scheme");
        assert_eq!(typed("ftp://example.com", KERNEL).refusal, "scheme");
        assert_eq!(typed("example.com", KERNEL).refusal, "");
        assert_eq!(typed("C:\\w\\a.txt", KERNEL).refusal, "");
        assert_eq!(typed("C:\\w\\a.txt", KERNEL).url, "file:///C:/w/a.txt");
    }
}

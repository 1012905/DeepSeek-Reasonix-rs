//! The loopback proxy-port sweep (parity target: `egress/loopback.go`): the
//! ports a confined command must not be able to hand its traffic to instead
//! of the egress proxy.

use std::net::IpAddr;
use std::sync::{Arc, OnceLock};

use crate::urlfield::{UrlFields, join_host_port};

const PROXY_ENV_KEYS: [&str; 6] = [
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "ALL_PROXY",
    "http_proxy",
    "https_proxy",
    "all_proxy",
];

/// Answers the host-port strings of the OS's own proxy settings (on macOS
/// the scutil runner; parity target: `egress/sysproxy_darwin.go`). The host
/// wires it at boot; unwired, the environment is the only source, which is
/// the non-darwin reference behavior (`egress/sysproxy_other.go`).
pub type SystemProxySource = Arc<dyn Fn() -> Vec<String> + Send + Sync>;

static SYSTEM_PROXY_SOURCE: OnceLock<SystemProxySource> = OnceLock::new();

/// Installs the OS proxy source; first install wins.
pub fn set_system_proxy_source(source: SystemProxySource) -> bool {
    SYSTEM_PROXY_SOURCE.set(source).is_ok()
}

fn system_proxies() -> Vec<String> {
    match SYSTEM_PROXY_SOURCE.get() {
        Some(source) => source(),
        None => Vec::new(),
    }
}

/// Lists the loopback ports of the proxies this host is set to use, from the
/// environment and the OS proxy settings, sorted and deduplicated. A command
/// that can reach loopback could hand its traffic to one of them instead of
/// to us.
pub fn loopback_proxy_ports(getenv: &dyn Fn(&str) -> String) -> Vec<i64> {
    loopback_proxy_ports_with(getenv, &system_proxies)
}

/// The same sweep with the OS source named, so tests pin the table without
/// touching the process-wide seam.
pub fn loopback_proxy_ports_with(
    getenv: &dyn Fn(&str) -> String,
    system: &dyn Fn() -> Vec<String>,
) -> Vec<i64> {
    let mut hostports = Vec::new();
    for key in PROXY_ENV_KEYS {
        let v = getenv(key);
        let v = v.trim();
        if !v.is_empty() {
            hostports.push(proxy_host_port(v));
        }
    }
    hostports.extend(system());
    let mut ports: Vec<i64> = Vec::new();
    for hp in hostports {
        let Ok((host, port_text)) = split_host_port_net(&hp) else {
            continue;
        };
        if !is_loopback_host(&host) {
            continue;
        }
        match port_text.parse::<i64>() {
            Ok(port) if port > 0 && !ports.contains(&port) => ports.push(port),
            _ => {}
        }
    }
    ports.sort_unstable();
    ports
}

/// Reads host:port from a proxy variable, which may omit the scheme; a
/// missing port is the scheme's default.
fn proxy_host_port(v: &str) -> String {
    let v = if v.contains("://") {
        v.to_string()
    } else {
        format!("http://{v}")
    };
    let Ok(u) = UrlFields::parse(&v) else {
        return String::new();
    };
    if !u.port().is_empty() {
        return u.host.clone();
    }
    let port = if u.scheme.starts_with("socks") {
        "1080"
    } else {
        "80"
    };
    join_host_port(u.hostname(), port)
}

fn is_loopback_host(host: &str) -> bool {
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

/// Go net.SplitHostPort: brackets take the host half, one colon outside
/// them, and a missing port or an unbracketed extra colon is an error.
fn split_host_port_net(host_port: &str) -> Result<(String, String), ()> {
    if let Some(rest) = host_port.strip_prefix('[') {
        let (host, after) = rest.split_once(']').ok_or(())?;
        let port = after.strip_prefix(':').ok_or(())?;
        return Ok((host.to_string(), port.to_string()));
    }
    let colon = host_port.rfind(':').ok_or(())?;
    if host_port[..colon].contains(':') {
        return Err(());
    }
    Ok((
        host_port[..colon].to_string(),
        host_port[colon + 1..].to_string(),
    ))
}

/// Reads the enabled proxies out of `scutil --proxy`, a dictionary of
/// "<Kind>Enable", "<Kind>Proxy" and "<Kind>Port" keys.
pub fn parse_scutil_proxies(out: &str) -> Vec<String> {
    let mut keys: std::collections::HashMap<&str, &str> = std::collections::HashMap::new();
    for line in out.split('\n') {
        if let Some((k, v)) = line.split_once(" : ") {
            keys.insert(k.trim(), v.trim());
        }
    }
    let mut hostports = Vec::new();
    for kind in ["HTTP", "HTTPS", "SOCKS"] {
        let enable = keys.get(&format!("{kind}Enable")[..]);
        let proxy = keys.get(&format!("{kind}Proxy")[..]);
        let port = keys.get(&format!("{kind}Port")[..]);
        if enable == Some(&"1")
            && proxy.is_some_and(|p| !p.is_empty())
            && port.is_some_and(|p| !p.is_empty())
        {
            hostports.push(join_host_port(
                proxy.expect("checked"),
                port.expect("checked"),
            ));
        }
    }
    hostports
}

// Go's url.splitHostPort differs from net.SplitHostPort in the error rules;
// the proxy variables flow through both, so the two stay separate.

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    // Go TestLoopbackProxyPorts, verbatim in expectation.
    #[test]
    fn loopback_proxy_ports() {
        let mut env: HashMap<&str, &str> = HashMap::new();
        env.insert("HTTPS_PROXY", "http://127.0.0.1:7890");
        env.insert("http_proxy", "localhost:8118");
        env.insert("ALL_PROXY", "socks5://[::1]");
        env.insert("HTTP_PROXY", "http://corp-proxy.example:3128");
        let getenv = |k: &str| env.get(k).copied().unwrap_or("").to_string();
        let got = loopback_proxy_ports_with(&getenv, &|| Vec::new());
        for want in [1080, 7890, 8118] {
            assert!(got.contains(&want), "ports = {got:?}, missing {want}");
        }
        assert!(
            !got.contains(&3128),
            "a remote proxy's port was closed: {got:?}"
        );
    }

    // Go TestParseScutilProxies, verbatim in expectation.
    #[test]
    fn parse_scutil_proxies_table() {
        let out = r"<dictionary> {
  ExceptionsList : <array> {
    0 : 127.0.0.1
  }
  HTTPEnable : 1
  HTTPPort : 7897
  HTTPProxy : 127.0.0.1
  HTTPSEnable : 0
  HTTPSPort : 7898
  HTTPSProxy : 127.0.0.1
  SOCKSEnable : 1
  SOCKSPort : 7899
  SOCKSProxy : 127.0.0.1
}";
        assert_eq!(
            parse_scutil_proxies(out),
            vec!["127.0.0.1:7897", "127.0.0.1:7899"]
        );
    }

    // Probed against Go 1.27 (proxyHostPort + net.SplitHostPort edges).
    #[test]
    fn proxy_host_port_defaults_the_schemes_port() {
        assert_eq!(proxy_host_port("127.0.0.1:7890"), "127.0.0.1:7890");
        assert_eq!(proxy_host_port("localhost:8118"), "localhost:8118");
        assert_eq!(proxy_host_port("http://127.0.0.1:7890"), "127.0.0.1:7890");
        assert_eq!(proxy_host_port("socks5://[::1]"), "[::1]:1080");
        assert_eq!(proxy_host_port("garbage"), "garbage:80");
        assert_eq!(proxy_host_port("http://u:p@h:8080"), "h:8080");
        // The host field keeps the multi-colon spelling; net.SplitHostPort
        // then refuses it, so the entry drops out of the sweep.
        assert_eq!(proxy_host_port("socks5://h:1:2"), "h:1:2");
    }

    #[test]
    fn split_host_port_net_errors() {
        assert_eq!(
            split_host_port_net("[::1]:1080").unwrap(),
            ("::1".to_string(), "1080".to_string())
        );
        assert!(split_host_port_net("garbage").is_err());
        assert!(split_host_port_net("::1:80").is_err());
        assert!(split_host_port_net("[::1]").is_err());
    }

    #[test]
    fn loopback_names() {
        assert!(is_loopback_host("localhost"));
        assert!(is_loopback_host("LocalHost"));
        assert!(is_loopback_host("127.0.0.1"));
        assert!(is_loopback_host("::1"));
        assert!(!is_loopback_host("corp-proxy.example"));
        assert!(!is_loopback_host("10.0.0.1"));
    }
}

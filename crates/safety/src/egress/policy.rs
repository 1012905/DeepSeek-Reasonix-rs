//! The domain policy of the egress proxy (parity target: `egress/policy.go`).

use std::net::IpAddr;

/// Why a connection was refused. Crosses to the client as the `X-Proxy-Error`
/// header and to the model through the host's attribution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Reason {
    /// A host no allow pattern names.
    NotAllowed,
    /// A host a deny pattern names; deny outranks allow.
    Denied,
    /// An allowed name that resolves only to loopback, link-local, metadata
    /// or this host's own addresses.
    Address,
    /// A host outside the list a person was asked about and refused.
    Declined,
}

impl Reason {
    /// The wire identity a client reads off `X-Proxy-Error`.
    pub fn as_str(self) -> &'static str {
        match self {
            Reason::NotAllowed => "blocked-by-allowlist",
            Reason::Denied => "blocked-by-denylist",
            Reason::Address => "blocked-by-address",
            Reason::Declined => "declined-by-user",
        }
    }

    /// Reads the identity back off the header; no sentence ever carries it.
    pub fn from_str_value(s: &str) -> Option<Reason> {
        [
            Reason::NotAllowed,
            Reason::Denied,
            Reason::Address,
            Reason::Declined,
        ]
        .into_iter()
        .find(|r| r.as_str() == s)
    }
}

impl std::fmt::Display for Reason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The domain list. A pattern is an exact host name, or `*.name` for every
/// name beneath it but not name itself. Matching is case-insensitive and
/// ignores a trailing root dot. An empty Allow admits nothing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Policy {
    pub allow: Vec<String>,
    pub deny: Vec<String>,
}

impl Policy {
    /// Returns the refusal for host, or None when the policy admits it.
    pub fn decide(&self, host: &str) -> Option<Reason> {
        let host = normalize_host(host);
        if host.is_empty() {
            return Some(Reason::NotAllowed);
        }
        if matches_any(&host, &self.deny) {
            return Some(Reason::Denied);
        }
        if !matches_any(&host, &self.allow) {
            return Some(Reason::NotAllowed);
        }
        None
    }
}

fn matches_any(host: &str, patterns: &[String]) -> bool {
    for raw in patterns {
        let pattern = normalize_host(raw);
        if let Some(suffix) = pattern.strip_prefix("*.") {
            if !suffix.is_empty() && host.ends_with(&format!(".{suffix}")) {
                return true;
            }
            continue;
        }
        if !pattern.is_empty() && pattern == host {
            return true;
        }
    }
    false
}

/// The spelling every comparison runs on: trimmed, lowercased, one trailing
/// root dot removed, IP literals in canonical form.
pub(crate) fn normalize_host(host: &str) -> String {
    let mut host = host.trim().to_ascii_lowercase();
    if host.ends_with('.') {
        host.truncate(host.len() - 1);
    }
    let bare = host.trim_matches(|c| c == '[' || c == ']');
    if let Ok(ip) = bare.parse::<IpAddr>() {
        return super::address::go_ip_string(ip);
    }
    host
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(allow: &[&str], deny: &[&str]) -> Policy {
        Policy {
            allow: allow.iter().map(|s| s.to_string()).collect(),
            deny: deny.iter().map(|s| s.to_string()).collect(),
        }
    }

    // Go TestPolicyDecide, verbatim in expectation.
    #[test]
    fn policy_decide() {
        let p = policy(
            &["github.com", "*.npmjs.org", "PyPI.org.", "10.0.0.5"],
            &["gist.github.com", "*.evil.npmjs.org"],
        );
        let cases: &[(&str, Option<Reason>)] = &[
            ("github.com", None),
            ("GitHub.com.", None),
            ("api.github.com", Some(Reason::NotAllowed)),
            ("gist.github.com", Some(Reason::Denied)),
            ("registry.npmjs.org", None),
            ("npmjs.org", Some(Reason::NotAllowed)),
            ("a.evil.npmjs.org", Some(Reason::Denied)),
            ("pypi.org", None),
            ("example.com", Some(Reason::NotAllowed)),
            ("", Some(Reason::NotAllowed)),
            ("10.0.0.5", None),
            ("[10.0.0.5]", None),
            ("notgithub.com", Some(Reason::NotAllowed)),
            ("github.com.attack.net", Some(Reason::NotAllowed)),
        ];
        for (host, want) in cases {
            assert_eq!(p.decide(host), *want, "Decide({host:?})");
        }
        assert_eq!(
            Policy::default().decide("github.com"),
            Some(Reason::NotAllowed)
        );
    }

    #[test]
    fn reason_identity_round_trips() {
        for r in [
            Reason::NotAllowed,
            Reason::Denied,
            Reason::Address,
            Reason::Declined,
        ] {
            assert_eq!(Reason::from_str_value(r.as_str()), Some(r));
        }
        assert_eq!(
            Reason::from_str_value("blocked-by-allowlist"),
            Some(Reason::NotAllowed)
        );
        assert_eq!(
            Reason::from_str_value("sentences do not carry identities"),
            None
        );
    }

    #[test]
    fn ip_literals_normalize_to_go_spelling() {
        assert_eq!(normalize_host("[::FFFF:127.0.0.1]"), "127.0.0.1");
        assert_eq!(normalize_host("::1"), "::1");
        assert_eq!(normalize_host("10.0.0.5"), "10.0.0.5");
        assert_eq!(normalize_host("Example.COM."), "example.com");
    }
}

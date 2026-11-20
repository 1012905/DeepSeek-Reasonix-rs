//! The decision core of the loopback egress proxy (parity target:
//! `egress/proxy.go`): what a connection to host:port is answered with,
//! before any HTTP. The transport (CONNECT hijack, tunnel, SOCKS dial, the
//! HTTP answers) stays with the caller, which wires a [`UpstreamSelector`],
//! an [`EgressApprover`] and a resolver into [`EgressGate`]; every seam is
//! a type here, never a parsed sentence.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};

use crate::egress::address::{forbidden_address, go_ip_string};
use crate::egress::policy::{Policy, Reason, normalize_host};
use crate::urlfield::{UrlFields, join_host_port};

/// One refused connection, recorded against the command whose token the
/// client presented.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Denial {
    pub host: String,
    pub reason: Reason,
}

/// Refusals a gate records are capped per command so one stuck client
/// cannot grow the ledger without end (Go `maxDenialsPerCmd`).
const MAX_DENIALS_PER_CMD: usize = 32;

/// Puts one command's out-of-list hosts to a person, one question at a time
/// (Go `Proxy.Ask`'s callback). Denied hosts are never asked.
pub trait EgressApprover: Send + Sync {
    /// True admits the host for the rest of that command; false is a
    /// refusal the person made; an error means nobody answered.
    fn approve_egress(&self, host: &str) -> Result<bool, ApprovalFailure>;
}

/// An approval that could not be asked or answered.
#[derive(Debug, Clone, thiserror::Error)]
#[error("egress approval failed: {0}")]
pub struct ApprovalFailure(pub String);

/// One command's asker: each host is decided once and remembered for the
/// rest of that command.
struct AskerState {
    approver: Arc<dyn EgressApprover>,
    answers: HashMap<String, Option<Reason>>,
}

impl AskerState {
    fn decide(&mut self, host: &str) -> Option<Reason> {
        if let Some(cached) = self.answers.get(host) {
            return *cached;
        }
        let answer = match self.approver.approve_egress(host) {
            Ok(true) => None,
            Ok(false) => Some(Reason::Declined),
            Err(_) => Some(Reason::NotAllowed),
        };
        self.answers.insert(host.to_string(), answer);
        answer
    }
}

/// Answers the name to addresses (Go `net.DefaultResolver`); the transport
/// wires a resolver because the decision core performs no I/O itself.
pub type Resolver = Arc<dyn Fn(&str) -> Result<Vec<IpAddr>, LookupError> + Send + Sync>;

/// This host's own addresses (Go `net.InterfaceAddrs`, cached once there);
/// the wiring owns the caching.
pub type LocalAddresses = Arc<dyn Fn() -> Vec<IpAddr> + Send + Sync>;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("name lookup failed: {0}")]
pub struct LookupError(pub String);

/// Answers which proxy the host itself would use for a request, the way
/// `http.ProxyFromEnvironment` does.
pub trait UpstreamSelector: Send + Sync {
    fn upstream_for(
        &self,
        scheme: &str,
        host: &str,
        port: &str,
    ) -> Result<Option<UpstreamProxy>, UpstreamFailure>;
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("upstream selection failed: {0}")]
pub struct UpstreamFailure(pub String);

/// An upstream proxy the egress proxy can chain through: HTTP or SOCKS5
/// (Go accepts `http`, `socks5`, `socks5h` and refuses anything else).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpstreamProxy {
    pub scheme: String,
    /// host[:port] the upstream is reached on.
    pub host: String,
    /// The upstream's own credentials; they go to the upstream alone,
    /// never to the target.
    pub credentials: Option<(String, Option<String>)>,
}

impl UpstreamProxy {
    pub fn is_socks5(&self) -> bool {
        self.scheme.starts_with("socks5")
    }

    /// Reads an upstream out of a proxy URL the way `http.ProxyURL` builds
    /// one, for selectors that answer from a configured address.
    pub fn parse(raw: &str) -> Option<UpstreamProxy> {
        let u = UrlFields::parse(raw).ok()?;
        if !u.has_authority {
            return None;
        }
        let credentials = u.userinfo.map(|info| match info.split_once(':') {
            Some((user, pass)) => (user.to_string(), Some(pass.to_string())),
            None => (info.clone(), None),
        });
        Some(UpstreamProxy {
            scheme: u.scheme,
            host: u.host,
            credentials,
        })
    }
}

/// Where one admitted connection goes next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Admission {
    /// Dial this checked address directly: the name resolved to a
    /// non-forbidden address, or the target was an IP literal.
    Direct(String),
    /// Hand host:port to the upstream proxy; the upstream resolves the
    /// name, so the checked answer rides as the name it was admitted by.
    Upstream(String),
    /// Refused: the denial is already recorded against the token; the
    /// transport answers 403 with `X-Proxy-Error` set to the reason.
    Refused(Denial),
    /// Not a refusal — the name did not resolve (and no upstream can take
    /// it) or the upstream could not be chosen. Go answers 502.
    Failed(AdmissionError),
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AdmissionError {
    #[error("egress: upstream proxy scheme not supported: {0}")]
    UpstreamScheme(String),
    #[error("name lookup failed: {0}")]
    Lookup(String),
    #[error("{0}")]
    Upstream(UpstreamFailure),
}

impl From<UpstreamFailure> for AdmissionError {
    fn from(err: UpstreamFailure) -> AdmissionError {
        AdmissionError::Upstream(err)
    }
}

/// The decision state of `egress.Proxy` without the server: the policy,
/// the per-command denial ledger, the per-command askers, and the two
/// network seams (resolver, own addresses) the host wires.
pub struct EgressGate {
    policy: Policy,
    denials: Mutex<HashMap<String, Vec<Denial>>>,
    askers: Mutex<HashMap<String, Arc<Mutex<AskerState>>>>,
    resolver: Option<Resolver>,
    local: Option<LocalAddresses>,
}

impl EgressGate {
    pub fn new(policy: Policy) -> EgressGate {
        EgressGate {
            policy,
            denials: Mutex::new(HashMap::new()),
            askers: Mutex::new(HashMap::new()),
            resolver: None,
            local: None,
        }
    }

    /// Wires the name-to-address resolver; unwired, a name lookup fails
    /// closed.
    pub fn with_resolver(mut self, resolver: Resolver) -> EgressGate {
        self.resolver = Some(resolver);
        self
    }

    /// Wires this host's own addresses; unwired, the host's own addresses
    /// are unknown and only the loopback, link-local and metadata checks
    /// guard the address space.
    pub fn with_local_addresses(mut self, local: LocalAddresses) -> EgressGate {
        self.local = Some(local);
        self
    }

    /// Lets a host outside the allow list be put to a person for the
    /// command holding token, instead of refused outright.
    pub fn set_asker(&self, token: &str, approver: Arc<dyn EgressApprover>) {
        if token.is_empty() {
            return;
        }
        self.askers.lock().expect("askers").insert(
            token.to_string(),
            Arc::new(Mutex::new(AskerState {
                approver,
                answers: HashMap::new(),
            })),
        );
    }

    /// Applies the policy, putting a host outside the list to the command's
    /// asker when it has one. None admits the host.
    pub fn decide(&self, token: &str, host: &str) -> Option<Reason> {
        match self.policy.decide(host) {
            Some(reason) if reason != Reason::NotAllowed => Some(reason),
            Some(_) => {
                let asker = self.askers.lock().expect("askers").get(token).cloned();
                match asker {
                    None => Some(Reason::NotAllowed),
                    Some(a) => a.lock().expect("asker").decide(&normalize_host(host)),
                }
            }
            None => None,
        }
    }

    /// Validates the selector's answer: HTTP and SOCKS5 proxies chain;
    /// anything else is reported (Go `ErrUpstreamScheme`).
    pub fn upstream_for(
        &self,
        scheme: &str,
        host: &str,
        port: &str,
        selector: &dyn UpstreamSelector,
    ) -> Result<Option<UpstreamProxy>, AdmissionError> {
        match selector.upstream_for(scheme, host, port)? {
            None => Ok(None),
            Some(up) => {
                if matches!(up.scheme.as_str(), "http" | "socks5" | "socks5h") {
                    Ok(Some(up))
                } else {
                    Err(AdmissionError::UpstreamScheme(up.scheme))
                }
            }
        }
    }

    /// Decides host:port and returns where to dial. Through an upstream the
    /// name is passed on, since the upstream resolves it; the local lookup
    /// still refuses a name resolving only to forbidden addresses. A
    /// refusal is recorded against the token before it is returned.
    pub fn admit(
        &self,
        token: &str,
        host: &str,
        port: &str,
        upstream: Option<&UpstreamProxy>,
    ) -> Admission {
        if let Some(reason) = self.decide(token, host) {
            return self.refuse(token, host, reason);
        }
        let bare = host.trim_matches(|c| c == '[' || c == ']');
        if let Ok(ip) = bare.parse::<IpAddr>() {
            if self.forbidden(ip) {
                return self.refuse(token, host, Reason::Address);
            }
            return Admission::Direct(join_host_port(&go_ip_string(ip), port));
        }
        let lookup = match self.resolver.as_ref() {
            None => Err(LookupError("no resolver wired".to_string())),
            Some(resolve) => resolve(host),
        };
        match lookup {
            Err(err) => match upstream {
                Some(_) => Admission::Upstream(join_host_port(host, port)),
                None => Admission::Failed(AdmissionError::Lookup(err.to_string())),
            },
            Ok(ips) => {
                for ip in ips {
                    if self.forbidden(ip) {
                        continue;
                    }
                    return match upstream {
                        Some(_) => Admission::Upstream(join_host_port(host, port)),
                        None => Admission::Direct(join_host_port(&go_ip_string(ip), port)),
                    };
                }
                self.refuse(token, host, Reason::Address)
            }
        }
    }

    fn forbidden(&self, ip: IpAddr) -> bool {
        match self.local.as_ref() {
            None => forbidden_address(ip, &|| Vec::new()),
            Some(local) => forbidden_address(ip, local.as_ref()),
        }
    }

    fn refuse(&self, token: &str, host: &str, reason: Reason) -> Admission {
        self.record(token, host, reason);
        Admission::Refused(Denial {
            host: host.to_string(),
            reason,
        })
    }

    fn record(&self, token: &str, host: &str, reason: Reason) {
        let mut denials = self.denials.lock().expect("denials");
        let list = denials.entry(token.to_string()).or_default();
        if list.iter().any(|d| d.host == host && d.reason == reason) {
            return;
        }
        if list.len() < MAX_DENIALS_PER_CMD {
            list.push(Denial {
                host: host.to_string(),
                reason,
            });
        }
    }

    /// Returns what was refused under token and ends the token.
    pub fn take_denials(&self, token: &str) -> Vec<Denial> {
        let out = self
            .denials
            .lock()
            .expect("denials")
            .remove(token)
            .unwrap_or_default();
        self.askers.lock().expect("askers").remove(token);
        out
    }

    /// Renders [`EgressGate::take_denials`] for the sandbox's egress route.
    pub fn refusals(&self, token: &str) -> Vec<String> {
        self.take_denials(token)
            .iter()
            .map(|d| format!("{} ({})", d.host, d.reason))
            .collect()
    }
}

/// Names one command's traffic. It rides the proxy URL as userinfo, which
/// clients return as `Proxy-Authorization` on every request. Twelve random
/// bytes as hex; on a CSPRNG failure the zero buffer stays, the way Go's
/// ignored error leaves it.
pub fn new_token() -> String {
    let mut b = [0u8; 12];
    let _ = getrandom::fill(&mut b);
    b.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The variables [`env`] sets, for callers that must clear inherited ones.
pub const ENV_KEYS: [&str; 12] = [
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "ALL_PROXY",
    "http_proxy",
    "https_proxy",
    "all_proxy",
    "NO_PROXY",
    "no_proxy",
    "npm_config_proxy",
    "npm_config_https_proxy",
    "YARN_HTTP_PROXY",
    "YARN_HTTPS_PROXY",
];

/// Points a command's clients at the proxy under token. NO_PROXY names
/// loopback alone: the sandbox leaves loopback open, and any other host
/// that bypassed the proxy would find direct egress shut.
pub fn env(token: &str, proxy_addr: &str) -> Vec<String> {
    let u = format!("http://{token}@{proxy_addr}");
    ENV_KEYS
        .iter()
        .map(|k| {
            let v = if k.eq_ignore_ascii_case("NO_PROXY") {
                "localhost,127.0.0.1,::1"
            } else {
                &u
            };
            format!("{k}={v}")
        })
        .collect()
}

/// Reads the username of a Basic `Proxy-Authorization` header; the token a
/// command's traffic is accounted under.
pub fn proxy_token(header: &str) -> String {
    let Some(raw) = header.strip_prefix("Basic ") else {
        return String::new();
    };
    let Some(decoded) = base64_decode(raw.trim()) else {
        return String::new();
    };
    let text = String::from_utf8_lossy(&decoded);
    match text.split_once(':') {
        Some((user, _)) => user.to_string(),
        None => text.to_string(),
    }
}

fn base64_decode(s: &str) -> Option<Vec<u8>> {
    fn value(c: u8) -> Option<u8> {
        match c {
            b'A'..=b'Z' => Some(c - b'A'),
            b'a'..=b'z' => Some(c - b'a' + 26),
            b'0'..=b'9' => Some(c - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let bytes = s.as_bytes();
    if !bytes.len().is_multiple_of(4) {
        return None;
    }
    let groups = bytes.len() / 4;
    let mut out = Vec::with_capacity(groups * 3);
    for (gi, group) in bytes.chunks(4).enumerate() {
        let last = gi == groups - 1;
        let pad = group.iter().filter(|&&b| b == b'=').count();
        if pad > 0 && (!last || pad > 2) {
            return None;
        }
        let mut vals = [0u8; 4];
        for (i, &b) in group.iter().enumerate() {
            if b == b'=' {
                if i < 2 || group[i..].iter().any(|&x| x != b'=') {
                    return None;
                }
            } else {
                vals[i] = value(b)?;
            }
        }
        let n = ((vals[0] as u32) << 18)
            | ((vals[1] as u32) << 12)
            | ((vals[2] as u32) << 6)
            | (vals[3] as u32);
        out.push((n >> 16) as u8);
        if pad < 2 {
            out.push((n >> 8) as u8);
        }
        if pad < 1 {
            out.push(n as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A selector fixed to one answer, standing in for the transport's
    /// environment/OS wiring.
    struct FixedUpstream(Option<UpstreamProxy>);

    impl UpstreamSelector for FixedUpstream {
        fn upstream_for(
            &self,
            _scheme: &str,
            _host: &str,
            _port: &str,
        ) -> Result<Option<UpstreamProxy>, UpstreamFailure> {
            Ok(self.0.clone())
        }
    }

    /// The Go test's ask closure: records what it was asked and answers
    /// per host.
    struct RecordedApprover(Arc<Mutex<Vec<String>>>);

    impl EgressApprover for RecordedApprover {
        fn approve_egress(&self, host: &str) -> Result<bool, ApprovalFailure> {
            self.0.lock().expect("asked").push(host.to_string());
            match host {
                "asked.test" => Ok(true),
                "broken.test" => Err(ApprovalFailure("nobody is there".to_string())),
                _ => Ok(false),
            }
        }
    }

    fn names_gate(allow: &[&str], names: &[(&str, &str)]) -> EgressGate {
        let map: HashMap<String, Vec<IpAddr>> = names
            .iter()
            .map(|(h, ip)| ((*h).to_string(), vec![ip.parse().expect(ip)]))
            .collect();
        EgressGate::new(Policy {
            allow: allow.iter().map(|s| s.to_string()).collect(),
            deny: Vec::new(),
        })
        .with_resolver(Arc::new(move |host: &str| {
            map.get(host)
                .cloned()
                .ok_or_else(|| LookupError(format!("no such host: {host}")))
        }))
        .with_local_addresses(Arc::new(Vec::new))
    }

    // Go TestForwardAdmitsAllowedAndRecordsRefusals, decision layer: the
    // same admits, the same recorded denials, the same token accounting.
    // The transport half (the backend must not see the proxy's own
    // credentials, the 403 bodies) stays with the HTTP server.
    #[test]
    fn forward_admits_allowed_and_records_refusals() {
        let g = names_gate(
            &["allowed.test", "sneaky.test"],
            &[
                ("allowed.test", "93.184.216.34"),
                ("other.test", "93.184.216.35"),
                ("sneaky.test", "127.0.0.1"),
            ],
        );
        assert_eq!(
            g.admit("cmd1", "allowed.test", "80", None),
            Admission::Direct("93.184.216.34:80".to_string())
        );
        assert_eq!(
            g.admit("cmd1", "other.test", "80", None),
            Admission::Refused(Denial {
                host: "other.test".to_string(),
                reason: Reason::NotAllowed
            })
        );
        assert_eq!(
            g.admit("cmd1", "sneaky.test", "80", None),
            Admission::Refused(Denial {
                host: "sneaky.test".to_string(),
                reason: Reason::Address
            })
        );
        g.admit("cmd1", "other.test", "80", None);
        let want = vec![
            Denial {
                host: "other.test".to_string(),
                reason: Reason::NotAllowed,
            },
            Denial {
                host: "sneaky.test".to_string(),
                reason: Reason::Address,
            },
        ];
        assert_eq!(g.take_denials("cmd1"), want);
        assert!(g.take_denials("cmd1").is_empty(), "denials were not taken");
        assert!(
            g.take_denials("cmd2").is_empty(),
            "another command was charged"
        );
    }

    // Go TestConnectTunnelsOnlyAllowedHosts, decision layer (the tunnel
    // itself is transport).
    #[test]
    fn connect_admits_only_allowed_hosts_on_443() {
        let g = names_gate(
            &["allowed.test"],
            &[
                ("allowed.test", "93.184.216.34"),
                ("other.test", "93.184.216.35"),
            ],
        );
        assert_eq!(
            g.admit("cmd1", "allowed.test", "443", None),
            Admission::Direct("93.184.216.34:443".to_string())
        );
        assert!(matches!(
            g.admit("cmd1", "other.test", "443", None),
            Admission::Refused(Denial {
                reason: Reason::NotAllowed,
                ..
            })
        ));
        assert_eq!(
            g.take_denials("cmd1"),
            vec![Denial {
                host: "other.test".to_string(),
                reason: Reason::NotAllowed
            }]
        );
    }

    // Go TestConnectChainsThroughUpstream, decision half: through an
    // upstream the name is handed on even when this host cannot resolve it.
    #[test]
    fn unresolvable_names_go_to_the_upstream() {
        let g = names_gate(&["unresolvable.test"], &[("allowed.test", "93.184.216.34")]);
        let up = UpstreamProxy {
            scheme: "http".to_string(),
            host: "upstream.test:3128".to_string(),
            credentials: Some(("u".to_string(), Some("p".to_string()))),
        };
        assert_eq!(
            g.admit("cmd1", "unresolvable.test", "443", Some(&up)),
            Admission::Upstream("unresolvable.test:443".to_string())
        );
    }

    // Go TestUnsupportedUpstreamIsReported: an https upstream cannot chain.
    #[test]
    fn unsupported_upstream_is_reported() {
        let g = EgressGate::new(Policy::default());
        let selector = FixedUpstream(Some(UpstreamProxy {
            scheme: "https".to_string(),
            host: "127.0.0.1:1080".to_string(),
            credentials: None,
        }));
        let err = g
            .upstream_for("http", "allowed.test", "80", &selector)
            .unwrap_err();
        assert!(err.to_string().contains("https"), "{err}");
    }

    // Go TestSOCKSUpstreamCarriesAllowedTraffic, decision layer: the policy
    // applies before any upstream is consulted.
    #[test]
    fn socks_upstream_still_answers_to_the_policy() {
        let g = names_gate(&["unresolvable.test"], &[("other.test", "93.184.216.35")]);
        let up = UpstreamProxy {
            scheme: "socks5h".to_string(),
            host: "127.0.0.1:1080".to_string(),
            credentials: None,
        };
        assert!(up.is_socks5());
        assert_eq!(
            g.admit("cmd1", "unresolvable.test", "80", Some(&up)),
            Admission::Upstream("unresolvable.test:80".to_string())
        );
        assert!(matches!(
            g.admit("cmd1", "other.test", "80", Some(&up)),
            Admission::Refused(Denial {
                reason: Reason::NotAllowed,
                ..
            })
        ));
    }

    // Go TestEnvCarriesTheTokenAndBypassesOnlyLoopback, verbatim in
    // expectation.
    #[test]
    fn env_carries_the_token_and_bypasses_only_loopback() {
        let tok = new_token();
        assert_eq!(tok.len(), 24, "token {tok} is not a fresh 12-byte hex name");
        assert!(tok.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_ne!(tok, new_token());
        let env = env(&tok, "127.0.0.1:9876");
        let want = format!("HTTPS_PROXY=http://{tok}@127.0.0.1:9876");
        assert!(env.contains(&want), "env = {env:?}");
        assert!(env.contains(&"NO_PROXY=localhost,127.0.0.1,::1".to_string()));
        assert!(env.contains(&"no_proxy=localhost,127.0.0.1,::1".to_string()));
        assert_eq!(env.len(), ENV_KEYS.len());
    }

    // Go TestAskPutsUnlistedHostsToAPerson: a host outside the list is put
    // to the command's asker once, however many connections ask for it;
    // denied hosts and other commands are never asked.
    #[test]
    fn ask_puts_unlisted_hosts_to_a_person() {
        let map: HashMap<String, Vec<IpAddr>> = [
            ("asked.test", "93.184.216.34"),
            ("refused.test", "93.184.216.35"),
            ("broken.test", "93.184.216.36"),
            ("gist.test", "93.184.216.37"),
        ]
        .iter()
        .map(|(h, ip)| ((*h).to_string(), vec![ip.parse().expect(ip)]))
        .collect();
        let g = EgressGate::new(Policy {
            allow: vec!["allowed.test".to_string()],
            deny: vec!["gist.test".to_string()],
        })
        .with_resolver(Arc::new(move |host: &str| {
            map.get(host)
                .cloned()
                .ok_or_else(|| LookupError(format!("no such host: {host}")))
        }))
        .with_local_addresses(Arc::new(Vec::new));
        let asked: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        g.set_asker("cmd1", Arc::new(RecordedApprover(asked.clone())));
        for _ in 0..3 {
            assert_eq!(
                g.admit("cmd1", "asked.test", "80", None),
                Admission::Direct("93.184.216.34:80".to_string())
            );
        }
        assert!(matches!(
            g.admit("cmd1", "refused.test", "80", None),
            Admission::Refused(Denial {
                reason: Reason::Declined,
                ..
            })
        ));
        assert!(matches!(
            g.admit("cmd1", "broken.test", "80", None),
            Admission::Refused(Denial {
                reason: Reason::NotAllowed,
                ..
            })
        ));
        assert!(matches!(
            g.admit("cmd1", "gist.test", "80", None),
            Admission::Refused(Denial {
                reason: Reason::Denied,
                ..
            })
        ));
        assert!(matches!(
            g.admit("cmd2", "asked.test", "80", None),
            Admission::Refused(Denial {
                reason: Reason::NotAllowed,
                ..
            })
        ));
        let mut seen = asked.lock().expect("asked").clone();
        seen.sort();
        assert_eq!(
            seen,
            vec![
                "asked.test".to_string(),
                "broken.test".to_string(),
                "refused.test".to_string()
            ],
            "asked each unlisted host once and no denied host"
        );
        g.take_denials("cmd1");
        assert!(
            matches!(
                g.admit("cmd1", "asked.test", "80", None),
                Admission::Refused(Denial {
                    reason: Reason::NotAllowed,
                    ..
                })
            ),
            "an ended token kept its asker"
        );
    }

    #[test]
    fn denials_are_capped_per_command() {
        let g = EgressGate::new(Policy::default());
        for i in 0..40 {
            g.record("cmd1", &format!("h{i}.test"), Reason::NotAllowed);
        }
        assert_eq!(g.take_denials("cmd1").len(), MAX_DENIALS_PER_CMD);
    }

    #[test]
    fn refusals_render_host_and_reason() {
        let g = EgressGate::new(Policy::default());
        g.record("cmd1", "other.test", Reason::NotAllowed);
        assert_eq!(
            g.refusals("cmd1"),
            vec!["other.test (blocked-by-allowlist)"]
        );
        assert!(g.refusals("cmd1").is_empty());
    }

    #[test]
    fn proxy_token_reads_the_basic_username() {
        assert_eq!(proxy_token("Basic dTpw"), "u");
        assert_eq!(proxy_token("Basic"), "");
        assert_eq!(proxy_token("Bearer dTpw"), "");
        assert_eq!(proxy_token("Basic !!!"), "");
        assert_eq!(proxy_token(""), "");
    }

    #[test]
    fn upstream_proxy_parse_reads_scheme_host_credentials() {
        let up = UpstreamProxy::parse("http://u:p@127.0.0.1:3128").unwrap();
        assert_eq!(up.scheme, "http");
        assert_eq!(up.host, "127.0.0.1:3128");
        assert_eq!(
            up.credentials,
            Some(("u".to_string(), Some("p".to_string())))
        );
        let up = UpstreamProxy::parse("socks5h://[::1]").unwrap();
        assert!(up.is_socks5());
        assert_eq!(up.host, "[::1]");
        assert_eq!(up.credentials, None);
        assert!(UpstreamProxy::parse("mailto:foo@bar").is_none());
    }
}

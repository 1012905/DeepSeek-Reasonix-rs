//! Decides which redirects a download may follow (parity target:
//! `internal/safety/redirectguard`). A client that authenticates and then
//! follows a redirect anywhere is how an artifact arrives from a host nobody
//! vouched for. [`FollowGuard`] follows only HTTPS, credential-free,
//! port-free redirects onto named hosts; [`StayOnOriginGuard`] holds a
//! user-chosen address to the origin the chain started on.

use crate::urlfield::UrlFields;

/// Go's own redirect default, restated because a check replaces that
/// default outright: without a limit a chain never stops.
pub const MAX_HOPS: usize = 10;

/// A parsed redirect target: the fields of `net/url.URL` the guards read.
/// The transport parses the response's `Location` into one of these and
/// hands it to [`FollowGuard::check`] or [`StayOnOriginGuard::check`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetUrl(UrlFields);

impl TargetUrl {
    /// Parses the way `url.Parse` does at the point the guards run (the
    /// client has already rejected what does not parse). None is a refusal,
    /// never an admission.
    pub fn parse(raw: &str) -> Option<TargetUrl> {
        UrlFields::parse(raw).ok().map(TargetUrl)
    }

    pub fn scheme(&self) -> &str {
        &self.0.scheme
    }

    /// Whether the authority carries userinfo (credentials).
    pub fn has_user(&self) -> bool {
        self.0.userinfo.is_some()
    }

    /// host[:port] as spelled (Go URL.Host).
    pub fn host(&self) -> &str {
        &self.0.host
    }

    pub fn hostname(&self) -> &str {
        self.0.hostname()
    }

    pub fn port(&self) -> &str {
        self.0.port()
    }

    /// The display form the refusal sentences carry.
    pub fn to_url_string(&self) -> String {
        self.0.to_url_string()
    }
}

/// Why a redirect was not followed. The identity a caller tells apart from
/// a transport failure: the same request goes to the same place, so no
/// retry can fix one (Go `ErrRefused`, which every refusal wraps).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RedirectRefusal {
    /// The bare sentinel, for callers that refuse before there is a target
    /// to name (Go `redirectguard.ErrRefused`).
    #[error("redirect refused")]
    Refused,
    #[error("redirect refused: stopped after {hops} hops")]
    TooManyHops { hops: usize },
    #[error("redirect refused: no target URL")]
    NoTargetUrl,
    #[error("redirect refused: not HTTPS: {target:?}")]
    NotHttps { target: String },
    #[error("redirect refused: carries credentials: {target:?}")]
    CarriesCredentials { target: String },
    #[error("redirect refused: no hostname: {target:?}")]
    NoHostname { target: String },
    #[error("redirect refused: untrusted host {host:?}")]
    UntrustedHost { host: String },
    #[error("redirect refused: no origin to stay on")]
    NoOrigin,
    #[error("redirect refused: downgrades HTTPS: {target:?}")]
    DowngradesHttps { target: String },
    /// A redirect to another host. From and To are host[:port] only, so
    /// the message is safe to show and carries no path or query (Go
    /// `OriginLeft`).
    #[error(
        "redirect refused: {from} redirected to {to}; if you trust {to}, set the address to {to}"
    )]
    OriginLeft { from: String, to: String },
}

impl RedirectRefusal {
    /// Every refusal is the ErrRefused identity: nothing a retry changes.
    pub fn permanent(&self) -> bool {
        true
    }
}

/// Follows a redirect only while it stays HTTPS, carries no credentials and
/// no port, and lands on one of hosts. A host is an exact name, or a
/// ".suffix" matching any name beneath it (Go `redirectguard.Follow`).
pub fn follow(hosts: &[&str]) -> FollowGuard {
    FollowGuard {
        hosts: hosts.iter().map(|s| s.to_string()).collect(),
    }
}

#[derive(Debug, Clone)]
pub struct FollowGuard {
    hosts: Vec<String>,
}

impl FollowGuard {
    /// `hops` is the length of the chain followed so far (Go `via`).
    pub fn check(&self, target: Option<&TargetUrl>, hops: usize) -> Result<(), RedirectRefusal> {
        if hops >= MAX_HOPS {
            return Err(RedirectRefusal::TooManyHops { hops: MAX_HOPS });
        }
        let Some(target) = target else {
            return Err(RedirectRefusal::NoTargetUrl);
        };
        if !target.scheme().eq_ignore_ascii_case("https") {
            return Err(RedirectRefusal::NotHttps {
                target: target.to_url_string(),
            });
        }
        if target.has_user() {
            return Err(RedirectRefusal::CarriesCredentials {
                target: target.to_url_string(),
            });
        }
        if target.hostname().is_empty() {
            return Err(RedirectRefusal::NoHostname {
                target: target.to_url_string(),
            });
        }
        // A port is refused rather than matched: release hosts answer on
        // 443, and an allowed name on another port is a different endpoint.
        if !target.port().is_empty() || !permitted(target.hostname(), &self.hosts) {
            return Err(RedirectRefusal::UntrustedHost {
                host: target.host().to_string(),
            });
        }
        Ok(())
    }
}

/// Holds a request to an address the user chose — which may be HTTP or
/// carry a port — to the origin the chain started on (Go
/// `redirectguard.StayOnOrigin`).
#[derive(Debug, Clone, Default)]
pub struct StayOnOriginGuard;

pub fn stay_on_origin() -> StayOnOriginGuard {
    StayOnOriginGuard
}

impl StayOnOriginGuard {
    /// `origin` is the chain's first request, `target` the redirect being
    /// decided, `hops` the length of the chain followed so far.
    pub fn check(
        &self,
        origin: Option<&TargetUrl>,
        target: Option<&TargetUrl>,
        hops: usize,
    ) -> Result<(), RedirectRefusal> {
        if hops >= MAX_HOPS {
            return Err(RedirectRefusal::TooManyHops { hops: MAX_HOPS });
        }
        let (Some(origin), Some(target)) = (origin, target) else {
            return Err(RedirectRefusal::NoOrigin);
        };
        if target.has_user() {
            return Err(RedirectRefusal::CarriesCredentials {
                target: target.to_url_string(),
            });
        }
        if origin.scheme().eq_ignore_ascii_case("https")
            && !target.scheme().eq_ignore_ascii_case("https")
        {
            return Err(RedirectRefusal::DowngradesHttps {
                target: target.to_url_string(),
            });
        }
        if !same_origin_fields(origin, target) {
            return Err(RedirectRefusal::OriginLeft {
                from: origin.host().to_string(),
                to: target.host().to_string(),
            });
        }
        Ok(())
    }
}

/// Reports whether two addresses share scheme, host and port. An address
/// that does not parse as absolute is no origin and matches nothing (Go
/// `redirectguard.SameOrigin`).
pub fn same_origin(a: &str, b: &str) -> bool {
    let (Some(left), Some(right)) = (TargetUrl::parse(a.trim()), TargetUrl::parse(b.trim())) else {
        return false;
    };
    if left.host().is_empty() || right.host().is_empty() {
        return false;
    }
    left.scheme().eq_ignore_ascii_case(right.scheme()) && same_origin_fields(&left, &right)
}

fn same_origin_fields(origin: &TargetUrl, target: &TargetUrl) -> bool {
    if !origin
        .hostname()
        .strip_suffix('.')
        .unwrap_or(origin.hostname())
        .eq_ignore_ascii_case(
            target
                .hostname()
                .strip_suffix('.')
                .unwrap_or(target.hostname()),
        )
    {
        return false;
    }
    let (from, to) = (effective_port(origin), effective_port(target));
    from == to || (from == "80" && to == "443" && !origin.scheme().eq_ignore_ascii_case("https"))
}

fn effective_port(url: &TargetUrl) -> String {
    if url.port().is_empty() {
        return if url.scheme().eq_ignore_ascii_case("https") {
            "443".to_string()
        } else {
            "80".to_string()
        };
    }
    url.port().to_string()
}

/// Compares the way DNS does: case-insensitively, and with the root label a
/// resolver accepts but a string comparison would not.
fn permitted(host: &str, hosts: &[String]) -> bool {
    let lowered = host.trim().to_ascii_lowercase();
    let host = lowered.strip_suffix('.').unwrap_or(&lowered);
    if host.is_empty() {
        return false;
    }
    for allowed in hosts {
        let allowed = allowed.to_ascii_lowercase();
        if allowed.starts_with('.') {
            if host.ends_with(&allowed) {
                return true;
            }
            continue;
        }
        if host == allowed {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    // The hosts a Studio release is served from, used as the subject
    // throughout (Go releaseHosts).
    const RELEASE_HOSTS: [&str; 4] = [
        "reasonix.io",
        ".reasonix.io",
        "github.com",
        ".githubusercontent.com",
    ];

    fn guard() -> FollowGuard {
        follow(&RELEASE_HOSTS)
    }

    fn check(raw: &str, hops: usize) -> Result<(), RedirectRefusal> {
        guard().check(Some(&TargetUrl::parse(raw).expect(raw)), hops)
    }

    // Go TestFollowRefusesEachUnsafeShape: one case per field, because a
    // guard that stopped reading the scheme would still pass a test that
    // only ever handed it another hostname.
    #[test]
    fn follow_refuses_each_unsafe_shape() {
        for (name, raw, want) in [
            (
                "a plain-HTTP downgrade",
                "http://dl.reasonix.io/studio/x.tar.gz",
                "NotHttps",
            ),
            (
                "credentials in the authority",
                "https://user:pw@dl.reasonix.io/x",
                "CarriesCredentials",
            ),
            (
                "a port, which no release host answers on",
                "https://dl.reasonix.io:8443/x",
                "UntrustedHost",
            ),
            (
                "a host that merely ends in ours",
                "https://dl.reasonix.io.evil.test/x",
                "UntrustedHost",
            ),
            ("an unrelated host", "https://evil.test/x", "UntrustedHost"),
        ] {
            let err = check(raw, 1).expect_err(name);
            assert!(err.permanent(), "{name}");
            match want {
                "NotHttps" => assert!(matches!(err, RedirectRefusal::NotHttps { .. }), "{err:?}"),
                "CarriesCredentials" => assert!(
                    matches!(err, RedirectRefusal::CarriesCredentials { .. }),
                    "{err:?}"
                ),
                _ => assert!(
                    matches!(err, RedirectRefusal::UntrustedHost { .. }),
                    "{err:?}"
                ),
            }
        }
    }

    // Go TestFollowStopsAChain.
    #[test]
    fn follow_stops_a_chain() {
        assert!(check("https://dl.reasonix.io/x", MAX_HOPS).is_err());
    }

    // Go TestFollowRefusesARequestWithNoTarget.
    #[test]
    fn follow_refuses_a_request_with_no_target() {
        assert_eq!(guard().check(None, 0), Err(RedirectRefusal::NoTargetUrl));
    }

    // Go TestFollowPassesTheHostsReleasesComeFrom: a guard that refused
    // these would break every download rather than protect one.
    #[test]
    fn follow_passes_the_hosts_releases_come_from() {
        for raw in [
            "https://dl.reasonix.io/studio/versions.json",
            "https://reasonix.io/studio/x",
            "https://github.com/esengine/DeepSeek-Reasonix/releases/download/v1/x",
            "https://objects.githubusercontent.com/blob/x",
        ] {
            assert!(
                check(raw, 1).is_ok(),
                "Follow({raw}) = err, want it followed"
            );
        }
    }

    // Go TestFollowComparesHostsTheWayDNSDoes: a resolver accepts both of
    // these for the same name; a string comparison accepts neither.
    #[test]
    fn follow_compares_hosts_the_way_dns_does() {
        for raw in ["https://DL.Reasonix.IO/x", "https://dl.reasonix.io./x"] {
            assert!(
                check(raw, 1).is_ok(),
                "Follow({raw}) = err, want the same name matched"
            );
        }
    }

    // Go TestFollowWithNoHostsTrustsNothing: an empty list must refuse
    // everything rather than wave everything through.
    #[test]
    fn follow_with_no_hosts_trusts_nothing() {
        assert!(
            follow(&[])
                .check(Some(&TargetUrl::parse("https://github.com/x").unwrap()), 0)
                .is_err()
        );
    }

    // Go TestStayOnOriginFollowsOnlyTheHostItStartedOn.
    #[test]
    fn stay_on_origin_follows_only_the_host_it_started_on() {
        let guard = stay_on_origin();
        for (name, from, to, hops, want) in [
            (
                "same host and port",
                "http://localhost:8080/v1",
                "http://localhost:8080/v2",
                1,
                true,
            ),
            (
                "path-only move on https",
                "https://api.example.com/v1",
                "https://api.example.com/v2",
                1,
                true,
            ),
            (
                "http to https upgrade on default ports",
                "http://relay.example.com/v1",
                "https://relay.example.com/v1",
                1,
                true,
            ),
            (
                "host case and root label",
                "https://API.example.com/v1",
                "https://api.example.com./v1",
                1,
                true,
            ),
            (
                "another port on the same host",
                "http://127.0.0.1:1/v1",
                "http://127.0.0.1:2/v1",
                1,
                false,
            ),
            (
                "another host",
                "https://api.example.com/v1",
                "https://evil.test/v1",
                1,
                false,
            ),
            (
                "a sibling subdomain",
                "https://api.example.com/v1",
                "https://cdn.example.com/v1",
                1,
                false,
            ),
            (
                "https downgraded to http",
                "https://api.example.com/v1",
                "http://api.example.com/v1",
                1,
                false,
            ),
            (
                "credentials in the target",
                "https://api.example.com/v1",
                "https://u:p@api.example.com/v1",
                1,
                false,
            ),
            (
                "too many hops",
                "https://api.example.com/v1",
                "https://api.example.com/v2",
                10,
                false,
            ),
        ] {
            let origin = TargetUrl::parse(from).expect(from);
            let target = TargetUrl::parse(to).expect(to);
            let err = guard.check(Some(&origin), Some(&target), hops);
            assert_eq!(err.is_ok(), want, "StayOnOrigin {name}: {err:?}");
            if let Err(e) = err {
                assert!(e.permanent(), "{name}: {e:?}");
            }
        }
    }

    // Go TestStayOnOriginNamesTheHostItWasSentTo.
    #[test]
    fn stay_on_origin_names_the_host_it_was_sent_to() {
        let origin = TargetUrl::parse("https://example.com/v1").unwrap();
        let target = TargetUrl::parse("https://www.example.com/v1?token=x").unwrap();
        let err = stay_on_origin()
            .check(Some(&origin), Some(&target), 1)
            .expect_err("want OriginLeft");
        match err {
            RedirectRefusal::OriginLeft { ref from, ref to } => {
                assert_eq!(from, "example.com");
                assert_eq!(to, "www.example.com");
            }
            other => panic!("err = {other:?}, want OriginLeft"),
        }
        assert!(err.permanent());
        assert!(
            !err.to_string().contains("token"),
            "must carry no query: {err}"
        );
    }

    // Go TestRefusalsAreRecognisedAndPermanent.
    #[test]
    fn refusals_are_recognised_and_permanent() {
        assert!(RedirectRefusal::Refused.permanent());
        assert!(guard().check(None, 0).is_err(), "a request with no URL");
        let empty = TargetUrl::parse("https:///x").unwrap();
        assert!(
            guard().check(Some(&empty), 0).is_err(),
            "a URL with no hostname"
        );
        assert!(
            matches!(
                stay_on_origin().check(Some(&empty), None, 0),
                Err(RedirectRefusal::NoOrigin)
            ),
            "no earlier request to stay on"
        );
        assert!(!permitted("", &RELEASE_HOSTS.map(String::from)));
        assert!(!permitted(" . ", &RELEASE_HOSTS.map(String::from)));
    }
}

//! The forbidden-address check of the egress proxy (parity target:
//! `egress/address.go`): loopback, the link, and the metadata services.

use std::net::{IpAddr, Ipv4Addr};

/// Cloud instance-metadata endpoints outside the link-local range: Alibaba,
/// Azure's wire server, and the AWS IPv6 endpoint (`100.100.100.200/32`,
/// `168.63.129.16/32`, `fd00:ec2::/32`).
const METADATA_V4: [Ipv4Addr; 2] = [
    Ipv4Addr::new(100, 100, 100, 200),
    Ipv4Addr::new(168, 63, 129, 16),
];

/// Renders an IP the way Go's net.IP.String() does: dotted quad for IPv4 and
/// for IPv4-mapped IPv6, RFC 5952 otherwise.
pub(crate) fn go_ip_string(ip: IpAddr) -> String {
    match ip {
        IpAddr::V4(v4) => v4.to_string(),
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => v4.to_string(),
            None => v6.to_string(),
        },
    }
}

/// Reports whether dialing ip would reach this machine, its link, or a
/// metadata service. An IPv6 form that embeds an IPv4 address is judged by
/// the address it carries. Private ranges stay reachable: a company registry
/// is a legitimate place to fetch from.
pub fn forbidden_address(ip: IpAddr, local: &dyn Fn() -> Vec<IpAddr>) -> bool {
    for candidate in embedded(ip) {
        if restricted_class(candidate) {
            return true;
        }
        match candidate {
            IpAddr::V4(v4) => {
                if METADATA_V4.contains(&v4) {
                    return true;
                }
            }
            IpAddr::V6(v6) => {
                if v6.octets()[0..4] == [0xfd, 0x00, 0x0e, 0xc2] {
                    return true;
                }
            }
        }
        for own in local() {
            if go_ip_equal(own, candidate) {
                return true;
            }
        }
    }
    false
}

/// Go's IP.Is* methods answer through To4, so an IPv4-mapped IPv6 spelling
/// is judged by the IPv4 address it carries.
fn restricted_class(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback() || v4.is_unspecified() || v4.is_link_local() || v4.is_multicast()
        }
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => {
                v4.is_loopback() || v4.is_unspecified() || v4.is_link_local() || v4.is_multicast()
            }
            None => {
                let o = v6.octets();
                v6.is_loopback()
                    || v6.is_unspecified()
                    // fe80::/10 unicast, ff01::/16 and ff02::/16 multicast,
                    // and the whole ff00::/8 multicast range.
                    || o[0] == 0xff
                    || (o[0] == 0xfe && o[1] & 0xc0 == 0x80)
            }
        },
    }
}

/// The candidates the address is judged by: itself, plus the IPv4 address a
/// NAT64 (`64:ff9b::/96`) or 6to4 (`2002::/16`) spelling carries, plus the
/// v4 form of an IPv4-mapped spelling.
fn embedded(ip: IpAddr) -> Vec<IpAddr> {
    let mut out = vec![ip];
    if let IpAddr::V6(v6) = ip {
        if let Some(v4) = v6.to_ipv4_mapped() {
            out.push(IpAddr::V4(v4));
            return out;
        }
        let o = v6.octets();
        if o[0..12] == [0x00, 0x64, 0xff, 0x9b, 0, 0, 0, 0, 0, 0, 0, 0] {
            out.push(IpAddr::V4(Ipv4Addr::new(o[12], o[13], o[14], o[15])));
        }
        if o[0..2] == [0x20, 0x02] {
            out.push(IpAddr::V4(Ipv4Addr::new(o[2], o[3], o[4], o[5])));
        }
    }
    out
}

/// Go net.IP.Equal: an IPv4 address equals its IPv4-mapped IPv6 spelling.
pub(crate) fn go_ip_equal(a: IpAddr, b: IpAddr) -> bool {
    match (a, b) {
        (IpAddr::V4(a), IpAddr::V6(b)) => b.to_ipv4_mapped().is_some_and(|m| m == a),
        (IpAddr::V6(a), IpAddr::V4(b)) => a.to_ipv4_mapped().is_some_and(|m| m == b),
        (a, b) => a == b,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Go TestForbiddenAddress, verbatim in expectation.
    #[test]
    fn forbidden_address_table() {
        let own = IpAddr::V4(Ipv4Addr::new(192, 168, 7, 20));
        let local = || vec![own];
        let cases: &[(&str, bool)] = &[
            ("127.0.0.1", true),
            ("::1", true),
            ("0.0.0.0", true),
            ("169.254.169.254", true),
            ("100.100.100.200", true),
            ("168.63.129.16", true),
            ("fd00:ec2::254", true),
            ("::ffff:127.0.0.1", true),
            ("64:ff9b::7f00:1", true),
            ("2002:7f00:1::", true),
            ("224.0.0.1", true),
            ("192.168.7.20", true),
            ("10.0.0.1", false),
            ("192.168.7.21", false),
            ("93.184.216.34", false),
            ("2606:4700::6810:84", false),
        ];
        for (s, want) in cases {
            let ip: IpAddr = s.parse().expect(s);
            assert_eq!(
                forbidden_address(ip, &local),
                *want,
                "forbiddenAddress({s})"
            );
        }
    }

    #[test]
    fn mapped_local_address_is_the_hosts_own() {
        let own = IpAddr::V4(Ipv4Addr::new(192, 168, 7, 20));
        let local = || vec![own];
        let mapped: IpAddr = "::ffff:192.168.7.20".parse().unwrap();
        assert!(forbidden_address(mapped, &local));
    }

    // Probed against Go 1.27 net.IP.String (see the parity report).
    #[test]
    fn ip_string_matches_go() {
        let cases: &[(&str, &str)] = &[
            ("::ffff:127.0.0.1", "127.0.0.1"),
            ("::1", "::1"),
            ("::", "::"),
            ("64:ff9b::7f00:1", "64:ff9b::7f00:1"),
            ("2002:7f00:1::", "2002:7f00:1::"),
            ("fd00:ec2::254", "fd00:ec2::254"),
            ("10.0.0.5", "10.0.0.5"),
            ("2606:4700::6810:84", "2606:4700::6810:84"),
            ("::ffff:0:7f00:1", "::ffff:0:7f00:1"),
        ];
        for (s, want) in cases {
            assert_eq!(go_ip_string(s.parse().unwrap()), *want, "go_ip_string({s})");
        }
    }
}

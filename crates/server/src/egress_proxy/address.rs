// The proxy's SSRF floor and port policy.
//
// Decision: the in-process OSS check already ran, but with a proxy the actual
// connect happens here, so this is the check that counts. The proxy resolves
// the host itself, refuses the whole destination if *any* answer is not a
// public unicast address (a mixed answer is a DNS-rebinding tell, not a
// partial success), and then connects to the address it validated, never to
// the name again.
//
// Decision: ports are open except SMTP. Tenant traffic is HTTP(S) by contract,
// but non-standard HTTP ports are common for APIs; SMTP ports are the one
// class every PaaS blocks to stop spam relaying, so they are refused always.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

/// SMTP submission and relay ports. Refused regardless of tier.
pub const BLOCKED_PORTS: &[u16] = &[25, 465, 587, 2525];

pub fn port_allowed(port: u16) -> bool {
    port != 0 && !BLOCKED_PORTS.contains(&port)
}

/// Which destinations the proxy may connect to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddressPolicy {
    /// Production: public unicast only.
    PublicOnly,
    /// Tests only: also loopback, so a local echo server can stand in for the
    /// internet. Never constructible from configuration.
    #[cfg(test)]
    AllowLoopbackForTests,
}

impl AddressPolicy {
    pub fn permits(self, ip: IpAddr) -> bool {
        match self {
            Self::PublicOnly => is_public(ip),
            #[cfg(test)]
            Self::AllowLoopbackForTests => is_public(ip) || ip.is_loopback(),
        }
    }
}

/// Public unicast, i.e. not private, loopback, link-local, metadata, CGNAT,
/// multicast, broadcast, documentation, benchmarking or reserved space, and
/// not an IPv6 form that embeds such an IPv4 address.
pub fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_public_v4(v4),
        IpAddr::V6(v6) => is_public_v6(v6),
    }
}

fn is_public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    let reserved = a == 0                                  // "this network"
        || a == 10                                         // RFC 1918
        || a == 127                                        // loopback
        || (a == 100 && (64..=127).contains(&b))           // CGNAT 100.64/10
        || (a == 169 && b == 254)                          // link-local, cloud metadata
        || (a == 172 && (16..=31).contains(&b))            // RFC 1918
        || (a == 192 && b == 0 && c == 0)                  // IETF protocol assignments
        || (a == 192 && b == 0 && c == 2)                  // TEST-NET-1
        || (a == 192 && b == 88 && c == 99)                // 6to4 relay anycast
        || (a == 192 && b == 168)                          // RFC 1918
        || (a == 198 && (18..=19).contains(&b))            // benchmarking
        || (a == 198 && b == 51 && c == 100)               // TEST-NET-2
        || (a == 203 && b == 0 && c == 113)                // TEST-NET-3
        || a >= 224; // multicast, reserved, broadcast
    !reserved
}

fn is_public_v6(ip: Ipv6Addr) -> bool {
    let seg = ip.segments();
    if ip.is_unspecified() || ip.is_loopback() {
        return false;
    }
    // IPv4-mapped (::ffff:a.b.c.d) and IPv4-compatible (::a.b.c.d): judge the
    // embedded address.
    if let Some(v4) = ip.to_ipv4_mapped() {
        return is_public_v4(v4);
    }
    if seg[..6] == [0, 0, 0, 0, 0, 0] {
        return is_public_v4(embedded_v4(seg[6], seg[7]));
    }
    // NAT64 well-known prefix 64:ff9b::/96 and local-use 64:ff9b:1::/48.
    if seg[0] == 0x64 && seg[1] == 0xff9b {
        if seg[2..6] == [0, 0, 0, 0] {
            return is_public_v4(embedded_v4(seg[6], seg[7]));
        }
        if seg[2] == 1 {
            return false;
        }
    }
    // 6to4 2002::/16 embeds an IPv4 address in bits 16..48.
    if seg[0] == 0x2002 {
        return is_public_v4(embedded_v4(seg[1], seg[2]));
    }
    let reserved = (seg[0] & 0xfe00) == 0xfc00                  // unique local fc00::/7 (incl. fd00:ec2::254)
        || (seg[0] & 0xffc0) == 0xfe80                          // link-local fe80::/10
        || (seg[0] & 0xffc0) == 0xfec0                          // site-local fec0::/10
        || (seg[0] & 0xff00) == 0xff00                          // multicast
        || (seg[0] == 0x0100 && seg[1..4] == [0, 0, 0])         // discard 100::/64
        || (seg[0] == 0x2001 && seg[1] == 0x0db8)               // documentation
        || (seg[0] == 0x2001 && seg[1] == 0x0000)               // Teredo
        || (seg[0] == 0x2001 && (seg[1] & 0xfff0) == 0x0010)    // ORCHIDv2 / benchmarking
        || (seg[0] & 0xe000) != 0x2000; // outside global unicast 2000::/3
    !reserved
}

fn embedded_v4(hi: u16, lo: u16) -> Ipv4Addr {
    Ipv4Addr::new((hi >> 8) as u8, hi as u8, (lo >> 8) as u8, lo as u8)
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ResolveError {
    #[error("destination port is not allowed")]
    PortBlocked,
    #[error("destination does not resolve")]
    NoAddress,
    #[error("destination resolves to a non-public address")]
    NotPublic,
}

/// Resolve `host:port` and return the addresses to connect to, all of which
/// passed `policy`. Fails if any answer is refused.
pub async fn resolve(
    host: &str,
    port: u16,
    policy: AddressPolicy,
) -> Result<Vec<SocketAddr>, ResolveError> {
    if !port_allowed(port) {
        return Err(ResolveError::PortBlocked);
    }
    let literal = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);
    let addrs: Vec<SocketAddr> = if let Ok(ip) = literal.parse::<IpAddr>() {
        vec![SocketAddr::new(ip, port)]
    } else {
        tokio::net::lookup_host((literal, port))
            .await
            .map_err(|_| ResolveError::NoAddress)?
            .collect()
    };
    if addrs.is_empty() {
        return Err(ResolveError::NoAddress);
    }
    if addrs.iter().any(|addr| !policy.permits(addr.ip())) {
        return Err(ResolveError::NotPublic);
    }
    Ok(addrs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().expect("test address parses")
    }

    #[test]
    fn private_and_special_addresses_are_not_public() {
        for addr in [
            "0.0.0.0",
            "10.1.2.3",
            "127.0.0.1",
            "100.64.0.1",
            "100.127.255.254",
            "169.254.169.254",
            "172.16.0.1",
            "172.31.255.255",
            "192.168.1.1",
            "192.0.0.8",
            "192.0.2.1",
            "198.18.0.1",
            "203.0.113.5",
            "224.0.0.1",
            "239.255.255.250",
            "255.255.255.255",
            "::",
            "::1",
            "fd00:ec2::254",
            "fc00::1",
            "fe80::1",
            "ff02::1",
            "2001:db8::1",
            "::ffff:127.0.0.1",
            "::ffff:169.254.169.254",
            "::ffff:10.0.0.1",
            "::10.0.0.1",
            "64:ff9b::a00:1",
            "2002:a00:1::1",
        ] {
            assert!(!is_public(ip(addr)), "{addr} must not be public");
        }
    }

    #[test]
    fn public_addresses_are_public() {
        for addr in [
            "1.1.1.1",
            "8.8.8.8",
            "100.63.255.255",
            "100.128.0.1",
            "172.15.0.1",
            "172.32.0.1",
            "93.184.216.34",
            "2606:4700:4700::1111",
            "::ffff:1.1.1.1",
            "64:ff9b::101:101",
        ] {
            assert!(is_public(ip(addr)), "{addr} should be public");
        }
    }

    #[test]
    fn smtp_ports_are_always_blocked() {
        for port in [25, 465, 587, 2525, 0] {
            assert!(!port_allowed(port), "{port} must be blocked");
        }
        for port in [80, 443, 8080, 8443, 22] {
            assert!(port_allowed(port), "{port} should be allowed");
        }
    }

    #[tokio::test]
    async fn resolve_refuses_internal_literals_and_names() {
        let policy = AddressPolicy::PublicOnly;
        assert_eq!(
            resolve("169.254.169.254", 80, policy).await,
            Err(ResolveError::NotPublic)
        );
        assert_eq!(
            resolve("[fd00:ec2::254]", 80, policy).await,
            Err(ResolveError::NotPublic)
        );
        assert_eq!(
            resolve("127.0.0.1", 443, policy).await,
            Err(ResolveError::NotPublic)
        );
        // `localhost` resolves to loopback without touching the network.
        assert_eq!(
            resolve("localhost", 443, policy).await,
            Err(ResolveError::NotPublic)
        );
        assert_eq!(
            resolve("1.1.1.1", 25, policy).await,
            Err(ResolveError::PortBlocked)
        );
        assert_eq!(
            resolve("1.1.1.1", 443, policy).await,
            Ok(vec![SocketAddr::new(ip("1.1.1.1"), 443)])
        );
    }

    #[tokio::test]
    async fn the_test_override_admits_loopback_only() {
        let policy = AddressPolicy::AllowLoopbackForTests;
        assert!(resolve("127.0.0.1", 8080, policy).await.is_ok());
        assert_eq!(
            resolve("10.0.0.1", 8080, policy).await,
            Err(ResolveError::NotPublic)
        );
    }
}

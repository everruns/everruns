// THREAT[TM-SSRF-001]: Resolve-then-check prevents SSRF via private IP access
// THREAT[TM-SSRF-005]: DNS pinning prevents DNS rebinding attacks
// THREAT[TM-SSRF-006]: IPv6-mapped-IPv4 canonicalization catches mapped addresses
//! DNS resolution policy for SSRF prevention
//!
//! Resolves hostnames to IP addresses and validates against blocked ranges
//! before allowing connections. Prevents SSRF attacks where DNS names resolve
//! to private/internal IP addresses.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, ToSocketAddrs};

/// Policy for DNS resolution and IP validation
///
/// When `block_private` is enabled, resolved IP addresses are checked against
/// known private/reserved ranges before the connection is established.
///
/// # Examples
///
/// ```
/// use fetchkit::DnsPolicy;
///
/// let policy = DnsPolicy::block_private_ips();
/// assert!(policy.is_blocked_ip("127.0.0.1".parse().unwrap()));
/// assert!(policy.is_blocked_ip("10.0.0.1".parse().unwrap()));
/// assert!(!policy.is_blocked_ip("8.8.8.8".parse().unwrap()));
/// ```
#[derive(Debug, Clone)]
pub struct DnsPolicy {
    /// Block private/reserved IP ranges
    pub block_private: bool,
}

/// Default is safe: blocks private/reserved IPs
impl Default for DnsPolicy {
    fn default() -> Self {
        Self::block_private_ips()
    }
}

impl DnsPolicy {
    /// Create a policy that blocks private/reserved IP ranges
    ///
    /// Blocks:
    /// - Loopback (127.0.0.0/8, ::1)
    /// - Private (10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16)
    /// - Link-local (169.254.0.0/16, fe80::/10)
    /// - Unspecified (0.0.0.0, ::)
    /// - Documentation (192.0.2.0/24, 198.51.100.0/24, 203.0.113.0/24)
    /// - Benchmarking (198.18.0.0/15)
    /// - Carrier-grade NAT (100.64.0.0/10)
    /// - Unique local IPv6 (fc00::/7)
    /// - Multicast (224.0.0.0/4, ff00::/8)
    /// - Broadcast (255.255.255.255)
    pub fn block_private_ips() -> Self {
        Self {
            block_private: true,
        }
    }

    /// Create a permissive policy (no IP blocking)
    pub fn allow_all() -> Self {
        Self {
            block_private: false,
        }
    }

    /// Check if an IP address is in a blocked range
    ///
    /// Handles IPv6-mapped IPv4 addresses by canonicalizing first.
    pub fn is_blocked_ip(&self, ip: IpAddr) -> bool {
        if !self.block_private {
            return false;
        }

        // THREAT[TM-SSRF-006]: Canonicalize IPv6-mapped IPv4 (::ffff:127.0.0.1 -> 127.0.0.1)
        let canonical = ip.to_canonical();

        match canonical {
            IpAddr::V4(ipv4) => is_blocked_ipv4(ipv4),
            IpAddr::V6(ipv6) => is_blocked_ipv6(ipv6),
        }
    }

    /// Produce the pinned socket addresses for a host that a transport must
    /// connect to.
    ///
    /// When the policy blocks private IPs this runs resolve-then-check and returns
    /// the validated address (TM-SSRF-001, TM-SSRF-005). When the policy is
    /// permissive (`allow_all`) it returns an empty vec, signalling that the
    /// transport may resolve the host itself.
    ///
    /// # Arguments
    /// * `host` - Hostname to resolve
    /// * `port` - Port to use in the returned SocketAddr
    pub fn pinned_addrs(&self, host: &str, port: u16) -> Result<Vec<SocketAddr>, DnsPolicyError> {
        if !self.block_private {
            return Ok(Vec::new());
        }
        let addr = self.resolve_and_validate(host, port)?;
        Ok(vec![addr])
    }

    /// Resolve a hostname and validate all resolved IPs against the policy
    ///
    /// Returns the first valid (non-blocked) socket address, or an error if
    /// all resolved addresses are blocked or resolution fails.
    ///
    /// # Arguments
    /// * `host` - Hostname to resolve
    /// * `port` - Port to use in the returned SocketAddr
    pub fn resolve_and_validate(
        &self,
        host: &str,
        port: u16,
    ) -> Result<SocketAddr, DnsPolicyError> {
        // Resolve hostname
        let addrs: Vec<SocketAddr> = format!("{}:{}", host, port)
            .to_socket_addrs()
            .map_err(|e| DnsPolicyError::ResolutionFailed(host.to_string(), e.to_string()))?
            .collect();

        if addrs.is_empty() {
            return Err(DnsPolicyError::NoAddresses(host.to_string()));
        }

        if !self.block_private {
            return Ok(addrs[0]);
        }

        // Check all resolved addresses
        let mut all_blocked = true;
        let mut first_valid = None;

        for addr in &addrs {
            if self.is_blocked_ip(addr.ip()) {
                tracing::debug!(
                    ip = %addr.ip(),
                    host = host,
                    "Blocked private/reserved IP"
                );
            } else {
                all_blocked = false;
                if first_valid.is_none() {
                    first_valid = Some(*addr);
                }
            }
        }

        if all_blocked {
            return Err(DnsPolicyError::AllAddressesBlocked(host.to_string()));
        }

        Ok(first_valid.unwrap())
    }
}

/// Errors from DNS policy validation
#[derive(Debug, thiserror::Error)]
pub enum DnsPolicyError {
    /// DNS resolution failed
    #[error("DNS resolution failed for {0}: {1}")]
    ResolutionFailed(String, String),

    /// No addresses returned from DNS
    #[error("No addresses resolved for {0}")]
    NoAddresses(String),

    /// All resolved addresses are in blocked ranges
    #[error("All resolved addresses for {0} are in blocked IP ranges (private/reserved)")]
    AllAddressesBlocked(String),
}

/// Check if an IPv4 address is in a blocked range
fn is_blocked_ipv4(ip: Ipv4Addr) -> bool {
    let octets = ip.octets();

    // Unspecified: 0.0.0.0
    if ip.is_unspecified() {
        return true;
    }

    // Loopback: 127.0.0.0/8
    if ip.is_loopback() {
        return true;
    }

    // Private: 10.0.0.0/8
    if octets[0] == 10 {
        return true;
    }

    // Private: 172.16.0.0/12
    if octets[0] == 172 && (16..=31).contains(&octets[1]) {
        return true;
    }

    // Private: 192.168.0.0/16
    if octets[0] == 192 && octets[1] == 168 {
        return true;
    }

    // Link-local: 169.254.0.0/16 (includes cloud metadata 169.254.169.254)
    // THREAT[TM-SSRF-003]: Block link-local range to prevent cloud metadata access
    if ip.is_link_local() {
        return true;
    }

    // Carrier-grade NAT: 100.64.0.0/10
    if octets[0] == 100 && (64..=127).contains(&octets[1]) {
        return true;
    }

    // Documentation: 192.0.2.0/24 (TEST-NET-1)
    if octets[0] == 192 && octets[1] == 0 && octets[2] == 2 {
        return true;
    }

    // Documentation: 198.51.100.0/24 (TEST-NET-2)
    if octets[0] == 198 && octets[1] == 51 && octets[2] == 100 {
        return true;
    }

    // Documentation: 203.0.113.0/24 (TEST-NET-3)
    if octets[0] == 203 && octets[1] == 0 && octets[2] == 113 {
        return true;
    }

    // Benchmarking: 198.18.0.0/15
    if octets[0] == 198 && (18..=19).contains(&octets[1]) {
        return true;
    }

    // Multicast: 224.0.0.0/4
    if ip.is_multicast() {
        return true;
    }

    // Broadcast: 255.255.255.255
    if ip.is_broadcast() {
        return true;
    }

    false
}

/// Check if an IPv6 address is in a blocked range
fn is_blocked_ipv6(ip: Ipv6Addr) -> bool {
    // Unspecified: ::
    if ip.is_unspecified() {
        return true;
    }

    // Loopback: ::1
    if ip.is_loopback() {
        return true;
    }

    // Multicast: ff00::/8
    if ip.is_multicast() {
        return true;
    }

    let segments = ip.segments();

    // Link-local: fe80::/10
    if segments[0] & 0xffc0 == 0xfe80 {
        return true;
    }

    // Unique local: fc00::/7
    if segments[0] & 0xfe00 == 0xfc00 {
        return true;
    }

    // THREAT[TM-SSRF-006]: IPv4-compatible IPv6 (deprecated, ::<ipv4>, prefix 0000:0000:...)
    // Extract embedded IPv4 from the low 32 bits and validate it
    if segments[0] == 0
        && segments[1] == 0
        && segments[2] == 0
        && segments[3] == 0
        && segments[4] == 0
        && segments[5] == 0
        && !(segments[6] == 0 && segments[7] == 0)
    // exclude :: (unspecified) and ::1 (loopback)
    {
        let ipv4 = Ipv4Addr::new(
            (segments[6] >> 8) as u8,
            segments[6] as u8,
            (segments[7] >> 8) as u8,
            segments[7] as u8,
        );
        if is_blocked_ipv4(ipv4) {
            return true;
        }
    }

    // THREAT[TM-SSRF-006]: 6to4 addresses (2002::/16) embed IPv4 in bits 16-47
    // e.g. 2002:7f00:0001:: encodes 127.0.0.1
    if segments[0] == 0x2002 {
        let ipv4 = Ipv4Addr::new(
            (segments[1] >> 8) as u8,
            segments[1] as u8,
            (segments[2] >> 8) as u8,
            segments[2] as u8,
        );
        if is_blocked_ipv4(ipv4) {
            return true;
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> DnsPolicy {
        DnsPolicy::block_private_ips()
    }

    // ==================== IPv4 blocked ranges ====================

    #[test]
    fn test_loopback_blocked() {
        let p = policy();
        assert!(p.is_blocked_ip("127.0.0.1".parse().unwrap()));
        assert!(p.is_blocked_ip("127.0.0.2".parse().unwrap()));
        assert!(p.is_blocked_ip("127.255.255.255".parse().unwrap()));
    }

    #[test]
    fn test_private_10_blocked() {
        let p = policy();
        assert!(p.is_blocked_ip("10.0.0.1".parse().unwrap()));
        assert!(p.is_blocked_ip("10.255.255.255".parse().unwrap()));
    }

    #[test]
    fn test_private_172_blocked() {
        let p = policy();
        assert!(p.is_blocked_ip("172.16.0.1".parse().unwrap()));
        assert!(p.is_blocked_ip("172.31.255.255".parse().unwrap()));
        // 172.15.x.x and 172.32.x.x are NOT private
        assert!(!p.is_blocked_ip("172.15.0.1".parse().unwrap()));
        assert!(!p.is_blocked_ip("172.32.0.1".parse().unwrap()));
    }

    #[test]
    fn test_private_192_168_blocked() {
        let p = policy();
        assert!(p.is_blocked_ip("192.168.0.1".parse().unwrap()));
        assert!(p.is_blocked_ip("192.168.255.255".parse().unwrap()));
    }

    #[test]
    fn test_link_local_blocked() {
        let p = policy();
        assert!(p.is_blocked_ip("169.254.0.1".parse().unwrap()));
        assert!(p.is_blocked_ip("169.254.169.254".parse().unwrap())); // Cloud metadata
        assert!(p.is_blocked_ip("169.254.255.255".parse().unwrap()));
    }

    #[test]
    fn test_carrier_grade_nat_blocked() {
        let p = policy();
        assert!(p.is_blocked_ip("100.64.0.1".parse().unwrap()));
        assert!(p.is_blocked_ip("100.127.255.255".parse().unwrap()));
        assert!(!p.is_blocked_ip("100.63.0.1".parse().unwrap()));
        assert!(!p.is_blocked_ip("100.128.0.1".parse().unwrap()));
    }

    #[test]
    fn test_documentation_ranges_blocked() {
        let p = policy();
        // TEST-NET-1
        assert!(p.is_blocked_ip("192.0.2.1".parse().unwrap()));
        // TEST-NET-2
        assert!(p.is_blocked_ip("198.51.100.1".parse().unwrap()));
        // TEST-NET-3
        assert!(p.is_blocked_ip("203.0.113.1".parse().unwrap()));
    }

    #[test]
    fn test_benchmarking_blocked() {
        let p = policy();
        assert!(p.is_blocked_ip("198.18.0.1".parse().unwrap()));
        assert!(p.is_blocked_ip("198.19.255.255".parse().unwrap()));
        assert!(!p.is_blocked_ip("198.17.0.1".parse().unwrap()));
        assert!(!p.is_blocked_ip("198.20.0.1".parse().unwrap()));
    }

    #[test]
    fn test_multicast_blocked() {
        let p = policy();
        assert!(p.is_blocked_ip("224.0.0.1".parse().unwrap()));
        assert!(p.is_blocked_ip("239.255.255.255".parse().unwrap()));
    }

    #[test]
    fn test_broadcast_blocked() {
        let p = policy();
        assert!(p.is_blocked_ip("255.255.255.255".parse().unwrap()));
    }

    #[test]
    fn test_unspecified_blocked() {
        let p = policy();
        assert!(p.is_blocked_ip("0.0.0.0".parse().unwrap()));
    }

    // ==================== IPv6 blocked ranges ====================

    #[test]
    fn test_ipv6_loopback_blocked() {
        let p = policy();
        assert!(p.is_blocked_ip("::1".parse().unwrap()));
    }

    #[test]
    fn test_ipv6_unspecified_blocked() {
        let p = policy();
        assert!(p.is_blocked_ip("::".parse().unwrap()));
    }

    #[test]
    fn test_ipv6_link_local_blocked() {
        let p = policy();
        assert!(p.is_blocked_ip("fe80::1".parse().unwrap()));
        assert!(p.is_blocked_ip("fe80::ffff:ffff:ffff:ffff".parse().unwrap()));
    }

    #[test]
    fn test_ipv6_unique_local_blocked() {
        let p = policy();
        assert!(p.is_blocked_ip("fc00::1".parse().unwrap()));
        assert!(p.is_blocked_ip("fd00::1".parse().unwrap()));
    }

    #[test]
    fn test_ipv6_multicast_blocked() {
        let p = policy();
        assert!(p.is_blocked_ip("ff02::1".parse().unwrap()));
    }

    // THREAT[TM-SSRF-006]: IPv6-mapped IPv4 addresses
    #[test]
    fn test_ipv6_mapped_ipv4_blocked() {
        let p = policy();
        // ::ffff:127.0.0.1 should be blocked (maps to loopback)
        assert!(p.is_blocked_ip("::ffff:127.0.0.1".parse().unwrap()));
        // ::ffff:10.0.0.1 should be blocked (maps to private)
        assert!(p.is_blocked_ip("::ffff:10.0.0.1".parse().unwrap()));
        // ::ffff:169.254.169.254 should be blocked (maps to metadata)
        assert!(p.is_blocked_ip("::ffff:169.254.169.254".parse().unwrap()));
        // ::ffff:8.8.8.8 should NOT be blocked (maps to public)
        assert!(!p.is_blocked_ip("::ffff:8.8.8.8".parse().unwrap()));
    }

    // THREAT[TM-SSRF-011]: IPv4-compatible IPv6 addresses (deprecated)
    #[test]
    fn test_ipv4_compatible_ipv6_blocked() {
        let p = policy();
        // ::7f00:1 is the hex form of ::127.0.0.1 (IPv4-compatible, deprecated)
        assert!(p.is_blocked_ip("::7f00:1".parse().unwrap()));
        // ::a00:1 is ::10.0.0.1 (private)
        assert!(p.is_blocked_ip("::a00:1".parse().unwrap()));
        // ::c0a8:1 is ::192.168.0.1 (private)
        assert!(p.is_blocked_ip("::c0a8:1".parse().unwrap()));
        // ::a9fe:a9fe is ::169.254.169.254 (metadata)
        assert!(p.is_blocked_ip("::a9fe:a9fe".parse().unwrap()));
        // ::808:808 is ::8.8.8.8 (public) — should NOT be blocked
        assert!(!p.is_blocked_ip("::808:808".parse().unwrap()));
    }

    // THREAT[TM-SSRF-011]: 6to4 addresses (2002::/16)
    #[test]
    fn test_6to4_ipv6_blocked() {
        let p = policy();
        // 2002:7f00:1:: encodes 127.0.0.1
        assert!(p.is_blocked_ip("2002:7f00:1::".parse().unwrap()));
        // 2002:a00:1:: encodes 10.0.0.1
        assert!(p.is_blocked_ip("2002:a00:1::".parse().unwrap()));
        // 2002:c0a8:1:: encodes 192.168.0.1
        assert!(p.is_blocked_ip("2002:c0a8:1::".parse().unwrap()));
        // 2002:a9fe:a9fe:: encodes 169.254.169.254
        assert!(p.is_blocked_ip("2002:a9fe:a9fe::".parse().unwrap()));
        // 2002:808:808:: encodes 8.8.8.8 (public) — should NOT be blocked
        assert!(!p.is_blocked_ip("2002:808:808::".parse().unwrap()));
    }

    // ==================== Public IPs allowed ====================

    #[test]
    fn test_public_ipv4_allowed() {
        let p = policy();
        assert!(!p.is_blocked_ip("8.8.8.8".parse().unwrap()));
        assert!(!p.is_blocked_ip("1.1.1.1".parse().unwrap()));
        assert!(!p.is_blocked_ip("93.184.216.34".parse().unwrap()));
        assert!(!p.is_blocked_ip("140.82.121.3".parse().unwrap()));
    }

    #[test]
    fn test_public_ipv6_allowed() {
        let p = policy();
        assert!(!p.is_blocked_ip("2001:4860:4860::8888".parse().unwrap()));
        assert!(!p.is_blocked_ip("2606:4700:4700::1111".parse().unwrap()));
    }

    // ==================== Policy modes ====================

    #[test]
    fn test_allow_all_permits_private() {
        let p = DnsPolicy::allow_all();
        assert!(!p.is_blocked_ip("127.0.0.1".parse().unwrap()));
        assert!(!p.is_blocked_ip("10.0.0.1".parse().unwrap()));
        assert!(!p.is_blocked_ip("169.254.169.254".parse().unwrap()));
    }

    #[test]
    fn test_default_blocks_private() {
        let p = DnsPolicy::default();
        assert!(p.is_blocked_ip("127.0.0.1".parse().unwrap()));
        assert!(p.is_blocked_ip("10.0.0.1".parse().unwrap()));
        assert!(p.is_blocked_ip("169.254.169.254".parse().unwrap()));
        assert!(!p.is_blocked_ip("8.8.8.8".parse().unwrap()));
    }

    // ==================== resolve_and_validate ====================

    #[test]
    fn test_resolve_loopback_blocked() {
        let p = policy();
        let result = p.resolve_and_validate("127.0.0.1", 80);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("blocked"), "Error was: {}", err);
    }

    #[test]
    fn test_resolve_private_blocked() {
        let p = policy();
        let result = p.resolve_and_validate("10.0.0.1", 80);
        assert!(result.is_err());
    }

    #[test]
    fn test_resolve_nonexistent_fails() {
        let p = policy();
        let result = p.resolve_and_validate("this-host-definitely-does-not-exist.invalid", 80);
        assert!(result.is_err());
    }

    #[test]
    fn test_resolve_allow_all_permits_loopback() {
        let p = DnsPolicy::allow_all();
        let result = p.resolve_and_validate("127.0.0.1", 80);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().ip(), "127.0.0.1".parse::<IpAddr>().unwrap());
    }
}

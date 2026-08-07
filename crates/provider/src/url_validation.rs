// Shared URL validation for SSRF prevention
//
// THREAT[TM-API-008]: Blocks requests to internal/private networks.
// Used by MCP server registration (create/update) and execution-time checks.
//
// Validates:
// - Scheme restricted to https/http
// - Hostname not localhost, loopback, or private IP
// - Blocks link-local (169.254.x.x), cloud metadata (169.254.169.254)
// - Blocks RFC1918 (10.x, 172.16-31.x, 192.168.x)
// - Blocks IPv6 loopback (::1) and link-local (fe80::)

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;
use url::Url;

/// DNS resolution is capped at this duration to prevent a stuck resolver from
/// stalling MCP tool execution/discovery past the outbound HTTP timeout.
const DNS_LOOKUP_TIMEOUT: Duration = Duration::from_secs(5);

/// Errors from URL safety validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UrlValidationError {
    /// URL could not be parsed.
    InvalidUrl(String),
    /// Scheme not allowed (must be http or https).
    DisallowedScheme(String),
    /// Hostname is missing.
    MissingHostname,
    /// Hostname resolves to or is a blocked target.
    BlockedHost(String),
}

impl std::fmt::Display for UrlValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidUrl(msg) => write!(f, "Invalid URL: {msg}"),
            Self::DisallowedScheme(scheme) => {
                write!(f, "Disallowed URL scheme: {scheme} (must be http or https)")
            }
            Self::MissingHostname => write!(f, "URL must have a hostname"),
            Self::BlockedHost(host) => {
                write!(f, "Blocked host: {host} (private/internal address)")
            }
        }
    }
}

impl std::error::Error for UrlValidationError {}

/// Validate a URL is safe for server-side requests (anti-SSRF).
///
/// Checks scheme, hostname patterns, and IP address ranges.
/// Does NOT perform DNS resolution — this is a static check suitable for
/// write-time validation. Complement with runtime DNS-pinned checks where possible.
pub fn validate_safe_url(raw_url: &str) -> Result<Url, UrlValidationError> {
    let url = Url::parse(raw_url).map_err(|e| UrlValidationError::InvalidUrl(e.to_string()))?;

    // Scheme check
    match url.scheme() {
        "http" | "https" => {}
        other => return Err(UrlValidationError::DisallowedScheme(other.to_string())),
    }

    // Must have a host
    let host = url.host_str().ok_or(UrlValidationError::MissingHostname)?;

    // Check hostname patterns
    if is_blocked_host(host) {
        return Err(UrlValidationError::BlockedHost(host.to_string()));
    }

    Ok(url)
}

/// Validate a URL is safe for server-side requests, including DNS resolution
/// to catch DNS-rebinding attacks (TM-TOOL-018).
///
/// Performs static checks first (`validate_safe_url`), then resolves the
/// hostname (with a 5-second timeout) and verifies that every returned IP is
/// in an allowed range.  Returns the parsed `Url` and the **resolved socket
/// addresses** so callers can pin the subsequent connection to those exact IPs
/// via `reqwest::ClientBuilder::resolve_to_addrs`, closing the TOCTOU window.
///
/// For IP-literal URLs the returned `Vec<SocketAddr>` is empty — the static
/// check already validated the IP.  For write-time registration checks,
/// `validate_safe_url` is sufficient; use this function at execution time
/// (i.e. just before each outbound HTTP call).
pub async fn validate_url_dns_pinned(
    raw_url: &str,
) -> Result<(Url, Vec<SocketAddr>), UrlValidationError> {
    validate_url_with_resolver(raw_url, default_dns_resolve).await
}

/// Inner implementation with an injectable resolver for unit testing.
async fn validate_url_with_resolver<R, F>(
    raw_url: &str,
    resolve: R,
) -> Result<(Url, Vec<SocketAddr>), UrlValidationError>
where
    R: Fn(String, u16) -> F,
    F: std::future::Future<Output = Result<Vec<SocketAddr>, std::io::Error>>,
{
    // Static checks first — fast path for obviously bad URLs / IP literals.
    let url = validate_safe_url(raw_url)?;
    let host = url
        .host_str()
        .ok_or(UrlValidationError::MissingHostname)?
        .to_string();

    // IP literals were already validated by the static check; no DNS needed.
    let bare = host
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(&host);
    if bare.parse::<IpAddr>().is_ok() {
        return Ok((url, Vec::new()));
    }

    // Resolve the hostname and reject if any address falls in a blocked range.
    let port = url.port_or_known_default().unwrap_or(443);
    let addrs = resolve(host.clone(), port)
        .await
        .map_err(|_| UrlValidationError::BlockedHost(host.clone()))?;

    if addrs.is_empty() {
        return Err(UrlValidationError::BlockedHost(host.clone()));
    }

    for addr in &addrs {
        if is_blocked_ip(addr.ip()) {
            tracing::warn!(
                host = %host,
                resolved_ip = %addr.ip(),
                "DNS rebinding check blocked: hostname resolves to private address"
            );
            return Err(UrlValidationError::BlockedHost(format!(
                "{host} resolves to blocked address {}",
                addr.ip()
            )));
        }
    }

    Ok((url, addrs))
}

/// Default resolver used in production: `tokio::net::lookup_host` with a
/// 5-second timeout so a stuck DNS server cannot stall MCP execution.
async fn default_dns_resolve(host: String, port: u16) -> Result<Vec<SocketAddr>, std::io::Error> {
    tokio::time::timeout(
        DNS_LOOKUP_TIMEOUT,
        tokio::net::lookup_host(format!("{host}:{port}")),
    )
    .await
    .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "DNS lookup timed out"))?
    .map(|iter| iter.collect())
}

/// Check if a hostname string is blocked (localhost, private IPs, metadata endpoints).
fn is_blocked_host(host: &str) -> bool {
    let host_lower = host.to_lowercase();

    // Localhost variants
    if host_lower == "localhost"
        || host_lower == "localhost."
        || host_lower.ends_with(".localhost")
        || host_lower.ends_with(".localhost.")
    {
        return true;
    }

    // Strip brackets from IPv6
    let bare = host_lower
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(&host_lower);

    // Try parsing as IP
    if let Ok(ip) = bare.parse::<IpAddr>() {
        return is_blocked_ip(ip);
    }

    // Cloud metadata hostname
    if host_lower == "metadata.google.internal" || host_lower == "metadata.google.internal." {
        return true;
    }

    false
}

/// Check if an IP address is in a blocked range (loopback, private, link-local,
/// CGNAT, documentation, IPv4-mapped IPv6 variants of any of those).
///
/// Use this for runtime DNS-pinned checks where the caller has already resolved
/// a hostname to its actual address. Static URL/hostname checks should use
/// [`validate_safe_url`] instead.
///
/// Self-hosted deployments can exempt specific ranges via the
/// [`SSRF_ALLOW_CIDRS_ENV`] environment variable; see [`operator_allowed_cidrs`].
pub fn is_blocked_ip(ip: IpAddr) -> bool {
    is_blocked_ip_with_allowlist(ip, operator_allowed_cidrs())
}

/// Operator allowlist env var: comma-separated CIDRs (e.g.
/// `10.42.0.0/16,10.43.0.0/16`) exempted from the private-range block.
pub const SSRF_ALLOW_CIDRS_ENV: &str = "EVERRUNS_SSRF_ALLOW_CIDRS";

/// CIDR ranges the operator explicitly exempted from SSRF blocking, parsed
/// once from [`SSRF_ALLOW_CIDRS_ENV`]. Empty (block everything private) unless
/// the deployment sets the variable.
///
/// This is a single-tenant/self-hosted escape hatch: it lets in-cluster
/// service URLs (e.g. `http://tools.ns.svc.cluster.local:8100/mcp`) pass
/// validation without exposing the service publicly. It applies to every
/// consumer of this module — MCP servers, web fetch egress, plugin and OAuth
/// fetches — so keep the ranges as narrow as possible. Hostname-pattern blocks
/// (`localhost`, `metadata.google.internal`) are unaffected.
pub fn operator_allowed_cidrs() -> &'static [Cidr] {
    static ALLOWED: std::sync::LazyLock<Vec<Cidr>> = std::sync::LazyLock::new(|| {
        let raw = std::env::var(SSRF_ALLOW_CIDRS_ENV).unwrap_or_default();
        let cidrs = parse_cidr_list(&raw);
        for cidr in &cidrs {
            tracing::warn!(
                cidr = %cidr,
                "SSRF protection: operator allowlisted a private range via {SSRF_ALLOW_CIDRS_ENV}"
            );
        }
        cidrs
    });
    &ALLOWED
}

/// Parse a comma-separated CIDR list, skipping (and logging) invalid entries.
fn parse_cidr_list(raw: &str) -> Vec<Cidr> {
    raw.split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .filter_map(|entry| match entry.parse::<Cidr>() {
            Ok(cidr) => Some(cidr),
            Err(()) => {
                tracing::warn!(entry, "Ignoring invalid CIDR in {SSRF_ALLOW_CIDRS_ENV}");
                None
            }
        })
        .collect()
}

/// An IP network in CIDR notation (`10.42.0.0/16`, `fd00::/8`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cidr {
    network: IpAddr,
    prefix: u8,
}

impl std::fmt::Display for Cidr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.network, self.prefix)
    }
}

impl std::str::FromStr for Cidr {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, ()> {
        let (addr, prefix) = s.split_once('/').ok_or(())?;
        let network: IpAddr = addr.trim().parse().map_err(|_| ())?;
        let prefix: u8 = prefix.trim().parse().map_err(|_| ())?;
        let max = match network {
            IpAddr::V4(_) => 32,
            IpAddr::V6(_) => 128,
        };
        if prefix > max {
            return Err(());
        }
        Ok(Self { network, prefix })
    }
}

impl Cidr {
    /// Whether `ip` falls inside this network. IPv4-mapped IPv6 addresses are
    /// compared as their embedded IPv4 address against IPv4 networks.
    fn contains(&self, ip: IpAddr) -> bool {
        let ip = match ip {
            IpAddr::V6(v6) => v6.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(ip),
            v4 => v4,
        };
        match (self.network, ip) {
            (IpAddr::V4(net), IpAddr::V4(ip)) => {
                // checked_shl(32) (prefix 0) yields None -> mask 0 -> match-all.
                let mask = u32::MAX
                    .checked_shl(32 - u32::from(self.prefix))
                    .unwrap_or(0);
                (u32::from(net) & mask) == (u32::from(ip) & mask)
            }
            (IpAddr::V6(net), IpAddr::V6(ip)) => {
                let mask = u128::MAX
                    .checked_shl(128 - u32::from(self.prefix))
                    .unwrap_or(0);
                (u128::from(net) & mask) == (u128::from(ip) & mask)
            }
            _ => false,
        }
    }
}

/// [`is_blocked_ip`] with an explicit allowlist (unit-testable without env).
fn is_blocked_ip_with_allowlist(ip: IpAddr, allowed: &[Cidr]) -> bool {
    let blocked = match ip {
        IpAddr::V4(v4) => is_blocked_ipv4(v4),
        IpAddr::V6(v6) => is_blocked_ipv6(v6),
    };
    blocked && !allowed.iter().any(|cidr| cidr.contains(ip))
}

fn is_blocked_ipv4(ip: Ipv4Addr) -> bool {
    let octets = ip.octets();

    // Loopback 127.0.0.0/8
    if octets[0] == 127 {
        return true;
    }

    // 0.0.0.0
    if ip.is_unspecified() {
        return true;
    }

    // RFC1918: 10.0.0.0/8
    if octets[0] == 10 {
        return true;
    }

    // RFC1918: 172.16.0.0/12
    if octets[0] == 172 && (16..=31).contains(&octets[1]) {
        return true;
    }

    // RFC1918: 192.168.0.0/16
    if octets[0] == 192 && octets[1] == 168 {
        return true;
    }

    // Link-local 169.254.0.0/16 (includes cloud metadata 169.254.169.254)
    if octets[0] == 169 && octets[1] == 254 {
        return true;
    }

    // RFC6598 Carrier-grade NAT: 100.64.0.0/10
    if octets[0] == 100 && (64..=127).contains(&octets[1]) {
        return true;
    }

    // RFC5737 Documentation: 192.0.2.0/24, 198.51.100.0/24, 203.0.113.0/24
    if (octets[0] == 192 && octets[1] == 0 && octets[2] == 2)
        || (octets[0] == 198 && octets[1] == 51 && octets[2] == 100)
        || (octets[0] == 203 && octets[1] == 0 && octets[2] == 113)
    {
        return true;
    }

    false
}

fn is_blocked_ipv6(ip: Ipv6Addr) -> bool {
    // Loopback ::1
    if ip.is_loopback() {
        return true;
    }

    // Unspecified ::
    if ip.is_unspecified() {
        return true;
    }

    // Link-local fe80::/10
    let segments = ip.segments();
    if segments[0] & 0xffc0 == 0xfe80 {
        return true;
    }

    // Unique local fc00::/7
    if segments[0] & 0xfe00 == 0xfc00 {
        return true;
    }

    // IPv4-mapped IPv6 (::ffff:x.x.x.x) — check the embedded v4
    if let Some(v4) = ip.to_ipv4_mapped() {
        return is_blocked_ipv4(v4);
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- Positive: valid public URLs ---

    #[test]
    fn accepts_https_public_url() {
        assert!(validate_safe_url("https://mcp.example.com/v1/mcp").is_ok());
    }

    #[test]
    fn accepts_http_public_url() {
        assert!(validate_safe_url("http://mcp.example.com/v1/mcp").is_ok());
    }

    #[test]
    fn accepts_url_with_port() {
        assert!(validate_safe_url("https://mcp.example.com:8443/v1/mcp").is_ok());
    }

    #[test]
    fn accepts_url_with_path_and_query() {
        assert!(validate_safe_url("https://api.example.com/mcp?key=val").is_ok());
    }

    // --- Negative: blocked schemes ---

    #[test]
    fn rejects_ftp_scheme() {
        let err = validate_safe_url("ftp://evil.com/file").unwrap_err();
        assert!(matches!(err, UrlValidationError::DisallowedScheme(_)));
    }

    #[test]
    fn rejects_file_scheme() {
        let err = validate_safe_url("file:///etc/passwd").unwrap_err();
        assert!(matches!(err, UrlValidationError::DisallowedScheme(_)));
    }

    #[test]
    fn rejects_javascript_scheme() {
        let err = validate_safe_url("javascript:alert(1)").unwrap_err();
        // url crate may parse this as opaque or as scheme error
        assert!(
            matches!(err, UrlValidationError::DisallowedScheme(_))
                || matches!(err, UrlValidationError::MissingHostname)
        );
    }

    #[test]
    fn rejects_data_scheme() {
        let err = validate_safe_url("data:text/plain,hello").unwrap_err();
        assert!(
            matches!(err, UrlValidationError::DisallowedScheme(_))
                || matches!(err, UrlValidationError::MissingHostname)
        );
    }

    // --- Negative: invalid URLs ---

    #[test]
    fn rejects_empty_string() {
        assert!(validate_safe_url("").is_err());
    }

    #[test]
    fn rejects_not_a_url() {
        assert!(validate_safe_url("not a url").is_err());
    }

    // --- Negative: localhost ---

    #[test]
    fn rejects_localhost() {
        let err = validate_safe_url("http://localhost/path").unwrap_err();
        assert!(matches!(err, UrlValidationError::BlockedHost(_)));
    }

    #[test]
    fn rejects_localhost_with_port() {
        let err = validate_safe_url("http://localhost:8080/path").unwrap_err();
        assert!(matches!(err, UrlValidationError::BlockedHost(_)));
    }

    #[test]
    fn rejects_subdomain_of_localhost() {
        let err = validate_safe_url("http://foo.localhost/path").unwrap_err();
        assert!(matches!(err, UrlValidationError::BlockedHost(_)));
    }

    // --- Negative: loopback IPs ---

    #[test]
    fn rejects_127_0_0_1() {
        let err = validate_safe_url("http://127.0.0.1/path").unwrap_err();
        assert!(matches!(err, UrlValidationError::BlockedHost(_)));
    }

    #[test]
    fn rejects_127_x_x_x() {
        let err = validate_safe_url("http://127.255.0.1/path").unwrap_err();
        assert!(matches!(err, UrlValidationError::BlockedHost(_)));
    }

    #[test]
    fn rejects_ipv6_loopback() {
        let err = validate_safe_url("http://[::1]/path").unwrap_err();
        assert!(matches!(err, UrlValidationError::BlockedHost(_)));
    }

    // --- Negative: private RFC1918 ---

    #[test]
    fn rejects_10_x() {
        let err = validate_safe_url("http://10.0.0.1/path").unwrap_err();
        assert!(matches!(err, UrlValidationError::BlockedHost(_)));
    }

    #[test]
    fn rejects_172_16_x() {
        let err = validate_safe_url("http://172.16.0.1/path").unwrap_err();
        assert!(matches!(err, UrlValidationError::BlockedHost(_)));
    }

    #[test]
    fn rejects_172_31_x() {
        let err = validate_safe_url("http://172.31.255.255/path").unwrap_err();
        assert!(matches!(err, UrlValidationError::BlockedHost(_)));
    }

    #[test]
    fn accepts_172_32_x() {
        // 172.32.x.x is NOT private
        assert!(validate_safe_url("http://172.32.0.1/path").is_ok());
    }

    #[test]
    fn rejects_192_168_x() {
        let err = validate_safe_url("http://192.168.1.1/path").unwrap_err();
        assert!(matches!(err, UrlValidationError::BlockedHost(_)));
    }

    // --- Negative: link-local / cloud metadata ---

    #[test]
    fn rejects_link_local() {
        let err = validate_safe_url("http://169.254.1.1/path").unwrap_err();
        assert!(matches!(err, UrlValidationError::BlockedHost(_)));
    }

    #[test]
    fn rejects_cloud_metadata_ip() {
        let err = validate_safe_url("http://169.254.169.254/latest/meta-data/").unwrap_err();
        assert!(matches!(err, UrlValidationError::BlockedHost(_)));
    }

    #[test]
    fn rejects_gce_metadata_hostname() {
        let err =
            validate_safe_url("http://metadata.google.internal/computeMetadata/v1/").unwrap_err();
        assert!(matches!(err, UrlValidationError::BlockedHost(_)));
    }

    // --- Negative: 0.0.0.0 ---

    #[test]
    fn rejects_unspecified_v4() {
        let err = validate_safe_url("http://0.0.0.0/path").unwrap_err();
        assert!(matches!(err, UrlValidationError::BlockedHost(_)));
    }

    // --- Negative: IPv6 special ---

    #[test]
    fn rejects_ipv6_unspecified() {
        let err = validate_safe_url("http://[::]/path").unwrap_err();
        assert!(matches!(err, UrlValidationError::BlockedHost(_)));
    }

    #[test]
    fn rejects_ipv6_link_local() {
        let err = validate_safe_url("http://[fe80::1]/path").unwrap_err();
        assert!(matches!(err, UrlValidationError::BlockedHost(_)));
    }

    #[test]
    fn rejects_ipv6_unique_local() {
        let err = validate_safe_url("http://[fd00::1]/path").unwrap_err();
        assert!(matches!(err, UrlValidationError::BlockedHost(_)));
    }

    #[test]
    fn rejects_ipv4_mapped_ipv6_private() {
        let err = validate_safe_url("http://[::ffff:127.0.0.1]/path").unwrap_err();
        assert!(matches!(err, UrlValidationError::BlockedHost(_)));
    }

    #[test]
    fn rejects_ipv4_mapped_ipv6_metadata() {
        let err =
            validate_safe_url("http://[::ffff:169.254.169.254]/latest/meta-data/").unwrap_err();
        assert!(matches!(err, UrlValidationError::BlockedHost(_)));
    }

    // --- Negative: carrier-grade NAT ---

    #[test]
    fn rejects_cgnat() {
        let err = validate_safe_url("http://100.64.0.1/path").unwrap_err();
        assert!(matches!(err, UrlValidationError::BlockedHost(_)));
    }

    // --- Display ---

    #[test]
    fn error_display_messages() {
        assert!(
            UrlValidationError::BlockedHost("localhost".into())
                .to_string()
                .contains("private/internal")
        );
        assert!(
            UrlValidationError::DisallowedScheme("ftp".into())
                .to_string()
                .contains("http or https")
        );
    }

    // --- Operator CIDR allowlist (EVERRUNS_SSRF_ALLOW_CIDRS) ---

    fn cidrs(raw: &str) -> Vec<Cidr> {
        parse_cidr_list(raw)
    }

    #[test]
    fn allowlist_exempts_matching_private_ipv4() {
        let allow = cidrs("10.42.0.0/16");
        assert!(!is_blocked_ip_with_allowlist(
            "10.42.7.1".parse().unwrap(),
            &allow
        ));
        // Other private ranges stay blocked.
        assert!(is_blocked_ip_with_allowlist(
            "10.43.0.1".parse().unwrap(),
            &allow
        ));
        assert!(is_blocked_ip_with_allowlist(
            "192.168.1.1".parse().unwrap(),
            &allow
        ));
    }

    #[test]
    fn allowlist_does_not_affect_public_ips() {
        let allow = cidrs("10.0.0.0/8");
        assert!(!is_blocked_ip_with_allowlist(
            "1.1.1.1".parse().unwrap(),
            &allow
        ));
    }

    #[test]
    fn allowlist_supports_multiple_entries_and_ipv6() {
        let allow = cidrs("10.42.0.0/16, fd00::/8");
        assert!(!is_blocked_ip_with_allowlist(
            "fd00::1".parse().unwrap(),
            &allow
        ));
        assert!(is_blocked_ip_with_allowlist(
            "fe80::1".parse().unwrap(),
            &allow
        ));
    }

    #[test]
    fn allowlist_matches_ipv4_mapped_ipv6_against_v4_cidr() {
        let allow = cidrs("10.0.0.0/8");
        assert!(!is_blocked_ip_with_allowlist(
            "::ffff:10.0.0.1".parse().unwrap(),
            &allow
        ));
    }

    #[test]
    fn allowlist_ignores_invalid_entries() {
        let allow = cidrs("not-a-cidr, 10.0.0.0/33, 10.0.0.0, ,10.42.0.0/16");
        assert_eq!(allow.len(), 1);
        assert_eq!(allow[0].to_string(), "10.42.0.0/16");
    }

    #[test]
    fn allowlist_prefix_zero_matches_family_wide() {
        let allow = cidrs("0.0.0.0/0");
        assert!(!is_blocked_ip_with_allowlist(
            "10.0.0.1".parse().unwrap(),
            &allow
        ));
        // /0 v4 must not match v6 addresses.
        assert!(is_blocked_ip_with_allowlist(
            "fd00::1".parse().unwrap(),
            &allow
        ));
    }

    #[test]
    fn allowlist_never_unblocks_localhost_hostname() {
        // Hostname-pattern blocks are independent of the IP allowlist.
        assert!(is_blocked_host("localhost"));
        assert!(is_blocked_host("metadata.google.internal"));
    }

    #[test]
    fn empty_allowlist_blocks_all_private() {
        assert!(is_blocked_ip_with_allowlist(
            "10.0.0.1".parse().unwrap(),
            &[]
        ));
    }

    // --- validate_url_dns_pinned: static pre-check path (IP literals) ---

    #[tokio::test]
    async fn dns_pinned_rejects_private_ip_literal() {
        let result = validate_url_dns_pinned("http://10.0.0.1/mcp").await;
        assert!(matches!(result, Err(UrlValidationError::BlockedHost(_))));
    }

    #[tokio::test]
    async fn dns_pinned_rejects_loopback_ip_literal() {
        let result = validate_url_dns_pinned("http://127.0.0.1/mcp").await;
        assert!(matches!(result, Err(UrlValidationError::BlockedHost(_))));
    }

    #[tokio::test]
    async fn dns_pinned_rejects_metadata_ip_literal() {
        let result = validate_url_dns_pinned("http://169.254.169.254/latest/meta-data/").await;
        assert!(matches!(result, Err(UrlValidationError::BlockedHost(_))));
    }

    #[tokio::test]
    async fn dns_pinned_rejects_localhost_hostname() {
        // "localhost" is caught by the static pre-check; resolver is never called.
        let result = validate_url_dns_pinned("http://localhost:8080/mcp").await;
        assert!(matches!(result, Err(UrlValidationError::BlockedHost(_))));
    }

    #[tokio::test]
    async fn dns_pinned_rejects_bad_scheme() {
        let result = validate_url_dns_pinned("ftp://example.com/mcp").await;
        assert!(matches!(
            result,
            Err(UrlValidationError::DisallowedScheme(_))
        ));
    }

    // --- validate_url_with_resolver: DNS resolution path (injectable resolver) ---

    // Simulates DNS rebinding: public URL resolves to a private IP.
    async fn private_ip_resolver(
        _host: String,
        _port: u16,
    ) -> Result<Vec<SocketAddr>, std::io::Error> {
        Ok(vec!["10.0.0.1:80".parse().unwrap()])
    }

    // Simulates a well-behaved response: public URL resolves to a public IP.
    async fn public_ip_resolver(
        _host: String,
        _port: u16,
    ) -> Result<Vec<SocketAddr>, std::io::Error> {
        Ok(vec!["1.1.1.1:443".parse().unwrap()])
    }

    // Simulates a DNS failure / timeout.
    async fn failing_resolver(
        _host: String,
        _port: u16,
    ) -> Result<Vec<SocketAddr>, std::io::Error> {
        Err(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "DNS lookup timed out",
        ))
    }

    // Simulates empty DNS response (NXDOMAIN / no records).
    async fn empty_resolver(_host: String, _port: u16) -> Result<Vec<SocketAddr>, std::io::Error> {
        Ok(vec![])
    }

    #[tokio::test]
    async fn dns_resolver_blocks_hostname_resolving_to_private_ip() {
        // Simulates the DNS rebinding attack: public URL resolves to private IP.
        let result =
            validate_url_with_resolver("http://evil.example.com/mcp", private_ip_resolver).await;
        assert!(
            matches!(result, Err(UrlValidationError::BlockedHost(_))),
            "expected BlockedHost, got {result:?}"
        );
    }

    #[tokio::test]
    async fn dns_resolver_allows_hostname_resolving_to_public_ip() {
        let (url, addrs) =
            validate_url_with_resolver("https://mcp.example.com/v1/mcp", public_ip_resolver)
                .await
                .expect("should succeed");
        assert_eq!(url.host_str(), Some("mcp.example.com"));
        // Caller pins the connection to these addrs via resolve_to_addrs.
        assert_eq!(addrs.len(), 1);
    }

    #[tokio::test]
    async fn dns_resolver_blocks_on_lookup_failure() {
        let result = validate_url_with_resolver("http://example.com/mcp", failing_resolver).await;
        assert!(matches!(result, Err(UrlValidationError::BlockedHost(_))));
    }

    #[tokio::test]
    async fn dns_resolver_blocks_empty_response() {
        let result = validate_url_with_resolver("http://example.com/mcp", empty_resolver).await;
        assert!(matches!(result, Err(UrlValidationError::BlockedHost(_))));
    }

    #[tokio::test]
    async fn dns_resolver_returns_addrs_for_connection_pinning() {
        let (_url, addrs) =
            validate_url_with_resolver("https://mcp.example.com/v1/mcp", public_ip_resolver)
                .await
                .unwrap();
        assert!(!addrs.is_empty());
    }
}

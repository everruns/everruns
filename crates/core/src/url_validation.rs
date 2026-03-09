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

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use url::Url;

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

/// Check if an IP address is in a blocked range.
fn is_blocked_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_blocked_ipv4(v4),
        IpAddr::V6(v6) => is_blocked_ipv6(v6),
    }
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
}

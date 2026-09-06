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

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, ToSocketAddrs};
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

/// Default resolver used in production. The system resolver is blocking, so it
/// runs on Tokio's blocking pool and is bounded by the same five-second timeout.
/// This keeps the provider contract free of Tokio's networking feature.
async fn default_dns_resolve(host: String, port: u16) -> Result<Vec<SocketAddr>, std::io::Error> {
    let lookup = tokio::task::spawn_blocking(move || {
        (host.as_str(), port)
            .to_socket_addrs()
            .map(|addresses| addresses.collect())
    });
    tokio::time::timeout(DNS_LOOKUP_TIMEOUT, lookup)
        .await
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "DNS lookup timed out"))?
        .map_err(|error| std::io::Error::other(format!("DNS lookup task failed: {error}")))?
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
pub fn is_blocked_ip(ip: IpAddr) -> bool {
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

    #[test]
    fn public_urls_preserve_scheme_port_path_and_query() {
        for input in [
            "https://mcp.example.com/v1/mcp",
            "http://mcp.example.com/v1/mcp",
            "https://mcp.example.com:8443/v1/mcp",
            "https://api.example.com/mcp?key=val",
            "http://172.32.0.1/path",
            "http://100.63.255.255/path",
            "http://100.128.0.0/path",
            "https://[2606:4700:4700::1111]/dns-query",
        ] {
            assert_eq!(validate_safe_url(input).unwrap().as_str(), input);
        }
    }

    #[test]
    fn invalid_urls_and_disallowed_schemes_have_distinct_errors() {
        for (input, scheme) in [
            ("ftp://evil.com/file", "ftp"),
            ("file:///etc/passwd", "file"),
            ("javascript:alert(1)", "javascript"),
            ("data:text/plain,hello", "data"),
        ] {
            assert_eq!(
                validate_safe_url(input),
                Err(UrlValidationError::DisallowedScheme(scheme.into()))
            );
        }
        for input in ["", "not a url"] {
            assert!(matches!(
                validate_safe_url(input),
                Err(UrlValidationError::InvalidUrl(_))
            ));
        }
        assert_eq!(
            UrlValidationError::BlockedHost("localhost".into()).to_string(),
            "Blocked host: localhost (private/internal address)"
        );
        assert_eq!(
            UrlValidationError::DisallowedScheme("ftp".into()).to_string(),
            "Disallowed URL scheme: ftp (must be http or https)"
        );
    }

    #[test]
    fn blocked_hosts_cover_all_private_families_and_normalized_spellings() {
        for (input, host) in [
            ("http://localhost/path", "localhost"),
            ("http://localhost:8080/path", "localhost"),
            ("http://foo.localhost/path", "foo.localhost"),
            ("http://LOCALHOST./path", "localhost."),
            ("http://foo.localhost./path", "foo.localhost."),
            ("http://127.0.0.1/path", "127.0.0.1"),
            ("http://127.255.0.1/path", "127.255.0.1"),
            ("http://2130706433/path", "127.0.0.1"),
            ("http://0x7f000001/path", "127.0.0.1"),
            ("http://[::1]/path", "[::1]"),
            ("http://10.0.0.1/path", "10.0.0.1"),
            ("http://172.16.0.1/path", "172.16.0.1"),
            ("http://172.31.255.255/path", "172.31.255.255"),
            ("http://192.168.1.1/path", "192.168.1.1"),
            ("http://169.254.1.1/path", "169.254.1.1"),
            (
                "http://169.254.169.254/latest/meta-data/",
                "169.254.169.254",
            ),
            (
                "http://metadata.google.internal/computeMetadata/v1/",
                "metadata.google.internal",
            ),
            (
                "http://metadata.google.internal./path",
                "metadata.google.internal.",
            ),
            ("http://0.0.0.0/path", "0.0.0.0"),
            ("http://[::]/path", "[::]"),
            ("http://[fe80::1]/path", "[fe80::1]"),
            ("http://[fd00::1]/path", "[fd00::1]"),
            ("http://[::ffff:127.0.0.1]/path", "[::ffff:7f00:1]"),
            (
                "http://[::ffff:169.254.169.254]/latest/meta-data/",
                "[::ffff:a9fe:a9fe]",
            ),
            ("http://100.64.0.1/path", "100.64.0.1"),
            ("http://100.127.255.255/path", "100.127.255.255"),
            ("http://192.0.2.1/path", "192.0.2.1"),
            ("http://198.51.100.1/path", "198.51.100.1"),
            ("http://203.0.113.1/path", "203.0.113.1"),
        ] {
            assert_eq!(
                validate_safe_url(input),
                Err(UrlValidationError::BlockedHost(host.into())),
                "{input}"
            );
        }
    }

    #[test]
    fn env_cidr_allowlist_does_not_unblock_private_ips() {
        if std::env::var_os("EVERRUNS_URL_VALIDATION_ALLOWLIST_TEST_CHILD").is_some() {
            assert!(is_blocked_ip("10.96.0.42".parse().unwrap()));
            assert!(is_blocked_ip("::ffff:10.96.0.42".parse().unwrap()));
            return;
        }
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "url_validation::tests::env_cidr_allowlist_does_not_unblock_private_ips",
                "--nocapture",
            ])
            .env("EVERRUNS_URL_VALIDATION_ALLOWLIST_TEST_CHILD", "1")
            .env("EVERRUNS_SSRF_ALLOW_CIDRS", "10.96.0.0/12")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
    }

    #[tokio::test]
    async fn static_rejection_and_public_literals_never_resolve_dns() {
        for (input, expected) in [
            (
                "http://10.0.0.1/mcp",
                UrlValidationError::BlockedHost("10.0.0.1".into()),
            ),
            (
                "http://127.0.0.1/mcp",
                UrlValidationError::BlockedHost("127.0.0.1".into()),
            ),
            (
                "http://169.254.169.254/latest/meta-data/",
                UrlValidationError::BlockedHost("169.254.169.254".into()),
            ),
            (
                "http://localhost:8080/mcp",
                UrlValidationError::BlockedHost("localhost".into()),
            ),
            (
                "ftp://example.com/mcp",
                UrlValidationError::DisallowedScheme("ftp".into()),
            ),
        ] {
            assert_eq!(
                validate_url_with_resolver(input, |_, _| async {
                    panic!("static failure must not resolve")
                })
                .await,
                Err(expected.clone())
            );
            assert_eq!(validate_url_dns_pinned(input).await, Err(expected));
        }
        for input in ["https://1.1.1.1/mcp", "https://[2606:4700:4700::1111]/mcp"] {
            let (url, addresses) = validate_url_with_resolver(input, |_, _| async {
                panic!("literal must not resolve")
            })
            .await
            .unwrap();
            assert_eq!(url.as_str(), input);
            assert!(addresses.is_empty());
        }
    }

    #[tokio::test]
    async fn dns_pinning_preserves_exact_addresses_and_requested_port() {
        for (input, port) in [
            ("https://mcp.example.com/v1/mcp", 443),
            ("http://mcp.example.com/v1/mcp", 80),
            ("https://mcp.example.com:8443/v1/mcp", 8443),
        ] {
            let (url, addresses) =
                validate_url_with_resolver(input, |host, actual_port| async move {
                    assert_eq!(host, "mcp.example.com");
                    assert_eq!(actual_port, port);
                    Ok(vec![
                        SocketAddr::new("1.1.1.1".parse().unwrap(), port),
                        SocketAddr::new("2606:4700:4700::1111".parse().unwrap(), port),
                    ])
                })
                .await
                .unwrap();
            assert_eq!(url.as_str(), input);
            assert_eq!(
                addresses,
                [
                    SocketAddr::new("1.1.1.1".parse().unwrap(), port),
                    SocketAddr::new("2606:4700:4700::1111".parse().unwrap(), port)
                ]
            );
        }
    }

    #[tokio::test]
    async fn any_private_dns_answer_rejects_the_entire_resolution() {
        for blocked in [
            "10.0.0.1",
            "127.0.0.1",
            "169.254.169.254",
            "::1",
            "::ffff:10.0.0.1",
        ] {
            for first in [true, false] {
                let mut answers = vec![
                    SocketAddr::new("1.1.1.1".parse().unwrap(), 80),
                    SocketAddr::new(blocked.parse().unwrap(), 80),
                ];
                if first {
                    answers.reverse();
                }
                let error =
                    validate_url_with_resolver("http://evil.example.com/mcp", move |_, _| {
                        let answers = answers.clone();
                        async move { Ok(answers) }
                    })
                    .await
                    .unwrap_err();
                assert_eq!(
                    error,
                    UrlValidationError::BlockedHost(format!(
                        "evil.example.com resolves to blocked address {}",
                        blocked.parse::<IpAddr>().unwrap()
                    ))
                );
            }
        }
    }

    #[tokio::test]
    async fn empty_and_failed_dns_lookups_fail_closed() {
        for fail in [false, true] {
            let result =
                validate_url_with_resolver("http://example.com/mcp", move |_, _| async move {
                    if fail {
                        Err(std::io::Error::new(
                            std::io::ErrorKind::TimedOut,
                            "DNS lookup timed out",
                        ))
                    } else {
                        Ok(vec![])
                    }
                })
                .await;
            assert_eq!(
                result,
                Err(UrlValidationError::BlockedHost("example.com".into()))
            );
        }
    }
}

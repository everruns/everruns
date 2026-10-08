//! Redirect URI policy for the MCP OAuth server: which URIs a client may
//! register, and when a requested URI matches a registered one.

/// Validate a registered redirect URI for an MCP OAuth client.
///
/// Policy (per spec/threat-model OAuth open-redirect prevention):
/// - Allow `https://` to any host (with absolute URL form, no fragment).
/// - Allow `http://` only for native loopback callbacks: any IPv4 address in
///   `127.0.0.0/8`, the IPv6 `[::1]` address, and the literal `localhost`
///   host. Any port is fine.
/// - Reject every other scheme — explicitly including `javascript:`, `data:`,
///   `file:`, `vbscript:`, custom app schemes, and unparseable/relative URIs.
/// - Reject URIs with a fragment component (RFC 6749 §3.1.2).
pub(super) fn validate_redirect_uri(raw: &str) -> Result<(), &'static str> {
    let parsed = url::Url::parse(raw).map_err(|_| "redirect_uri must be an absolute URL")?;
    if parsed.fragment().is_some() {
        return Err("redirect_uri must not contain a fragment");
    }
    match parsed.scheme() {
        "https" => {
            if parsed.host().is_none() {
                return Err("https redirect_uri must include a host");
            }
            Ok(())
        }
        "http" => match parsed.host() {
            Some(url::Host::Domain("localhost")) => Ok(()),
            Some(url::Host::Ipv4(ip)) if ip.is_loopback() => Ok(()),
            Some(url::Host::Ipv6(ip)) if ip.is_loopback() => Ok(()),
            _ => Err("http redirect_uri is only allowed for loopback hosts"),
        },
        _ => Err("redirect_uri scheme is not allowed"),
    }
}

/// Whether a redirect URI is an `http://` loopback callback — the shape only a
/// native client can serve.
pub(super) fn is_loopback_http_uri(raw: &str) -> bool {
    let Ok(parsed) = url::Url::parse(raw) else {
        return false;
    };
    if parsed.scheme() != "http" {
        return false;
    }
    match parsed.host() {
        Some(url::Host::Domain("localhost")) => true,
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        _ => false,
    }
}

/// Whether a requested redirect URI matches a registered one.
///
/// Exact string match, except for native loopback callbacks (RFC 8252 §7.3):
/// the port is ignored, because native clients bind an ephemeral port at
/// authorization time, and `localhost`, `127.0.0.0/8` and `[::1]` are treated
/// as one host, because proxies and client libraries rewrite one to another
/// (seen breaking Codex and Claude Desktop against other servers). Scheme,
/// path and query must still match exactly, and only `http://` loopback URIs
/// get this leniency, so a web callback is never widened.
pub(super) fn redirect_uri_matches(registered: &str, requested: &str) -> bool {
    if registered == requested {
        return true;
    }
    if !is_loopback_http_uri(registered) || !is_loopback_http_uri(requested) {
        return false;
    }
    let (Ok(a), Ok(b)) = (url::Url::parse(registered), url::Url::parse(requested)) else {
        return false;
    };
    a.path() == b.path()
        && a.query() == b.query()
        && a.username() == b.username()
        && a.password() == b.password()
        && a.fragment().is_none()
        && b.fragment().is_none()
}

pub(super) fn redirect_uri_registered(registered_uris: &[String], requested: &str) -> bool {
    registered_uris
        .iter()
        .any(|registered| redirect_uri_matches(registered, requested))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_redirect_uri_accepts_safe_schemes() {
        for uri in [
            "https://example.com/cb",
            "https://example.com:8443/cb?next=1",
            "http://localhost/cb",
            "http://localhost:9999/cb",
            "http://127.0.0.1:9999/cb",
            "http://[::1]:9999/cb",
        ] {
            assert!(
                validate_redirect_uri(uri).is_ok(),
                "expected {uri} to be accepted",
            );
        }
    }

    #[test]
    fn test_validate_redirect_uri_rejects_unsafe_schemes() {
        for uri in [
            "javascript:alert(1)",
            "data:text/html,<script>alert(1)</script>",
            "file:///tmp/cb",
            "vbscript:msgbox(1)",
            "myapp://callback",
            "http://example.com/cb",     // non-loopback http
            "http://10.0.0.1:9999/cb",   // non-loopback IPv4
            "http://[2001:db8::1]/cb",   // non-loopback IPv6
            "http://localhost.evil.com", // suffix attack
            "//example.com/cb",          // protocol-relative
            "/relative",
            "",
            "https://example.com/cb#frag", // fragment forbidden
            "not a url",
        ] {
            assert!(
                validate_redirect_uri(uri).is_err(),
                "expected {uri} to be rejected",
            );
        }
    }

    #[test]
    fn test_loopback_http_uri_detection() {
        assert!(is_loopback_http_uri("http://localhost:8080/cb"));
        assert!(is_loopback_http_uri("http://127.0.0.1:1455/cb"));
        assert!(is_loopback_http_uri("http://[::1]:9000/cb"));
        // https loopback is a normal web callback, not a native one.
        assert!(!is_loopback_http_uri("https://localhost/cb"));
        assert!(!is_loopback_http_uri("http://evil.example/cb"));
        assert!(!is_loopback_http_uri("not a url"));
    }

    #[test]
    fn test_redirect_uri_matches_loopback_ignores_port_and_host_alias() {
        // Native clients register without a port and bind an ephemeral one.
        assert!(redirect_uri_matches(
            "http://127.0.0.1/callback",
            "http://127.0.0.1:53124/callback"
        ));
        // Something in front rewrote 127.0.0.1 to localhost, or the reverse.
        assert!(redirect_uri_matches(
            "http://127.0.0.1:1455/cb",
            "http://localhost:1455/cb"
        ));
        assert!(redirect_uri_matches(
            "http://localhost/cb",
            "http://[::1]:9000/cb"
        ));
        assert!(redirect_uri_registered(
            &[
                "https://app.example.com/cb".into(),
                "http://localhost/cb".into()
            ],
            "http://127.0.0.1:7777/cb"
        ));
    }

    #[test]
    fn test_redirect_uri_matches_stays_strict_elsewhere() {
        // Path and query still have to match.
        assert!(!redirect_uri_matches(
            "http://127.0.0.1/callback",
            "http://127.0.0.1:5000/other"
        ));
        assert!(!redirect_uri_matches(
            "http://127.0.0.1/cb?a=1",
            "http://127.0.0.1/cb?a=2"
        ));
        // Web callbacks get no port or host leniency.
        assert!(!redirect_uri_matches(
            "https://app.example.com/cb",
            "https://app.example.com:8443/cb"
        ));
        assert!(!redirect_uri_matches(
            "https://localhost/cb",
            "https://127.0.0.1/cb"
        ));
        // A loopback registration never admits a remote host or https.
        assert!(!redirect_uri_matches(
            "http://127.0.0.1/cb",
            "http://evil.example/cb"
        ));
        assert!(!redirect_uri_matches(
            "http://127.0.0.1/cb",
            "https://127.0.0.1/cb"
        ));
        assert!(!redirect_uri_registered(&[], "http://127.0.0.1/cb"));
    }
}

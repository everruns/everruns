//! Browserless URL validation helpers.
//!
//! Decision: Reuse everruns-core SSRF validation for all user-provided Browserless
//! URLs so Browserless tools match the threat model and share one policy with MCP
//! and provider base URL validation.
//!
//! Decision: The session network access list is checked at every navigation entry
//! and, when it is non-empty, translated into rejection patterns the Browserless
//! transport can apply to redirects and requests the page discovers. CDP enforces
//! the same predicate directly. Patterns are escaped from the access list; they
//! never interpolate raw policy text into a regular expression.

use everruns_core::network_access::NetworkAccessList;
use everruns_core::tools::ToolExecutionResult;
use serde_json::Value;
use url::Url;

/// Shown when the session allow/deny list rejects a URL the static SSRF check allowed.
pub(crate) const SESSION_NETWORK_DENIED: &str =
    "URL is blocked by the session's network access policy";

/// THREAT[TM-TOOL-015]: Block Browserless navigation to internal/private hosts.
/// Mitigation: Reuse shared URL validation so localhost, RFC1918, link-local,
/// and metadata endpoints are rejected before any Browserless API call.
pub(crate) fn validate_browserless_url(url: &str) -> Result<(), ToolExecutionResult> {
    everruns_contracts::url_validation::validate_safe_url(url)
        .map(|_| ())
        .map_err(|e| ToolExecutionResult::tool_error(format!("URL is blocked by policy: {e}")))
}

/// Documents that never leave the browser. The network ACL does not apply.
pub(crate) fn is_local_browser_url(url: &str) -> bool {
    let url = url.trim();
    url.is_empty()
        || url.starts_with("about:")
        || url.starts_with("data:")
        || url.starts_with("blob:")
}

#[cfg(test)] // The canonical predicate the transport patterns are tested against.
/// Static SSRF policy plus the session network ACL.
///
/// Local browser documents are allowed. A missing or empty access list adds no
/// extra restriction beyond the static check.
pub(crate) fn browser_request_allowed(access: Option<&NetworkAccessList>, url: &str) -> bool {
    if is_local_browser_url(url) {
        return true;
    }
    if validate_browserless_url(url).is_err() {
        return false;
    }
    access.is_none_or(|access| access.is_url_allowed(url))
}

/// THREAT[TM-TOOL-015][TM-TOOL-053]: Reject a navigation the static policy or the
/// session network ACL does not allow, before any browser request is issued.
pub(crate) fn validate_browserless_navigation(
    access: Option<&NetworkAccessList>,
    url: &str,
) -> Result<(), ToolExecutionResult> {
    validate_browserless_url(url)?;
    if let Some(access) = access
        && !access.is_url_allowed(url)
    {
        return Err(ToolExecutionResult::tool_error(SESSION_NETWORK_DENIED));
    }
    Ok(())
}

/// THREAT[TM-TOOL-015][TM-TOOL-053]: Validate nested navigate actions in interaction steps.
pub(crate) fn validate_interaction_steps(
    access: Option<&NetworkAccessList>,
    steps: &[Value],
) -> Result<(), ToolExecutionResult> {
    for step in steps {
        if step.get("action").and_then(|v| v.as_str()) == Some("navigate") {
            let url = step
                .get("value")
                .and_then(|v| v.as_str())
                .ok_or_else(|| ToolExecutionResult::tool_error("navigate requires value (URL)"))?;
            validate_browserless_navigation(access, url)?;
        }
    }

    Ok(())
}

/// Rejection patterns for Browserless transports that match `url.match(pattern)`.
///
/// Empty when the session access list imposes nothing. Otherwise the patterns
/// reject static-SSRF hosts and anything the access list denies, including
/// redirects and subresources. An allowlist is expressed as one negative
/// lookahead so a URL is rejected unless it matches an allowed pattern.
pub(crate) fn transport_reject_patterns(access: &NetworkAccessList) -> Vec<String> {
    if access.is_empty() {
        return Vec::new();
    }

    let mut patterns = vec![ssrf_reject_regex()];
    for pattern in &access.blocked {
        if let Some(matcher) = acl_match_regex(pattern) {
            patterns.push(format!("^{matcher}"));
        }
    }
    if !access.allowed.is_empty() {
        patterns.push(allowlist_reject_regex(&access.allowed));
    }
    patterns
}

fn allowlist_reject_regex(allowed: &[String]) -> String {
    let alternatives: Vec<String> = allowed
        .iter()
        .filter_map(|pattern| acl_match_regex(pattern))
        .collect();
    if alternatives.is_empty() {
        // A non-empty allowlist that grants nothing must fail closed.
        return r"^https?://".to_string();
    }
    format!(r"^(?!{})https?://", alternatives.join("|"))
}

/// A regular expression matching URLs the access-list pattern permits.
///
/// `None` when the pattern cannot grant or deny anything, matching
/// `NetworkAccessList` (invalid URL prefixes never match).
fn acl_match_regex(pattern: &str) -> Option<String> {
    if is_http_prefix(pattern) {
        return prefix_match_regex(pattern);
    }
    let (wildcard, host) = match pattern.strip_prefix("*.") {
        Some(suffix) => (true, suffix),
        None => (false, pattern),
    };
    if host.is_empty() || host.contains('/') || host.contains(' ') {
        return None;
    }
    Some(host_match_regex(host, wildcard))
}

fn is_http_prefix(pattern: &str) -> bool {
    pattern.split_once("://").is_some_and(|(scheme, _)| {
        scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https")
    })
}

fn host_match_regex(host: &str, wildcard: bool) -> String {
    let labels = if wildcard { r"(?:[^./?#@]+\.)*" } else { "" };
    format!(
        "https?://(?:[^/?#@]+@)?{labels}{}(?::[0-9]+)?(?:[/?#]|$)",
        regex_literal_ci(host),
        labels = labels,
    )
}

/// Match the URL-crate serialization of an HTTP prefix, plus an explicit
/// default port. Browserless sees the raw request URL; `is_url_allowed`
/// compares parsed URLs, which drop `:443` and `:80`.
fn prefix_match_regex(pattern: &str) -> Option<String> {
    let prefix = Url::parse(pattern).ok()?;
    let serialized = prefix.as_str();
    let scheme_end = serialized.find("://")? + 3;
    let scheme = &serialized[..scheme_end - 3];
    let rest = &serialized[scheme_end..];
    let host_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let hostport = &rest[..host_end];
    let tail = &rest[host_end..];
    let (host, port_suffix) = split_hostport(hostport);
    if host.is_empty() {
        return None;
    }
    let port = if port_suffix.is_empty() {
        match scheme {
            "https" => "(?::443)?".to_string(),
            "http" => "(?::80)?".to_string(),
            _ => String::new(),
        }
    } else {
        regex_escape(port_suffix)
    };
    Some(format!(
        "{}://{}{}{}",
        regex_literal_ci(scheme),
        regex_literal_ci(host),
        port,
        regex_escape(tail)
    ))
}

fn split_hostport(hostport: &str) -> (&str, &str) {
    if let Some(end) = hostport.find(']') {
        return hostport.split_at(end + 1);
    }
    match hostport.rfind(':') {
        Some(index) => hostport.split_at(index),
        None => (hostport, ""),
    }
}

fn ssrf_reject_regex() -> String {
    let localhost = format!(r"(?:[A-Za-z0-9-]+\.)*{}\.?", regex_literal_ci("localhost"));
    let metadata = format!(r"{}\.?", regex_literal_ci("metadata.google.internal"));
    let host = [
        localhost.as_str(),
        metadata.as_str(),
        r"127(?:\.[0-9]{1,3}){3}",
        r"10(?:\.[0-9]{1,3}){3}",
        r"192\.168(?:\.[0-9]{1,3}){2}",
        r"172\.(?:1[6-9]|2[0-9]|3[01])(?:\.[0-9]{1,3}){2}",
        r"169\.254(?:\.[0-9]{1,3}){2}",
        r"0\.0\.0\.0",
        r"100\.(?:6[4-9]|[7-9][0-9]|1[01][0-9]|12[0-7])(?:\.[0-9]{1,3}){2}",
        r"192\.0\.2(?:\.[0-9]{1,3})",
        r"198\.51\.100(?:\.[0-9]{1,3})",
        r"203\.0\.113(?:\.[0-9]{1,3})",
        // Integer and hex spellings. The URL parser folds these to dotted form;
        // rejecting every such host fails closed for the few public ones.
        r"[0-9]{4,}",
        r"0[xX][0-9a-fA-F]+",
        r"\[::1\]",
        r"\[::\]",
        r"\[fe[89abAB][0-9a-fA-F]*:[0-9a-fA-F:]*\]",
        r"\[f[cdCD][0-9a-fA-F]*:[0-9a-fA-F:]*\]",
        r"\[::ffff:[0-9a-fA-F:.]+\]",
    ]
    .into_iter()
    .map(ToString::to_string)
    .collect::<Vec<_>>()
    .join("|");
    format!(r"^https?://(?:[^/?#@]+@)?(?:{host})(?::[0-9]+)?(?:[/?#]|$)")
}

fn regex_literal_ci(text: &str) -> String {
    let mut out = String::new();
    for ch in text.chars() {
        if ch.is_ascii_alphabetic() {
            out.push('[');
            out.push(ch.to_ascii_lowercase());
            out.push(ch.to_ascii_uppercase());
            out.push(']');
        } else {
            push_escaped(&mut out, ch);
        }
    }
    out
}

fn regex_escape(text: &str) -> String {
    let mut out = String::new();
    for ch in text.chars() {
        push_escaped(&mut out, ch);
    }
    out
}

fn push_escaped(out: &mut String, ch: char) {
    if matches!(
        ch,
        '.' | '^' | '$' | '*' | '+' | '?' | '(' | ')' | '[' | ']' | '{' | '}' | '|' | '\\' | '/'
    ) {
        out.push('\\');
    }
    out.push(ch);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn patterns_reject(patterns: &[String], url: &str) -> bool {
        patterns.iter().any(|pattern| {
            fancy_regex::Regex::new(pattern)
                .unwrap_or_else(|error| panic!("invalid pattern {pattern}: {error}"))
                .is_match(url)
                .unwrap_or(true)
        })
    }

    fn assert_same_decision(access: &NetworkAccessList, url: &str) {
        let canonical = !browser_request_allowed(Some(access), url);
        let transport = patterns_reject(&transport_reject_patterns(access), url);
        assert_eq!(
            canonical, transport,
            "acl={access:?} url={url} canonical_blocked={canonical} transport_blocked={transport}"
        );
    }

    #[test]
    fn local_documents_skip_the_network_policy() {
        let access = NetworkAccessList::allow_only(["example.com"]);
        assert!(browser_request_allowed(Some(&access), "about:blank"));
        assert!(browser_request_allowed(Some(&access), "data:text/html,hi"));
        assert!(browser_request_allowed(
            Some(&access),
            "blob:https://example.com/id"
        ));
        assert!(browser_request_allowed(None, "https://example.com/"));
        assert!(!browser_request_allowed(
            Some(&access),
            "https://evil.test/"
        ));
    }

    #[test]
    fn transport_patterns_match_the_session_policy() {
        let policies = [
            NetworkAccessList::allow_only(["example.com"]),
            NetworkAccessList::allow_only(["*.example.com"]),
            NetworkAccessList::allow_only(["https://example.com/api/"]),
            NetworkAccessList::allow_only(["https://example.com"]),
            NetworkAccessList::block(["evil.test"]),
            NetworkAccessList::block(["*.evil.test"]),
            NetworkAccessList::block(["https://evil.test/admin"]),
            NetworkAccessList {
                allowed: vec!["*.example.com".to_string()],
                blocked: vec!["secret.example.com".to_string()],
            },
            NetworkAccessList::allow_only(["<none>"]),
        ];
        let urls = [
            "https://example.com/",
            "https://example.com/api/v1",
            "https://example.com/api",
            "https://EXAMPLE.com/Path",
            "http://example.com:8443/x",
            "https://example.com:443/api/v1",
            "https://user:pw@example.com/x",
            "https://example.com.evil.test/",
            "https://sub.example.com/",
            "https://a.b.example.com/z",
            "https://evil.test/",
            "https://EVIL.test/a",
            "https://a.evil.test/",
            "https://notevil.test/",
            "https://evil.test/admin/x",
            "https://evil.test/other",
            "http://example.com/api/",
            "https://secret.example.com/",
            "https://ok.example.com/",
            "https://example.com@evil.test/",
            "http://127.0.0.1/path",
            "http://127.0.0.1:8080/path",
            "http://user@10.1.2.3/admin",
            "http://2130706433/path",
            "http://0x7f000001/path",
            "http://[::1]/path",
            "http://[::1]:8080/path",
            "http://localhost/path",
            "http://LOCALHOST:8080/path",
            "http://foo.localhost/path",
            "http://169.254.169.254/latest/meta-data/",
            "http://metadata.google.internal/computeMetadata/v1/",
            "http://172.16.0.1/path",
            "http://172.31.255.255/path",
            "http://172.32.0.1/path",
            "http://192.168.1.1/path",
            "http://100.64.0.1/path",
            "http://100.128.0.0/path",
            "http://8.8.8.8/dns",
            "https://1.1.1.1/mcp",
            "http://[2606:4700:4700::1111]/dns-query",
            "http://[fe80::1]/path",
            "http://[fd00::1]/path",
            "http://[::ffff:127.0.0.1]/path",
            "http://[::ffff:7f00:1]/path",
            "https://example.com/apiv1",
        ];
        for access in &policies {
            assert!(
                !transport_reject_patterns(access).is_empty(),
                "a non-empty acl must arm the transport"
            );
            for url in urls {
                assert_same_decision(access, url);
            }
        }
    }

    #[test]
    fn an_empty_access_list_adds_no_transport_patterns() {
        assert!(transport_reject_patterns(&NetworkAccessList::default()).is_empty());
    }
}

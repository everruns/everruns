// Network access list — controls which hosts/URLs an agent can reach.
//
// Design Decision: Network access is a top-level field on Harness, Agent, and Session
// (not per-capability config) because it's a cross-cutting security concern that applies
// to all network-capable capabilities (web_fetch, future bashkit HTTP, etc.).
//
// Merge semantics (each layer can only narrow, never widen):
// - allowed: intersection (child can only keep or remove entries)
// - blocked: union (child can add more blocks, never remove them)
//
// If no layer sets `allowed`, all hosts are permitted (open by default).
// If any layer sets `allowed`, only those patterns are permitted.
// `blocked` always takes precedence over `allowed`.

use serde::{Deserialize, Serialize};

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

/// Network access list controlling which hosts/URLs an agent session can reach.
///
/// - `allowed`: if non-empty, only URLs matching these patterns are permitted.
/// - `blocked`: URLs matching these patterns are always denied (takes precedence over allowed).
///
/// Pattern format:
/// - `example.com` — exact domain match (any port, any path)
/// - `*.example.com` — domain and all subdomains
/// - `https://example.com/api/` — exact URL prefix (scheme + host + path)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[cfg_attr(
    feature = "openapi",
    schema(example = json!({"allowed": ["*.example.com", "https://api.acme.com/"], "blocked": ["169.254.169.254"]}))
)]
pub struct NetworkAccessList {
    /// Allowed host patterns. If non-empty, only matching URLs are permitted.
    /// An empty list means "no restriction from this layer" (inherit parent).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed: Vec<String>,

    /// Blocked host patterns. Always denied, even if matched by `allowed`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocked: Vec<String>,
}

impl NetworkAccessList {
    /// Create an access list that allows only the given patterns.
    pub fn allow_only(patterns: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            allowed: patterns.into_iter().map(Into::into).collect(),
            blocked: Vec::new(),
        }
    }

    /// Create an access list that blocks the given patterns (everything else allowed).
    pub fn block(patterns: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            allowed: Vec::new(),
            blocked: patterns.into_iter().map(Into::into).collect(),
        }
    }

    /// Returns true if this list imposes no restrictions (no patterns set).
    pub fn is_empty(&self) -> bool {
        self.allowed.is_empty() && self.blocked.is_empty()
    }

    /// Check whether a URL is permitted by this access list.
    ///
    /// Returns `true` if the URL is allowed, `false` if blocked.
    ///
    /// - `blocked` always takes precedence.
    /// - Empty `allowed` list = no restriction (all URLs allowed).
    /// - Non-empty `allowed` list = only matching URLs allowed.
    pub fn is_url_allowed(&self, url: &str) -> bool {
        // Blocked always takes precedence
        if !self.blocked.is_empty() && matches_any_pattern(url, &self.blocked) {
            return false;
        }

        // If no allowed patterns, everything (not blocked) is allowed
        if self.allowed.is_empty() {
            return true;
        }

        // Must match at least one allowed pattern
        matches_any_pattern(url, &self.allowed)
    }
}

/// Merge a parent and child network access list.
///
/// - `allowed`: intersection. If child specifies allowed patterns, only those that
///   also match the parent's allowed list survive. If child is empty, parent is inherited.
/// - `blocked`: union. All blocked patterns from both layers are combined.
pub fn merge_network_access(
    parent: Option<&NetworkAccessList>,
    child: Option<&NetworkAccessList>,
) -> Option<NetworkAccessList> {
    match (parent, child) {
        (None, None) => None,
        (Some(p), None) => Some(p.clone()),
        (None, Some(c)) => Some(c.clone()),
        (Some(parent), Some(child)) => {
            // Blocked: union
            let mut blocked = parent.blocked.clone();
            for pattern in &child.blocked {
                if !blocked.contains(pattern) {
                    blocked.push(pattern.clone());
                }
            }

            // Allowed: intersection semantics
            let mut allowed = if child.allowed.is_empty() {
                // Child doesn't restrict further — inherit parent
                parent.allowed.clone()
            } else if parent.allowed.is_empty() {
                // Parent is open — child narrows
                child.allowed.clone()
            } else {
                // Both have allowlists — keep only child entries that match parent
                child
                    .allowed
                    .iter()
                    .filter(|child_pattern| {
                        parent
                            .allowed
                            .iter()
                            .any(|parent_pattern| pattern_is_subset(child_pattern, parent_pattern))
                    })
                    .cloned()
                    .collect()
            };

            // If both parent and child had non-empty allowed lists but intersection
            // is empty, nothing should be accessible. We use a sentinel pattern that
            // can never match a real URL so `is_url_allowed` returns false for everything.
            if allowed.is_empty() && !parent.allowed.is_empty() && !child.allowed.is_empty() {
                allowed = vec!["<none>".to_string()];
            }

            let result = NetworkAccessList { allowed, blocked };
            if result.is_empty() {
                None
            } else {
                Some(result)
            }
        }
    }
}

/// Check if a URL matches any of the given patterns.
fn matches_any_pattern(url: &str, patterns: &[String]) -> bool {
    let parsed = match url::Url::parse(url) {
        Ok(u) => u,
        Err(_) => return false,
    };

    let host = match parsed.host_str() {
        Some(h) => h.to_lowercase(),
        None => return false,
    };

    for pattern in patterns {
        if pattern_matches_url(pattern, &parsed, &host) {
            return true;
        }
    }
    false
}

/// Whether a pattern names an HTTP URL prefix rather than a domain.
fn is_http_prefix(pattern: &str) -> bool {
    pattern.split_once("://").is_some_and(|(scheme, _)| {
        scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https")
    })
}

/// Check if a single pattern matches a URL.
fn pattern_matches_url(pattern: &str, parsed: &url::Url, host: &str) -> bool {
    if is_http_prefix(pattern) {
        // Invalid prefixes must not become scheme-wide grants (e.g. https://).
        return url::Url::parse(pattern)
            .is_ok_and(|prefix| parsed.as_str().starts_with(prefix.as_str()));
    }
    domain_pattern_matches_host(pattern, host)
}

/// Match a domain pattern against an already-normalized host, never a whole URL.
fn domain_pattern_matches_host(pattern: &str, host: &str) -> bool {
    if let Some(suffix) = pattern.strip_prefix("*.") {
        let suffix_lower = suffix.to_lowercase();
        return host == suffix_lower || host.ends_with(&format!(".{suffix_lower}"));
    }
    host == pattern.to_lowercase()
}

/// Check if a child pattern is a subset of a parent pattern.
///
/// Used during merge to determine if a child's allowed entry is permitted by the parent.
fn pattern_is_subset(child: &str, parent: &str) -> bool {
    // THREAT[TM-AGENT-018]: narrowing must use the same URL normalization as
    // enforcement. Raw dot segments and domain-looking paths are not authority.
    if is_http_prefix(parent) {
        if !is_http_prefix(child) {
            return false;
        }
        let (Ok(child), Ok(parent)) = (url::Url::parse(child), url::Url::parse(parent)) else {
            return false;
        };
        return child.as_str().starts_with(parent.as_str());
    }

    if is_http_prefix(child) {
        let Ok(child) = url::Url::parse(child) else {
            return false;
        };
        return child
            .host_str()
            .is_some_and(|host| domain_pattern_matches_host(parent, &host.to_lowercase()));
    }

    if parent.starts_with("*.") {
        let child_host = child.strip_prefix("*.").unwrap_or(child).to_lowercase();
        return domain_pattern_matches_host(parent, &child_host);
    }

    // An exact domain parent cannot grant a child's subdomain wildcard.
    child.to_lowercase() == parent.to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exact_domain_match() {
        let acl = NetworkAccessList::allow_only(["Example.COM"]);
        assert!(acl.is_url_allowed("https://EXAMPLE.COM:8443/path"));
        assert!(acl.is_url_allowed("https://example.com/path"));
        for url in [
            "https://example.com@outside.test",
            "https://example.com.evil.test",
            "https://notexample.com",
        ] {
            assert!(!acl.is_url_allowed(url), "{url}");
        }
        assert!(acl.is_url_allowed("http://example.com"));
        assert!(!acl.is_url_allowed("https://other.com"));
        assert!(!acl.is_url_allowed("https://sub.example.com"));
    }

    #[test]
    fn test_wildcard_domain_match() {
        let acl = NetworkAccessList::allow_only(["*.example.com"]);
        assert!(acl.is_url_allowed("https://api.example.com/v1"));
        assert!(acl.is_url_allowed("https://example.com/path"));
        assert!(acl.is_url_allowed("https://deep.sub.example.com"));
        for url in [
            "https://other.com",
            "https://notexample.com",
            "https://example.com.evil.test",
            "https://api.example.com@outside.test",
        ] {
            assert!(!acl.is_url_allowed(url), "{url}");
        }
    }

    #[test]
    fn test_url_prefix_match() {
        let acl = NetworkAccessList::allow_only(["https://api.example.com/v1/"]);
        assert!(acl.is_url_allowed("https://api.example.com/v1/users"));
        assert!(acl.is_url_allowed("HTTPS://API.EXAMPLE.COM:443/v1/users"));
        assert!(!acl.is_url_allowed("https://api.example.com/V1/users"));
        assert!(!acl.is_url_allowed("https://api.example.com:8443/v1/users"));
        assert!(!acl.is_url_allowed("https://api.example.com/v2/users"));
        assert!(!acl.is_url_allowed("http://api.example.com/v1/users"));
    }

    #[test]
    fn test_blocked_takes_precedence() {
        let acl = NetworkAccessList {
            allowed: vec!["*.example.com".to_string()],
            blocked: vec!["evil.example.com".to_string()],
        };
        assert!(acl.is_url_allowed("https://api.example.com"));
        assert!(!acl.is_url_allowed("https://evil.example.com"));
    }

    #[test]
    fn test_empty_acl_allows_all() {
        let acl = NetworkAccessList::default();
        assert!(acl.is_url_allowed("https://anything.com"));
    }

    #[test]
    fn test_blocked_only() {
        let acl = NetworkAccessList::block(["evil.com"]);
        assert!(!acl.is_url_allowed("https://evil.com/path"));
        assert!(acl.is_url_allowed("https://good.com"));
    }

    #[test]
    fn merge_absent_layers_preserves_complete_policy() {
        let policy = NetworkAccessList {
            allowed: vec!["*.example.com".into()],
            blocked: vec!["private.example.com".into()],
        };
        for (parent, child, expected) in [
            (None, None, None),
            (Some(&policy), None, Some(policy.clone())),
            (None, Some(&policy), Some(policy.clone())),
        ] {
            assert_eq!(merge_network_access(parent, child), expected);
        }
    }

    #[test]
    fn test_merge_blocked_union() {
        let parent = NetworkAccessList::block(["evil.com"]);
        let child = NetworkAccessList::block(["bad.com", "evil.com"]);
        let result = merge_network_access(Some(&parent), Some(&child)).unwrap();
        assert_eq!(result.blocked, ["evil.com", "bad.com"]);
        assert!(!result.is_url_allowed("https://evil.com"));
        assert!(!result.is_url_allowed("https://bad.com"));
        assert!(result.is_url_allowed("https://good.com"));
    }

    #[test]
    fn test_merge_allowed_intersection() {
        let parent = NetworkAccessList::allow_only(["*.example.com", "*.github.com"]);
        let child = NetworkAccessList::allow_only(["api.example.com", "other.com"]);
        let result = merge_network_access(Some(&parent), Some(&child)).unwrap();
        // api.example.com is subset of *.example.com → kept
        // other.com is not subset of either parent → dropped
        assert_eq!(result.allowed, vec!["api.example.com".to_string()]);
    }

    #[test]
    fn test_merge_empty_intersection_blocks_all() {
        // If parent allows only A and child allows only B (disjoint),
        // intersection is empty → must block everything, not return None (open).
        let parent = NetworkAccessList::allow_only(["parent.com"]);
        let child = NetworkAccessList::allow_only(["child.com"]);
        let result = merge_network_access(Some(&parent), Some(&child)).unwrap();
        for next in [NetworkAccessList::default(), parent.clone(), child.clone()] {
            let next = merge_network_access(Some(&result), Some(&next)).unwrap();
            for url in [
                "https://parent.com",
                "https://child.com",
                "https://anything.com",
            ] {
                assert!(!next.is_url_allowed(url), "later layer reopened {url}");
            }
        }
        assert!(!result.is_url_allowed("https://parent.com"));
        assert!(!result.is_url_allowed("https://child.com"));
        assert!(!result.is_url_allowed("https://anything.com"));
    }

    #[test]
    fn test_merge_child_inherits_parent_allowed() {
        let parent = NetworkAccessList::allow_only(["example.com"]);
        let child = NetworkAccessList::block(["evil.com"]); // no allowed → inherit
        let result = merge_network_access(Some(&parent), Some(&child)).unwrap();
        assert_eq!(result.allowed, vec!["example.com".to_string()]);
        assert_eq!(result.blocked, vec!["evil.com".to_string()]);
    }

    #[test]
    fn serialization_uses_public_fields_and_omits_empty_lists() {
        let wire = serde_json::json!({
            "allowed": ["*.example.com"], "blocked": ["evil.com"]
        });
        let policy: NetworkAccessList = serde_json::from_value(wire.clone()).unwrap();
        assert!(policy.is_url_allowed("https://api.example.com"));
        assert!(!policy.is_url_allowed("https://evil.com"));
        assert_eq!(serde_json::to_value(&policy).unwrap(), wire);
        let open: NetworkAccessList = serde_json::from_value(serde_json::json!({})).unwrap();
        assert!(open.is_url_allowed("https://outside.test"));
        assert_eq!(serde_json::to_value(open).unwrap(), serde_json::json!({}));
    }

    #[test]
    fn test_invalid_url_denied() {
        let acl = NetworkAccessList::allow_only(["example.com"]);
        for url in [
            "not-a-url",
            "https://",
            "data:text/plain,example.com",
            "mailto:me@example.com",
        ] {
            assert!(!acl.is_url_allowed(url), "{url}");
        }
    }
    #[test]
    fn merged_url_prefix_cannot_escape_parent_domain_via_path_or_query() {
        let parent = NetworkAccessList::allow_only(["*.example.com"]);
        for child_pattern in [
            "https://outside.test/path.example.com",
            "https://outside.test/?next=.example.com",
        ] {
            let child = NetworkAccessList::allow_only([child_pattern]);
            let merged = merge_network_access(Some(&parent), Some(&child)).unwrap();
            assert!(!parent.is_url_allowed(child_pattern));
            assert!(
                !merged.is_url_allowed(child_pattern),
                "escaped parent domain: {child_pattern}"
            );
        }
    }

    #[test]
    fn merged_url_prefix_cannot_escape_parent_path_after_normalization() {
        let parent = NetworkAccessList::allow_only(["https://api.example.com/v1/"]);
        for path in ["../admin", "%2e%2e/admin", ".%2e/admin"] {
            let child =
                NetworkAccessList::allow_only([format!("https://api.example.com/v1/{path}")]);
            let merged = merge_network_access(Some(&parent), Some(&child)).unwrap();
            assert!(!parent.is_url_allowed("https://api.example.com/admin"));
            assert!(
                !merged.is_url_allowed("https://api.example.com/admin"),
                "{path}"
            );
        }
    }

    #[test]
    fn malformed_url_allow_prefixes_do_not_grant_scheme_wide_access() {
        for pattern in ["https://", "http://", "https:///"] {
            let policy = NetworkAccessList::allow_only([pattern]);
            assert!(
                !policy.is_url_allowed("https://outside.test/data"),
                "{pattern}"
            );
            assert!(
                !policy.is_url_allowed("http://outside.test/data"),
                "{pattern}"
            );
        }
    }
    #[test]
    fn legitimate_narrowing_preserves_access_and_rejects_siblings() {
        for (parent, child, allowed, denied) in [
            (
                "*.example.com",
                "https://api.example.com/v1/",
                "https://api.example.com/v1/users",
                "https://other.example.com/v1/users",
            ),
            (
                "API.EXAMPLE.COM",
                "https://api.example.com/v1/",
                "https://api.example.com/v1/users",
                "https://api.example.com/v2/users",
            ),
            (
                "HTTPS://API.EXAMPLE.COM:443/v1/",
                "https://api.example.com/v1/./users/",
                "https://api.example.com/v1/users/42",
                "https://api.example.com/v1/admin",
            ),
            (
                "*.example.com",
                "*.sub.example.com",
                "https://deep.sub.example.com",
                "https://other.example.com",
            ),
        ] {
            let parent = NetworkAccessList::allow_only([parent]);
            let child = NetworkAccessList::allow_only([child]);
            let merged = merge_network_access(Some(&parent), Some(&child)).unwrap();
            assert!(parent.is_url_allowed(allowed), "{allowed}");
            assert!(merged.is_url_allowed(allowed), "{allowed}");
            assert!(!merged.is_url_allowed(denied), "{denied}");
        }
        let parent = NetworkAccessList::allow_only(["example.com"]);
        let child = NetworkAccessList::allow_only(["*.example.com"]);
        let merged = merge_network_access(Some(&parent), Some(&child)).unwrap();
        assert!(!merged.is_url_allowed("https://sub.example.com"));
    }
}

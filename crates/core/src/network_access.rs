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

/// Check if a single pattern matches a URL.
fn pattern_matches_url(pattern: &str, parsed: &url::Url, host: &str) -> bool {
    // URL prefix pattern (starts with http:// or https://)
    // Parse the pattern to normalize scheme+host (url::Url lowercases these).
    if pattern.starts_with("http://") || pattern.starts_with("https://") {
        if let Ok(pattern_url) = url::Url::parse(pattern) {
            return parsed.as_str().starts_with(pattern_url.as_str());
        }
        return parsed.as_str().starts_with(pattern);
    }

    // Wildcard domain pattern: *.example.com
    if let Some(suffix) = pattern.strip_prefix("*.") {
        let suffix_lower = suffix.to_lowercase();
        return host == suffix_lower || host.ends_with(&format!(".{suffix_lower}"));
    }

    // Exact domain match
    host == pattern.to_lowercase()
}

/// Check if a child pattern is a subset of a parent pattern.
///
/// Used during merge to determine if a child's allowed entry is permitted by the parent.
fn pattern_is_subset(child: &str, parent: &str) -> bool {
    // URL prefix: child must start with parent prefix
    if parent.starts_with("http://") || parent.starts_with("https://") {
        if child.starts_with("http://") || child.starts_with("https://") {
            return child.starts_with(parent);
        }
        // Domain child vs URL parent: domain could match broader — be conservative, allow
        return false;
    }

    // Wildcard parent *.example.com
    if let Some(parent_suffix) = parent.strip_prefix("*.") {
        let parent_lower = parent_suffix.to_lowercase();
        if let Some(child_suffix) = child.strip_prefix("*.") {
            // *.sub.example.com is subset of *.example.com
            let child_lower = child_suffix.to_lowercase();
            return child_lower == parent_lower
                || child_lower.ends_with(&format!(".{parent_lower}"));
        }
        // exact child domain: api.example.com is subset of *.example.com
        let child_lower = child.to_lowercase();
        return child_lower == parent_lower || child_lower.ends_with(&format!(".{parent_lower}"));
    }

    // Exact parent domain: only exact match is a subset
    child.to_lowercase() == parent.to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exact_domain_match() {
        let acl = NetworkAccessList::allow_only(["example.com"]);
        assert!(acl.is_url_allowed("https://example.com/path"));
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
        assert!(!acl.is_url_allowed("https://other.com"));
    }

    #[test]
    fn test_url_prefix_match() {
        let acl = NetworkAccessList::allow_only(["https://api.example.com/v1/"]);
        assert!(acl.is_url_allowed("https://api.example.com/v1/users"));
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
    fn test_merge_none_none() {
        assert_eq!(merge_network_access(None, None), None);
    }

    #[test]
    fn test_merge_parent_only() {
        let parent = NetworkAccessList::allow_only(["example.com"]);
        let result = merge_network_access(Some(&parent), None);
        assert_eq!(result, Some(parent));
    }

    #[test]
    fn test_merge_child_only() {
        let child = NetworkAccessList::allow_only(["example.com"]);
        let result = merge_network_access(None, Some(&child));
        assert_eq!(result, Some(child));
    }

    #[test]
    fn test_merge_blocked_union() {
        let parent = NetworkAccessList::block(["evil.com"]);
        let child = NetworkAccessList::block(["bad.com"]);
        let result = merge_network_access(Some(&parent), Some(&child)).unwrap();
        assert_eq!(result.blocked.len(), 2);
        assert!(result.blocked.contains(&"evil.com".to_string()));
        assert!(result.blocked.contains(&"bad.com".to_string()));
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
        // Result has a sentinel pattern that never matches real URLs
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
    fn test_case_insensitive_matching() {
        let acl = NetworkAccessList::allow_only(["Example.COM"]);
        assert!(acl.is_url_allowed("https://example.com/path"));
        assert!(acl.is_url_allowed("https://EXAMPLE.COM/path"));
    }

    #[test]
    fn test_serialization_roundtrip() {
        let acl = NetworkAccessList {
            allowed: vec!["*.example.com".to_string()],
            blocked: vec!["evil.com".to_string()],
        };
        let json = serde_json::to_string(&acl).unwrap();
        let parsed: NetworkAccessList = serde_json::from_str(&json).unwrap();
        assert_eq!(acl, parsed);
    }

    #[test]
    fn test_empty_serialization() {
        let acl = NetworkAccessList::default();
        let json = serde_json::to_string(&acl).unwrap();
        assert_eq!(json, "{}");
    }

    #[test]
    fn test_invalid_url_denied() {
        let acl = NetworkAccessList::allow_only(["example.com"]);
        assert!(!acl.is_url_allowed("not-a-url"));
    }
}

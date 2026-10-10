//! Per-organization extension of the system egress allowlist.
//!
//! On a curated deployment (`EVERRUNS_EGRESS_POLICY`), the embedded allowlist
//! blocks writes to an organization's own APIs and MCP servers. A platform
//! user can grant an organization the right to extend it; the org's admins
//! then maintain a short list of extra host patterns, which widen the
//! allowlist for that org's requests only.
//!
//! Decision: the extension only ever widens the *allowlist*. The deny list,
//! the SSRF checks and the per-request `NetworkAccessList` still apply, and an
//! extension is consulted only for a request the policy would otherwise refuse
//! as not allowlisted, so the common path never pays for a lookup. Patterns
//! are validated on write to public hostnames, so a granted org cannot point
//! the deployment at internal addresses or wildcard a whole public suffix.

use crate::runtime::network_access::NetworkAccessList;
use crate::runtime::typed_id::OrgId;
use crate::url_validation::validate_safe_url;
use async_trait::async_trait;

/// Most patterns one organization may add to the allowlist.
pub const MAX_ORG_EGRESS_ALLOWLIST_PATTERNS: usize = 50;

/// Longest single pattern accepted, in bytes.
pub const MAX_ORG_EGRESS_PATTERN_LEN: usize = 512;

/// Resolves an organization's allowlist extension at the egress boundary.
///
/// Implementations return `Ok(None)` when the org has no grant or no
/// patterns, and `Err` when the lookup itself failed. The boundary treats an
/// error as "no extension" (fail closed) and does not cache it, so a transient
/// outage costs the org its extension only while it lasts.
#[async_trait]
pub trait OrgEgressAllowlist: Send + Sync {
    /// The org's granted extension, as an allow-only list.
    async fn extension(&self, org_id: &OrgId) -> Result<Option<NetworkAccessList>, String>;
}

/// Build the enforced extension from stored state: nothing unless granted and
/// non-empty. Revoking a grant keeps the stored patterns but stops this.
pub fn org_egress_extension(granted: bool, patterns: &[String]) -> Option<NetworkAccessList> {
    (granted && !patterns.is_empty()).then(|| NetworkAccessList::allow_only(patterns.to_vec()))
}

/// Second-level labels that, under a two-letter country code, are public
/// suffixes (`co.uk`, `com.au`, `ac.jp`). A wildcard directly over one of them
/// would span unrelated registrants. A heuristic, not the full public suffix
/// list: it covers the common registries without a new dependency.
const COUNTRY_SECOND_LEVELS: &[&str] = &[
    "ac", "co", "com", "edu", "gov", "govt", "ltd", "me", "mil", "ne", "net", "nic", "or", "org",
    "plc", "sch",
];

/// Top-level names that never resolve on the public internet.
const NON_PUBLIC_TLDS: &[&str] = &[
    "arpa",
    "corp",
    "home",
    "internal",
    "intranet",
    "lan",
    "local",
    "localdomain",
    "localhost",
    "private",
];

/// Validate and normalize a whole extension list.
///
/// Blank lines are dropped and duplicates collapsed; the result keeps the
/// caller's order. Fails on the first invalid pattern, naming it.
pub fn validate_org_egress_patterns(patterns: &[String]) -> Result<Vec<String>, String> {
    let mut normalized: Vec<String> = Vec::new();
    for raw in patterns {
        if raw.trim().is_empty() {
            continue;
        }
        let pattern = validate_org_egress_pattern(raw)?;
        if !normalized.contains(&pattern) {
            normalized.push(pattern);
        }
    }
    if normalized.len() > MAX_ORG_EGRESS_ALLOWLIST_PATTERNS {
        return Err(format!(
            "At most {MAX_ORG_EGRESS_ALLOWLIST_PATTERNS} patterns are allowed, got {}",
            normalized.len()
        ));
    }
    Ok(normalized)
}

/// Validate one pattern and return its normalized form.
///
/// Accepts the `NetworkAccessList` syntax (`example.com`, `*.example.com`,
/// `https://example.com/path/`) for public hostnames only.
pub fn validate_org_egress_pattern(raw: &str) -> Result<String, String> {
    let pattern = raw.trim();
    if pattern.is_empty() {
        return Err("Pattern cannot be empty".to_string());
    }
    if pattern.len() > MAX_ORG_EGRESS_PATTERN_LEN {
        return Err(format!(
            "Pattern is longer than {MAX_ORG_EGRESS_PATTERN_LEN} characters"
        ));
    }
    let invalid = |why: &str| Err(format!("Invalid pattern '{pattern}': {why}"));

    if let Some((scheme, _)) = pattern.split_once("://") {
        if !scheme.eq_ignore_ascii_case("https") && !scheme.eq_ignore_ascii_case("http") {
            return invalid("URL patterns must use http or https");
        }
        let url = match validate_safe_url(pattern) {
            Ok(url) => url,
            Err(error) => return invalid(&error.to_string()),
        };
        if !url.username().is_empty() || url.password().is_some() {
            return invalid("URL patterns cannot carry credentials");
        }
        if url.query().is_some() || url.fragment().is_some() {
            return invalid("URL patterns cannot carry a query or fragment");
        }
        let Some(url::Host::Domain(host)) = url.host() else {
            return invalid("use a hostname, not an IP address");
        };
        if let Err(why) = validate_public_hostname(host) {
            return invalid(&why);
        }
        return Ok(url.to_string());
    }

    let lowered = pattern.to_ascii_lowercase();
    let (wildcard, host) = match lowered.strip_prefix("*.") {
        Some(rest) => (true, rest),
        None => (false, lowered.as_str()),
    };
    if host.contains(['*', '/', ':', '@', '?', '#']) || host.chars().any(char::is_whitespace) {
        return invalid("use a hostname, `*.hostname`, or an http(s) URL prefix");
    }
    if let Err(why) = validate_public_hostname(host) {
        return invalid(&why);
    }
    if validate_safe_url(&format!("https://{host}/")).is_err() {
        return invalid("private or internal addresses are not allowed");
    }
    if wildcard {
        let labels: Vec<&str> = host.split('.').collect();
        let over_country_suffix =
            labels.len() == 2 && labels[1].len() == 2 && COUNTRY_SECOND_LEVELS.contains(&labels[0]);
        if labels.len() < 2 || over_country_suffix {
            return invalid("a wildcard must sit under a registered domain, not a public suffix");
        }
    }
    Ok(lowered)
}

/// A public DNS hostname: at least two labels, LDH characters, no IP literal,
/// no reserved non-public top-level name.
fn validate_public_hostname(host: &str) -> Result<(), String> {
    let host = host.trim_end_matches('.');
    if host.is_empty() || host.len() > 253 {
        return Err("hostname is empty or too long".to_string());
    }
    if host.parse::<std::net::IpAddr>().is_ok() || host.starts_with('[') {
        return Err("use a hostname, not an IP address".to_string());
    }
    let labels: Vec<&str> = host.split('.').collect();
    if labels.len() < 2 {
        return Err("use a fully qualified public hostname".to_string());
    }
    for label in &labels {
        let valid = !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        if !valid {
            return Err(format!("'{label}' is not a valid hostname label"));
        }
    }
    let tld = labels[labels.len() - 1].to_ascii_lowercase();
    if tld.chars().all(|c| c.is_ascii_digit()) {
        return Err("use a hostname, not an IP address".to_string());
    }
    if NON_PUBLIC_TLDS.contains(&tld.as_str()) {
        return Err(format!("'.{tld}' is not a public domain"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_public_hostnames_wildcards_and_url_prefixes() {
        for (input, expected) in [
            ("api.acme-corp.com", "api.acme-corp.com"),
            ("  API.Acme-Corp.com ", "api.acme-corp.com"),
            ("*.acme-corp.com", "*.acme-corp.com"),
            ("*.acme.co.uk", "*.acme.co.uk"),
            ("mcp.acme.io", "mcp.acme.io"),
            (
                "https://api.acme-corp.com/v1/",
                "https://api.acme-corp.com/v1/",
            ),
            ("https://API.acme-corp.com", "https://api.acme-corp.com/"),
        ] {
            assert_eq!(
                validate_org_egress_pattern(input).as_deref(),
                Ok(expected),
                "{input}"
            );
        }
    }

    #[test]
    fn rejects_broad_private_and_malformed_patterns() {
        for input in [
            "",
            "*",
            "*.*",
            "*.com",
            "*.co.uk",
            "*.com.au",
            "com",
            "localhost",
            "app.localhost",
            "*.localhost",
            "printer.local",
            "db.internal",
            "metadata.google.internal",
            "10.0.0.1",
            "127.0.0.1",
            "169.254.169.254",
            "[::1]",
            "http://127.0.0.1/",
            "http://10.1.2.3/api/",
            "https://[::1]/",
            "https://203.0.113.9/",
            "http://localhost:8080/",
            "ftp://files.acme.com/",
            "https://user:pass@acme.com/",
            "https://acme.com/?token=x",
            "api.*.acme.com",
            "acme.com/path",
            "acme.com:8443",
            "-bad.acme.com",
            "acme..com",
            "1.2.3",
        ] {
            assert!(
                validate_org_egress_pattern(input).is_err(),
                "{input:?} must be rejected"
            );
        }
    }

    #[test]
    fn list_validation_drops_blanks_dedupes_and_caps() {
        let list = vec![
            "api.acme.com".to_string(),
            "".to_string(),
            "API.acme.com".to_string(),
            "*.acme.io".to_string(),
        ];
        assert_eq!(
            validate_org_egress_patterns(&list),
            Ok(vec!["api.acme.com".to_string(), "*.acme.io".to_string()])
        );

        let too_many: Vec<String> = (0..=MAX_ORG_EGRESS_ALLOWLIST_PATTERNS)
            .map(|i| format!("host{i}.acme.com"))
            .collect();
        assert!(validate_org_egress_patterns(&too_many).is_err());
        assert!(
            validate_org_egress_patterns(&too_many[..MAX_ORG_EGRESS_ALLOWLIST_PATTERNS]).is_ok()
        );

        let error = validate_org_egress_patterns(&["ok.acme.com".into(), "*.com".into()])
            .expect_err("one bad pattern fails the list");
        assert!(error.contains("*.com"), "{error}");
    }

    #[test]
    fn extension_is_enforced_only_when_granted_and_non_empty() {
        let patterns = vec!["api.acme.com".to_string()];
        assert!(org_egress_extension(false, &patterns).is_none());
        assert!(org_egress_extension(true, &[]).is_none());
        let extension = org_egress_extension(true, &patterns).expect("granted");
        assert!(extension.is_url_allowed("https://api.acme.com/v1"));
        assert!(!extension.is_url_allowed("https://other.acme.com/"));
    }
}

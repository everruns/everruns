//! System-wide outbound allowlist ("green list").
//!
//! An optional, host-owned global allowlist of well-known public resources
//! (package registries, source hosting, AI/cloud provider APIs, ...). It is an
//! internal, curated list — not agent/session/user configuration — shipped as
//! an embedded TOML (`system_allowlist.toml`) and grouped by category so it
//! stays manageable.
//!
//! When enabled via `EVERRUNS_SYSTEM_ALLOWLIST_ENABLED`, the egress boundary
//! denies any outbound request whose URL does not match one of the groups, in
//! addition to (and independently of) the per-agent/session
//! [`NetworkAccessList`]. It is disabled by default, so the default behavior is
//! unchanged. See `specs/system-allowlist.md`.

use crate::network_access::NetworkAccessList;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};

/// Environment variable that enables the global system allowlist.
pub const SYSTEM_ALLOWLIST_ENABLED_ENV: &str = "EVERRUNS_SYSTEM_ALLOWLIST_ENABLED";

/// Embedded TOML source of the curated allowlist.
const EMBEDDED_TOML: &str = include_str!("system_allowlist.toml");

#[derive(Debug, Clone, Deserialize)]
struct AllowlistFile {
    #[serde(default)]
    groups: BTreeMap<String, GroupSpec>,
}

#[derive(Debug, Clone, Deserialize)]
struct GroupSpec {
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    allowed: Vec<String>,
}

/// A named category of allowed host patterns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AllowGroup {
    pub name: String,
    pub description: Option<String>,
    pub allowed: Vec<String>,
}

/// Curated, system-wide outbound allowlist.
///
/// Matching reuses [`NetworkAccessList`] semantics: the flattened set of group
/// patterns forms a single non-empty `allowed` list, so only URLs matching at
/// least one pattern are permitted.
#[derive(Debug, Clone)]
pub struct SystemAllowlist {
    groups: Vec<AllowGroup>,
    acl: NetworkAccessList,
}

impl SystemAllowlist {
    /// Parse a TOML document into a `SystemAllowlist`.
    pub fn from_toml(source: &str) -> Result<Self, toml::de::Error> {
        let file: AllowlistFile = toml::from_str(source)?;
        let mut groups = Vec::with_capacity(file.groups.len());
        let mut patterns = Vec::new();
        for (name, spec) in file.groups {
            patterns.extend(spec.allowed.iter().cloned());
            groups.push(AllowGroup {
                name,
                description: spec.description,
                allowed: spec.allowed,
            });
        }
        Ok(Self {
            groups,
            acl: NetworkAccessList::allow_only(patterns),
        })
    }

    /// The curated allowlist embedded in the binary (parsed once and cached).
    pub fn embedded() -> Arc<SystemAllowlist> {
        static EMBEDDED: OnceLock<Arc<SystemAllowlist>> = OnceLock::new();
        EMBEDDED
            .get_or_init(|| {
                Arc::new(
                    SystemAllowlist::from_toml(EMBEDDED_TOML)
                        .expect("embedded system_allowlist.toml is valid"),
                )
            })
            .clone()
    }

    /// Resolve the active allowlist from the environment.
    ///
    /// Returns `Some(embedded)` when `EVERRUNS_SYSTEM_ALLOWLIST_ENABLED` is
    /// `true` or `1`, otherwise `None` (no global enforcement).
    pub fn from_env() -> Option<Arc<SystemAllowlist>> {
        let enabled = std::env::var(SYSTEM_ALLOWLIST_ENABLED_ENV)
            .map(|value| value == "true" || value == "1")
            .unwrap_or(false);
        enabled.then(SystemAllowlist::embedded)
    }

    /// Categories in the allowlist.
    pub fn groups(&self) -> &[AllowGroup] {
        &self.groups
    }

    /// Whether the given URL matches any allowed pattern in any group.
    pub fn is_url_allowed(&self, url: &str) -> bool {
        self.acl.is_url_allowed(url)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_allowlist_parses_and_has_groups() {
        let allowlist = SystemAllowlist::embedded();
        assert!(
            !allowlist.groups().is_empty(),
            "embedded allowlist should define groups"
        );
        // Every group should contribute at least one pattern.
        for group in allowlist.groups() {
            assert!(
                !group.allowed.is_empty(),
                "group {} has no patterns",
                group.name
            );
        }
    }

    #[test]
    fn embedded_allowlist_permits_known_public_resources() {
        let allowlist = SystemAllowlist::embedded();
        for url in [
            "https://registry.npmjs.org/left-pad",
            "https://static.crates.io/crates/serde/serde-1.0.0.crate",
            "https://files.pythonhosted.org/packages/abc.whl",
            "https://api.openai.com/v1/responses",
            "https://api.anthropic.com/v1/messages",
            "https://codeload.github.com/owner/repo/tar.gz/main",
            "https://ghcr.io/v2/owner/image/manifests/latest",
        ] {
            assert!(allowlist.is_url_allowed(url), "should allow {url}");
        }
    }

    #[test]
    fn embedded_allowlist_denies_unlisted_hosts() {
        let allowlist = SystemAllowlist::embedded();
        for url in [
            "https://evil.example.com/payload",
            "http://169.254.169.254/latest/meta-data/",
            "https://random-blog.net/post",
        ] {
            assert!(!allowlist.is_url_allowed(url), "should deny {url}");
        }
    }

    #[test]
    fn from_toml_flattens_group_patterns() {
        let allowlist = SystemAllowlist::from_toml(
            r#"
            [groups.alpha]
            description = "first"
            allowed = ["*.alpha.test"]

            [groups.beta]
            allowed = ["beta.test"]
            "#,
        )
        .expect("valid toml");

        assert_eq!(allowlist.groups().len(), 2);
        assert!(allowlist.is_url_allowed("https://api.alpha.test/x"));
        assert!(allowlist.is_url_allowed("https://beta.test/y"));
        assert!(!allowlist.is_url_allowed("https://gamma.test/z"));
    }
}

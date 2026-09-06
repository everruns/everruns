//! System-wide outbound allowlist ("green list").
//!
//! An optional, host-owned global allowlist of well-known public resources for
//! tenant/agent runtime egress. It is an internal, curated list — not
//! agent/session/user configuration — shipped as an embedded TOML
//! (`system_allowlist.toml`) and grouped by category so it stays manageable.
//!
//! When enabled via `EVERRUNS_SYSTEM_ALLOWLIST_ENABLED`, the egress boundary
//! denies any request routed through `EgressService` whose URL does not match
//! one of the groups, in addition to (and independently of) the
//! per-agent/session [`NetworkAccessList`]. Host-owned services do not use this
//! boundary. It is disabled by default, so the default behavior is unchanged.
//! See `knowledge/operations/system-allowlist.md`.

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
        // Fail closed: an allowlist with no patterns must deny everything. An
        // empty `allowed` list in `NetworkAccessList` means "no restriction"
        // (allow all), so an empty/misconfigured allowlist would otherwise
        // silently disable enforcement. Substitute a sentinel that can never
        // match a real URL, mirroring `merge_network_access`'s `<none>` guard.
        let acl = if patterns.is_empty() {
            NetworkAccessList::allow_only(["<none>"])
        } else {
            NetworkAccessList::allow_only(patterns)
        };
        Ok(Self { groups, acl })
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
    fn embedded_policy_permits_curated_services_and_denies_tenant_controlled_hosts() {
        let allowlist = SystemAllowlist::embedded();
        assert!(!allowlist.groups().is_empty());
        assert!(allowlist.groups().iter().all(|g| !g.allowed.is_empty()));
        for (url, expected) in [
            ("https://registry.npmjs.org/left-pad", true),
            (
                "https://static.crates.io/crates/serde/serde-1.0.0.crate",
                true,
            ),
            ("https://files.pythonhosted.org/packages/abc.whl", true),
            ("https://api.openai.com/v1/responses", true),
            ("https://api.anthropic.com/v1/messages", true),
            ("https://codeload.github.com/owner/repo/tar.gz/main", true),
            ("https://ghcr.io/v2/owner/image/manifests/latest", true),
            (
                "https://gcp-us-central1.turbopuffer.com/v2/namespaces/org_1__kidx_a",
                true,
            ),
            (
                "https://my-resource.openai.azure.com/openai/deployments/gpt-4/chat/completions",
                true,
            ),
            (
                "https://my-resource.services.ai.azure.com/openai/v1/responses",
                true,
            ),
            ("https://evil.example.com/payload", false),
            ("http://169.254.169.254/latest/meta-data/", false),
            ("https://random-blog.net/post", false),
            (
                "https://attacker123.execute-api.us-east-1.amazonaws.com/collect?data=secret",
                false,
            ),
            (
                "https://evil-bucket.s3.us-west-2.amazonaws.com/collect?data=secret",
                false,
            ),
            (
                "https://evil-bucket.s3-website-us-west-2.amazonaws.com/collect?data=secret",
                false,
            ),
            ("https://attacker-org.github.io/collect?data=secret", false),
            (
                "https://evil.z13.web.core.windows.net/collect?data=secret",
                false,
            ),
            ("https://evil.azureedge.net/collect?data=secret", false),
            (
                "https://evil-bucket.nyc3.digitaloceanspaces.com/collect?data=secret",
                false,
            ),
            ("https://api.openai.com.evil.test/", false),
            ("https://api.openai.com@evil.test/", false),
        ] {
            assert_eq!(allowlist.is_url_allowed(url), expected, "{url}");
        }
    }

    #[test]
    fn empty_allowlist_fails_closed() {
        // No groups, empty groups, and groups with no patterns must all deny
        // every URL rather than silently allowing all traffic.
        for source in ["", "[groups.empty]\n", "[groups.empty]\nallowed = []\n"] {
            let allowlist = SystemAllowlist::from_toml(source).expect("valid toml");
            assert!(
                !allowlist.is_url_allowed("https://example.com/"),
                "empty allowlist (source: {source:?}) must deny all URLs"
            );
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

        assert_eq!(
            allowlist.groups(),
            &[
                AllowGroup {
                    name: "alpha".into(),
                    description: Some("first".into()),
                    allowed: vec!["*.alpha.test".into()]
                },
                AllowGroup {
                    name: "beta".into(),
                    description: None,
                    allowed: vec!["beta.test".into()]
                },
            ]
        );
        assert!(SystemAllowlist::from_toml("[groups.bad]\nallowed = 42").is_err());
        assert!(allowlist.is_url_allowed("https://api.alpha.test/x"));
        assert!(allowlist.is_url_allowed("https://beta.test/y"));
        assert!(!allowlist.is_url_allowed("https://gamma.test/z"));
    }
    #[test]
    fn environment_activation_is_exact_and_process_isolated() {
        const CHILD: &str = "EVERRUNS_TEST_ALLOWLIST_EXPECTED";
        if let Ok(expected) = std::env::var(CHILD) {
            assert_eq!(SystemAllowlist::from_env().is_some(), expected == "enabled");
            return;
        }
        for (value, enabled) in [
            (None, false),
            (Some("true"), true),
            (Some("1"), true),
            (Some("false"), false),
            (Some("0"), false),
            (Some("TRUE"), false),
            (Some(" true"), false),
        ] {
            let mut command = std::process::Command::new(std::env::current_exe().unwrap());
            command
                .args([
                    "--exact",
                    "system_allowlist::tests::environment_activation_is_exact_and_process_isolated",
                    "--nocapture",
                ])
                .env(CHILD, if enabled { "enabled" } else { "disabled" })
                .env_remove("EVERRUNS_SYSTEM_ALLOWLIST_ENABLED");
            if let Some(value) = value {
                command.env("EVERRUNS_SYSTEM_ALLOWLIST_ENABLED", value);
            }
            let output = command.output().unwrap();
            assert!(
                output.status.success(),
                "value={value:?}: {} {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
}

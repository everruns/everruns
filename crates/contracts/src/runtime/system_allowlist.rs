//! System-wide outbound allowlist ("green list").
//!
//! An optional, host-owned global allowlist of well-known public resources for
//! tenant/agent runtime egress. It is an internal, curated list — not
//! agent/session/user configuration — shipped as an embedded TOML
//! (`system_allowlist.toml`) and grouped by category so it stays manageable.
//!
//! When a curated mode is active (`EVERRUNS_EGRESS_POLICY`), the egress boundary
//! denies requests routed through `EgressService` that need the allowlist and
//! do not match one of the groups, in addition to (and independently of) the
//! per-agent/session [`NetworkAccessList`]. Host-owned services do not use this
//! boundary. It is disabled by default, so the default behavior is unchanged.
//!
//! [`SystemEgressPolicy`] is what the boundary actually enforces. It pairs the
//! allowlist with a curated deny list and a mode (`EVERRUNS_EGRESS_POLICY`):
//! `curated-all` sends every request through the allowlist, `curated-writes`
//! only requests that can carry data out (a body, a write method, MCP and
//! integration traffic), letting plain reads reach any public host that is not
//! denied. Decision: reads are open because the useful set of readable sites is
//! unbounded, while the abuse an open-signup tenant can do with a read is
//! bounded by the URL length cap, the deny list, and per-org rate limits.

use crate::runtime::network_access::NetworkAccessList;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};

/// Legacy switch: `true`/`1` selects [`EgressPolicyMode::CuratedAll`] when
/// [`EGRESS_POLICY_ENV`] is unset.
pub const SYSTEM_ALLOWLIST_ENABLED_ENV: &str = "EVERRUNS_SYSTEM_ALLOWLIST_ENABLED";

/// Selects the system egress policy mode: `open`, `curated-writes`, or
/// `curated-all`. Takes precedence over [`SYSTEM_ALLOWLIST_ENABLED_ENV`].
pub const EGRESS_POLICY_ENV: &str = "EVERRUNS_EGRESS_POLICY";

/// Longest URL an open read may use. Reads of non-allowlisted hosts can carry
/// data out in the URL itself; the cap bounds that channel per request.
pub const OPEN_READ_MAX_URL_LEN: usize = 2048;

/// Embedded TOML source of the curated allowlist.
const EMBEDDED_TOML: &str = include_str!("system_allowlist.toml");

/// Embedded TOML source of the curated deny list.
const EMBEDDED_DENYLIST_TOML: &str = include_str!("system_denylist.toml");

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

#[derive(Debug, Clone, Deserialize)]
struct DenylistFile {
    #[serde(default)]
    groups: BTreeMap<String, DenyGroupSpec>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct DenyGroupSpec {
    #[serde(default)]
    #[allow(dead_code, reason = "documentation for maintainers")]
    description: Option<String>,
    denied: Vec<String>,
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
    #[expect(
        clippy::expect_used,
        reason = "the embedded allowlist is validated by its tests"
    )]
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

    /// Categories in the allowlist.
    pub fn groups(&self) -> &[AllowGroup] {
        &self.groups
    }

    /// Whether the given URL matches any allowed pattern in any group.
    pub fn is_url_allowed(&self, url: &str) -> bool {
        self.acl.is_url_allowed(url)
    }
}

/// How strictly the system egress policy constrains tenant traffic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EgressPolicyMode {
    /// No system policy. Self-hosted default.
    Open,
    /// Writes need the allowlist; reads may reach any host not denied.
    CuratedWrites,
    /// Every request needs the allowlist.
    CuratedAll,
}

impl EgressPolicyMode {
    /// Parse an [`EGRESS_POLICY_ENV`] value. Exact, lowercase spellings only.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "open" => Some(Self::Open),
            "curated-writes" => Some(Self::CuratedWrites),
            "curated-all" => Some(Self::CuratedAll),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::CuratedWrites => "curated-writes",
            Self::CuratedAll => "curated-all",
        }
    }
}

/// Whether a request can only read, or can carry data to its destination.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EgressAccess {
    Read,
    Write,
}

impl EgressAccess {
    /// `GET`/`HEAD` without a body read; anything else writes.
    pub fn classify(method: &str, has_body: bool) -> Self {
        let method = method.trim();
        if !has_body && (method.eq_ignore_ascii_case("GET") || method.eq_ignore_ascii_case("HEAD"))
        {
            Self::Read
        } else {
            Self::Write
        }
    }
}

/// Why the system egress policy refused a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EgressPolicyDenial {
    /// The host is on the system deny list.
    Denylisted,
    /// The request needs the allowlist and the host is not on it.
    NotAllowlisted,
    /// An open read's URL is longer than [`OPEN_READ_MAX_URL_LEN`].
    UrlTooLong,
    /// An open read addresses a raw IP instead of a hostname.
    IpLiteral,
}

impl EgressPolicyDenial {
    /// Short machine-readable reason for logs.
    pub fn reason(self) -> &'static str {
        match self {
            Self::Denylisted => "denylisted",
            Self::NotAllowlisted => "not_allowlisted",
            Self::UrlTooLong => "url_too_long",
            Self::IpLiteral => "ip_literal",
        }
    }

    /// Message a tool can show the model, naming what to do differently.
    pub fn message(self, url: &str) -> String {
        match self {
            Self::Denylisted => format!(
                "Endpoint blocked by system policy: {url} is a request-capture, tunnel, or \
                 out-of-band testing service, which this deployment never contacts."
            ),
            Self::NotAllowlisted => format!(
                "Endpoint blocked by system policy: {url} is not on the allowlist of \
                 permitted public resources for requests that send data. Plain GET reads \
                 may still be allowed."
            ),
            Self::UrlTooLong => format!(
                "Endpoint blocked by system policy: reads of hosts outside the allowlist \
                 are limited to {OPEN_READ_MAX_URL_LEN}-character URLs ({url})."
            ),
            Self::IpLiteral => format!(
                "Endpoint blocked by system policy: reads of hosts outside the allowlist \
                 must use a hostname, not an IP address ({url})."
            ),
        }
    }
}

/// How a permitted request passed the system policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EgressPolicyGrant {
    /// The host is on the allowlist.
    Allowlisted,
    /// A read of a host outside the allowlist (`curated-writes` only). The
    /// boundary meters these per org.
    OpenRead,
}

/// The deployment-wide egress policy: mode, allowlist, and deny list.
#[derive(Debug, Clone)]
pub struct SystemEgressPolicy {
    mode: EgressPolicyMode,
    allowlist: Arc<SystemAllowlist>,
    denylist: NetworkAccessList,
}

impl SystemEgressPolicy {
    /// Build a policy from parts. `deny_patterns` uses allowlist pattern syntax.
    pub fn new(
        mode: EgressPolicyMode,
        allowlist: Arc<SystemAllowlist>,
        deny_patterns: Vec<String>,
    ) -> Self {
        Self {
            mode,
            allowlist,
            denylist: NetworkAccessList {
                allowed: Vec::new(),
                blocked: deny_patterns,
            },
        }
    }

    /// Every request must match `allowlist`; no deny list. Mirrors the
    /// original allowlist-only behavior, mostly for tests.
    pub fn allowlist_only(allowlist: Arc<SystemAllowlist>) -> Self {
        Self::new(EgressPolicyMode::CuratedAll, allowlist, Vec::new())
    }

    /// The embedded allowlist and deny list in `mode`.
    #[expect(
        clippy::expect_used,
        reason = "the embedded deny list is validated by its tests"
    )]
    pub fn embedded(mode: EgressPolicyMode) -> Self {
        let file: DenylistFile =
            toml::from_str(EMBEDDED_DENYLIST_TOML).expect("embedded system_denylist.toml is valid");
        let deny = file
            .groups
            .into_values()
            .flat_map(|group| group.denied)
            .collect();
        Self::new(mode, SystemAllowlist::embedded(), deny)
    }

    /// Resolve the active policy from the environment. `None` in `open` mode.
    ///
    /// [`EGRESS_POLICY_ENV`] wins; when unset, the legacy
    /// [`SYSTEM_ALLOWLIST_ENABLED_ENV`] (`true`/`1`) selects `curated-all`. An
    /// unrecognized [`EGRESS_POLICY_ENV`] value fails closed to `curated-all`
    /// rather than silently opening egress.
    pub fn from_env() -> Option<Arc<SystemEgressPolicy>> {
        static RESOLVED: OnceLock<Option<Arc<SystemEgressPolicy>>> = OnceLock::new();
        RESOLVED
            .get_or_init(|| {
                let mode = Self::mode_from_env(
                    std::env::var(EGRESS_POLICY_ENV).ok().as_deref(),
                    std::env::var(SYSTEM_ALLOWLIST_ENABLED_ENV).ok().as_deref(),
                );
                (mode != EgressPolicyMode::Open).then(|| Arc::new(Self::embedded(mode)))
            })
            .clone()
    }

    fn mode_from_env(policy: Option<&str>, legacy: Option<&str>) -> EgressPolicyMode {
        match policy {
            Some(value) => EgressPolicyMode::parse(value).unwrap_or_else(|| {
                tracing::error!(
                    value,
                    "unrecognized {EGRESS_POLICY_ENV}; enforcing curated-all"
                );
                EgressPolicyMode::CuratedAll
            }),
            None if matches!(legacy, Some("true" | "1")) => EgressPolicyMode::CuratedAll,
            None => EgressPolicyMode::Open,
        }
    }

    pub fn mode(&self) -> EgressPolicyMode {
        self.mode
    }

    pub fn allowlist(&self) -> &SystemAllowlist {
        &self.allowlist
    }

    /// Whether the deny list matches `url`.
    pub fn is_denied(&self, url: &str) -> bool {
        !self.denylist.is_url_allowed(url)
    }

    /// Decide one request. `extra_allowed` widens the allowlist for this
    /// request only (an org's own extension); it never overrides the deny list.
    pub fn check(
        &self,
        url: &str,
        access: EgressAccess,
        extra_allowed: Option<&NetworkAccessList>,
    ) -> Result<EgressPolicyGrant, EgressPolicyDenial> {
        if self.mode == EgressPolicyMode::Open {
            return Ok(EgressPolicyGrant::Allowlisted);
        }
        if self.is_denied(url) {
            return Err(EgressPolicyDenial::Denylisted);
        }
        let extra =
            extra_allowed.is_some_and(|list| !list.allowed.is_empty() && list.is_url_allowed(url));
        if extra || self.allowlist.is_url_allowed(url) {
            return Ok(EgressPolicyGrant::Allowlisted);
        }
        if self.mode == EgressPolicyMode::CuratedAll || access == EgressAccess::Write {
            return Err(EgressPolicyDenial::NotAllowlisted);
        }
        if url.len() > OPEN_READ_MAX_URL_LEN {
            return Err(EgressPolicyDenial::UrlTooLong);
        }
        let host_is_ip = url::Url::parse(url).ok().is_some_and(|parsed| {
            matches!(
                parsed.host(),
                Some(url::Host::Ipv4(_)) | Some(url::Host::Ipv6(_))
            )
        });
        if host_is_ip {
            return Err(EgressPolicyDenial::IpLiteral);
        }
        Ok(EgressPolicyGrant::OpenRead)
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
            ("https://shipmail.to/auth.md", true),
            ("https://api.agentmail.to/v0/inboxes", true),
            ("https://agentid.com/.well-known/agentid", true),
            ("https://visti.sh/mcp", true),
            (
                "https://visti.sh/.well-known/oauth-authorization-server",
                true,
            ),
            ("https://stend.sh/mcp", true),
            ("https://visti.sh.evil.test/mcp", false),
            ("https://evil.stend.sh.example/mcp", false),
            ("https://shipmail.to.evil.test/", false),
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
    fn embedded_denylist_parses_and_blocks_capture_services() {
        let policy = SystemEgressPolicy::embedded(EgressPolicyMode::CuratedWrites);
        for url in [
            "https://webhook.site/abc?data=secret",
            "https://x.oast.fun/",
            "https://abc.ngrok-free.app/collect",
            "https://eo123.m.pipedream.net/",
        ] {
            assert!(policy.is_denied(url), "{url}");
            assert_eq!(
                policy.check(url, EgressAccess::Read, None),
                Err(EgressPolicyDenial::Denylisted),
                "{url}"
            );
        }
        assert!(!policy.is_denied("https://docs.rs/serde"));
    }

    #[test]
    fn curated_writes_opens_reads_and_gates_writes() {
        let policy = SystemEgressPolicy::embedded(EgressPolicyMode::CuratedWrites);
        let blog = "https://random-blog.net/post";
        assert_eq!(
            policy.check(blog, EgressAccess::Read, None),
            Ok(EgressPolicyGrant::OpenRead)
        );
        assert_eq!(
            policy.check(blog, EgressAccess::Write, None),
            Err(EgressPolicyDenial::NotAllowlisted)
        );
        assert_eq!(
            policy.check("https://api.github.com/gists", EgressAccess::Write, None),
            Ok(EgressPolicyGrant::Allowlisted)
        );
        let long = format!(
            "https://random-blog.net/?q={}",
            "a".repeat(OPEN_READ_MAX_URL_LEN)
        );
        assert_eq!(
            policy.check(&long, EgressAccess::Read, None),
            Err(EgressPolicyDenial::UrlTooLong)
        );
        for ip in ["http://203.0.113.7/x", "http://[2001:db8::1]/x"] {
            assert_eq!(
                policy.check(ip, EgressAccess::Read, None),
                Err(EgressPolicyDenial::IpLiteral),
                "{ip}"
            );
        }
    }

    #[test]
    fn curated_all_gates_reads_too() {
        let policy = SystemEgressPolicy::embedded(EgressPolicyMode::CuratedAll);
        assert_eq!(
            policy.check("https://random-blog.net/post", EgressAccess::Read, None),
            Err(EgressPolicyDenial::NotAllowlisted)
        );
    }

    #[test]
    fn extra_allowed_widens_but_never_beats_the_denylist() {
        let policy = SystemEgressPolicy::embedded(EgressPolicyMode::CuratedAll);
        let extra = NetworkAccessList::allow_only(["*.customer.example", "*.webhook.site"]);
        assert_eq!(
            policy.check(
                "https://api.customer.example/v1",
                EgressAccess::Write,
                Some(&extra)
            ),
            Ok(EgressPolicyGrant::Allowlisted)
        );
        assert_eq!(
            policy.check("https://webhook.site/x", EgressAccess::Write, Some(&extra)),
            Err(EgressPolicyDenial::Denylisted)
        );
        // An empty extension must not read as "allow everything".
        assert_eq!(
            policy.check(
                "https://api.customer.example/v1",
                EgressAccess::Write,
                Some(&NetworkAccessList::default())
            ),
            Err(EgressPolicyDenial::NotAllowlisted)
        );
    }

    #[test]
    fn access_classification() {
        assert_eq!(EgressAccess::classify("GET", false), EgressAccess::Read);
        assert_eq!(EgressAccess::classify("head", false), EgressAccess::Read);
        assert_eq!(EgressAccess::classify("GET", true), EgressAccess::Write);
        assert_eq!(EgressAccess::classify("POST", false), EgressAccess::Write);
        assert_eq!(
            EgressAccess::classify("OPTIONS", false),
            EgressAccess::Write
        );
    }

    #[test]
    fn mode_resolution_prefers_policy_and_fails_closed() {
        use EgressPolicyMode::*;
        for (policy, legacy, expected) in [
            (None, None, Open),
            (None, Some("true"), CuratedAll),
            (None, Some("1"), CuratedAll),
            (None, Some("TRUE"), Open),
            (Some("open"), Some("true"), Open),
            (Some("curated-writes"), None, CuratedWrites),
            (Some("curated-all"), None, CuratedAll),
            (Some("Curated-Writes"), None, CuratedAll),
            (Some(""), None, CuratedAll),
        ] {
            assert_eq!(
                SystemEgressPolicy::mode_from_env(policy, legacy),
                expected,
                "{policy:?} {legacy:?}"
            );
        }
    }
}

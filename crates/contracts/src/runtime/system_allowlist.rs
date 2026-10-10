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

use crate::runtime::network_access::{NetworkAccessList, is_http_prefix};
use serde::Deserialize;
use std::collections::{BTreeMap, HashSet};
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

/// Host patterns compiled for lookup instead of a linear scan.
///
/// Same pattern syntax and matching as [`NetworkAccessList`], but every
/// request is checked against the whole curated list, so the patterns are
/// indexed: exact hosts and wildcard suffixes go into hash sets, and a host is
/// matched by looking up itself and each of its parent domains. A check costs
/// one lookup per label of the host, however long the list grows. URL-prefix
/// patterns are rare and stay a short scan.
///
/// Decision: one trailing dot is dropped from the request host, so
/// `webhook.site.` (the same host in DNS) cannot slip past the deny list.
#[derive(Debug, Clone, Default)]
struct CompiledPatterns {
    exact: HashSet<String>,
    suffixes: HashSet<String>,
    prefixes: Vec<String>,
}

impl CompiledPatterns {
    fn new<'a>(patterns: impl IntoIterator<Item = &'a String>) -> Self {
        let mut compiled = Self::default();
        for pattern in patterns {
            if is_http_prefix(pattern) {
                // An invalid prefix never matches, as in `NetworkAccessList`.
                if let Ok(prefix) = url::Url::parse(pattern) {
                    compiled.prefixes.push(prefix.as_str().to_string());
                }
            } else if let Some(suffix) = pattern.strip_prefix("*.") {
                compiled.suffixes.insert(suffix.to_lowercase());
            } else {
                compiled.exact.insert(pattern.to_lowercase());
            }
        }
        compiled
    }

    fn matches(&self, parsed: &url::Url) -> bool {
        let Some(host) = parsed.host_str() else {
            return false;
        };
        let host = host.to_ascii_lowercase();
        let host = host.strip_suffix('.').unwrap_or(&host);
        if self.exact.contains(host) || self.suffixes.contains(host) {
            return true;
        }
        let mut rest = host;
        while let Some((_, parent)) = rest.split_once('.') {
            if self.suffixes.contains(parent) {
                return true;
            }
            rest = parent;
        }
        self.prefixes
            .iter()
            .any(|prefix| parsed.as_str().starts_with(prefix.as_str()))
    }

    fn matches_url(&self, url: &str) -> bool {
        url::Url::parse(url).is_ok_and(|parsed| self.matches(&parsed))
    }
}

/// Curated, system-wide outbound allowlist.
///
/// Matching follows [`NetworkAccessList`] pattern semantics over the flattened
/// patterns of every group, so only URLs matching at least one pattern are
/// permitted. An allowlist with no patterns matches nothing (fails closed).
#[derive(Debug, Clone)]
pub struct SystemAllowlist {
    groups: Vec<AllowGroup>,
    patterns: CompiledPatterns,
}

impl SystemAllowlist {
    /// Parse a TOML document into a `SystemAllowlist`.
    pub fn from_toml(source: &str) -> Result<Self, toml::de::Error> {
        let file: AllowlistFile = toml::from_str(source)?;
        let groups: Vec<AllowGroup> = file
            .groups
            .into_iter()
            .map(|(name, spec)| AllowGroup {
                name,
                description: spec.description,
                allowed: spec.allowed,
            })
            .collect();
        // Fail closed: unlike `NetworkAccessList`, where an empty `allowed`
        // list means "no restriction", compiled patterns with no entries match
        // nothing, so an empty or misconfigured allowlist denies everything.
        let patterns = CompiledPatterns::new(groups.iter().flat_map(|group| &group.allowed));
        Ok(Self { groups, patterns })
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
        self.patterns.matches_url(url)
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
    denylist: CompiledPatterns,
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
            denylist: CompiledPatterns::new(&deny_patterns),
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
        self.denylist.matches_url(url)
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
        // Parse once for every list. A URL that does not parse matches no
        // pattern, so it can never be an open read either.
        let Ok(parsed) = url::Url::parse(url) else {
            return Err(EgressPolicyDenial::NotAllowlisted);
        };
        if self.denylist.matches(&parsed) {
            return Err(EgressPolicyDenial::Denylisted);
        }
        if self.allowlist.patterns.matches(&parsed) {
            return Ok(EgressPolicyGrant::Allowlisted);
        }
        if extra_allowed.is_some_and(|list| !list.allowed.is_empty() && list.is_url_allowed(url)) {
            return Ok(EgressPolicyGrant::Allowlisted);
        }
        if self.mode == EgressPolicyMode::CuratedAll || access == EgressAccess::Write {
            return Err(EgressPolicyDenial::NotAllowlisted);
        }
        if url.len() > OPEN_READ_MAX_URL_LEN {
            return Err(EgressPolicyDenial::UrlTooLong);
        }
        if matches!(
            parsed.host(),
            Some(url::Host::Ipv4(_)) | Some(url::Host::Ipv6(_))
        ) {
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
            // Vendors the broad list adds.
            ("https://api.linear.app/graphql", true),
            ("https://acme.atlassian.net/rest/api/3/issue", true),
            ("https://graph.microsoft.com/v1.0/me", true),
            ("https://contoso.sharepoint.com/sites/x", true),
            ("https://api.notion.com/v1/pages", true),
            ("https://slack.com/api/chat.postMessage", true),
            (
                "https://api.ebay.com/buy/browse/v1/item_summary/search",
                true,
            ),
            ("https://www.amazon.de/dp/B000", true),
            ("https://api.stripe.com/v1/charges", true),
            ("https://api.procore.com/rest/v1.0/projects", true),
            ("https://developer.api.autodesk.com/oss/v2/buckets", true),
            ("https://en.wikipedia.org/wiki/Rust", true),
            ("https://grokipedia.com/page/Rust", true),
            ("https://api.search.brave.com/res/v1/web/search", true),
            ("https://www.usa.gov/", true),
            ("https://diia.gov.ua/", true),
            ("https://www.gov.uk/", true),
            ("https://ec.europa.eu/", true),
            ("https://dev-123.us.auth0.com/oauth/token", true),
            ("https://api.workos.com/user_management", true),
            ("https://api.novaposhta.ua/v2.0/json/", true),
            ("https://docs.everruns.com/", true),
            ("https://bashkit.sh/", true),
            // Approved exceptions (approved_exceptions group).
            ("https://evil.web.app/", true),
            ("https://evil.firebaseapp.com/", true),
            ("https://evil.supabase.co/functions/v1/f", true),
            ("https://hooks.zapier.com/hooks/catch/1/x", true),
            ("https://www.mit.edu/", true),
            ("https://writer.substack.com/api/v1/posts", true),
            ("https://hook.eu1.make.com/x", true),
            (
                "https://bedrock-runtime.us-east-1.amazonaws.com/model/x/converse",
                true,
            ),
            (
                "https://bedrock-runtime.mars-1.amazonaws.com/model/x",
                false,
            ),
            // Customer code, pages, forms, and relays under or near listed
            // vendors stay out.
            ("https://docs.google.com/forms/d/e/x/formResponse", false),
            ("https://script.google.com/macros/s/x/exec", false),
            ("https://forms.office.com/r/x", false),
            ("https://myvm.westus.cloudapp.azure.com/", false),
            ("https://evil.azurewebsites.net/", false),
            ("https://evil.blob.core.windows.net/c/x", false),
            ("https://evil.vercel.app/", false),
            ("https://evil.netlify.app/", false),
            ("https://evil.pages.dev/", false),
            ("https://evil.workers.dev/", false),
            ("https://evil.herokuapp.com/", false),
            ("https://evil.fly.dev/", false),
            ("https://evil.onrender.com/", false),
            ("https://evil.appspot.com/", false),
            ("https://us-central1-evil.cloudfunctions.net/f", false),
            ("https://evil-abc.a.run.app/", false),
            ("https://evil.myshopify.com/", false),
            ("https://evil.notion.site/", false),
            ("https://evil.gitlab.io/", false),
            ("https://shop.prom.ua/", false),
            ("https://evil.app.n8n.cloud/webhook/x", false),
            ("https://services.cloud.mongodb.com/app/x/endpoint/y", false),
            ("https://forms.hubspot.com/uploads/form/v2/1/x", false),
            ("https://pastebin.com/api/api_post.php", false),
            ("https://bit.ly/x", false),
            ("https://evil.sandbox.e2b.app/", false),
            ("https://evil.modal.run/", false),
            ("https://example.gov.evil.test/", false),
            // No Russian government (or other .ru) domains, by owner decision.
            ("https://www.gov.ru/", false),
            ("https://kremlin.ru/", false),
            ("https://www.gosuslugi.ru/", false),
        ] {
            assert_eq!(allowlist.is_url_allowed(url), expected, "{url}");
        }
    }

    #[test]
    fn compiled_patterns_match_like_network_access_list() {
        let patterns: Vec<String> = [
            "Example.COM",
            "*.Wild.test",
            "https://prefix.test/api/",
            "https://",
            "exact.test",
        ]
        .map(String::from)
        .to_vec();
        let compiled = CompiledPatterns::new(&patterns);
        let reference = NetworkAccessList::allow_only(patterns.clone());
        for url in [
            "https://EXAMPLE.com:8443/path",
            "https://sub.example.com/",
            "https://wild.test/",
            "https://a.b.wild.test/x",
            "https://notwild.test/",
            "https://wild.test.evil/",
            "https://prefix.test/api/v1",
            "https://prefix.test/apix",
            "https://prefix.test/other",
            "https://exact.test/",
            "https://x.exact.test/",
            "https://exact.test@evil.test/",
            "mailto:someone@exact.test",
            "not a url",
        ] {
            assert_eq!(
                compiled.matches_url(url),
                reference.is_url_allowed(url),
                "{url}"
            );
        }
    }

    #[test]
    fn trailing_dot_hosts_match_their_patterns() {
        let policy = SystemEgressPolicy::embedded(EgressPolicyMode::CuratedWrites);
        assert_eq!(
            policy.check("https://webhook.site./x", EgressAccess::Read, None),
            Err(EgressPolicyDenial::Denylisted)
        );
        assert_eq!(
            policy.check("https://api.openai.com./v1", EgressAccess::Write, None),
            Ok(EgressPolicyGrant::Allowlisted)
        );
    }

    #[test]
    fn unparseable_urls_are_never_open_reads() {
        let policy = SystemEgressPolicy::embedded(EgressPolicyMode::CuratedWrites);
        assert_eq!(
            policy.check("not a url", EgressAccess::Read, None),
            Err(EgressPolicyDenial::NotAllowlisted)
        );
    }

    #[test]
    fn embedded_allowlist_has_no_duplicate_patterns() {
        let allowlist = SystemAllowlist::embedded();
        let mut seen = std::collections::HashMap::new();
        for group in allowlist.groups() {
            for pattern in &group.allowed {
                if let Some(other) = seen.insert(pattern.to_lowercase(), &group.name) {
                    panic!("{pattern} is listed in both {other} and {}", group.name);
                }
            }
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

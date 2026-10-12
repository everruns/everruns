//! Organization side of web search: the provider choices, the organization's
//! defaults (`web_search_defaults` on organization settings) and its search
//! accounts.
//!
//! Decision: these live in the contract, not in the capability, because three
//! places read them: the server validates and stores the defaults, the worker
//! fetches them for the tool, and the tool resolves plans from them.
//!
//! Decision: the Automatic order is an order, never an exclusion. Native search
//! is switched off with `allow_native`; an account leaves Automatic by being
//! disconnected. Entries the order does not mention keep their natural place
//! (native first, unlisted accounts after the listed ones), and free search is
//! always last, so connecting an account changes Automatic from the next search
//! without anyone editing the order.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{WEB_SEARCH_DEFAULT_RESULTS, WEB_SEARCH_MAX_RESULTS};

/// Connector capability a connector declares when its organization accounts
/// can answer `web_search` (the counterpart of `sandbox_provisioning`).
pub const WEB_SEARCH_CONNECTOR_CAPABILITY: &str = "web_search";

/// Connection providers whose organization accounts have a search driver.
/// Their connectors declare [`WEB_SEARCH_CONNECTOR_CAPABILITY`]; a test in
/// `everruns-integrations` keeps the two in step.
pub const WEB_SEARCH_ACCOUNT_PROVIDERS: &[&str] = &["brave_search", "parallel"];

/// Model provider types whose drivers run the model's own search (native
/// search, billed by the model provider). Bedrock is not here: Anthropic does
/// not offer web search there, and its driver has no server tools.
pub const NATIVE_WEB_SEARCH_PROVIDER_TYPES: &[&str] = &[
    "openai",
    "azure_openai",
    "chatgpt",
    "openrouter",
    "anthropic",
    "gemini",
];

/// Plain-words price of native search, shown wherever it is offered.
pub const NATIVE_WEB_SEARCH_PRICE_NOTE: &str =
    "Billed by the model provider, about $10 per 1,000 searches";

/// Entry names in the Automatic order besides account ids.
pub const AUTOMATIC_NATIVE: &str = "native";
pub const AUTOMATIC_FREE: &str = "free";

/// Whether models of this provider type offer native search.
pub fn native_web_search_supported(provider_type: &str) -> bool {
    NATIVE_WEB_SEARCH_PROVIDER_TYPES.contains(&provider_type)
}

/// Whether this model searches natively through its provider's driver: the
/// same model rules the drivers apply before swapping in the hosted tool.
pub fn native_web_search_supported_model(provider_type: &str, model: &str) -> bool {
    match provider_type {
        "openai" | "azure_openai" | "chatgpt" => {
            crate::native_web_search::openai_has_native_web_search(model)
        }
        "openrouter" => crate::native_web_search::openrouter_native_vendor(model).is_some(),
        "anthropic" => crate::native_web_search::anthropic_has_native_web_search(model),
        "gemini" => crate::native_web_search::gemini_has_native_web_search(model),
        _ => false,
    }
}

/// Whether model provider search comes first in the plan an agent's provider
/// setting resolves to under these organization settings (`None`: no
/// organization, the built-in defaults), for a model that has it. This is
/// the per-turn question native search answers: only a first entry runs as
/// the model's own search, since anything before it must be tried first.
pub fn native_search_first(
    provider: SearchProviderSetting,
    org: Option<&WebSearchOrgSettings>,
) -> bool {
    let defaults = org.map(|org| &org.defaults);
    if defaults.is_some_and(|defaults| !defaults.allow_native) {
        return false;
    }
    let provider = match provider {
        SearchProviderSetting::OrgDefault => {
            defaults.map_or(SearchProviderSetting::Auto, |defaults| defaults.provider)
        }
        provider => provider,
    };
    match provider {
        SearchProviderSetting::Native => true,
        SearchProviderSetting::Auto | SearchProviderSetting::OrgDefault => {
            // Automatic puts native first unless the order lists a connected
            // account ahead of it; an order without native keeps it first.
            let Some(org) = org else { return true };
            let order = &org.defaults.automatic_order;
            !order.iter().any(|entry| entry == AUTOMATIC_NATIVE)
                || !org
                    .defaults
                    .automatic_order
                    .iter()
                    .take_while(|entry| entry.as_str() != AUTOMATIC_NATIVE)
                    .any(|entry| {
                        Uuid::parse_str(entry)
                            .is_ok_and(|id| org.accounts.iter().any(|account| account.id == id))
                    })
        }
        SearchProviderSetting::Brave
        | SearchProviderSetting::Parallel
        | SearchProviderSetting::Account
        | SearchProviderSetting::Free => false,
    }
}

/// Apply the organization's current settings to a call's native search
/// request: drop it when native is not first in the agent's plan, and add the
/// organization's blocked domains otherwise. A request without the option is
/// left alone.
pub fn resolve_native_web_search(
    options: &mut std::collections::HashMap<String, serde_json::Value>,
    org: Option<&WebSearchOrgSettings>,
) {
    use crate::native_web_search::{NATIVE_WEB_SEARCH_OPTION, NativeWebSearch};
    let Some(mut native) = NativeWebSearch::from_driver_options(options) else {
        options.remove(NATIVE_WEB_SEARCH_OPTION);
        return;
    };
    let provider = serde_json::from_value(serde_json::Value::from(native.provider.clone()))
        .unwrap_or(SearchProviderSetting::OrgDefault);
    if !native_search_first(provider, org) {
        options.remove(NATIVE_WEB_SEARCH_OPTION);
        return;
    }
    for domain in org
        .map(|org| org.defaults.blocked_domains.as_slice())
        .unwrap_or_default()
    {
        if !native.blocked_domains.contains(domain) {
            native.blocked_domains.push(domain.clone());
        }
    }
    let (key, value) = native.to_driver_option();
    options.insert(key, value);
}

/// Which provider web search uses.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum SearchProviderSetting {
    /// Follow the organization's default (agents only).
    #[default]
    OrgDefault,
    /// Model provider search when the model has it, then the organization's
    /// search accounts, then free search.
    Auto,
    /// The model provider's own search.
    Native,
    /// Brave Search with a key from the session, the user, the application, or
    /// else the organization's first Brave account.
    Brave,
    /// Parallel with a key from the session, the user, the application, or
    /// else the organization's first Parallel account.
    Parallel,
    /// One named organization search account (`account_id`).
    Account,
    /// Keyless best-effort search.
    Free,
}

impl SearchProviderSetting {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OrgDefault => "org_default",
            Self::Auto => "auto",
            Self::Native => "native",
            Self::Brave => "brave",
            Self::Parallel => "parallel",
            Self::Account => "account",
            Self::Free => "free",
        }
    }
}

/// What happens when a provider cannot answer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum SearchFallbackMode {
    /// Try the next provider in the plan; free search is always last.
    #[default]
    Next,
    /// Use only the chosen provider.
    None,
}

/// An organization's web search defaults.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub struct WebSearchDefaults {
    /// Provider for agents set to the organization default: `auto`, `native`,
    /// `brave`, `account` (with `account_id`) or `free`.
    #[serde(default = "default_provider")]
    pub provider: SearchProviderSetting,
    /// The search account when `provider` is `account`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<Uuid>,
    /// Automatic order: `native`, search account ids, and `free`, which is
    /// always last. Entries left out keep their natural place.
    #[serde(default = "default_automatic_order")]
    pub automatic_order: Vec<String>,
    /// Whether agents may use the model provider's own search (billed by the
    /// model provider).
    #[serde(default = "default_true")]
    pub allow_native: bool,
    /// When a provider cannot answer: `next` falls back (free search last),
    /// `none` stops. An agent's own `fallback` wins.
    #[serde(default)]
    pub fallback: SearchFallbackMode,
    /// Results per search when neither the model nor the agent asks (1 to 20).
    #[serde(default = "default_results")]
    pub results_per_search: u32,
    /// Sites no search in this organization returns, on top of each agent's.
    #[serde(default)]
    pub blocked_domains: Vec<String>,
}

fn default_provider() -> SearchProviderSetting {
    SearchProviderSetting::Auto
}

fn default_automatic_order() -> Vec<String> {
    vec![AUTOMATIC_NATIVE.to_string(), AUTOMATIC_FREE.to_string()]
}

fn default_true() -> bool {
    true
}

fn default_results() -> u32 {
    WEB_SEARCH_DEFAULT_RESULTS
}

impl Default for WebSearchDefaults {
    fn default() -> Self {
        Self {
            provider: default_provider(),
            account_id: None,
            automatic_order: default_automatic_order(),
            allow_native: true,
            fallback: SearchFallbackMode::Next,
            results_per_search: WEB_SEARCH_DEFAULT_RESULTS,
            blocked_domains: Vec::new(),
        }
    }
}

impl WebSearchDefaults {
    /// Validate against the organization's search accounts and return the
    /// normalized defaults (trimmed domains, free search appended last).
    pub fn validated(mut self, account_ids: &[Uuid]) -> Result<Self, String> {
        match self.provider {
            SearchProviderSetting::OrgDefault => {
                return Err(
                    "web_search_defaults.provider cannot be org_default; choose auto, native, \
                     brave, parallel, account or free"
                        .into(),
                );
            }
            SearchProviderSetting::Account => {
                let id = self
                    .account_id
                    .ok_or("web_search_defaults.account_id is required when provider is account")?;
                if !account_ids.contains(&id) {
                    return Err(format!("Search account {id} not found"));
                }
            }
            _ if self.account_id.is_some() => {
                return Err(
                    "web_search_defaults.account_id is only allowed when provider is account"
                        .into(),
                );
            }
            _ => {}
        }
        if !(1..=WEB_SEARCH_MAX_RESULTS).contains(&self.results_per_search) {
            return Err(format!(
                "web_search_defaults.results_per_search must be between 1 and \
                 {WEB_SEARCH_MAX_RESULTS}, got {}",
                self.results_per_search
            ));
        }
        let mut order: Vec<String> = Vec::new();
        let last = self.automatic_order.len().saturating_sub(1);
        for (index, entry) in self.automatic_order.iter().enumerate() {
            let entry = entry.trim().to_ascii_lowercase();
            if order.contains(&entry) {
                return Err(format!(
                    "web_search_defaults.automatic_order lists {entry} twice"
                ));
            }
            match entry.as_str() {
                AUTOMATIC_NATIVE => {}
                AUTOMATIC_FREE if index == last => {}
                AUTOMATIC_FREE => {
                    return Err(
                        "web_search_defaults.automatic_order: free search is always last".into(),
                    );
                }
                other => {
                    let id = Uuid::parse_str(other).map_err(|_| {
                        format!(
                            "web_search_defaults.automatic_order entries are native, free or a \
                             search account id, got {other:?}"
                        )
                    })?;
                    if !account_ids.contains(&id) {
                        return Err(format!("Search account {id} not found"));
                    }
                }
            }
            order.push(entry);
        }
        if order.last().map(String::as_str) != Some(AUTOMATIC_FREE) {
            order.push(AUTOMATIC_FREE.to_string());
        }
        self.automatic_order = order;
        self.blocked_domains = normalize_domains(&self.blocked_domains)
            .map_err(|error| format!("web_search_defaults.blocked_domains: {error}"))?;
        Ok(self)
    }
}

/// Trim and check bare host names (`example.com`), dropping duplicates.
pub fn normalize_domains(domains: &[String]) -> Result<Vec<String>, String> {
    let mut out: Vec<String> = Vec::new();
    for domain in domains {
        let domain = domain.trim().to_ascii_lowercase();
        if domain.is_empty() || domain.contains(['/', ' ', ':']) {
            return Err(format!(
                "domains are bare host names like example.com, got {domain:?}"
            ));
        }
        if !out.contains(&domain) {
            out.push(domain);
        }
    }
    Ok(out)
}

/// How long a host reuses an organization's web search settings, so a change
/// reaches running sessions within this window without one lookup per call.
pub const WEB_SEARCH_SETTINGS_TTL: std::time::Duration = std::time::Duration::from_secs(30);

type SettingsCacheEntry = (
    web_time::Instant,
    Option<std::sync::Arc<WebSearchOrgSettings>>,
);

static SETTINGS_CACHE: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<String, SettingsCacheEntry>>,
> = std::sync::LazyLock::new(Default::default);

/// The organization's web search settings through `resolver`, cached for
/// [`WEB_SEARCH_SETTINGS_TTL`] under `key` (an organization or session). `None`
/// when the host has none or the lookup failed (logged): callers then use the
/// built-in defaults rather than failing the search or the turn.
pub async fn cached_web_search_settings(
    resolver: &dyn crate::runtime::connection_services::UserConnectionResolver,
    key: &str,
    session_id: crate::runtime::typed_id::SessionId,
) -> Option<std::sync::Arc<WebSearchOrgSettings>> {
    let now = web_time::Instant::now();
    if let Ok(cache) = SETTINGS_CACHE.lock()
        && let Some((at, settings)) = cache.get(key)
        && now.duration_since(*at) < WEB_SEARCH_SETTINGS_TTL
    {
        return settings.clone();
    }
    let settings = match resolver.get_web_search_settings(session_id).await {
        Ok(settings) => settings.map(std::sync::Arc::new),
        Err(error) => {
            tracing::debug!(error = %error, "web search settings lookup failed");
            return None;
        }
    };
    if let Ok(mut cache) = SETTINGS_CACHE.lock() {
        cache.retain(|_, (at, _)| now.duration_since(*at) < WEB_SEARCH_SETTINGS_TTL);
        cache.insert(key.to_string(), (now, settings.clone()));
    }
    settings
}

/// One organization search account, without its key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SearchAccount {
    /// Organization connection id.
    pub id: Uuid,
    /// Connection provider, for example `brave_search`.
    pub provider: String,
    /// Account name the admin gave it.
    pub name: String,
}

/// What a running agent needs from its organization to plan a search.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct WebSearchOrgSettings {
    #[serde(default)]
    pub defaults: WebSearchDefaults,
    /// Search accounts in the organization's listing order.
    #[serde(default)]
    pub accounts: Vec<SearchAccount>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    #[test]
    fn new_organizations_get_automatic_with_native_and_fallback() {
        let defaults: WebSearchDefaults = serde_json::from_str("{}").unwrap();
        assert_eq!(defaults, WebSearchDefaults::default());
        assert_eq!(defaults.provider, SearchProviderSetting::Auto);
        assert_eq!(defaults.automatic_order, ["native", "free"]);
        assert!(defaults.allow_native);
        assert_eq!(defaults.fallback, SearchFallbackMode::Next);
        assert_eq!(defaults.results_per_search, 8);
    }

    #[test]
    fn validation_normalizes_the_automatic_order() {
        let account = id(7);
        let defaults = WebSearchDefaults {
            automatic_order: vec![account.to_string(), " Native ".into()],
            blocked_domains: vec![" Pinterest.com".into(), "pinterest.com".into()],
            ..WebSearchDefaults::default()
        }
        .validated(&[account])
        .unwrap();
        assert_eq!(
            defaults.automatic_order,
            [account.to_string(), "native".into(), "free".into()]
        );
        assert_eq!(defaults.blocked_domains, ["pinterest.com"]);
    }

    #[test]
    fn validation_rejects_bad_defaults() {
        let account = id(7);
        let cases: Vec<(WebSearchDefaults, &str)> = vec![
            (
                WebSearchDefaults {
                    provider: SearchProviderSetting::OrgDefault,
                    ..Default::default()
                },
                "cannot be org_default",
            ),
            (
                WebSearchDefaults {
                    provider: SearchProviderSetting::Account,
                    ..Default::default()
                },
                "account_id is required",
            ),
            (
                WebSearchDefaults {
                    provider: SearchProviderSetting::Account,
                    account_id: Some(id(9)),
                    ..Default::default()
                },
                "not found",
            ),
            (
                WebSearchDefaults {
                    account_id: Some(account),
                    ..Default::default()
                },
                "only allowed when provider is account",
            ),
            (
                WebSearchDefaults {
                    results_per_search: 21,
                    ..Default::default()
                },
                "between 1 and 20",
            ),
            (
                WebSearchDefaults {
                    results_per_search: 0,
                    ..Default::default()
                },
                "between 1 and 20",
            ),
            (
                WebSearchDefaults {
                    automatic_order: vec!["free".into(), "native".into()],
                    ..Default::default()
                },
                "always last",
            ),
            (
                WebSearchDefaults {
                    automatic_order: vec!["native".into(), "native".into()],
                    ..Default::default()
                },
                "twice",
            ),
            (
                WebSearchDefaults {
                    automatic_order: vec!["bing".into()],
                    ..Default::default()
                },
                "search account id",
            ),
            (
                WebSearchDefaults {
                    automatic_order: vec![id(9).to_string()],
                    ..Default::default()
                },
                "not found",
            ),
            (
                WebSearchDefaults {
                    blocked_domains: vec!["https://x.com".into()],
                    ..Default::default()
                },
                "bare host names",
            ),
        ];
        for (defaults, expected) in cases {
            let error = defaults.clone().validated(&[account]).unwrap_err();
            assert!(error.contains(expected), "{defaults:?}: {error}");
        }
        assert!(
            WebSearchDefaults {
                provider: SearchProviderSetting::Account,
                account_id: Some(account),
                ..Default::default()
            }
            .validated(&[account])
            .is_ok()
        );
    }

    #[test]
    fn unknown_fields_are_rejected() {
        assert!(serde_json::from_str::<WebSearchDefaults>(r#"{"provder": "free"}"#).is_err());
    }

    #[test]
    fn native_search_is_a_provider_type_and_model_property() {
        assert!(native_web_search_supported("openrouter"));
        assert!(native_web_search_supported("anthropic"));
        assert!(native_web_search_supported("gemini"));
        assert!(!native_web_search_supported("bedrock"));
        assert!(!native_web_search_supported("mistral"));
        assert!(native_web_search_supported_model("openai", "gpt-6.1-sol"));
        assert!(!native_web_search_supported_model(
            "openai",
            "gpt-3.5-turbo"
        ));
        assert!(native_web_search_supported_model(
            "openrouter",
            "anthropic/claude-opus-5-5"
        ));
        assert!(!native_web_search_supported_model(
            "openrouter",
            "mistralai/mistral-large"
        ));
        assert!(native_web_search_supported_model(
            "anthropic",
            "claude-opus-5-5"
        ));
        assert!(!native_web_search_supported_model(
            "anthropic",
            "claude-3-5-sonnet-20241022"
        ));
        assert!(native_web_search_supported_model(
            "gemini",
            "gemini-3.8-flash"
        ));
        assert!(!native_web_search_supported_model(
            "gemini",
            "gemini-1.5-pro"
        ));
        assert!(!native_web_search_supported_model(
            "bedrock",
            "claude-opus-5-5"
        ));
    }

    fn org(defaults: serde_json::Value, accounts: &[Uuid]) -> WebSearchOrgSettings {
        WebSearchOrgSettings {
            defaults: serde_json::from_value(defaults).unwrap(),
            accounts: accounts
                .iter()
                .map(|id| SearchAccount {
                    id: *id,
                    provider: "brave_search".into(),
                    name: "Team".into(),
                })
                .collect(),
        }
    }

    #[test]
    fn native_first_by_setting_and_org_defaults() {
        use SearchProviderSetting::*;
        let a = id(0xa);
        let fresh = org(serde_json::json!({}), &[a]);
        let off = org(serde_json::json!({"allow_native": false}), &[a]);
        let account_first = org(
            serde_json::json!({"automatic_order": [a.to_string(), "native"]}),
            &[a],
        );
        // An order naming an account that is gone does not push native down.
        let stale_first = org(
            serde_json::json!({"automatic_order": [id(9).to_string(), "native"]}),
            &[a],
        );
        // An order that leaves native out keeps it in its natural first place.
        let native_unlisted = org(
            serde_json::json!({"automatic_order": [a.to_string()]}),
            &[a],
        );
        let free_default = org(serde_json::json!({"provider": "free"}), &[a]);
        let native_default = org(
            serde_json::json!({"provider": "native", "automatic_order": [a.to_string()]}),
            &[a],
        );
        let cases: Vec<(SearchProviderSetting, Option<&WebSearchOrgSettings>, bool)> = vec![
            (OrgDefault, None, true),
            (Auto, None, true),
            (Native, None, true),
            (Free, None, false),
            (Brave, None, false),
            (Parallel, None, false),
            (OrgDefault, Some(&fresh), true),
            (Auto, Some(&fresh), true),
            (Auto, Some(&off), false),
            (Native, Some(&off), false),
            (OrgDefault, Some(&off), false),
            (Auto, Some(&account_first), false),
            (OrgDefault, Some(&account_first), false),
            (Native, Some(&account_first), true),
            (Auto, Some(&stale_first), true),
            (Auto, Some(&native_unlisted), true),
            (OrgDefault, Some(&free_default), false),
            (Auto, Some(&free_default), true),
            (OrgDefault, Some(&native_default), true),
            (Account, Some(&fresh), false),
        ];
        for (provider, org, expected) in cases {
            assert_eq!(
                native_search_first(provider, org),
                expected,
                "{provider:?} {org:?}"
            );
        }
    }

    #[test]
    fn resolving_the_request_drops_it_or_adds_org_blocked_domains() {
        use crate::native_web_search::{NATIVE_WEB_SEARCH_OPTION, NativeWebSearch};
        use std::collections::HashMap;
        let request = |provider: &str| {
            HashMap::from([NativeWebSearch {
                provider: provider.into(),
                blocked_domains: vec!["x.com".into()],
                ..Default::default()
            }
            .to_driver_option()])
        };
        let blocking = org(
            serde_json::json!({"blocked_domains": ["x.com", "pinterest.com"]}),
            &[],
        );
        let mut options = request("auto");
        resolve_native_web_search(&mut options, Some(&blocking));
        assert_eq!(
            NativeWebSearch::from_driver_options(&options)
                .unwrap()
                .blocked_domains,
            ["x.com", "pinterest.com"]
        );
        let mut options = request("org_default");
        resolve_native_web_search(
            &mut options,
            Some(&org(serde_json::json!({"allow_native": false}), &[])),
        );
        assert!(!options.contains_key(NATIVE_WEB_SEARCH_OPTION));
        // No organization: built-in defaults keep the request.
        let mut options = request("org_default");
        resolve_native_web_search(&mut options, None);
        assert!(options.contains_key(NATIVE_WEB_SEARCH_OPTION));
        // A malformed request is dropped, never sent half-read.
        let mut options = HashMap::from([(
            NATIVE_WEB_SEARCH_OPTION.to_string(),
            serde_json::json!({"max_uses": "x"}),
        )]);
        resolve_native_web_search(&mut options, None);
        assert!(options.is_empty());
        // Nothing requested stays nothing.
        let mut options = HashMap::new();
        resolve_native_web_search(&mut options, Some(&blocking));
        assert!(options.is_empty());
    }
}

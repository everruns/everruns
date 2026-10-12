//! Search plans: the ordered providers one call tries.
//!
//! A plan is resolved live on every call from the agent's setting and the
//! organization's current defaults (cached briefly by the tool), so a key
//! connected or an Automatic order changed mid-session is used from the next
//! search. Resolution is pure ([`resolve_plan`]) so the server's preview and
//! the tool share it; turning entries into drivers ([`plan_steps`]) is the
//! tool's job. Model provider search never runs here: when the model runs it,
//! the driver replaces the function tool, so at tool time a native entry is
//! only recorded as the attempt that could not answer.

use std::sync::Arc;
use std::time::Instant;

use everruns_contracts::runtime::web_search::{
    AUTOMATIC_FREE, AUTOMATIC_NATIVE, SearchContext, SearchDriver, SearchExchange, SearchFailure,
    SearchFailureKind, SearchRequest, SearchResponse, WEB_SEARCH_DEFAULT_RESULTS,
    WEB_SEARCH_MAX_RESULTS, WEB_SEARCH_RAW_CAP_BYTES, WebSearchOrgSettings,
};
use serde::Serialize;
use uuid::Uuid;

use super::config::{FallbackMode, ProviderSetting, WebSearchConfig};

/// One provider in a resolved plan, before drivers are attached.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PlanEntry {
    /// The model provider's own search.
    Native,
    /// Brave with a session, user or application key, else the
    /// organization's first Brave account.
    Brave,
    /// One organization search account.
    Account { id: Uuid },
    /// Keyless free search.
    Free,
}

impl PlanEntry {
    /// Stable id recorded in the plan: `native`, `brave`, `account:<id>`, `free`.
    pub fn id(&self) -> String {
        match self {
            Self::Native => "native".into(),
            Self::Brave => "brave".into(),
            Self::Account { id } => format!("account:{id}"),
            Self::Free => "free".into(),
        }
    }
}

/// What a plan is resolved from.
#[derive(Clone, Copy)]
pub struct PlanInputs<'a> {
    pub config: &'a WebSearchConfig,
    /// The organization's defaults and accounts; `None` outside an
    /// organization (Framework, serve), which means the built-in defaults.
    pub org: Option<&'a WebSearchOrgSettings>,
    /// The model running this turn has search of its own. At tool time this
    /// is `false`: had the model searched natively, the tool would not run.
    pub model_has_native: bool,
    /// A Brave key from the session, the user or the application exists.
    pub brave_key: bool,
}

/// A plan with the settings it was resolved under.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResolvedPlan {
    /// The setting in effect after following the organization default.
    pub provider: ProviderSetting,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_id: Option<Uuid>,
    pub fallback: FallbackMode,
    pub entries: Vec<PlanEntry>,
    /// Results per search when the model does not ask for a number.
    pub limit: u32,
    /// Organization blocked domains, applied on top of the agent's.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub org_blocked_domains: Vec<String>,
    /// Whether the organization allows model provider search.
    pub allow_native: bool,
}

impl ResolvedPlan {
    pub fn ids(&self) -> Vec<String> {
        self.entries.iter().map(PlanEntry::id).collect()
    }
}

/// Resolve the agent's setting against the organization's defaults.
pub fn resolve_plan(inputs: PlanInputs<'_>) -> ResolvedPlan {
    let config = inputs.config;
    let defaults = inputs.org.map(|org| &org.defaults);
    let (provider, account_id) = match config.provider {
        ProviderSetting::OrgDefault => defaults.map_or((ProviderSetting::Auto, None), |d| {
            (d.provider, d.account_id)
        }),
        provider => (provider, config.account_id),
    };
    let fallback = config
        .fallback
        .or(defaults.map(|d| d.fallback))
        .unwrap_or_default();
    let allow_native = defaults.is_none_or(|d| d.allow_native);
    let native = allow_native && inputs.model_has_native;
    let mut entries = match (provider, account_id) {
        (ProviderSetting::Auto | ProviderSetting::OrgDefault, _) => {
            let mut entries = automatic(inputs, native);
            if fallback == FallbackMode::None {
                entries.truncate(1);
            }
            entries
        }
        (ProviderSetting::Native, _) => vec![PlanEntry::Native],
        (ProviderSetting::Brave, _) => vec![PlanEntry::Brave],
        (ProviderSetting::Account, Some(id)) => vec![PlanEntry::Account { id }],
        // Validation keeps an account setting from losing its id; a stray one
        // degrades to free search.
        (ProviderSetting::Account, None) | (ProviderSetting::Free, _) => vec![PlanEntry::Free],
    };
    if fallback == FallbackMode::Next && !entries.contains(&PlanEntry::Free) {
        entries.push(PlanEntry::Free);
    }
    ResolvedPlan {
        provider,
        account_id,
        fallback,
        entries,
        limit: config
            .limit
            .or(defaults.map(|d| d.results_per_search))
            .unwrap_or(WEB_SEARCH_DEFAULT_RESULTS)
            .clamp(1, WEB_SEARCH_MAX_RESULTS),
        org_blocked_domains: defaults
            .map(|d| d.blocked_domains.clone())
            .unwrap_or_default(),
        allow_native,
    }
}

/// Automatic: the organization's order, with native first and unlisted
/// accounts after the listed ones unless the order says otherwise, then a
/// session or application Brave key, then free search, always last.
fn automatic(inputs: PlanInputs<'_>, native: bool) -> Vec<PlanEntry> {
    let accounts: Vec<Uuid> = inputs
        .org
        .map(|org| org.accounts.iter().map(|account| account.id).collect())
        .unwrap_or_default();
    let order: &[String] = inputs
        .org
        .map_or(&[], |org| org.defaults.automatic_order.as_slice());
    let mut entries = Vec::new();
    if native && !order.iter().any(|entry| entry == AUTOMATIC_NATIVE) {
        entries.push(PlanEntry::Native);
    }
    for entry in order {
        match entry.as_str() {
            AUTOMATIC_NATIVE if native => entries.push(PlanEntry::Native),
            AUTOMATIC_NATIVE | AUTOMATIC_FREE => {}
            other => {
                if let Ok(id) = Uuid::parse_str(other)
                    && accounts.contains(&id)
                {
                    entries.push(PlanEntry::Account { id });
                }
            }
        }
    }
    for id in accounts {
        let entry = PlanEntry::Account { id };
        if !entries.contains(&entry) {
            entries.push(entry);
        }
    }
    if inputs.brave_key {
        entries.push(PlanEntry::Brave);
    }
    entries.push(PlanEntry::Free);
    entries
}

/// The drivers a plan can use for this call.
#[derive(Clone)]
pub struct PlanDrivers {
    pub free: Arc<dyn SearchDriver>,
    /// Present when a session, user or application Brave key is available.
    pub brave: Option<Arc<dyn SearchDriver>>,
    /// Organization search accounts this host can search with, by id, with
    /// their connection provider.
    pub accounts: Vec<(Uuid, String, Arc<dyn SearchDriver>)>,
}

impl PlanDrivers {
    fn account(&self, id: Uuid) -> Option<&Arc<dyn SearchDriver>> {
        self.accounts
            .iter()
            .find(|(account, _, _)| *account == id)
            .map(|(_, _, driver)| driver)
    }
}

/// One entry of a plan.
#[derive(Clone)]
pub enum PlanStep {
    Run(Arc<dyn SearchDriver>),
    /// A provider the setting names that cannot run now; recorded as a failed
    /// attempt so people see why the next one answered.
    Unavailable {
        id: String,
        label: String,
        failure: SearchFailure,
    },
}

impl PlanStep {
    pub fn id(&self) -> &str {
        match self {
            Self::Run(driver) => driver.id(),
            Self::Unavailable { id, .. } => id,
        }
    }

    fn unavailable(id: impl Into<String>, label: &str, message: &str) -> Self {
        Self::Unavailable {
            id: id.into(),
            label: label.into(),
            failure: SearchFailure::new(SearchFailureKind::NotConfigured, message),
        }
    }
}

/// Attach drivers to a resolved plan.
pub fn plan_steps(plan: &ResolvedPlan, drivers: &PlanDrivers) -> Vec<PlanStep> {
    plan.entries
        .iter()
        .map(|entry| match entry {
            PlanEntry::Free => PlanStep::Run(drivers.free.clone()),
            PlanEntry::Native => PlanStep::unavailable(
                "native",
                "Model provider search",
                if plan.allow_native {
                    "The model running this turn has no search of its own"
                } else {
                    "Model provider search is turned off for this organization"
                },
            ),
            PlanEntry::Brave => drivers
                .brave
                .clone()
                .or_else(|| {
                    drivers
                        .accounts
                        .iter()
                        .find(|(_, provider, _)| {
                            provider == super::BRAVE_SEARCH_CONNECTION_PROVIDER
                        })
                        .map(|(_, _, driver)| driver.clone())
                })
                .map_or_else(
                    || {
                        PlanStep::unavailable(
                            "brave",
                            "Brave Search",
                            "No Brave Search key is connected",
                        )
                    },
                    PlanStep::Run,
                ),
            PlanEntry::Account { id } => drivers.account(*id).cloned().map_or_else(
                || {
                    PlanStep::unavailable(
                        entry.id(),
                        "Search account",
                        "The search account is not connected in this organization",
                    )
                },
                PlanStep::Run,
            ),
        })
        .collect()
}

/// One provider's try, as people see it in the details view.
#[derive(Debug, Clone, Serialize)]
pub struct Attempt {
    pub provider: String,
    pub label: String,
    pub ok: bool,
    pub duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_kind: Option<SearchFailureKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
    /// Requests and verbatim responses, for the raw view.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub exchanges: Vec<SearchExchange>,
}

/// What running a plan produced.
pub struct SearchOutcome {
    /// `(provider id, label)` of the provider that answered.
    pub answered_by: Option<(String, String)>,
    pub response: Option<SearchResponse>,
    pub attempts: Vec<Attempt>,
}

impl SearchOutcome {
    /// One sentence per failed attempt before the answer, for the model when
    /// `show_provider` is on and for narration.
    pub fn fallback_notes(&self) -> Vec<String> {
        let answered = self.answered_by.as_ref().map(|(_, label)| label.as_str());
        self.attempts
            .iter()
            .filter(|attempt| !attempt.ok)
            .map(|attempt| match answered {
                Some(answered) => format!(
                    "{}: {}; answered by {answered}.",
                    attempt.label,
                    attempt.message.as_deref().unwrap_or("failed")
                ),
                None => format!(
                    "{}: {}.",
                    attempt.label,
                    attempt.message.as_deref().unwrap_or("failed")
                ),
            })
            .collect()
    }
}

/// Keep an attempt's raw responses within [`WEB_SEARCH_RAW_CAP_BYTES`] in
/// total, cutting the later ones, so one search cannot bloat the event log.
fn capped(mut exchanges: Vec<SearchExchange>) -> Vec<SearchExchange> {
    let mut budget = WEB_SEARCH_RAW_CAP_BYTES;
    for exchange in &mut exchanges {
        let Some(body) = &mut exchange.body else {
            continue;
        };
        if body.len() > budget {
            let mut cut = budget;
            while !body.is_char_boundary(cut) {
                cut -= 1;
            }
            body.truncate(cut);
            exchange.truncated = true;
        }
        budget -= body.len();
    }
    exchanges
}

/// Try each step in order until one answers.
pub async fn run_plan(
    steps: &[PlanStep],
    request: &SearchRequest,
    context: &SearchContext,
) -> SearchOutcome {
    let mut attempts = Vec::new();
    for step in steps {
        let driver = match step {
            PlanStep::Run(driver) => driver,
            PlanStep::Unavailable { id, label, failure } => {
                attempts.push(Attempt {
                    provider: id.clone(),
                    label: label.clone(),
                    ok: false,
                    duration_ms: 0,
                    result_count: None,
                    failure_kind: Some(failure.kind),
                    message: Some(failure.message.clone()),
                    notes: Vec::new(),
                    exchanges: Vec::new(),
                });
                continue;
            }
        };
        let started = Instant::now();
        let result = driver.search(request, context).await;
        let duration_ms = started.elapsed().as_millis() as u64;
        match result {
            Ok(mut response) => {
                attempts.push(Attempt {
                    provider: driver.id().into(),
                    label: driver.label().into(),
                    ok: true,
                    duration_ms,
                    result_count: Some(response.hits.len()),
                    failure_kind: None,
                    message: None,
                    notes: response.notes.clone(),
                    exchanges: capped(std::mem::take(&mut response.exchanges)),
                });
                return SearchOutcome {
                    answered_by: Some((driver.id().into(), driver.label().into())),
                    response: Some(response),
                    attempts,
                };
            }
            Err(failure) => attempts.push(Attempt {
                provider: driver.id().into(),
                label: driver.label().into(),
                ok: false,
                duration_ms,
                result_count: None,
                failure_kind: Some(failure.kind),
                message: Some(failure.message),
                notes: Vec::new(),
                exchanges: capped(failure.exchanges),
            }),
        }
    }
    SearchOutcome {
        answered_by: None,
        response: None,
        attempts,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use everruns_contracts::runtime::web_search::{SearchDriverCapabilities, SearchHit};

    struct Fake {
        id: &'static str,
        result: Result<usize, SearchFailureKind>,
    }

    #[async_trait]
    impl SearchDriver for Fake {
        fn id(&self) -> &str {
            self.id
        }
        fn label(&self) -> &str {
            self.id
        }
        fn capabilities(&self) -> SearchDriverCapabilities {
            SearchDriverCapabilities::default()
        }
        fn egress_hosts(&self) -> &'static [&'static str] {
            &[]
        }
        async fn search(
            &self,
            _request: &SearchRequest,
            _context: &SearchContext,
        ) -> Result<SearchResponse, SearchFailure> {
            match self.result {
                Ok(n) => Ok(SearchResponse {
                    hits: (0..n)
                        .map(|i| SearchHit::new("t", format!("https://e.com/{i}")))
                        .collect(),
                    ..Default::default()
                }),
                Err(kind) => Err(SearchFailure::new(kind, "rate limited")),
            }
        }
    }

    fn drivers(brave: Option<Result<usize, SearchFailureKind>>) -> PlanDrivers {
        PlanDrivers {
            free: Arc::new(Fake {
                id: "free",
                result: Ok(2),
            }),
            brave: brave.map(|result| {
                Arc::new(Fake {
                    id: "brave",
                    result,
                }) as Arc<_>
            }),
            accounts: Vec::new(),
        }
    }

    fn plan(config: &WebSearchConfig, drivers: &PlanDrivers) -> Vec<PlanStep> {
        let resolved = resolve_plan(PlanInputs {
            config,
            org: None,
            model_has_native: false,
            brave_key: drivers.brave.is_some(),
        });
        plan_steps(&resolved, drivers)
    }

    const A: Uuid = Uuid::from_u128(0xa);
    const B: Uuid = Uuid::from_u128(0xb);

    fn org(defaults: serde_json::Value) -> WebSearchOrgSettings {
        WebSearchOrgSettings {
            defaults: serde_json::from_value(defaults).unwrap(),
            accounts: [A, B]
                .into_iter()
                .map(
                    |id| everruns_contracts::runtime::web_search::SearchAccount {
                        id,
                        provider: "brave_search".into(),
                        name: format!("{id}"),
                    },
                )
                .collect(),
        }
    }

    /// Agent setting x organization defaults x native available x session key.
    #[test]
    fn plans_by_setting_org_defaults_and_native() {
        let a = format!("account:{A}");
        let b = format!("account:{B}");
        let with_accounts = org(serde_json::json!({}));
        let b_first = org(serde_json::json!({"automatic_order": [B.to_string()]}));
        let no_native = org(serde_json::json!({"allow_native": false}));
        let free_default = org(serde_json::json!({"provider": "free"}));
        let account_default = org(serde_json::json!({"provider": "account", "account_id": B}));
        let stop = org(serde_json::json!({"fallback": "none"}));
        let native_last = org(serde_json::json!({"automatic_order": [A.to_string(), "native"]}));
        let empty = WebSearchOrgSettings::default();
        use serde_json::json;
        // (agent config, org settings, model has native, Brave key, expected plan)
        type Case<'a> = (
            serde_json::Value,
            Option<&'a WebSearchOrgSettings>,
            bool,
            bool,
            Vec<&'a str>,
        );
        #[rustfmt::skip]
        let cases: Vec<Case> = vec![
            // No organization: built-in Automatic.
            (json!({}), None, false, false, vec!["free"]),
            (json!({}), None, false, true, vec!["brave", "free"]),
            (json!({}), None, true, false, vec!["native", "free"]),
            // Organization default, new org (Automatic, no accounts).
            (json!({}), Some(&empty), true, false, vec!["native", "free"]),
            (json!({}), Some(&empty), false, false, vec!["free"]),
            // Accounts join Automatic in listing order, after native.
            (json!({}), Some(&with_accounts), true, false, vec!["native", &a, &b, "free"]),
            (json!({}), Some(&with_accounts), false, true, vec![&a, &b, "brave", "free"]),
            // The order is an order: listed first, unlisted after.
            (json!({}), Some(&b_first), true, false, vec!["native", &b, &a, "free"]),
            (json!({}), Some(&native_last), true, false, vec![&a, "native", &b, "free"]),
            // Native off for the org.
            (json!({}), Some(&no_native), true, false, vec![&a, &b, "free"]),
            (json!({"provider": "native"}), Some(&no_native), true, false, vec!["native", "free"]),
            // Org default provider.
            (json!({}), Some(&free_default), true, false, vec!["free"]),
            (json!({}), Some(&account_default), true, false, vec![&b, "free"]),
            // Org fallback none: Automatic keeps its first entry only.
            (json!({}), Some(&stop), true, false, vec!["native"]),
            (json!({}), Some(&stop), false, false, vec![&a]),
            // The agent's own fallback wins over the org's.
            (json!({"fallback": "next", "provider": "native"}), Some(&stop), false, false, vec!["native", "free"]),
            (json!({"provider": "auto", "fallback": "none"}), None, false, false, vec!["free"]),
            // Explicit settings ignore the org default provider.
            (json!({"provider": "brave"}), Some(&free_default), false, false, vec!["brave", "free"]),
            (json!({"provider": "brave", "fallback": "none"}), None, false, true, vec!["brave"]),
            (json!({"provider": "account", "account_id": A}), Some(&with_accounts), true, false, vec![&a, "free"]),
            (json!({"provider": "native"}), None, false, false, vec!["native", "free"]),
            (json!({"provider": "native", "fallback": "none"}), None, false, false, vec!["native"]),
            (json!({"provider": "free"}), Some(&with_accounts), true, true, vec!["free"]),
        ];
        for (config, org, native, brave_key, expected) in cases {
            let parsed = WebSearchConfig::from_value(&config).unwrap();
            let resolved = resolve_plan(PlanInputs {
                config: &parsed,
                org,
                model_has_native: native,
                brave_key,
            });
            assert_eq!(
                resolved.ids(),
                expected,
                "{config} native={native} key={brave_key} org={org:?}"
            );
        }
    }

    #[test]
    fn limits_and_blocked_domains_come_from_the_org_unless_the_agent_sets_them() {
        let org = org(serde_json::json!({"results_per_search": 3, "blocked_domains": ["x.com"]}));
        let resolve = |config: serde_json::Value| {
            let config = WebSearchConfig::from_value(&config).unwrap();
            resolve_plan(PlanInputs {
                config: &config,
                org: Some(&org),
                model_has_native: false,
                brave_key: false,
            })
        };
        assert_eq!(resolve(serde_json::json!({})).limit, 3);
        assert_eq!(resolve(serde_json::json!({"limit": 5})).limit, 5);
        assert_eq!(
            resolve(serde_json::json!({})).org_blocked_domains,
            ["x.com"]
        );
    }

    #[test]
    fn steps_name_why_a_provider_cannot_run() {
        let config = WebSearchConfig::from_value(
            &serde_json::json!({"provider": "account", "account_id": A}),
        )
        .unwrap();
        let org = org(serde_json::json!({"allow_native": false}));
        let resolved = resolve_plan(PlanInputs {
            config: &config,
            org: Some(&org),
            model_has_native: false,
            brave_key: false,
        });
        // The account exists in the org but this host has no driver for it.
        let steps = plan_steps(&resolved, &drivers(None));
        let PlanStep::Unavailable { failure, .. } = &steps[0] else {
            panic!("expected an unavailable account step");
        };
        assert!(failure.message.contains("not connected"));
        // An org Brave account answers for the Brave setting without a session key.
        let mut with_account = drivers(None);
        with_account.accounts.push((
            A,
            "brave_search".into(),
            Arc::new(Fake {
                id: "account",
                result: Ok(1),
            }),
        ));
        let brave = WebSearchConfig::from_value(&serde_json::json!({"provider": "brave"})).unwrap();
        assert_eq!(plan(&brave, &with_account)[0].id(), "account");
        let native =
            WebSearchConfig::from_value(&serde_json::json!({"provider": "native"})).unwrap();
        let resolved = resolve_plan(PlanInputs {
            config: &native,
            org: Some(&org),
            model_has_native: true,
            brave_key: false,
        });
        let PlanStep::Unavailable { failure, .. } = &plan_steps(&resolved, &with_account)[0] else {
            panic!("native never runs at tool time");
        };
        assert!(failure.message.contains("turned off"));
    }

    fn context() -> SearchContext {
        SearchContext {
            egress: Arc::new(everruns_contracts::runtime::DisabledEgressService),
            network_access: None,
        }
    }

    #[tokio::test]
    async fn falls_back_and_records_every_attempt() {
        let drivers = drivers(Some(Err(SearchFailureKind::RateLimited)));
        let plan = plan(
            &WebSearchConfig::from_value(&serde_json::json!({"provider": "brave"})).unwrap(),
            &drivers,
        );
        let outcome = run_plan(&plan, &SearchRequest::new("q"), &context()).await;
        assert_eq!(outcome.answered_by.unwrap().0, "free");
        assert_eq!(outcome.attempts.len(), 2);
        assert_eq!(
            outcome.attempts[0].failure_kind,
            Some(SearchFailureKind::RateLimited)
        );
        assert_eq!(outcome.attempts[1].result_count, Some(2));
    }

    #[test]
    fn raw_responses_share_one_budget_per_attempt() {
        let big = |n: usize| SearchExchange::get("u").with_body(200, &vec![b'x'; n]);
        let exchanges = capped(vec![big(WEB_SEARCH_RAW_CAP_BYTES - 10), big(100), big(100)]);
        let total: usize = exchanges
            .iter()
            .filter_map(|exchange| exchange.body.as_ref())
            .map(String::len)
            .sum();
        assert_eq!(total, WEB_SEARCH_RAW_CAP_BYTES);
        assert!(!exchanges[0].truncated);
        assert!(exchanges[1].truncated);
        assert_eq!(exchanges[2].body.as_deref(), Some(""));
    }

    #[tokio::test]
    async fn no_fallback_stops_at_the_named_provider() {
        let drivers = drivers(None);
        let plan = plan(
            &WebSearchConfig::from_value(
                &serde_json::json!({"provider": "brave", "fallback": "none"}),
            )
            .unwrap(),
            &drivers,
        );
        let outcome = run_plan(&plan, &SearchRequest::new("q"), &context()).await;
        assert!(outcome.response.is_none());
        assert_eq!(
            outcome.fallback_notes(),
            ["Brave Search: No Brave Search key is connected."]
        );
    }
}

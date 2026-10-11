//! Search plans: the ordered providers one call tries.
//!
//! A plan is resolved live on every call from the agent's setting, so a key
//! connected mid-session is used from the next search. Model provider search
//! never reaches this code: when the model runs it, the driver replaces the
//! function tool, so a plan built here starts after native.

use std::sync::Arc;
use std::time::Instant;

use everruns_contracts::runtime::web_search::{
    SearchContext, SearchDriver, SearchExchange, SearchFailure, SearchFailureKind, SearchRequest,
    SearchResponse, WEB_SEARCH_RAW_CAP_BYTES,
};
use serde::Serialize;

use super::config::{FallbackMode, ProviderSetting, WebSearchConfig};

/// The drivers a plan can use for this call.
#[derive(Clone)]
pub struct PlanDrivers {
    pub free: Arc<dyn SearchDriver>,
    /// Present when a Brave key is available to this session.
    pub brave: Option<Arc<dyn SearchDriver>>,
}

/// One entry of a plan.
#[derive(Clone)]
pub enum PlanStep {
    Run(Arc<dyn SearchDriver>),
    /// A provider the setting names that cannot run now; recorded as a failed
    /// attempt so people see why the next one answered.
    Unavailable {
        id: &'static str,
        label: &'static str,
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
}

/// Resolve the agent's setting to an ordered plan.
pub fn resolve_plan(config: &WebSearchConfig, drivers: &PlanDrivers) -> Vec<PlanStep> {
    let free = PlanStep::Run(drivers.free.clone());
    let brave = || match &drivers.brave {
        Some(driver) => PlanStep::Run(driver.clone()),
        None => PlanStep::Unavailable {
            id: "brave",
            label: "Brave Search",
            failure: SearchFailure::new(
                SearchFailureKind::NotConfigured,
                "No Brave Search key is connected",
            ),
        },
    };
    let mut steps = match config.provider {
        // Automatic: native (handled by the model driver), then search
        // accounts, then free search, which is always last.
        ProviderSetting::OrgDefault | ProviderSetting::Auto => {
            let mut steps: Vec<PlanStep> = drivers.brave.iter().map(|_| brave()).collect();
            steps.push(free.clone());
            return steps;
        }
        ProviderSetting::Native => vec![PlanStep::Unavailable {
            id: "native",
            label: "Model provider search",
            failure: SearchFailure::new(
                SearchFailureKind::NotConfigured,
                "The model running this turn has no search of its own",
            ),
        }],
        ProviderSetting::Brave => vec![brave()],
        ProviderSetting::Free => vec![free.clone()],
    };
    if config.fallback == FallbackMode::Next && !steps.iter().any(|step| step.id() == "free") {
        steps.push(free);
    }
    steps
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
        let answered = self
            .answered_by
            .as_ref()
            .map(|(_, label)| label.as_str());
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
                    provider: (*id).into(),
                    label: (*label).into(),
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
            brave: brave.map(|result| Arc::new(Fake { id: "brave", result }) as Arc<_>),
        }
    }

    fn ids(config: serde_json::Value, drivers: &PlanDrivers) -> Vec<String> {
        let config = WebSearchConfig::from_value(&config).unwrap();
        resolve_plan(&config, drivers)
            .iter()
            .map(|step| step.id().to_string())
            .collect()
    }

    #[test]
    fn plans_by_setting() {
        let with_key = drivers(Some(Ok(1)));
        let no_key = drivers(None);
        let cases = [
            (serde_json::json!({}), &with_key, vec!["brave", "free"]),
            (serde_json::json!({}), &no_key, vec!["free"]),
            (serde_json::json!({"provider": "auto", "fallback": "none"}), &no_key, vec!["free"]),
            (serde_json::json!({"provider": "brave"}), &with_key, vec!["brave", "free"]),
            (serde_json::json!({"provider": "brave"}), &no_key, vec!["brave", "free"]),
            (
                serde_json::json!({"provider": "brave", "fallback": "none"}),
                &with_key,
                vec!["brave"],
            ),
            (serde_json::json!({"provider": "native"}), &no_key, vec!["native", "free"]),
            (
                serde_json::json!({"provider": "native", "fallback": "none"}),
                &no_key,
                vec!["native"],
            ),
            (serde_json::json!({"provider": "free"}), &with_key, vec!["free"]),
        ];
        for (config, drivers, expected) in cases {
            assert_eq!(ids(config.clone(), drivers), expected, "{config}");
        }
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
        let plan = resolve_plan(
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
        let exchanges = capped(vec![
            big(WEB_SEARCH_RAW_CAP_BYTES - 10),
            big(100),
            big(100),
        ]);
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
        let plan = resolve_plan(
            &WebSearchConfig::from_value(&serde_json::json!({"provider": "brave", "fallback": "none"}))
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

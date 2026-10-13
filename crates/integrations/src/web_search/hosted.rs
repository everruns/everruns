//! The deployment's own search: a provider the host pays for and offers every
//! organization without a key (on app.everruns.com, Everruns search).
//!
//! Decision: the host hands over any `SearchDriver`, configured with its own
//! key, and this wraps it under the stable plan id `hosted`. The id is what
//! the Automatic order stores and what the host's billing counts in each
//! search's recorded attempts, so it must not change with the backend. The
//! wrapper drops the backend's raw exchanges: they belong to the host's
//! account (a Cloudflare URL carries its account id), not to the tenant.
//!
//! Decision: the host pays for every hosted search, so it may refuse one
//! before it runs (an organization out of credit) through a
//! [`HostedSearchGate`]. A refusal is an ordinary failed attempt with the
//! gate's reason, and the plan moves on to its next provider.

use std::sync::Arc;

use async_trait::async_trait;
use everruns_contracts::runtime::web_search::{
    AUTOMATIC_HOSTED, SearchContext, SearchDriver, SearchDriverCapabilities, SearchFailure,
    SearchFailureKind, SearchRequest, SearchResponse,
};
use everruns_contracts::typed_id::OrgId;

/// Plan id of the hosted search, also its Automatic order entry.
pub const HOSTED_SEARCH_ID: &str = AUTOMATIC_HOSTED;

/// Decides, per search, whether an organization may use the hosted search.
#[async_trait]
pub trait HostedSearchGate: Send + Sync {
    /// `Err` carries a plain-words reason people see in the search's details.
    async fn admit(&self, org: Option<&OrgId>) -> Result<(), String>;
}

/// A search provider the deployment offers to every organization.
#[derive(Clone)]
pub struct HostedWebSearch {
    name: String,
    description: String,
    price_note: Option<String>,
    driver: Arc<dyn SearchDriver>,
    gate: Option<Arc<dyn HostedSearchGate>>,
}

impl HostedWebSearch {
    /// `name` is what people see ("Everruns search"); `driver` runs the
    /// searches with the deployment's own key.
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        driver: Arc<dyn SearchDriver>,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            price_note: None,
            driver,
            gate: None,
        }
    }

    /// Check each search against `gate` before it runs.
    pub fn with_gate(mut self, gate: Arc<dyn HostedSearchGate>) -> Self {
        self.gate = Some(gate);
        self
    }

    /// Plain-words price shown next to the provider.
    pub fn with_price_note(mut self, note: impl Into<String>) -> Self {
        self.price_note = Some(note.into());
        self
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn description(&self) -> &str {
        &self.description
    }

    pub fn price_note(&self) -> Option<&str> {
        self.price_note.as_deref()
    }

    /// The driver a plan runs: the backend under the `hosted` id and name.
    pub fn driver(&self) -> Arc<dyn SearchDriver> {
        Arc::new(HostedDriver {
            name: self.name.clone(),
            inner: self.driver.clone(),
        })
    }

    /// The driver for one search by `org`: [`Self::driver`] when the gate
    /// admits it (or there is none), else one that fails with its reason.
    pub async fn driver_for(&self, org: Option<&OrgId>) -> Arc<dyn SearchDriver> {
        let refusal = match &self.gate {
            Some(gate) => gate.admit(org).await.err(),
            None => None,
        };
        match refusal {
            None => self.driver(),
            Some(reason) => Arc::new(RefusedDriver {
                name: self.name.clone(),
                reason,
                inner: self.driver.clone(),
            }),
        }
    }
}

struct RefusedDriver {
    name: String,
    reason: String,
    inner: Arc<dyn SearchDriver>,
}

#[async_trait]
impl SearchDriver for RefusedDriver {
    fn id(&self) -> &str {
        HOSTED_SEARCH_ID
    }

    fn label(&self) -> &str {
        &self.name
    }

    fn capabilities(&self) -> SearchDriverCapabilities {
        self.inner.capabilities()
    }

    fn egress_hosts(&self) -> &'static [&'static str] {
        &[]
    }

    async fn search(
        &self,
        _: &SearchRequest,
        _: &SearchContext,
    ) -> Result<SearchResponse, SearchFailure> {
        Err(SearchFailure::new(
            SearchFailureKind::Unavailable,
            self.reason.clone(),
        ))
    }
}

struct HostedDriver {
    name: String,
    inner: Arc<dyn SearchDriver>,
}

#[async_trait]
impl SearchDriver for HostedDriver {
    fn id(&self) -> &str {
        HOSTED_SEARCH_ID
    }

    fn label(&self) -> &str {
        &self.name
    }

    fn capabilities(&self) -> SearchDriverCapabilities {
        self.inner.capabilities()
    }

    fn egress_hosts(&self) -> &'static [&'static str] {
        self.inner.egress_hosts()
    }

    async fn search(
        &self,
        request: &SearchRequest,
        context: &SearchContext,
    ) -> Result<SearchResponse, SearchFailure> {
        match self.inner.search(request, context).await {
            Ok(mut response) => {
                response.exchanges.clear();
                Ok(response)
            }
            Err(mut failure) => {
                failure.exchanges.clear();
                Err(failure)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_contracts::runtime::web_search::{SearchExchange, SearchFailureKind, SearchHit};

    struct Backend {
        fail: bool,
    }

    #[async_trait]
    impl SearchDriver for Backend {
        fn id(&self) -> &str {
            "brave"
        }
        fn label(&self) -> &str {
            "Brave Search"
        }
        fn capabilities(&self) -> SearchDriverCapabilities {
            SearchDriverCapabilities {
                freshness: true,
                domain_filters: true,
                excerpts: false,
            }
        }
        fn egress_hosts(&self) -> &'static [&'static str] {
            &["api.search.brave.com"]
        }
        async fn search(
            &self,
            _: &SearchRequest,
            _: &SearchContext,
        ) -> Result<SearchResponse, SearchFailure> {
            let exchange = SearchExchange::get("https://api.example/accounts/secret-account");
            if self.fail {
                return Err(
                    SearchFailure::new(SearchFailureKind::RateLimited, "slow down")
                        .with_exchanges(vec![exchange]),
                );
            }
            Ok(SearchResponse {
                hits: vec![SearchHit::new("Tokio", "https://tokio.rs/")],
                exchanges: vec![exchange],
                ..Default::default()
            })
        }
    }

    fn context() -> SearchContext {
        SearchContext {
            egress: Arc::new(super::super::http::DirectEgress::new()),
            network_access: None,
        }
    }

    #[tokio::test]
    async fn runs_the_backend_under_the_hosted_id_without_its_exchanges() {
        let hosted = HostedWebSearch::new(
            "Everruns search",
            "Search included with Everruns",
            Arc::new(Backend { fail: false }),
        )
        .with_price_note("$6 per 1,000 searches");
        assert_eq!(hosted.price_note(), Some("$6 per 1,000 searches"));
        let driver = hosted.driver();
        assert_eq!(driver.id(), "hosted");
        assert_eq!(driver.label(), "Everruns search");
        assert!(driver.capabilities().freshness);
        assert_eq!(driver.egress_hosts(), ["api.search.brave.com"]);
        let response = driver
            .search(&SearchRequest::new("tokio"), &context())
            .await
            .unwrap();
        assert_eq!(response.hits.len(), 1);
        assert!(response.exchanges.is_empty());
    }

    #[tokio::test]
    async fn a_failure_keeps_its_kind_and_drops_its_exchanges() {
        let driver =
            HostedWebSearch::new("Everruns search", "", Arc::new(Backend { fail: true })).driver();
        let failure = driver
            .search(&SearchRequest::new("tokio"), &context())
            .await
            .unwrap_err();
        assert_eq!(failure.kind, SearchFailureKind::RateLimited);
        assert!(failure.exchanges.is_empty());
    }

    struct OutOfCredit;

    #[async_trait]
    impl HostedSearchGate for OutOfCredit {
        async fn admit(&self, org: Option<&OrgId>) -> Result<(), String> {
            match org {
                Some(_) => Err("The organization is out of credit".into()),
                None => Ok(()),
            }
        }
    }

    #[tokio::test]
    async fn a_refused_search_fails_with_the_gates_reason_and_never_reaches_the_backend() {
        let hosted = HostedWebSearch::new("Everruns search", "", Arc::new(Backend { fail: false }))
            .with_gate(Arc::new(OutOfCredit));
        let org = OrgId::new();
        let refused = hosted.driver_for(Some(&org)).await;
        assert_eq!(refused.id(), "hosted");
        let failure = refused
            .search(&SearchRequest::new("tokio"), &context())
            .await
            .unwrap_err();
        assert_eq!(failure.kind, SearchFailureKind::Unavailable);
        assert_eq!(failure.message, "The organization is out of credit");
        assert!(failure.exchanges.is_empty());

        let admitted = hosted.driver_for(None).await;
        let response = admitted
            .search(&SearchRequest::new("tokio"), &context())
            .await
            .unwrap();
        assert_eq!(response.hits.len(), 1);
    }
}

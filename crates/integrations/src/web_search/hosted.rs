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

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use everruns_contracts::runtime::web_search::{
    AUTOMATIC_HOSTED, SearchContext, SearchDriver, SearchDriverCapabilities, SearchFailure,
    SearchFailureKind, SearchRequest, SearchResponse,
};
use everruns_contracts::typed_id::{OrgId, SessionId};
use serde::{Deserialize, Serialize};

/// Engine identity, stable across resume, supplied only by the host context.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostedSearchCall {
    pub org_id: OrgId,
    pub session_id: SessionId,
    pub idempotency_key: uuid::Uuid,
}

/// A sanitized outcome; uncertain delivery must never be treated as free.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", content = "result", rename_all = "snake_case")]
pub enum HostedSearchCompletion {
    Answered(SearchResponse),
    NotDispatched(SearchFailure),
    Unknown(SearchFailure),
}

/// Ownership of one admitted call. Start fences duplicate or expired owners.
#[async_trait]
pub trait HostedSearchPermit: Send + Sync {
    fn reference(&self) -> String;
    async fn start(&self) -> Result<(), String>;
    async fn complete(&self, outcome: &HostedSearchCompletion) -> Result<(), String>;
}

pub enum HostedSearchAdmission {
    Run(Arc<dyn HostedSearchPermit>),
    Replay {
        reference: String,
        outcome: Result<SearchResponse, SearchFailure>,
    },
}

/// Optional official embedding hook for atomic admission and settlement.
/// It runs only if the hosted step is actually attempted, never during planning.
#[async_trait]
pub trait HostedSearchPermitGate: Send + Sync {
    async fn acquire(
        &self,
        call: &HostedSearchCall,
        request: &SearchRequest,
    ) -> Result<HostedSearchAdmission, String>;
}

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
    permit_gate: Option<Arc<dyn HostedSearchPermitGate>>,
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
            permit_gate: None,
        }
    }

    /// Check each search against `gate` before it runs.
    pub fn with_gate(mut self, gate: Arc<dyn HostedSearchGate>) -> Self {
        self.gate = Some(gate);
        self
    }

    pub fn with_permit_gate(mut self, gate: Arc<dyn HostedSearchPermitGate>) -> Self {
        self.permit_gate = Some(gate);
        self
    }

    /// Build one lazy invocation; planning alone acquires no permit.
    pub fn driver_for_call(&self, call: Option<HostedSearchCall>) -> Arc<HostedSearchInvocation> {
        let org = call.as_ref().map(|call| call.org_id);
        self.invocation(org, call)
    }

    fn invocation(
        &self,
        org: Option<OrgId>,
        call: Option<HostedSearchCall>,
    ) -> Arc<HostedSearchInvocation> {
        Arc::new(HostedSearchInvocation {
            name: self.name.clone(),
            inner: self.driver.clone(),
            gate: self.permit_gate.clone(),
            permission: self.gate.clone(),
            org,
            call,
            reference: Mutex::new(None),
        })
    }

    pub(super) fn driver_for_context(
        &self,
        context: Option<&everruns_contracts::runtime::tool_context::ToolContext>,
    ) -> Arc<HostedSearchInvocation> {
        let call = context.and_then(|context| {
            Some(HostedSearchCall {
                org_id: context.org_id?,
                session_id: context.session_id,
                idempotency_key: context.idempotency_key.as_ref()?.parse().ok()?,
            })
        });
        self.invocation(context.and_then(|context| context.org_id), call)
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
        if self.permit_gate.is_some() {
            return Arc::new(RefusedDriver {
                name: self.name.clone(),
                reason: "Hosted search requires a stable call identity".into(),
                inner: self.driver.clone(),
            });
        }
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

/// One engine call, not a reusable global driver. The permit starts before
/// dispatch, and its sanitized outcome reaches durable host settlement.
/// Cancellation or ambiguous delivery leaves reconciliation to the host;
/// dropping a future is never proof that a supplier charged nothing.
pub struct HostedSearchInvocation {
    name: String,
    inner: Arc<dyn SearchDriver>,
    gate: Option<Arc<dyn HostedSearchPermitGate>>,
    org: Option<OrgId>,
    permission: Option<Arc<dyn HostedSearchGate>>,
    call: Option<HostedSearchCall>,
    reference: Mutex<Option<String>>,
}

impl HostedSearchInvocation {
    /// Safe-to-record receipt belonging only to this hosted invocation.
    pub fn billing_reference(&self) -> Option<String> {
        self.reference
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }
}

#[async_trait]
impl SearchDriver for HostedSearchInvocation {
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
        fn unavailable(reason: impl Into<String>) -> SearchFailure {
            SearchFailure::new(SearchFailureKind::Unavailable, reason)
        }
        if let Some(permission) = &self.permission {
            permission
                .admit(self.org.as_ref())
                .await
                .map_err(unavailable)?;
        }
        let Some(gate) = &self.gate else {
            let mut outcome = self.inner.search(request, context).await;
            sanitize(&mut outcome);
            return outcome;
        };
        let Some(call) = &self.call else {
            return Err(unavailable("Hosted search requires a stable call identity"));
        };
        if call.idempotency_key.is_nil() {
            return Err(unavailable("Hosted search requires a stable call identity"));
        }
        let admission = gate.acquire(call, request).await.map_err(unavailable)?;
        let permit = match admission {
            HostedSearchAdmission::Replay {
                reference,
                mut outcome,
            } => {
                *self.reference.lock().unwrap_or_else(|p| p.into_inner()) = Some(reference);
                sanitize(&mut outcome);
                return outcome;
            }
            HostedSearchAdmission::Run(permit) => permit,
        };
        *self.reference.lock().unwrap_or_else(|p| p.into_inner()) = Some(permit.reference());
        if let Err(reason) = permit.start().await {
            let failure = unavailable(reason);
            if let Err(error) = permit
                .complete(&HostedSearchCompletion::NotDispatched(failure.clone()))
                .await
            {
                tracing::warn!(%error, "hosted search start cleanup pending");
            }
            return Err(failure);
        }
        let mut outcome = match tokio::time::timeout(
            Duration::from_secs(20),
            self.inner.search(request, context),
        )
        .await
        {
            Ok(outcome) => outcome,
            Err(_) => Err(unavailable("Hosted search timed out")),
        };
        sanitize(&mut outcome);
        let completion = match &outcome {
            Ok(response) => HostedSearchCompletion::Answered(response.clone()),
            Err(failure)
                if matches!(
                    failure.kind,
                    SearchFailureKind::NetworkDenied | SearchFailureKind::NotConfigured
                ) =>
            {
                HostedSearchCompletion::NotDispatched(failure.clone())
            }
            Err(failure) => HostedSearchCompletion::Unknown(failure.clone()),
        };
        if let Err(error) = permit.complete(&completion).await {
            // Preserve an answer for the engine journal; retrying the supplier
            // would duplicate spend. The receipt lets the host repair this.
            tracing::warn!(%error, "hosted search settlement pending");
        }
        outcome
    }
}

fn sanitize(outcome: &mut Result<SearchResponse, SearchFailure>) {
    match outcome {
        Ok(response) => response.exchanges.clear(),
        Err(failure) => failure.exchanges.clear(),
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
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Default)]
    struct Lifecycle {
        acquired: AtomicUsize,
        started: AtomicUsize,
        completed: AtomicUsize,
    }

    struct Permit(Arc<Lifecycle>);

    #[async_trait]
    impl HostedSearchPermit for Permit {
        fn reference(&self) -> String {
            "fixture-permit".into()
        }
        async fn start(&self) -> Result<(), String> {
            self.0.started.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        async fn complete(&self, outcome: &HostedSearchCompletion) -> Result<(), String> {
            let HostedSearchCompletion::Answered(response) = outcome else {
                panic!("expected answer")
            };
            assert!(
                response.exchanges.is_empty(),
                "host exchanges never enter settlement"
            );
            self.0.completed.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    struct PermitGate(Arc<Lifecycle>);

    #[async_trait]
    impl HostedSearchPermitGate for PermitGate {
        async fn acquire(
            &self,
            call: &HostedSearchCall,
            _: &SearchRequest,
        ) -> Result<HostedSearchAdmission, String> {
            assert!(!call.idempotency_key.is_nil());
            self.0.acquired.fetch_add(1, Ordering::SeqCst);
            Ok(HostedSearchAdmission::Run(Arc::new(Permit(self.0.clone()))))
        }
    }

    #[tokio::test]
    async fn permits_are_lazy_and_started_before_the_sanitized_completion() {
        let lifecycle = Arc::new(Lifecycle::default());
        let hosted = HostedWebSearch::new("Hosted", "", Arc::new(Backend { fail: false }))
            .with_permit_gate(Arc::new(PermitGate(lifecycle.clone())));
        let driver = hosted.driver_for_call(Some(HostedSearchCall {
            org_id: OrgId::new(),
            session_id: SessionId::new(),
            idempotency_key: uuid::Uuid::new_v4(),
        }));
        assert_eq!(
            lifecycle.acquired.load(Ordering::SeqCst),
            0,
            "planning spends nothing"
        );
        let response = driver
            .search(&SearchRequest::new("fixture"), &context())
            .await
            .unwrap();
        assert!(response.exchanges.is_empty());
        assert_eq!(lifecycle.acquired.load(Ordering::SeqCst), 1);
        assert_eq!(lifecycle.started.load(Ordering::SeqCst), 1);
        assert_eq!(lifecycle.completed.load(Ordering::SeqCst), 1);
    }

    #[derive(Default)]
    struct Probe {
        calls: AtomicUsize,
        acquired: AtomicUsize,
        starts: AtomicUsize,
        completed: AtomicUsize,
        outcome: Mutex<Option<HostedSearchCompletion>>,
        start_failure: bool,
        callback_failure: bool,
        replay: bool,
        backend_failure: bool,
        pending: bool,
        entered: tokio::sync::Notify,
    }

    struct ProbeGate(Arc<Probe>);
    struct ProbePermit(Arc<Probe>);
    struct ProbeBackend(Arc<Probe>);

    #[async_trait]
    impl HostedSearchPermitGate for ProbeGate {
        async fn acquire(
            &self,
            _: &HostedSearchCall,
            _: &SearchRequest,
        ) -> Result<HostedSearchAdmission, String> {
            self.0.acquired.fetch_add(1, Ordering::SeqCst);
            if self.0.replay {
                return Ok(HostedSearchAdmission::Replay {
                    reference: "probe-hold".into(),
                    outcome: Ok(probe_response()),
                });
            }
            Ok(HostedSearchAdmission::Run(Arc::new(ProbePermit(
                self.0.clone(),
            ))))
        }
    }
    #[async_trait]
    impl HostedSearchPermit for ProbePermit {
        fn reference(&self) -> String {
            "probe-hold".into()
        }
        async fn start(&self) -> Result<(), String> {
            self.0.starts.fetch_add(1, Ordering::SeqCst);
            if self.0.start_failure {
                Err("fenced".into())
            } else {
                Ok(())
            }
        }
        async fn complete(&self, outcome: &HostedSearchCompletion) -> Result<(), String> {
            self.0.completed.fetch_add(1, Ordering::SeqCst);
            *self.0.outcome.lock().unwrap() = Some(outcome.clone());
            if self.0.callback_failure {
                Err("callback unavailable".into())
            } else {
                Ok(())
            }
        }
    }
    #[async_trait]
    impl SearchDriver for ProbeBackend {
        fn id(&self) -> &str {
            "brave"
        }
        fn label(&self) -> &str {
            "Fixture"
        }
        fn capabilities(&self) -> SearchDriverCapabilities {
            Default::default()
        }
        fn egress_hosts(&self) -> &'static [&'static str] {
            &[]
        }
        async fn search(
            &self,
            _: &SearchRequest,
            _: &SearchContext,
        ) -> Result<SearchResponse, SearchFailure> {
            self.0.calls.fetch_add(1, Ordering::SeqCst);
            self.0.entered.notify_one();
            if self.0.pending {
                return std::future::pending().await;
            }
            if self.0.backend_failure {
                return Err(SearchFailure::new(
                    SearchFailureKind::Unavailable,
                    "uncertain delivery",
                )
                .with_exchanges(probe_response().exchanges));
            }
            Ok(probe_response())
        }
    }
    fn probe_response() -> SearchResponse {
        SearchResponse {
            hits: vec![SearchHit::new("Fixture", "https://example.invalid")],
            exchanges: vec![SearchExchange::get(
                "https://example.invalid/private-host-account",
            )],
            ..Default::default()
        }
    }
    fn probe_hosted(probe: &Arc<Probe>) -> HostedWebSearch {
        HostedWebSearch::new("Hosted", "", Arc::new(ProbeBackend(probe.clone())))
            .with_permit_gate(Arc::new(ProbeGate(probe.clone())))
    }
    fn call() -> HostedSearchCall {
        HostedSearchCall {
            org_id: OrgId::new(),
            session_id: SessionId::new(),
            idempotency_key: uuid::Uuid::new_v4(),
        }
    }
    #[tokio::test]
    async fn a_completed_permit_replays_without_starting_or_calling_the_backend() {
        let probe = Arc::new(Probe {
            replay: true,
            ..Default::default()
        });
        let driver = probe_hosted(&probe).driver_for_call(Some(call()));
        let response = driver
            .search(&SearchRequest::new("fixture"), &context())
            .await
            .unwrap();
        assert!(response.exchanges.is_empty());
        assert_eq!(driver.billing_reference().as_deref(), Some("probe-hold"));
        assert_eq!(probe.calls.load(Ordering::SeqCst), 0);
        assert_eq!(probe.starts.load(Ordering::SeqCst), 0);
        assert_eq!(probe.completed.load(Ordering::SeqCst), 0);
    }
    #[tokio::test]
    async fn missing_identity_and_direct_driver_cannot_bypass_a_permit_gate() {
        let probe = Arc::new(Probe::default());
        let hosted = probe_hosted(&probe);
        for driver in [hosted.driver(), hosted.driver_for_call(None)] {
            assert!(
                driver
                    .search(&SearchRequest::new("fixture"), &context())
                    .await
                    .is_err()
            );
        }
        assert_eq!(probe.acquired.load(Ordering::SeqCst), 0);
        assert_eq!(probe.calls.load(Ordering::SeqCst), 0);
    }
    #[tokio::test]
    async fn a_failed_start_never_dispatches_and_reports_proven_no_dispatch() {
        let probe = Arc::new(Probe {
            start_failure: true,
            ..Default::default()
        });
        let driver = probe_hosted(&probe).driver_for_call(Some(call()));
        assert!(
            driver
                .search(&SearchRequest::new("fixture"), &context())
                .await
                .is_err()
        );
        assert_eq!(probe.calls.load(Ordering::SeqCst), 0);
        assert!(matches!(
            *probe.outcome.lock().unwrap(),
            Some(HostedSearchCompletion::NotDispatched(_))
        ));
    }
    #[tokio::test]
    async fn lost_settlement_callback_preserves_answer_and_reconciliation_receipt() {
        let probe = Arc::new(Probe {
            callback_failure: true,
            ..Default::default()
        });
        let driver = probe_hosted(&probe).driver_for_call(Some(call()));
        let response = driver
            .search(&SearchRequest::new("fixture"), &context())
            .await
            .unwrap();
        assert_eq!(response.hits.len(), 1);
        assert!(response.exchanges.is_empty());
        assert_eq!(driver.billing_reference().as_deref(), Some("probe-hold"));
        assert_eq!(probe.calls.load(Ordering::SeqCst), 1);
    }
    #[tokio::test]
    async fn an_uncertain_backend_failure_is_not_reported_as_free() {
        let probe = Arc::new(Probe {
            backend_failure: true,
            ..Default::default()
        });
        let driver = probe_hosted(&probe).driver_for_call(Some(call()));
        assert!(
            driver
                .search(&SearchRequest::new("fixture"), &context())
                .await
                .is_err()
        );
        let guard = probe.outcome.lock().unwrap();
        let Some(HostedSearchCompletion::Unknown(failure)) = &*guard else {
            panic!("unknown outcome must be retained")
        };
        assert!(failure.exchanges.is_empty());
    }
    #[tokio::test]
    async fn cancellation_after_dispatch_does_not_report_a_release() {
        let probe = Arc::new(Probe {
            pending: true,
            ..Default::default()
        });
        let driver = probe_hosted(&probe).driver_for_call(Some(call()));
        let task = tokio::spawn(async move {
            driver
                .search(&SearchRequest::new("fixture"), &context())
                .await
        });
        probe.entered.notified().await;
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert_eq!(probe.starts.load(Ordering::SeqCst), 1);
        assert_eq!(probe.calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            probe.completed.load(Ordering::SeqCst),
            0,
            "the host reconciles; drop is not a free failure"
        );
    }

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

//! Stability: alpha — explicit provider-native offline completions.
//!
//! Build a [`Batch`] with one provider and a stable application tenant/account
//! scope. Submit message lists with [`Batch::submit_requests`] or independent
//! [`Completion`] values made with bare model IDs,
//! persist the returned [`BatchTicket`], then reconstruct a client to poll,
//! collect or cancel it. No Engine, worker, streaming, or tool execution is involved.
//!
//! Tickets are untrusted routing data, never authorization. Authenticate and
//! authorize access to their storage before loading them. Keep the scope stable
//! across credential rotation, and distinct between tenants and provider accounts.

use std::{collections::HashSet, time::Duration};

use serde::{Deserialize, Serialize};

use crate::{Provider, llm::Completion};
use everruns_contracts::batch::endpoint_identity;
pub use everruns_contracts::batch::{
    BatchDriver, BatchError, BatchItemError, BatchRequest, BatchResult, BatchStatus,
};

const TICKET_VERSION: u32 = 1;
const MAX_REQUESTS: usize = 100_000;

/// Credential-free resumable reference. Store it in access-controlled durable
/// application storage. It contains request IDs, never their prompts or results.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BatchTicket {
    version: u32,
    scope: String,
    provider: String,
    endpoint: String,
    format: String,
    job_id: String,
    request_ids: Vec<String>,
}

impl BatchTicket {
    /// Provider job ID, useful for reconciliation in the provider console.
    pub fn job_id(&self) -> &str {
        &self.job_id
    }

    /// Request IDs in submission order.
    pub fn request_ids(&self) -> &[String] {
        &self.request_ids
    }
}

impl std::fmt::Debug for BatchTicket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BatchTicket")
            .field("version", &self.version)
            .field("requests", &self.request_ids.len())
            .finish_non_exhaustive()
    }
}

/// One provider/account's batch client and submission builder.
///
/// `scope` is a trusted application tenant/account identifier, not an API key.
/// Requests are sent only by [`submit`](Self::submit) or
/// [`submit_requests`](Self::submit_requests). Dropping the client or
/// ticket never cancels a provider job. There are no background tasks or retries.
pub struct Batch {
    provider: Provider,
    scope: String,
    requests: Vec<BatchRequest>,
    request_ids: HashSet<String>,
    timeout: Duration,
    max_response_bytes: usize,
}

impl Batch {
    /// Configure one provider and a stable tenant/account scope.
    pub fn new(provider: impl Into<Provider>, scope: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            scope: scope.into(),
            requests: Vec::new(),
            request_ids: HashSet::new(),
            timeout: Duration::from_secs(60),
            max_response_bytes: 256 * 1024 * 1024,
        }
    }

    /// Bound each provider HTTP request, including authentication and reads.
    /// This is not a deadline for the provider's offline processing.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Bound each provider response before it is buffered. Default: 256 MiB.
    pub fn max_response_bytes(mut self, bytes: usize) -> Self {
        self.max_response_bytes = bytes;
        self
    }

    /// Add a completion made with a bare [`crate::Model`]. The batch owns the
    /// provider; a completion with its own provider is rejected rather than rerouted.
    /// IDs must be unique ASCII letters, digits, underscores or hyphens (1–64 bytes).
    pub fn request(
        mut self,
        id: impl Into<String>,
        completion: Completion,
    ) -> Result<Self, BatchError> {
        let id = id.into();
        if !valid_id(&id) || self.request_ids.contains(&id) {
            return Err(BatchError::InvalidRequest(
                "invalid or duplicate request ID",
            ));
        }
        if self.requests.len() >= MAX_REQUESTS {
            return Err(BatchError::InvalidRequest("too many batch requests"));
        }
        let (messages, config) = completion.into_batch_request()?;
        self.request_ids.insert(id.clone());
        self.requests.push(BatchRequest {
            id,
            messages,
            config,
        });
        Ok(self)
    }

    fn driver(&self) -> Result<&dyn BatchDriver, BatchError> {
        if self.scope.is_empty()
            || self.scope.len() > 256
            || self.timeout.is_zero()
            || self.max_response_bytes == 0
        {
            return Err(BatchError::InvalidRequest(
                "scope, timeout and response limit must be nonempty",
            ));
        }
        self.provider
            .driver()
            .map_err(|_| BatchError::Unsupported)?
            .batch_driver()
            .ok_or(BatchError::Unsupported)
    }

    /// Submit once. Persist the ticket before doing any further work.
    /// On [`BatchError::SubmissionUncertain`], reconcile with the provider;
    /// retrying creation can incur duplicate work and charges.
    pub async fn submit(mut self) -> Result<BatchTicket, BatchError> {
        let requests = std::mem::take(&mut self.requests);
        self.submit_batch(requests).await
    }

    /// Submit an explicit list of requests using the shared provider-message
    /// type ([`crate::llm::Message`]), the same contract core's conversion helpers return.
    /// Each request owns its messages and [`crate::LlmCallConfig`]; the client
    /// supplies the provider, credentials and tenant/account scope.
    ///
    /// Reuses the builder's validation, submission and ticket path. The client
    /// remains available to poll, collect or cancel. Buffered builder requests
    /// cannot be mixed with this method: call [`Self::submit`] for those.
    /// Uncertain submissions must be reconciled before another creation attempt.
    pub async fn submit_requests(
        &self,
        requests: Vec<BatchRequest>,
    ) -> Result<BatchTicket, BatchError> {
        if !self.requests.is_empty() {
            return Err(BatchError::InvalidRequest(
                "submit_requests cannot be combined with buffered builder requests",
            ));
        }
        self.submit_batch(requests).await
    }

    async fn submit_batch(&self, requests: Vec<BatchRequest>) -> Result<BatchTicket, BatchError> {
        let driver = self.driver()?;
        if requests.is_empty() {
            return Err(BatchError::InvalidRequest("batch has no requests"));
        }
        if requests.len() > MAX_REQUESTS {
            return Err(BatchError::InvalidRequest("too many batch requests"));
        }
        let mut seen = HashSet::with_capacity(requests.len());
        let mut request_ids = Vec::with_capacity(requests.len());
        for request in &requests {
            if !valid_id(&request.id) || !seen.insert(&request.id) {
                return Err(BatchError::InvalidRequest(
                    "invalid or duplicate request ID",
                ));
            }
            if request.messages.is_empty() {
                return Err(BatchError::InvalidRequest("batch request has no messages"));
            }
            request_ids.push(request.id.clone());
        }
        let job_id = driver
            .submit(
                self.provider.endpoint(),
                requests,
                self.timeout,
                self.max_response_bytes,
            )
            .await?;
        self.ticket(job_id, request_ids)
    }

    fn ticket(&self, job_id: String, request_ids: Vec<String>) -> Result<BatchTicket, BatchError> {
        Ok(BatchTicket {
            version: TICKET_VERSION,
            scope: self.scope.clone(),
            provider: self.provider.id().to_string(),
            endpoint: endpoint_identity(self.provider.endpoint()),
            format: self.driver()?.batch_format().to_owned(),
            job_id,
            request_ids,
        })
    }

    /// Recover a ticket after independently reconciling an uncertain submission
    /// with the provider. Supply the original IDs from your durable intent in
    /// submission order. Checks access to the known job without creating work;
    /// collection still verifies that its results match these IDs exactly.
    pub async fn recover(
        &self,
        job_id: impl Into<String>,
        request_ids: impl IntoIterator<Item = impl Into<String>>,
    ) -> Result<BatchTicket, BatchError> {
        let mut ids = Vec::new();
        let mut seen = HashSet::new();
        for id in request_ids {
            let id = id.into();
            if !valid_id(&id) || !seen.insert(id.clone()) || ids.len() >= MAX_REQUESTS {
                return Err(BatchError::InvalidRequest(
                    "invalid, duplicate or too many recovery request IDs",
                ));
            }
            ids.push(id);
        }
        if ids.is_empty() {
            return Err(BatchError::InvalidRequest(
                "recovery requires the original request IDs",
            ));
        }
        let ticket = self.ticket(job_id.into(), ids)?;
        self.poll(&ticket).await?;
        Ok(ticket)
    }

    fn check(&self, ticket: &BatchTicket) -> Result<&dyn BatchDriver, BatchError> {
        let driver = self.driver()?;
        if ticket.version != TICKET_VERSION
            || ticket.request_ids.is_empty()
            || ticket.request_ids.len() > MAX_REQUESTS
        {
            return Err(BatchError::InvalidTicket);
        }
        let mut seen = HashSet::new();
        if ticket
            .request_ids
            .iter()
            .any(|id| !valid_id(id) || !seen.insert(id))
        {
            return Err(BatchError::InvalidTicket);
        }
        // THREAT[TM-TENANT-021]: Never use a ticket to select credentials or an endpoint.
        if ticket.scope != self.scope
            || ticket.provider != self.provider.id().as_str()
            || ticket.endpoint != endpoint_identity(self.provider.endpoint())
            || ticket.format != driver.batch_format()
        {
            return Err(BatchError::ScopeMismatch);
        }
        Ok(driver)
    }

    /// Check once. The application schedules the next poll (usually ≥60 seconds).
    pub async fn poll(&self, ticket: &BatchTicket) -> Result<BatchStatus, BatchError> {
        self.check(ticket)?
            .poll(
                self.provider.endpoint(),
                &ticket.job_id,
                self.timeout,
                self.max_response_bytes,
            )
            .await
    }

    /// Collect a finished job. Repeated collection is safe and returns results
    /// in submission order. Missing, extra or duplicate remote IDs fail the
    /// entire collection; results are never silently assigned by position.
    pub async fn collect(&self, ticket: &BatchTicket) -> Result<Vec<BatchResult>, BatchError> {
        let driver = self.check(ticket)?;
        if driver
            .poll(
                self.provider.endpoint(),
                &ticket.job_id,
                self.timeout,
                self.max_response_bytes,
            )
            .await?
            != BatchStatus::Ended
        {
            return Err(BatchError::NotReady);
        }
        let results = driver
            .collect(
                self.provider.endpoint(),
                &ticket.job_id,
                self.timeout,
                self.max_response_bytes,
            )
            .await?;
        let mut by_id = std::collections::HashMap::new();
        for result in results {
            if by_id.insert(result.id.clone(), result).is_some() {
                return Err(BatchError::Protocol("duplicate result ID"));
            }
        }
        if by_id.len() != ticket.request_ids.len()
            || ticket.request_ids.iter().any(|id| !by_id.contains_key(id))
        {
            return Err(BatchError::Protocol(
                "result IDs do not match the submission",
            ));
        }
        ticket
            .request_ids
            .iter()
            .map(|id| {
                by_id
                    .remove(id)
                    .ok_or(BatchError::Protocol("missing result ID"))
            })
            .collect()
    }

    /// Request cancellation of the whole provider job. Keep polling until it
    /// ends and collect successes and cancellations. Already running requests
    /// may complete and be billed; dropping a future does not cancel the job.
    pub async fn cancel(&self, ticket: &BatchTicket) -> Result<BatchStatus, BatchError> {
        self.check(ticket)?
            .cancel(
                self.provider.endpoint(),
                &ticket.job_id,
                self.timeout,
                self.max_response_bytes,
            )
            .await
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
}

impl std::fmt::Debug for Batch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Batch")
            .field("requests", &self.requests.len())
            .finish_non_exhaustive()
    }
}

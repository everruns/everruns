//! Provider-native offline requests. No credentials or prompts belong in tickets.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{LlmCallConfig, LlmResponse, ProviderEndpoint, driver_registry::Message};

/// Errors never include provider bodies, prompts, credentials, or endpoint URLs.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum BatchError {
    #[error("provider does not support native batches")]
    Unsupported,
    #[error("provider batch operation is unsupported: {0}")]
    UnsupportedOperation(&'static str),
    #[error("invalid batch request: {0}")]
    InvalidRequest(&'static str),
    #[error("ticket does not belong to this batch client")]
    ScopeMismatch,
    #[error("invalid batch ticket")]
    InvalidTicket,
    #[error("batch is still processing")]
    NotReady,
    #[error("provider batch ended without item results: {reason:?}")]
    JobFailed { reason: BatchItemError },
    #[error("invalid provider batch response: {0}")]
    Protocol(&'static str),
    #[error("provider batch HTTP error ({status})")]
    Http { status: u16 },
    #[error("batch transport failed; retry this operation with the same ticket")]
    Transport,
    #[error(
        "batch submission may have succeeded; reconcile with the provider before submitting again"
    )]
    SubmissionUncertain,
    #[error("batch response exceeds the configured byte limit")]
    ResponseTooLarge,
}

impl BatchError {
    /// Whether retrying this *operation* is safe and potentially useful.
    /// Never automatically resubmit an uncertain creation.
    pub fn retryable(&self) -> bool {
        matches!(
            self,
            Self::Transport
                | Self::Http {
                    status: 408 | 429 | 500..=599
                }
        )
    }
}

/// One independent model request, correlated by a caller-chosen ID.
#[derive(Debug, Clone)]
pub struct BatchRequest {
    pub id: String,
    pub messages: Vec<Message>,
    pub config: LlmCallConfig,
}

/// Provider job lifecycle. Cancellation is asynchronous and may retain successes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum BatchStatus {
    Processing,
    Cancelling,
    Ended,
}

/// A per-request failure; retry decisions belong to the caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum BatchItemError {
    InvalidRequest,
    Authentication,
    Permission,
    RateLimited,
    Server,
    Cancelled,
    Expired,
    Other,
}

/// A single completed request. Collection never executes returned tool calls.
#[derive(Debug, Clone)]
pub struct BatchResult {
    pub id: String,
    pub result: std::result::Result<LlmResponse, BatchItemError>,
}

/// Optional provider SPI exposed by [`crate::ChatDriver::batch_driver`].
///
/// Implementations must not retry creation after an ambiguous response, must
/// validate remote IDs before constructing URLs, and must bound response reads.
/// `max_bytes` applies to each HTTP response; `timeout` bounds each HTTP request
/// within an operation (for example uploading a file then creating a job).
/// Poll, collect and cancel are repeatable with the same provider job ID.
#[async_trait]
pub trait BatchDriver: Send + Sync {
    /// Identifies this driver's persisted job format, including its version.
    fn batch_format(&self) -> &'static str;

    async fn submit(
        &self,
        endpoint: &ProviderEndpoint,
        requests: Vec<BatchRequest>,
        timeout: std::time::Duration,
        max_bytes: usize,
    ) -> Result<String, BatchError>;

    async fn poll(
        &self,
        endpoint: &ProviderEndpoint,
        job_id: &str,
        timeout: std::time::Duration,
        max_bytes: usize,
    ) -> Result<BatchStatus, BatchError>;

    async fn collect(
        &self,
        endpoint: &ProviderEndpoint,
        job_id: &str,
        timeout: std::time::Duration,
        max_bytes: usize,
    ) -> Result<Vec<BatchResult>, BatchError>;

    /// Collect against the original request IDs. Providers whose cancellation
    /// omits unprocessed items can report those IDs as cancelled or expired,
    /// while preserving successes. Never infer request identity from position.
    /// The default retains compatibility with drivers returning every item.
    async fn collect_for_requests(
        &self,
        endpoint: &ProviderEndpoint,
        job_id: &str,
        _request_ids: &[String],
        timeout: std::time::Duration,
        max_bytes: usize,
    ) -> Result<Vec<BatchResult>, BatchError> {
        self.collect(endpoint, job_id, timeout, max_bytes).await
    }

    async fn cancel(
        &self,
        endpoint: &ProviderEndpoint,
        job_id: &str,
        timeout: std::time::Duration,
        max_bytes: usize,
    ) -> Result<BatchStatus, BatchError>;
}

/// An endpoint identity hash for credential-free tickets. Authentication can
/// rotate; callers must additionally bind tickets to a trusted tenant/account.
pub fn endpoint_identity(endpoint: &ProviderEndpoint) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(
        endpoint.base_url().unwrap_or_default().as_bytes(),
    ))
}

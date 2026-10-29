//! Credential-owning host dispatch, deliberately separate from worker metadata.
//!
//! No listener, issuer, default policy or production host implementation is
//! installed. Implementing `DispatchHost` is trusted application code, not an
//! authentication mechanism. Journal-only worker capabilities grant no dispatch
//! authority. The host must derive current effective user/custom policy and
//! assignment authority, not accept scope or receipt claims from an executor.

mod openrouter;
mod receipt;
mod transport;

pub use openrouter::{DispatchPlan, TextDispatchPolicy, TextMessage, TextRequest, TextRole};
pub use receipt::{ReceiptReview, SupplierReceipt, UsageObservation};
pub use transport::DispatchTransport;

use crate::runtime_provider::ProviderEndpoint;
use async_trait::async_trait;
use futures::StreamExt;
use uuid::Uuid;

pub const OPENROUTER_API: &str = "https://openrouter.ai/api/v1";
pub const OPENROUTER_CHAT: &str = "https://openrouter.ai/api/v1/chat/completions";

/// These identifiers are binding data, never permission or transport identity.
/// Only the trusted host supplies them after its own authenticated admission.
#[derive(Clone, PartialEq, Eq)]
pub struct DispatchIdentity {
    pub generation_id: Uuid,
    pub organization: String,
    pub session_id: Option<Uuid>,
    pub supplier_account: String,
    pub provider_id: crate::typed_id::ProviderId,
    pub started_at: crate::rt::UtcTimestamp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DispatchError {
    #[error("unsupported bounded request or policy")]
    Unsupported,
    #[error("host admission refused")]
    Refused,
    #[error("supplier operation remains uncertain; do not retry dispatch")]
    Uncertain,
    #[error("canonical supplier receipt requires review")]
    Review,
}

/// Host-owned authenticated context is captured by this implementation, never
/// serialized in `TextRequest`. Reserve the complete frozen plan and allocate a
/// durable generation ID before returning. A repeated operation must bind the
/// original plan and identity; it must not get another dispatch opportunity.
#[async_trait]
pub trait DispatchHost: Send + Sync {
    type Permit: DispatchPermit;
    async fn reserve(&self, plan: &DispatchPlan) -> Result<Self::Permit, DispatchError>;
}

#[async_trait]
pub trait DispatchPermit: Send {
    /// Recheck live authenticated assignment, effective policy, revocation,
    /// serving account and the frozen plan; atomically win one durable start.
    /// Return credentials only after that commit. Cancellation/lost responses
    /// must retain the original funding and may never authorize an automatic retry.
    async fn start(
        &mut self,
        plan: &DispatchPlan,
    ) -> Result<(DispatchIdentity, ProviderEndpoint), DispatchError>;

    /// Durably reconcile this host-observed receipt under the original identity
    /// and financial owner. Review observations retain the hold; complete ones
    /// settle once. Journal with `generation_id`, never a new post-call ID.
    /// No legacy/event fallback is permitted on failure or cancellation.
    async fn record(&mut self, receipt: &SupplierReceipt) -> Result<(), DispatchError>;
}

/// Finite, non-streaming text operation. No retries, fallback, native tools,
/// continuation, compaction, media, caching controls or arbitrary headers.
/// The host owns `policy`, `host` and `client`; only `request` is caller data.
pub async fn dispatch_text<H: DispatchHost>(
    host: &H,
    client: &DispatchTransport,
    policy: &TextDispatchPolicy,
    request: &TextRequest,
) -> Result<String, DispatchError> {
    let plan = policy.compile(request)?;
    let mut permit = host.reserve(&plan).await?;
    let (identity, endpoint) = permit.start(&plan).await?;
    if identity.generation_id.is_nil()
        || identity.organization.is_empty()
        || identity.organization.len() > 256
        || identity.supplier_account.is_empty()
        || identity.supplier_account.len() > 256
        || endpoint.base_url() != Some(OPENROUTER_API)
    {
        return Err(DispatchError::Uncertain);
    }
    // THREAT[TM-LLM-057]: credential resolution follows durable host admission;
    // neither a caller header nor an alternate endpoint can change this request.
    let response = crate::rt::timeout(policy.timeout, async {
        let auth = endpoint
            .resolve("POST", OPENROUTER_CHAT, plan.body())
            .await
            .map_err(|_| DispatchError::Uncertain)?;
        if auth.headers.len() != 1 || !auth.headers[0].0.eq_ignore_ascii_case("authorization") {
            return Err(DispatchError::Uncertain);
        }
        let mut request = client
            .0
            .post(OPENROUTER_CHAT)
            .header("authorization", &auth.headers[0].1);
        for (name, value) in openrouter::CONTROL_HEADERS {
            request = request.header(name, value);
        }
        let response = request
            .body(plan.body().to_vec())
            .send()
            .await
            .map_err(|_| DispatchError::Uncertain)?;
        if response.status().as_u16() != 200 {
            return Err(DispatchError::Uncertain);
        }
        let mut stream = response.bytes_stream();
        let mut body = Vec::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| DispatchError::Uncertain)?;
            if chunk.len() > policy.max_response_bytes.saturating_sub(body.len()) {
                return Err(DispatchError::Uncertain);
            }
            body.extend_from_slice(&chunk);
        }
        Ok(body)
    })
    .await
    .map_err(|_| DispatchError::Uncertain)??;
    let (receipt, content) = receipt::parse(identity, &plan, &response)?;
    permit
        .record(&receipt)
        .await
        .map_err(|_| DispatchError::Uncertain)?;
    if receipt.review().is_some() {
        return Err(DispatchError::Review);
    }
    content.ok_or(DispatchError::Review)
}

#[cfg(test)]
mod tests;

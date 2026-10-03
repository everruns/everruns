//! Durable hooks for provider-side background responses (EVE-1134).
//!
//! OpenAI background mode keeps a long response running at the provider, so
//! a lost connection can re-attach instead of posting (and paying for) the
//! call again. Within one process the driver does that on its own. Across a
//! worker restart it needs two things the driver cannot own, supplied here by
//! the host:
//!
//! - a [`BackgroundResponseJournal`] that persists the response id outside the
//!   process, so the durable retry of the same call re-attaches to it;
//! - a turn-cancel signal, so an explicit cancellation stops the response at
//!   the provider even while this process is still reading it. Ownership loss
//!   (another worker reclaimed the task) must not fire it: the new owner is
//!   about to re-attach to that same response.
//!
//! Decision: a record is keyed by a fingerprint of the request, not by an
//! attempt counter. The durable retry rebuilds the same request from the same
//! transcript, so an equal fingerprint means "the same call"; anything else is
//! a different call and the stale response is cancelled rather than resumed.
//!
//! This module is transport-free so contract-only consumers (the host, the
//! durable store) can implement it without the `http` feature.

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::error::Result;

/// A background response that an attempt of this call left at the provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackgroundResponseRecord {
    /// Provider response id (`resp_...`).
    pub response_id: String,
    /// [`request_fingerprint`] of the request that created the response.
    pub request_fingerprint: String,
}

/// Durable slot for the in-flight background response of one call.
///
/// Implementations scope the slot to one turn and fence concurrent writers.
/// Errors are reported, never fatal: a driver that cannot persist the id
/// keeps the in-process behaviour (cancel when abandoned, re-post on retry).
#[async_trait]
pub trait BackgroundResponseJournal: Send + Sync {
    /// The record an earlier attempt saved, if any.
    async fn load(&self) -> Result<Option<BackgroundResponseRecord>>;
    /// Replace the record; `None` clears it once the response is terminal.
    async fn store(&self, record: Option<&BackgroundResponseRecord>) -> Result<()>;
}

/// What a host hands a background-capable driver for one call. Empty by
/// default, which keeps the in-process behaviour.
#[derive(Clone, Default)]
pub struct BackgroundCallContext {
    journal: Option<Arc<dyn BackgroundResponseJournal>>,
    cancel: Option<tokio::sync::watch::Receiver<bool>>,
}

impl BackgroundCallContext {
    /// Persist the response id through `journal` so a restart can re-attach.
    pub fn with_journal(mut self, journal: Arc<dyn BackgroundResponseJournal>) -> Self {
        self.journal = Some(journal);
        self
    }

    /// Cancel the response at the provider when `cancel` turns `true`. Fire it
    /// only for an explicit turn cancellation, never for ownership loss.
    pub fn with_cancel(mut self, cancel: tokio::sync::watch::Receiver<bool>) -> Self {
        self.cancel = Some(cancel);
        self
    }

    pub fn journal(&self) -> Option<&Arc<dyn BackgroundResponseJournal>> {
        self.journal.as_ref()
    }

    pub fn cancel_signal(&self) -> Option<tokio::sync::watch::Receiver<bool>> {
        self.cancel.clone()
    }
}

impl std::fmt::Debug for BackgroundCallContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BackgroundCallContext")
            .field("journal", &self.journal.is_some())
            .field("cancel", &self.cancel.is_some())
            .finish()
    }
}

/// Identity of a request for re-attachment. `metadata` is excluded because it
/// carries per-attempt ids (`exec_id`) that differ between the original call
/// and its durable retry without changing what is being asked.
pub fn request_fingerprint(body: &Value) -> String {
    let mut body = body.clone();
    if let Some(object) = body.as_object_mut() {
        object.remove("metadata");
    }
    let bytes = serde_json::to_vec(&body).unwrap_or_default();
    hex::encode(Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn fingerprint_ignores_per_attempt_metadata_only() {
        let first = json!({"model":"m","input":[{"role":"user","content":"hi"}],
            "metadata":{"exec_id":"exec_1"}});
        let retry = json!({"model":"m","input":[{"role":"user","content":"hi"}],
            "metadata":{"exec_id":"exec_2"}});
        let next_step = json!({"model":"m","input":[{"role":"user","content":"hi"},
            {"type":"function_call_output","call_id":"c","output":"ok"}],
            "metadata":{"exec_id":"exec_2"}});
        assert_eq!(request_fingerprint(&first), request_fingerprint(&retry));
        assert_ne!(request_fingerprint(&first), request_fingerprint(&next_step));
    }

    #[test]
    fn empty_context_debug_hides_internals() {
        let context = BackgroundCallContext::default();
        assert!(context.journal().is_none());
        assert!(context.cancel_signal().is_none());
        assert_eq!(
            format!("{context:?}"),
            "BackgroundCallContext { journal: false, cancel: false }"
        );
    }
}

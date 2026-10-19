//! Whether the control plane can serve encrypted checkpoint stores.
//!
//! Decision (EVE-1261): a control plane without `SECRETS_ENCRYPTION_KEY` runs
//! turns without compaction checkpoints and the native-async / Agents API
//! journals, the same way the in-process worker does
//! (`DirectWorkerAdapters` returns `None` for those stores). The key stays
//! "Recommended", not required. A remote worker learns this once, when it
//! connects (`probe`), and again if a checkpoint read is refused later
//! (a control plane restarted without the key); from then on its adapters
//! hand the runtime no such stores instead of failing every turn.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use everruns_internal_protocol::CHECKPOINT_ENCRYPTION_NOT_CONFIGURED;
use everruns_internal_protocol::proto;

use super::{GrpcClient, uuid_to_proto};

/// Shared by every clone of one [`GrpcClient`].
#[derive(Debug, Default)]
pub(crate) struct CheckpointAvailability {
    unavailable: AtomicBool,
}

impl CheckpointAvailability {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub(crate) fn available(&self) -> bool {
        !self.unavailable.load(Ordering::Relaxed)
    }

    /// Record that the control plane has no checkpoint encryption. Logs once.
    pub(crate) fn mark_unavailable(&self) {
        if !self.unavailable.swap(true, Ordering::Relaxed) {
            tracing::warn!(
                "control plane has no SECRETS_ENCRYPTION_KEY; running turns without \
                 compaction checkpoints and native-async / Agents API journals"
            );
        }
    }
}

/// Whether `status` is the control plane's "no checkpoint encryption" answer.
pub(crate) fn is_encryption_not_configured(status: &tonic::Status) -> bool {
    status.code() == tonic::Code::FailedPrecondition
        && status.message() == CHECKPOINT_ENCRYPTION_NOT_CONFIGURED
}

impl GrpcClient {
    /// Whether the control plane serves encrypted checkpoint stores, as far as
    /// this client has learned.
    pub fn checkpoint_stores_available(&self) -> bool {
        self.checkpoints.available()
    }

    /// Map a checkpoint or journal RPC failure, noting a missing key so the
    /// next turn runs without these stores. The current call still fails: a
    /// journal or an install cannot pretend it wrote.
    pub(crate) fn checkpoint_status_to_error(
        &self,
        status: tonic::Status,
    ) -> everruns_contracts::error::AgentLoopError {
        if is_encryption_not_configured(&status) {
            self.checkpoints.mark_unavailable();
        }
        super::grpc_status_to_error(status)
    }

    /// Ask the control plane once whether it has checkpoint encryption.
    ///
    /// Reads the checkpoint of a random session: an encrypted control plane
    /// answers "none", one without a key refuses with
    /// [`CHECKPOINT_ENCRYPTION_NOT_CONFIGURED`]. Any other failure leaves the
    /// stores enabled; a later refusal still disables them.
    pub async fn probe_checkpoint_stores(&self) {
        let mut client = self.inner.client();
        let result = client
            .get_compaction_checkpoint(proto::GetCompactionCheckpointRequest {
                session_id: Some(uuid_to_proto(uuid::Uuid::now_v7())),
                provider_type: String::new(),
                model: String::new(),
            })
            .await;
        match result {
            Err(status) if is_encryption_not_configured(&status) => {
                self.checkpoints.mark_unavailable();
            }
            Err(status) => tracing::debug!(
                code = ?status.code(),
                "checkpoint store probe failed; keeping checkpoint stores enabled"
            ),
            Ok(_) => {}
        }
    }
}

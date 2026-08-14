// Stream-liveness heartbeater bridging ReasonAtom to the durable task store (EVE-531).

use async_trait::async_trait;
use everruns_core::{durability::StreamHeartbeater, durability::StreamProgress};
use uuid::Uuid;

use crate::grpc_durable_store::GrpcDurableStore;

/// Heartbeater that calls `heartbeat_task` via the durable store gRPC client.
///
/// Created per-task in `execute_reason_activity` and wired into `GrpcWorkerAdapters`.
/// Errors are logged but never propagated so they cannot disrupt the streaming loop.
pub struct GrpcTaskHeartbeater {
    pub store: GrpcDurableStore,
    pub task_id: Uuid,
    pub worker_id: String,
}

#[async_trait]
impl StreamHeartbeater for GrpcTaskHeartbeater {
    async fn heartbeat(&self, progress: StreamProgress) {
        let details = serde_json::json!({
            "accumulated_len": progress.accumulated_len,
            "last_delta_at": progress.last_delta_at,
        });
        let mut store = self.store.clone();
        if let Err(e) = store
            .heartbeat_task(self.task_id, &self.worker_id, Some(details))
            .await
        {
            tracing::debug!(
                task_id = %self.task_id,
                error = %e,
                "Stream heartbeat failed (non-fatal)"
            );
        }
    }
}

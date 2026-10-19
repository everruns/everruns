//! Session task reaper scans.
//!
//! Handler bodies for the `WorkerService` RPCs in this group. The trait impl in
//! `super::super::worker_service_impl` is a delegation layer only: a trait impl
//! cannot span modules, so the work lives here and the trait forwards to it.
//!
//! Decision: only the reaper's orphan scan and retention prune remain RPCs;
//! they run across every org. Task lifecycle and messages are internal
//! commands (`domains/session_tasks/commands/worker`).

use crate::worker_link::grpc_service::*;

impl WorkerServiceImpl {
    pub(crate) async fn handle_list_orphaned_session_tasks(
        &self,
        request: Request<ListOrphanedSessionTasksRequest>,
    ) -> Result<Response<ListOrphanedSessionTasksResponse>, Status> {
        let req = request.into_inner();
        if req.stale_after_seconds <= 0 {
            return Err(Status::invalid_argument(
                "stale_after_seconds must be positive",
            ));
        }
        if req.limit <= 0 {
            return Err(Status::invalid_argument("limit must be positive"));
        }
        let stale_after = chrono::Duration::seconds(req.stale_after_seconds);
        // Cap the response size regardless of what the caller asks for.
        let limit = req.limit.min(1000);

        let pairs = self
            .db
            .list_orphaned_session_task_ids(stale_after, limit)
            .await
            .map_err(|e| {
                tracing::error!("Failed to list orphaned session tasks: {e}");
                Status::internal("Failed to list orphaned session tasks")
            })?;

        let entries = pairs
            .into_iter()
            .map(|(org_id, session_id, task_id)| OrphanedSessionTaskEntry {
                session_id: session_id.uuid().to_string(),
                task_id,
                org_id,
            })
            .collect();

        Ok(Response::new(ListOrphanedSessionTasksResponse { entries }))
    }

    pub(crate) async fn handle_prune_terminal_session_tasks(
        &self,
        request: Request<PruneTerminalSessionTasksRequest>,
    ) -> Result<Response<PruneTerminalSessionTasksResponse>, Status> {
        let req = request.into_inner();
        if req.ttl_seconds <= 0 {
            return Err(Status::invalid_argument("ttl_seconds must be positive"));
        }
        if req.limit <= 0 {
            return Err(Status::invalid_argument("limit must be positive"));
        }
        let ttl = chrono::Duration::seconds(req.ttl_seconds);
        // Cap the batch size regardless of what the caller asks for.
        let limit = req.limit.min(1000);

        let pruned = self
            .db
            .prune_terminal_session_tasks_with_artifacts(ttl, limit)
            .await
            .map_err(|e| {
                tracing::error!("Failed to prune terminal session tasks: {e}");
                Status::internal("Failed to prune terminal session tasks")
            })?;

        Ok(Response::new(PruneTerminalSessionTasksResponse {
            pruned: pruned as i64,
        }))
    }
}

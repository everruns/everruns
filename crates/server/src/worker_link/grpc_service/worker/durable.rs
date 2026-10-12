//! Durable tasks and workers.
//!
//! Handler bodies for the `WorkerService` RPCs in this group. The trait impl in
//! `super::super::worker_service_impl` is a delegation layer only: a trait impl
//! cannot span modules, so the work lives here and the trait forwards to it.
//!
//! Workers claim the queue's activities that are not turns (cleanup, the
//! session task reaper, scheduled invocations); turns run on session actor
//! leases (`actors.rs`).

use super::support::*;
use crate::worker_link::grpc_service::*;
use everruns_durable::{EventLog, TaskQueue, WorkerRegistry};

impl WorkerServiceImpl {
    pub(crate) async fn handle_claim_durable_tasks(
        &self,
        request: Request<ClaimDurableTasksRequest>,
    ) -> Result<Response<ClaimDurableTasksResponse>, Status> {
        let req = request.into_inner();
        let store = self.durable_store()?;

        let tasks = store
            .claim_task(&req.worker_id, &req.activity_types, req.max_tasks as usize)
            .await
            .map_err(|e| {
                tracing::error!("Failed to claim tasks: {}", e);
                Status::internal("Failed to claim tasks")
            })?;

        Ok(Response::new(ClaimDurableTasksResponse {
            tasks: tasks.into_iter().map(claimed_task_to_proto).collect(),
        }))
    }

    pub(crate) async fn handle_complete_durable_task(
        &self,
        request: Request<CompleteDurableTaskRequest>,
    ) -> Result<Response<CompleteDurableTaskResponse>, Status> {
        let req = request.into_inner();
        let store = self.durable_store()?;
        let task_id = parse_uuid(req.task_id.as_ref())?;
        let worker_id = &req.worker_id;

        let output = req
            .output
            .map(|s| everruns_internal_protocol::proto_struct_to_json(&s))
            .unwrap_or_else(|| serde_json::json!({}));

        // Get task info before completing (to get workflow_id and activity_id for event)
        let task_info = match store.get_task(task_id).await {
            Ok(info) => Some(info),
            Err(e) => {
                tracing::warn!(%task_id, error = %e, "Failed to get task info for event");
                None
            }
        };

        // complete_task verifies worker ownership to prevent duplicate completion
        match store
            .complete_task(task_id, worker_id, output.clone())
            .await
        {
            Ok(()) => {
                if let Some(info) = task_info {
                    record_activity_completed(
                        store.as_ref(),
                        info.workflow_id,
                        info.activity_id,
                        output,
                    )
                    .await;
                }
                Ok(Response::new(CompleteDurableTaskResponse { success: true }))
            }
            Err(StoreError::TaskNotOwned(_)) => {
                // Task was reclaimed by another worker - not an error, just return false
                tracing::info!(
                    %task_id,
                    %worker_id,
                    "Task completion rejected: task was reclaimed or already completed"
                );
                Ok(Response::new(CompleteDurableTaskResponse {
                    success: false,
                }))
            }
            Err(e) => {
                tracing::error!("Failed to complete task: {}", e);
                Err(Status::internal("Failed to complete task"))
            }
        }
    }

    pub(crate) async fn handle_fail_durable_task(
        &self,
        request: Request<FailDurableTaskRequest>,
    ) -> Result<Response<FailDurableTaskResponse>, Status> {
        let req = request.into_inner();
        let store = self.durable_store()?;
        let task_id = parse_uuid(req.task_id.as_ref())?;

        // Get task info before failing (to get workflow_id and activity_id for event)
        let task_info = match store.get_task(task_id).await {
            Ok(info) => Some(info),
            Err(e) => {
                tracing::warn!(%task_id, error = %e, "Failed to get task info for event");
                None
            }
        };

        let outcome = match store
            .fail_task_with_retry(task_id, &req.error, req.retryable.unwrap_or(true))
            .await
        {
            Ok(outcome) => outcome,
            Err(StoreError::TaskNotOwned(_)) => {
                // EVE-639: fail_task now no-ops if the task is no longer 'claimed'
                // (a concurrent stale-reclaim already requeued or sealed it). The
                // failure is absorbed by the reclaimer; report it as a retry
                // (the task is back in the queue) rather than an internal error.
                tracing::info!(
                    %task_id,
                    "Task failure absorbed: task was already reclaimed"
                );
                return Ok(Response::new(FailDurableTaskResponse {
                    failed: true,
                    will_retry: true,
                }));
            }
            Err(e) => {
                tracing::error!("Failed to fail task: {}", e);
                return Err(Status::internal("Failed to fail task"));
            }
        };

        // Check if task will be retried
        let will_retry = matches!(outcome, TaskFailureOutcome::WillRetry { .. });
        // A workflow-bound task out of retries fails its workflow; the store's
        // compare-and-set lets only one caller record it.
        if !will_retry
            && let Some(workflow_id) = task_info.as_ref().and_then(|info| info.workflow_id)
        {
            let error = WorkflowError::new(req.error.clone());
            let won = store
                .try_fail_workflow(workflow_id, error)
                .await
                .map_err(|error| {
                    tracing::error!(%workflow_id, %error, "Failed to terminalize workflow");
                    Status::internal("Failed to terminalize workflow")
                })?;
            if won {
                record_workflow_failed(store.as_ref(), workflow_id, req.error.clone()).await;
            }
        }

        // Notify NATS when task goes back to pending for retry
        if let (true, Some(broadcaster), Some(info)) =
            (will_retry, &self.task_broadcaster, &task_info)
        {
            broadcaster.notify_task_available(&info.activity_type).await;
        }

        // Record ActivityFailed event
        if let Some(info) = task_info {
            record_activity_failed(
                store.as_ref(),
                info.workflow_id,
                info.activity_id,
                req.error.clone(),
                will_retry,
            )
            .await;
        }

        Ok(Response::new(FailDurableTaskResponse {
            failed: true,
            will_retry,
        }))
    }

    pub(crate) async fn handle_heartbeat_durable_task(
        &self,
        request: Request<HeartbeatDurableTaskRequest>,
    ) -> Result<Response<HeartbeatDurableTaskResponse>, Status> {
        let req = request.into_inner();
        let store = self.durable_store()?;
        let task_id = parse_uuid(req.task_id.as_ref())?;

        let response = store
            .heartbeat_task(task_id, &req.worker_id, None)
            .await
            .map_err(|e| {
                tracing::error!("Failed to heartbeat task: {}", e);
                Status::internal("Failed to heartbeat task")
            })?;

        Ok(Response::new(HeartbeatDurableTaskResponse {
            acknowledged: response.accepted,
            should_cancel: response.should_cancel,
        }))
    }

    pub(crate) async fn handle_register_durable_worker(
        &self,
        request: Request<RegisterDurableWorkerRequest>,
    ) -> Result<Response<RegisterDurableWorkerResponse>, Status> {
        let req = request.into_inner();
        let store = self.durable_store()?;

        let worker_info = WorkerInfo {
            id: req.worker_id,
            worker_group: req.worker_group,
            activity_types: req.activity_types,
            max_concurrency: req.max_concurrency as u32,
            current_load: 0,
            status: "active".to_string(),
            accepting_tasks: true,
            backpressure_reason: None,
            started_at: chrono::Utc::now(),
            last_heartbeat_at: chrono::Utc::now(),
            hostname: None,
            version: None,
            metadata: None,
            tasks_completed: 0,
            tasks_failed: 0,
            avg_task_duration_ms: None,
        };

        store.register_worker(worker_info).await.map_err(|e| {
            tracing::error!("Failed to register worker: {}", e);
            Status::internal("Failed to register worker")
        })?;

        Ok(Response::new(RegisterDurableWorkerResponse {
            registered: true,
        }))
    }

    pub(crate) async fn handle_heartbeat_durable_worker(
        &self,
        request: Request<HeartbeatDurableWorkerRequest>,
    ) -> Result<Response<HeartbeatDurableWorkerResponse>, Status> {
        let req = request.into_inner();
        let store = self.durable_store()?;

        let heartbeat = store
            .worker_heartbeat(
                &req.worker_id,
                req.current_load as usize,
                req.accepting_tasks,
            )
            .await
            .map_err(|e| {
                tracing::error!("Failed to heartbeat worker: {}", e);
                Status::internal("Failed to heartbeat worker")
            })?;

        Ok(Response::new(HeartbeatDurableWorkerResponse {
            acknowledged: true,
            draining: heartbeat.draining,
        }))
    }

    /// The worker drains itself on shutdown, so the queue hands it nothing
    /// new while its in-flight tasks and turns finish.
    pub(crate) async fn handle_drain_durable_worker(
        &self,
        request: Request<DrainDurableWorkerRequest>,
    ) -> Result<Response<DrainDurableWorkerResponse>, Status> {
        let req = request.into_inner();
        let store = self.durable_store()?;

        store.drain_worker(&req.worker_id).await.map_err(|e| {
            tracing::error!(error = %e, "Failed to drain worker");
            Status::internal("Failed to drain worker")
        })?;
        tracing::info!(worker_id = %req.worker_id, "Worker draining itself for shutdown");

        Ok(Response::new(DrainDurableWorkerResponse {}))
    }

    pub(crate) async fn handle_deregister_durable_worker(
        &self,
        request: Request<DeregisterDurableWorkerRequest>,
    ) -> Result<Response<DeregisterDurableWorkerResponse>, Status> {
        let req = request.into_inner();
        let store = self.durable_store()?;

        let tasks_reclaimed = store.deregister_worker(&req.worker_id).await.map_err(|e| {
            tracing::error!("Failed to deregister worker: {}", e);
            Status::internal("Failed to deregister worker")
        })?;

        if tasks_reclaimed > 0 {
            tracing::info!(
                worker_id = %req.worker_id,
                tasks_reclaimed,
                "Worker deregistered with task reclamation"
            );
        }

        Ok(Response::new(DeregisterDurableWorkerResponse {
            deregistered: true,
            tasks_reclaimed: tasks_reclaimed as i32,
        }))
    }
}

/// A claimed task on the wire. The claim reads the workflow status in the
/// same statement, so the worker's pre-execution check needs no extra read.
fn claimed_task_to_proto(t: everruns_durable::ClaimedTask) -> proto::DurableClaimedTask {
    use everruns_internal_protocol::uuid_to_proto_uuid;
    proto::DurableClaimedTask {
        id: Some(uuid_to_proto_uuid(t.id)),
        workflow_id: t.workflow_id.map(uuid_to_proto_uuid),
        activity_id: t.activity_id,
        activity_type: t.activity_type,
        input: Some(everruns_internal_protocol::json_to_proto_struct(&t.input)),
        attempt: t.attempt as i32,
        max_attempts: t.max_attempts as i32,
        workflow_status: t
            .workflow_status
            .map(|s| workflow_status_to_proto(s).into()),
    }
}

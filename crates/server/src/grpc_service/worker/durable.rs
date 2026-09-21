//! Durable workflows, tasks, and workers.
//!
//! Handler bodies for the `WorkerService` RPCs in this group. The trait impl in
//! `super::super::worker_service_impl` is a delegation layer only: a trait impl
//! cannot span modules, so the work lives here and the trait forwards to it.

use super::support::*;
use crate::grpc_service::*;

impl WorkerServiceImpl {
    pub(crate) async fn handle_create_durable_workflow(
        &self,
        request: Request<CreateDurableWorkflowRequest>,
    ) -> Result<Response<CreateDurableWorkflowResponse>, Status> {
        use everruns_internal_protocol::uuid_to_proto_uuid;

        let req = request.into_inner();
        let store = self.durable_store()?;

        // Generate or use provided workflow ID
        let workflow_id = if let Some(proto_id) = req.workflow_id {
            parse_uuid(Some(&proto_id))?
        } else {
            uuid::Uuid::now_v7()
        };

        // Convert proto Struct to serde_json::Value
        let input = req
            .input
            .map(|s| everruns_internal_protocol::proto_struct_to_json(&s))
            .unwrap_or_else(|| serde_json::json!({}));

        // Create workflow instance
        store
            .create_workflow(workflow_id, &req.workflow_type, input, None)
            .await
            .map_err(|e| {
                tracing::error!("Failed to create durable workflow: {}", e);
                Status::internal("Failed to create workflow")
            })?;

        Ok(Response::new(CreateDurableWorkflowResponse {
            workflow_id: Some(uuid_to_proto_uuid(workflow_id)),
        }))
    }

    pub(crate) async fn handle_get_durable_workflow_status(
        &self,
        request: Request<GetDurableWorkflowStatusRequest>,
    ) -> Result<Response<GetDurableWorkflowStatusResponse>, Status> {
        let req = request.into_inner();
        let store = self.durable_store()?;
        let workflow_id = parse_uuid(req.workflow_id.as_ref())?;

        let info = store.get_workflow_info(workflow_id).await.map_err(|e| {
            if matches!(e, StoreError::WorkflowNotFound(_)) {
                return Status::not_found("Workflow not found");
            }
            tracing::error!("Failed to get workflow status: {}", e);
            Status::internal("Failed to get workflow status")
        })?;

        let status = workflow_status_to_proto(info.status);
        let output = info
            .result
            .map(|o| everruns_internal_protocol::json_to_proto_struct(&o));
        let error = info.error.map(|e| e.message);

        Ok(Response::new(GetDurableWorkflowStatusResponse {
            status: status.into(),
            output,
            error,
        }))
    }

    pub(crate) async fn handle_update_durable_workflow_status(
        &self,
        request: Request<UpdateDurableWorkflowStatusRequest>,
    ) -> Result<Response<UpdateDurableWorkflowStatusResponse>, Status> {
        let req = request.into_inner();
        let store = self.durable_store()?;
        let workflow_id = parse_uuid(req.workflow_id.as_ref())?;

        let status = proto_to_workflow_status(req.status());
        let output = req
            .output
            .map(|s| everruns_internal_protocol::proto_struct_to_json(&s));
        let error = req.error.clone().map(WorkflowError::new);

        store
            .update_workflow_status(workflow_id, status, output.clone(), error)
            .await
            .map_err(|e| {
                tracing::error!("Failed to update workflow status: {}", e);
                Status::internal("Failed to update workflow status")
            })?;

        // Record terminal workflow event based on new status
        match status {
            WorkflowStatus::Completed => {
                record_workflow_completed(
                    store.as_ref(),
                    workflow_id,
                    output.unwrap_or_else(|| serde_json::json!({})),
                )
                .await;
            }
            WorkflowStatus::Failed => {
                record_workflow_failed(
                    store.as_ref(),
                    workflow_id,
                    req.error.unwrap_or_else(|| "Unknown error".to_string()),
                )
                .await;
            }
            WorkflowStatus::Cancelled => {
                record_workflow_cancelled(
                    store.as_ref(),
                    workflow_id,
                    Some(req.error.unwrap_or_else(|| "Cancelled".to_string())),
                )
                .await;
            }
            _ => {
                // No event for Pending/Running status changes
            }
        }

        Ok(Response::new(UpdateDurableWorkflowStatusResponse {
            updated: true,
        }))
    }

    pub(crate) async fn handle_enqueue_durable_task(
        &self,
        request: Request<EnqueueDurableTaskRequest>,
    ) -> Result<Response<EnqueueDurableTaskResponse>, Status> {
        let req = request.into_inner();
        let store = self.durable_store()?;

        let task_def = req
            .task
            .ok_or_else(|| Status::invalid_argument("Missing task definition"))?;
        let workflow_id = parse_uuid(task_def.workflow_id.as_ref())?;

        let input = task_def
            .input
            .map(|s| everruns_internal_protocol::proto_struct_to_json(&s))
            .unwrap_or_else(|| serde_json::json!({}));

        // For now, use default activity options
        // TODO: Map proto options to ActivityOptions when needed
        let options = ActivityOptions::default();

        let event = WorkflowEvent::ActivityScheduled {
            activity_id: task_def.activity_id.clone(),
            activity_type: task_def.activity_type.clone(),
            input: input.clone(),
            options: options.clone(),
        };
        append_event(store.as_ref(), workflow_id, event)
            .await
            .map_err(|e| {
                tracing::error!("Failed to append ActivityScheduled event: {}", e);
                Status::internal("Failed to append ActivityScheduled event")
            })?;

        let task = TaskDefinition {
            workflow_id: Some(workflow_id),
            activity_id: task_def.activity_id.clone(),
            activity_type: task_def.activity_type.clone(),
            input,
            options,
        };

        let task_id = store.enqueue_task(task).await.map_err(|e| {
            if matches!(
                e,
                everruns_durable::StoreError::TaskQueueLimitExceeded { .. }
            ) {
                tracing::warn!("Task queue limit exceeded: {}", e);
                Status::resource_exhausted(e.to_string())
            } else {
                tracing::error!("Failed to enqueue task: {}", e);
                Status::internal("Failed to enqueue task")
            }
        })?;

        // Notify NATS subscribers (no-op for PG backend — PG uses DB triggers)
        if let Some(broadcaster) = &self.task_broadcaster {
            broadcaster
                .notify_task_available(&task_def.activity_type)
                .await;
        }

        use everruns_internal_protocol::uuid_to_proto_uuid;
        Ok(Response::new(EnqueueDurableTaskResponse {
            task_id: Some(uuid_to_proto_uuid(task_id)),
        }))
    }

    pub(crate) async fn handle_claim_durable_tasks(
        &self,
        request: Request<ClaimDurableTasksRequest>,
    ) -> Result<Response<ClaimDurableTasksResponse>, Status> {
        use everruns_internal_protocol::uuid_to_proto_uuid;

        let req = request.into_inner();
        let store = self.durable_store()?;

        let tasks = store
            .claim_task(&req.worker_id, &req.activity_types, req.max_tasks as usize)
            .await
            .map_err(|e| {
                tracing::error!("Failed to claim tasks: {}", e);
                Status::internal("Failed to claim tasks")
            })?;

        let proto_tasks: Vec<proto::DurableClaimedTask> = tasks
            .into_iter()
            .map(|t| proto::DurableClaimedTask {
                id: Some(uuid_to_proto_uuid(t.id)),
                workflow_id: t.workflow_id.map(uuid_to_proto_uuid),
                activity_id: t.activity_id,
                activity_type: t.activity_type,
                input: Some(everruns_internal_protocol::json_to_proto_struct(&t.input)),
                attempt: t.attempt as i32,
                max_attempts: t.max_attempts as i32,
            })
            .collect();

        Ok(Response::new(ClaimDurableTasksResponse {
            tasks: proto_tasks,
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

        // complete_task now verifies worker ownership to prevent duplicate scheduling
        match store
            .complete_task(task_id, worker_id, output.clone())
            .await
        {
            Ok(()) => {
                // Record ActivityCompleted event
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
                    terminal_failure_owner: false,
                }));
            }
            Err(e) => {
                tracing::error!("Failed to fail task: {}", e);
                return Err(Status::internal("Failed to fail task"));
            }
        };

        // Check if task will be retried
        let will_retry = matches!(outcome, TaskFailureOutcome::WillRetry { .. });
        let terminal_failure_owner = if will_retry {
            false
        } else if let Some(workflow_id) = task_info.as_ref().and_then(|info| info.workflow_id) {
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
            won
        } else {
            false
        };

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
            terminal_failure_owner,
        }))
    }

    pub(crate) async fn handle_heartbeat_durable_task(
        &self,
        request: Request<HeartbeatDurableTaskRequest>,
    ) -> Result<Response<HeartbeatDurableTaskResponse>, Status> {
        let req = request.into_inner();
        let store = self.durable_store()?;
        let task_id = parse_uuid(req.task_id.as_ref())?;

        let details = req
            .details
            .map(|s| everruns_internal_protocol::proto_struct_to_json(&s));

        let response = store
            .heartbeat_task(task_id, &req.worker_id, details)
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

    pub(crate) async fn handle_count_active_durable_workflows(
        &self,
        _request: Request<CountActiveDurableWorkflowsRequest>,
    ) -> Result<Response<CountActiveDurableWorkflowsResponse>, Status> {
        let store = self.durable_store()?;

        let count = store.count_active_workflows().await.map_err(|e| {
            tracing::error!("Failed to count active workflows: {}", e);
            Status::internal("Failed to count active workflows")
        })?;

        Ok(Response::new(CountActiveDurableWorkflowsResponse { count }))
    }

    pub(crate) async fn handle_send_durable_workflow_signal(
        &self,
        request: Request<SendDurableWorkflowSignalRequest>,
    ) -> Result<Response<SendDurableWorkflowSignalResponse>, Status> {
        let req = request.into_inner();
        let workflow_id = uuid::Uuid::parse_str(&req.workflow_id)
            .map_err(|e| Status::invalid_argument(format!("Invalid workflow_id: {}", e)))?;
        let proto_signal = req
            .signal
            .ok_or_else(|| Status::invalid_argument("signal is required"))?;

        let payload = proto_signal
            .payload
            .as_ref()
            .map(everruns_internal_protocol::proto_value_to_json)
            .unwrap_or(serde_json::json!({}));
        let signal = everruns_durable::WorkflowSignal::new(proto_signal.signal_type, payload);

        let store = self.durable_store()?;
        store.send_signal(workflow_id, signal).await.map_err(|e| {
            tracing::error!("Failed to send workflow signal: {}", e);
            Status::internal("Failed to send workflow signal")
        })?;

        Ok(Response::new(SendDurableWorkflowSignalResponse {}))
    }

    pub(crate) async fn handle_get_and_consume_durable_workflow_signals(
        &self,
        request: Request<GetAndConsumeDurableWorkflowSignalsRequest>,
    ) -> Result<Response<GetAndConsumeDurableWorkflowSignalsResponse>, Status> {
        let req = request.into_inner();
        let workflow_id = uuid::Uuid::parse_str(&req.workflow_id)
            .map_err(|e| Status::invalid_argument(format!("Invalid workflow_id: {}", e)))?;

        let store = self.durable_store()?;
        let signals = if req.signal_type.is_empty() {
            store.consume_pending_signals(workflow_id).await
        } else {
            store
                .consume_pending_signals_by_type(workflow_id, &req.signal_type)
                .await
        }
        .map_err(|e| {
            tracing::error!("Failed to consume pending signals: {}", e);
            Status::internal("Failed to consume pending signals")
        })?;

        let proto_signals = signals
            .into_iter()
            .map(|s| ProtoDurableWorkflowSignal {
                signal_type: s.signal_type,
                payload: Some(json_value_to_proto(s.payload)),
                sent_at: s.sent_at.to_rfc3339(),
            })
            .collect();

        Ok(Response::new(GetAndConsumeDurableWorkflowSignalsResponse {
            signals: proto_signals,
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

        store
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
        }))
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

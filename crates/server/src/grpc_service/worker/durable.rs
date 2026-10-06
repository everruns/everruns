//! Durable workflows, tasks, and workers.
//!
//! Handler bodies for the `WorkerService` RPCs in this group. The trait impl in
//! `super::super::worker_service_impl` is a delegation layer only: a trait impl
//! cannot span modules, so the work lives here and the trait forwards to it.

use super::support::*;
use crate::grpc_service::*;
use everruns_durable::{DurableAdmin, EventLog, SignalStore, TaskQueue, WorkerRegistry};

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

        // Proto options are not mapped yet. Turn semantics the engine needs
        // (idempotent waiting-turn resolution) derive from the activity id, so
        // workers of any version get the same enqueue behavior.
        let options = everruns_worker::durable_turn::activity_options_for(&task_def.activity_id);

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

        let enqueued = match req.claim_for_worker_id.as_deref() {
            Some(worker_id) => store
                .enqueue_claimed_task(task, worker_id)
                .await
                .map(|enqueued| (enqueued.task_id(), enqueued.into_claimed())),
            None => store
                .enqueue_task(task)
                .await
                .map(|task_id| (task_id, None)),
        };
        let (task_id, claimed) = enqueued.map_err(|e| {
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

        // Notify NATS subscribers (no-op for PG backend — PG uses DB triggers).
        // A task enqueued claimed has its worker already.
        if claimed.is_none()
            && let Some(broadcaster) = &self.task_broadcaster
        {
            broadcaster
                .notify_task_available(&task_def.activity_type)
                .await;
        }

        use everruns_internal_protocol::uuid_to_proto_uuid;
        Ok(Response::new(EnqueueDurableTaskResponse {
            task_id: Some(uuid_to_proto_uuid(task_id)),
            claimed: claimed.map(claimed_task_to_proto),
        }))
    }

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

        if let Some(hand_off) = req.hand_off {
            return self
                .complete_durable_task_with_hand_off(
                    task_id, worker_id, output, task_info, hand_off,
                )
                .await;
        }

        // complete_task now verifies worker ownership to prevent duplicate scheduling
        match store
            .complete_task(task_id, worker_id, output.clone())
            .await
        {
            Ok(()) => {
                let Some(info) = task_info else {
                    return Ok(Response::new(CompleteDurableTaskResponse {
                        success: true,
                        drained_signal_count: None,
                        ..Default::default()
                    }));
                };
                let workflow_id = info.workflow_id;
                // Drain only after the completion succeeded, as the worker's
                // own consume call would. A failed drain is left to the worker.
                let drain = async {
                    let signal_type = req.drain_signal_type.as_deref()?;
                    match store
                        .consume_pending_signals_by_type(workflow_id?, signal_type)
                        .await
                    {
                        Ok(signals) => Some(signals.len() as u32),
                        Err(e) => {
                            tracing::warn!(%task_id, error = %e, "Failed to drain signals on completion");
                            None
                        }
                    }
                };
                let record = record_activity_completed(
                    store.as_ref(),
                    workflow_id,
                    info.activity_id,
                    output,
                );
                let ((), drained_signal_count) = tokio::join!(record, drain);
                Ok(Response::new(CompleteDurableTaskResponse {
                    success: true,
                    drained_signal_count,
                    ..Default::default()
                }))
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
                    drained_signal_count: None,
                    ..Default::default()
                }))
            }
            Err(e) => {
                tracing::error!("Failed to complete task: {}", e);
                Err(Status::internal("Failed to complete task"))
            }
        }
    }

    /// Complete a turn step and hand its workflow to the next step in one
    /// atomic store write, recording the step's history events around it
    /// (see `everruns_worker::turn_store::hand_off_and_record`).
    async fn complete_durable_task_with_hand_off(
        &self,
        task_id: uuid::Uuid,
        worker_id: &str,
        output: serde_json::Value,
        task_info: Option<everruns_durable::TaskInfo>,
        hand_off: everruns_internal_protocol::proto::DurableHandOff,
    ) -> Result<Response<CompleteDurableTaskResponse>, Status> {
        use everruns_internal_protocol::proto::durable_hand_off::Next;
        use everruns_internal_protocol::proto_struct_to_json;
        use everruns_worker::turn_store::{TurnHandOff, TurnNext, hand_off_and_record};

        let store = self.durable_store()?;
        let (Some(info), Some(next)) = (task_info, hand_off.next) else {
            return Err(Status::invalid_argument(
                "A hand-off needs a workflow task and a next step",
            ));
        };
        let Some(workflow_id) = info.workflow_id else {
            return Err(Status::invalid_argument("A hand-off needs a workflow task"));
        };
        let json = |value: Option<prost_types::Struct>| {
            value
                .map(|value| proto_struct_to_json(&value))
                .unwrap_or_else(|| serde_json::json!({}))
        };
        let (next, queued_type) = match next {
            Next::Step(step) => {
                let queued_type = step.activity_type.clone();
                (
                    TurnNext::Step {
                        activity_id: step.activity_id,
                        activity_type: step.activity_type,
                        input: json(step.input),
                        claim_for: step.claim_for_worker_id,
                    },
                    Some(queued_type),
                )
            }
            Next::Complete(complete) => (
                TurnNext::Complete {
                    event_output: json(complete.event_output),
                    stored_output: complete.stored_output.map(|s| proto_struct_to_json(&s)),
                    error: complete.error.map(WorkflowError::new),
                },
                None,
            ),
        };
        let hand_off = TurnHandOff {
            workflow_id,
            drain: hand_off
                .drain_signal_type
                .map(|signal_type| everruns_durable::SignalDrain {
                    signal_type,
                    limit: hand_off.drain_limit as usize,
                }),
            next,
        };

        match hand_off_and_record(
            store.as_ref(),
            None,
            task_id,
            &info.activity_id,
            worker_id,
            output,
            hand_off,
        )
        .await
        {
            Ok(claimed) => {
                // Notify NATS subscribers of a step left in the queue (no-op
                // for PG backend — PG uses DB triggers).
                if claimed.is_none()
                    && let (Some(activity_type), Some(broadcaster)) =
                        (queued_type, &self.task_broadcaster)
                {
                    broadcaster.notify_task_available(&activity_type).await;
                }
                Ok(Response::new(CompleteDurableTaskResponse {
                    success: true,
                    drained_signal_count: None,
                    handed_off: true,
                    claimed: claimed.map(claimed_task_to_proto),
                }))
            }
            Err(StoreError::TaskNotOwned(_)) => {
                tracing::info!(
                    %task_id,
                    %worker_id,
                    "Hand-off rejected: task was reclaimed or already completed"
                );
                Ok(Response::new(CompleteDurableTaskResponse::default()))
            }
            Err(e) => {
                tracing::error!(error = %e, "Failed to complete task and hand off");
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
        let signals = if req.peek.unwrap_or(false) {
            store.get_pending_signals(workflow_id).await.map(|signals| {
                signals
                    .into_iter()
                    .filter(|s| req.signal_type.is_empty() || s.signal_type == req.signal_type)
                    .collect()
            })
        } else if req.signal_type.is_empty() {
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

    /// The worker drains itself on shutdown, so its chained steps go back to
    /// the queue while its in-flight turns finish.
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

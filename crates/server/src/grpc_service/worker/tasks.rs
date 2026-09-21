//! Session task lifecycle and task messages.
//!
//! Handler bodies for the `WorkerService` RPCs in this group. The trait impl in
//! `super::super::worker_service_impl` is a delegation layer only: a trait impl
//! cannot span modules, so the work lives here and the trait forwards to it.

use crate::grpc_service::*;

impl WorkerServiceImpl {
    pub(crate) async fn handle_create_session_task(
        &self,
        request: Request<CreateSessionTaskRequest>,
    ) -> Result<Response<SessionTaskResponse>, Status> {
        let req = request.into_inner();
        let create = req
            .create
            .ok_or_else(|| Status::invalid_argument("Missing create payload"))?;
        let input = everruns_internal_protocol::proto_to_create_session_task(create)
            .map_err(|e| Status::invalid_argument(format!("Invalid task create payload: {e}")))?;

        let task = self
            .session_task_registry()
            .create(input)
            .await
            .map_err(|e| {
                tracing::error!("Failed to create session task: {e}");
                Status::internal("Failed to create session task")
            })?;

        Ok(Response::new(SessionTaskResponse {
            task: Some(everruns_internal_protocol::session_task_to_proto(&task)),
        }))
    }

    pub(crate) async fn handle_update_session_task(
        &self,
        request: Request<UpdateSessionTaskRequest>,
    ) -> Result<Response<OptionalSessionTaskResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;
        let update_proto = req
            .update
            .ok_or_else(|| Status::invalid_argument("Missing update payload"))?;
        let update = everruns_internal_protocol::proto_to_session_task_update(update_proto);

        let task = self
            .session_task_registry()
            .update(session_id.into(), &req.task_id, update)
            .await
            .map_err(|e| {
                tracing::error!("Failed to update session task: {e}");
                Status::internal("Failed to update session task")
            })?;

        Ok(Response::new(OptionalSessionTaskResponse {
            task: task
                .as_ref()
                .map(everruns_internal_protocol::session_task_to_proto),
        }))
    }

    pub(crate) async fn handle_get_session_task(
        &self,
        request: Request<GetSessionTaskRequest>,
    ) -> Result<Response<OptionalSessionTaskResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;

        let task = self
            .session_task_registry()
            .get(session_id.into(), &req.task_id)
            .await
            .map_err(|e| {
                tracing::error!("Failed to get session task: {e}");
                Status::internal("Failed to get session task")
            })?;

        Ok(Response::new(OptionalSessionTaskResponse {
            task: task
                .as_ref()
                .map(everruns_internal_protocol::session_task_to_proto),
        }))
    }

    pub(crate) async fn handle_list_session_tasks(
        &self,
        request: Request<ListSessionTasksRequest>,
    ) -> Result<Response<ListSessionTasksResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;

        let filter = if req.kind.is_some() || req.state.is_some() {
            Some(everruns_core::SessionTaskFilter {
                kind: req.kind,
                state: req
                    .state
                    .as_deref()
                    .map(everruns_core::SessionTaskState::from),
            })
        } else {
            None
        };

        let tasks = self
            .session_task_registry()
            .list(session_id.into(), filter.as_ref())
            .await
            .map_err(|e| {
                tracing::error!("Failed to list session tasks: {e}");
                Status::internal("Failed to list session tasks")
            })?;

        Ok(Response::new(ListSessionTasksResponse {
            tasks: tasks
                .iter()
                .map(everruns_internal_protocol::session_task_to_proto)
                .collect(),
        }))
    }

    pub(crate) async fn handle_request_cancel_session_task(
        &self,
        request: Request<RequestCancelSessionTaskRequest>,
    ) -> Result<Response<OptionalSessionTaskResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;

        let task = self
            .session_task_registry()
            .request_cancel(session_id.into(), &req.task_id)
            .await
            .map_err(|e| {
                tracing::error!("Failed to request session task cancel: {e}");
                Status::internal("Failed to request session task cancel")
            })?;

        Ok(Response::new(OptionalSessionTaskResponse {
            task: task
                .as_ref()
                .map(everruns_internal_protocol::session_task_to_proto),
        }))
    }

    pub(crate) async fn handle_record_session_task_message(
        &self,
        request: Request<RecordSessionTaskMessageRequest>,
    ) -> Result<Response<SessionTaskMessageResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;
        let message_proto = req
            .message
            .ok_or_else(|| Status::invalid_argument("Missing message payload"))?;
        let message = everruns_internal_protocol::proto_to_new_task_message(message_proto);

        let stored = self
            .session_task_registry()
            .record_message(session_id.into(), &req.task_id, message)
            .await
            .map_err(|e| {
                tracing::error!("Failed to record session task message: {e}");
                Status::internal("Failed to record session task message")
            })?;

        Ok(Response::new(SessionTaskMessageResponse {
            message: Some(everruns_internal_protocol::task_message_to_proto(&stored)),
        }))
    }

    pub(crate) async fn handle_list_session_task_messages(
        &self,
        request: Request<ListSessionTaskMessagesRequest>,
    ) -> Result<Response<ListSessionTaskMessagesResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;

        let messages = self
            .session_task_registry()
            .list_messages(session_id.into(), &req.task_id, req.limit, None)
            .await
            .map_err(|e| {
                tracing::error!("Failed to list session task messages: {e}");
                Status::internal("Failed to list session task messages")
            })?;

        Ok(Response::new(ListSessionTaskMessagesResponse {
            messages: messages
                .iter()
                .map(everruns_internal_protocol::task_message_to_proto)
                .collect(),
        }))
    }

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
            .map(|(session_id, task_id)| OrphanedSessionTaskEntry {
                session_id: session_id.uuid().to_string(),
                task_id,
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

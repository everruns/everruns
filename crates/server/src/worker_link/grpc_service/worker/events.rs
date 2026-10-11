//! Event emission and exec commits.
//!
//! Handler bodies for the `WorkerService` RPCs in this group. The trait impl in
//! `super::super::worker_service_impl` is a delegation layer only: a trait impl
//! cannot span modules, so the work lives here and the trait forwards to it.

use crate::worker_link::grpc_service::*;
use everruns_internal_protocol::proto::ActorFence;

fn append_fence(fence: &ActorFence) -> crate::services::ActorAppendFence<'_> {
    crate::services::ActorAppendFence {
        holder: &fence.holder,
        fence: fence.fence,
    }
}

/// A refused fence is `FAILED_PRECONDITION` with the lease-lost marker the
/// worker stops on (`everruns_worker::ACTOR_LEASE_LOST`); nothing was stored.
fn fenced_append_status(error: anyhow::Error) -> Status {
    if let Some(rejected) = error.downcast_ref::<crate::storage::ActorLeaseFenceRejected>() {
        return Status::failed_precondition(format!(
            "{}: {rejected}",
            everruns_worker::ACTOR_LEASE_LOST
        ));
    }
    tracing::error!("Failed to emit fenced events: {}", error);
    Status::internal("Failed to store events")
}

impl WorkerServiceImpl {
    pub(crate) async fn handle_emit_event_stream(
        &self,
        request: Request<Streaming<EmitEventRequest>>,
    ) -> Result<Response<EmitEventStreamResponse>, Status> {
        let mut stream = request.into_inner();
        let mut event_requests: Vec<everruns_core::EventRequest> = Vec::new();
        // The first message's fence is the stream's; every message must carry
        // the same one (a holder never mixes leased and unleased writes).
        let mut fence: Option<Option<ActorFence>> = None;

        // Collect all event requests from the stream, converting proto to core types
        while let Some(req) = stream.message().await? {
            match &fence {
                None => fence = Some(req.actor_fence.clone()),
                Some(first) if *first != req.actor_fence => {
                    return Err(Status::invalid_argument(
                        "every message of an event stream must carry the same actor fence",
                    ));
                }
                Some(_) => {}
            }
            let proto_event_request = match req.event {
                Some(e) => e,
                None => {
                    tracing::warn!("Received emit_event_stream request without event");
                    continue;
                }
            };

            // Convert proto EventRequest to core EventRequest using typed conversions
            let core_event_request = match proto_event_request_to_schema(proto_event_request) {
                Ok(e) => e,
                Err(e) => {
                    tracing::warn!("Failed to convert proto event request to core: {}", e);
                    continue;
                }
            };

            event_requests.push(core_event_request);
        }

        // Emit all events through the EventService
        let (events_processed, last_event) = match fence.flatten() {
            Some(fence) => self
                .event_service
                .emit_batch_fenced(event_requests, append_fence(&fence))
                .await
                .map_err(fenced_append_status)?,
            None => self
                .event_service
                .emit_batch_returning_last(event_requests)
                .await
                .map_err(|e| {
                    tracing::error!("Failed to emit event batch: {}", e);
                    Status::internal("Failed to store events")
                })?,
        };

        Ok(Response::new(EmitEventStreamResponse {
            events_processed,
            last_event: last_event.as_ref().map(schema_event_to_proto),
        }))
    }

    pub(crate) async fn handle_emit_event(
        &self,
        request: Request<EmitEventRequest>,
    ) -> Result<Response<EmitEventResponse>, Status> {
        let req = request.into_inner();
        let proto_event_request = req
            .event
            .ok_or_else(|| Status::invalid_argument("Missing event"))?;

        // Convert proto EventRequest to core EventRequest using typed conversions
        let core_event_request = proto_event_request_to_schema(proto_event_request)
            .map_err(|e| Status::invalid_argument(format!("Invalid event: {e}")))?;

        // Emit through the EventService
        let stored_event = match req.actor_fence {
            Some(fence) => self
                .event_service
                .emit_batch_fenced(vec![core_event_request], append_fence(&fence))
                .await
                .map_err(fenced_append_status)?
                .1
                .ok_or_else(|| Status::internal("Failed to store event"))?,
            None => self
                .event_service
                .emit(core_event_request)
                .await
                .map_err(|e| {
                    tracing::error!("Failed to emit event: {}", e);
                    Status::internal("Failed to store event")
                })?,
        };

        // Return the full stored event with id and sequence
        Ok(Response::new(EmitEventResponse {
            event: Some(schema_event_to_proto(&stored_event)),
        }))
    }

    pub(crate) async fn handle_commit_exec(
        &self,
        _request: Request<CommitExecRequest>,
    ) -> Result<Response<CommitExecResponse>, Status> {
        // No-op for now - exec_id tracking for idempotency can be added later
        Ok(Response::new(CommitExecResponse { committed: true }))
    }
}

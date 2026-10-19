//! Batched event store over `EmitEventStream` (see `crate::write_behind`).

use everruns_contracts::error::{AgentLoopError, Result};
use everruns_internal_protocol::proto;

use super::{GrpcAdapter, core_event_request_to_proto, grpc_status_to_error, proto_event_to_core};
use crate::actor_lease_store::{ACTOR_LEASE_LOST, ActorFence, actor_lease_lost};
use crate::core::Event;
use crate::core::events::EventRequest;

fn proto_fence(fence: &ActorFence) -> proto::ActorFence {
    proto::ActorFence {
        holder: fence.holder.clone(),
        fence: fence.fence_i64(),
    }
}

/// `request` as an unfenced `EmitEvent` (no actor lease writes it).
pub(super) fn unfenced_request(request: &EventRequest) -> Result<proto::EmitEventRequest> {
    Ok(proto::EmitEventRequest {
        event: Some(core_event_request_to_proto(request)?),
        actor_fence: None,
    })
}

/// A refused fenced append is the lease-lost error; anything else maps as
/// usual.
fn fenced_status_to_error(
    session_id: everruns_contracts::typed_id::SessionId,
    status: tonic::Status,
) -> AgentLoopError {
    if status.code() == tonic::Code::FailedPrecondition
        && status.message().starts_with(ACTOR_LEASE_LOST)
    {
        return actor_lease_lost(session_id, status.message().to_string());
    }
    grpc_status_to_error(status)
}

impl GrpcAdapter {
    /// Store events in order with one `EmitEventStream` call; the control
    /// plane inserts consecutive durable events in one statement. Returns the
    /// last event as stored.
    pub(crate) async fn emit_stored_batch(
        &self,
        requests: Vec<EventRequest>,
    ) -> Result<Option<Event>> {
        self.emit_stream(requests, None).await
    }

    /// Store `requests` as the holder of the actor lease `fence` (see
    /// `WorkerAdapters::emit_events_fenced`). One blocking event goes as a
    /// unary `EmitEvent`, a batch as one `EmitEventStream`; both carry the
    /// fence. A lone ephemeral event under NATS is no log append (the control
    /// plane skips PostgreSQL for it) and goes fire-and-forget, unfenced.
    pub(crate) async fn emit_fenced(
        &self,
        mut requests: Vec<EventRequest>,
        fence: &ActorFence,
    ) -> Result<Option<Event>> {
        let session_id = fence.session_id;
        if requests.len() == 1
            && let Some(request) = requests.pop()
        {
            if request.is_ephemeral() && super::server_supports_ephemeral_skip() {
                return crate::core::event_emitter::EventEmitter::emit(self, request)
                    .await
                    .map(Some);
            }
            let grpc_request = proto::EmitEventRequest {
                event: Some(core_event_request_to_proto(&request)?),
                actor_fence: Some(proto_fence(fence)),
            };
            let response = self
                .client
                .inner
                .client()
                .emit_event(grpc_request)
                .await
                .map_err(|status| fenced_status_to_error(session_id, status))?;
            let event = response
                .into_inner()
                .event
                .ok_or_else(|| AgentLoopError::store("No event in response"))?;
            return proto_event_to_core(event).map(Some);
        }
        self.emit_stream(requests, Some(fence)).await
    }

    async fn emit_stream(
        &self,
        requests: Vec<EventRequest>,
        fence: Option<&ActorFence>,
    ) -> Result<Option<Event>> {
        let expected = requests.len();
        let messages = requests
            .iter()
            .map(|request| {
                Ok(proto::EmitEventRequest {
                    event: Some(core_event_request_to_proto(request)?),
                    actor_fence: fence.map(proto_fence),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let mut client = self.client.inner.client();
        let response = client
            .emit_event_stream(tonic::codegen::tokio_stream::iter(messages))
            .await
            .map_err(|status| match fence {
                Some(fence) => fenced_status_to_error(fence.session_id, status),
                None => grpc_status_to_error(status),
            })?;
        let response = response.into_inner();
        let processed = response.events_processed;
        if usize::try_from(processed).ok() != Some(expected) {
            return Err(AgentLoopError::store(format!(
                "control plane stored {processed} of {expected} events"
            )));
        }
        response.last_event.map(proto_event_to_core).transpose()
    }
}

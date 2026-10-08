//! Batched event store over `EmitEventStream` (see `crate::write_behind`).

use everruns_contracts::error::{AgentLoopError, Result};
use everruns_internal_protocol::proto;

use super::{GrpcAdapter, core_event_request_to_proto, grpc_status_to_error, proto_event_to_core};
use crate::core::Event;
use crate::core::events::EventRequest;

impl GrpcAdapter {
    /// Store events in order with one `EmitEventStream` call; the control
    /// plane inserts consecutive durable events in one statement. Returns the
    /// last event as stored.
    pub(crate) async fn emit_stored_batch(
        &self,
        requests: Vec<EventRequest>,
    ) -> Result<Option<Event>> {
        let expected = requests.len();
        let messages = requests
            .iter()
            .map(|request| {
                Ok(proto::EmitEventRequest {
                    event: Some(core_event_request_to_proto(request)?),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let mut client = self.client.inner.client();
        let response = client
            .emit_event_stream(tonic::codegen::tokio_stream::iter(messages))
            .await
            .map_err(grpc_status_to_error)?;
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

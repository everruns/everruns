// Partial-stream store over the control plane's `GetPartialStream` RPC (EVE-532).
//
// Why: a reason step retried after its worker died must find the stream the
// lost attempt left open, so it completes that message under its id instead
// of announcing the step and the message again. The events live on the
// server, so the worker asks it.
//
// Decision: a server without the RPC (`UNIMPLEMENTED`, an older control plane
// during a rolling upgrade) answers "no partial", the behavior before this
// store existed. Any other failure is an error, which `ReasonAtom` treats as
// an unknown prior.
//
// Decision: the lookup sees only what reached the server. The dead attempt
// stored its leading events write-behind, so the unflushed ones are absent and
// the retry announces what never landed.
//
// Its own module because `grpc_adapters.rs` is on the source-size ratchet's
// debt list.

use crate::core::durability::{PartialStreamState, PartialStreamStore};
use crate::grpc_adapters::{GrpcClient, grpc_status_to_error, uuid_to_proto};
use async_trait::async_trait;
use everruns_contracts::error::{AgentLoopError, Result};
use everruns_contracts::typed_id::{MessageId, SessionId};
use everruns_internal_protocol::proto;

/// [`PartialStreamStore`] answered by the control plane.
pub(crate) struct GrpcPartialStreamStore {
    client: GrpcClient,
}

impl GrpcPartialStreamStore {
    pub(crate) fn new(client: GrpcClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl PartialStreamStore for GrpcPartialStreamStore {
    async fn get_partial_stream(
        &self,
        session_id: SessionId,
        turn_id: &str,
    ) -> Result<Option<PartialStreamState>> {
        let mut client = self.client.inner.client();
        let response = client
            .get_partial_stream(proto::GetPartialStreamRequest {
                session_id: Some(uuid_to_proto(session_id.uuid())),
                turn_id: turn_id.to_string(),
            })
            .await;
        match response {
            Ok(response) => response.into_inner().partial.map(from_proto).transpose(),
            Err(status) if status.code() == tonic::Code::Unimplemented => Ok(None),
            Err(status) => Err(grpc_status_to_error(status)),
        }
    }
}

fn from_proto(partial: proto::PartialStream) -> Result<PartialStreamState> {
    let message_id = MessageId::parse(&partial.message_id)
        .map_err(|error| AgentLoopError::store(format!("partial stream message id: {error}")))?;
    let reasoning_state = (!partial.reasoning_state_json.is_empty())
        .then(|| serde_json::from_slice(&partial.reasoning_state_json))
        .transpose()
        .map_err(|error| {
            AgentLoopError::store(format!("partial stream reasoning state: {error}"))
        })?;
    Ok(PartialStreamState {
        reasoning_state,
        message_id,
        accumulated: partial.accumulated,
        attempt_settled: partial.attempt_settled,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wire(message_id: &str, reasoning_state_json: Vec<u8>) -> proto::PartialStream {
        proto::PartialStream {
            message_id: message_id.to_string(),
            accumulated: "so far".to_string(),
            reasoning_state_json,
            attempt_settled: true,
        }
    }

    #[test]
    fn a_wire_partial_keeps_its_id_text_and_settled_flag() {
        let message_id = MessageId::new();
        let partial = from_proto(wire(&message_id.to_string(), Vec::new())).unwrap();
        assert_eq!(partial.message_id, message_id);
        assert_eq!(partial.accumulated, "so far");
        assert!(partial.attempt_settled);
        assert!(partial.reasoning_state.is_none());
    }

    #[test]
    fn a_wire_partial_carries_its_reasoning_state() {
        let state = everruns_contracts::reasoning_updates::ReasoningState {
            epoch: "epoch".into(),
            baseline: Some(everruns_contracts::ReasoningEffort::Low),
            effective: Some(everruns_contracts::ReasoningEffort::Max),
            pending: None,
        };
        let json = serde_json::to_vec(&state).unwrap();
        let partial = from_proto(wire(&MessageId::new().to_string(), json)).unwrap();
        assert_eq!(partial.reasoning_state.unwrap().effective, state.effective);
    }

    #[test]
    fn a_malformed_wire_partial_is_an_error() {
        assert!(from_proto(wire("not-a-message-id", Vec::new())).is_err());
        assert!(from_proto(wire(&MessageId::new().to_string(), b"{".to_vec())).is_err());
    }
}

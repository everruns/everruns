//! The server's turns on the framework's turn entry point.
//!
//! Decisions:
//! - The server starts, continues and cancels every turn through
//!   `everruns_core::host::TurnBackend`, the same trait the framework's
//!   sessions use; in production that is `DurableRunner`. There is no
//!   server-only runner trait.
//! - The server persists input itself before it starts a turn (the message's
//!   `input.message` event, or a waiting-turn resolution's events), so it
//!   always starts from stored input: `TurnInput::StoredMessage` with the
//!   session's `TurnScope`, or `TurnInput::RecordedToolResults` keyed by the
//!   resolution id.
//! - The turn id on these requests only labels the ticket: a platform turn's
//!   id is minted by its input step. Callers that need no result drop the
//!   ticket; the workflow already started before `start_turn` returned.

use everruns_contracts::error::AgentLoopError;
use everruns_contracts::typed_id::{AgentId, HarnessId, MessageId, SessionId, TurnId};
use everruns_core::host::{TurnBackend, TurnInput, TurnRequest, TurnScope};
use uuid::Uuid;

/// Start the turn `request` describes on `backend` and drop its ticket.
pub async fn start(backend: &dyn TurnBackend, request: TurnRequest) -> anyhow::Result<()> {
    backend.start_turn(request).await.map(drop).map_err(error)
}

/// A turn backend failure as the server reports it: a store failure keeps
/// the store's own message, which is what the server always logged and
/// returned.
pub fn error(error: AgentLoopError) -> anyhow::Error {
    match error {
        AgentLoopError::MessageStore(message) => anyhow::anyhow!(message),
        other => anyhow::Error::new(other),
    }
}

/// The scope of a session in `org_id` that runs `harness_id` and `agent_id`.
pub fn scope(org_id: i64, harness_id: HarnessId, agent_id: Option<AgentId>) -> TurnScope {
    TurnScope::new(org_id, harness_id, agent_id)
}

/// Start (or steer) the session's turn from the stored message `message_id`.
pub fn stored_message(
    session_id: SessionId,
    scope: TurnScope,
    message_id: MessageId,
    request_id: Option<String>,
) -> TurnRequest {
    TurnRequest::new(
        session_id,
        TurnId::new(),
        TurnInput::StoredMessage { message_id },
    )
    .with_scope(scope)
    .with_request_id(request_id)
}

/// Continue the session's parked turn from the tool resolution
/// `resolution_id`, whose events the server already recorded.
pub fn recorded_tool_results(session_id: SessionId, resolution_id: Uuid) -> TurnRequest {
    TurnRequest::new(
        session_id,
        TurnId::new(),
        TurnInput::RecordedToolResults { resolution_id },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_message_names_the_scope_and_request() {
        let session_id = SessionId::new();
        let harness_id = HarnessId::new();
        let message_id = MessageId::new();
        let request = stored_message(
            session_id,
            scope(7, harness_id, None),
            message_id,
            Some("req".to_string()),
        );
        assert_eq!(request.session_id, session_id);
        assert_eq!(request.scope, Some(TurnScope::new(7, harness_id, None)));
        assert_eq!(request.request_id.as_deref(), Some("req"));
        assert!(matches!(
            request.input,
            TurnInput::StoredMessage { message_id: id } if id == message_id
        ));
    }

    #[test]
    fn store_failures_keep_their_message() {
        let failure = error(AgentLoopError::store("Failed to get workflow status"));
        assert_eq!(failure.to_string(), "Failed to get workflow status");
        let cancelled = error(AgentLoopError::Cancelled);
        assert!(cancelled.downcast_ref::<AgentLoopError>().is_some());
    }

    #[test]
    fn recorded_tool_results_carry_the_resolution() {
        let resolution_id = Uuid::now_v7();
        let request = recorded_tool_results(SessionId::new(), resolution_id);
        assert_eq!(request.scope, None);
        assert!(matches!(
            request.input,
            TurnInput::RecordedToolResults { resolution_id: id } if id == resolution_id
        ));
    }
}

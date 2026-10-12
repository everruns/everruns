//! Partial-stream lookup for ContinuePartial recovery (EVE-532).
//!
//! A worker re-running a reason step asks for the stream a lost attempt left
//! open in the turn, so it can complete that message under its id instead of
//! announcing the step and the message a second time.
//!
//! Decisions:
//! - Answered by `PgPartialStreamStore` over the canonical `events` table. A
//!   worker stores a phase's leading events write-behind, so a dead worker's
//!   unflushed events are simply absent and the lookup sees what reached the
//!   server; the retry then announces what never landed.
//! - No shared PostgreSQL pool is `FAILED_PRECONDITION`, as for native async:
//!   the store cannot answer, which the worker treats as an unknown prior.

use super::support::*;
use crate::worker_link::grpc_service::*;
use everruns_core::durability::PartialStreamStore;

impl WorkerServiceImpl {
    pub(crate) async fn handle_get_partial_stream(
        &self,
        request: Request<proto::GetPartialStreamRequest>,
    ) -> Result<Response<proto::GetPartialStreamResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?.into();
        if req.turn_id.is_empty() {
            return Err(Status::invalid_argument("missing turn_id"));
        }
        let partial = crate::storage::PgPartialStreamStore::new(self.db.pool().clone())
            .get_partial_stream(session_id, &req.turn_id)
            .await
            .map_err(|error| internal_status("Failed to look up partial stream", error))?
            .map(|partial| {
                let reasoning_state_json = partial
                    .reasoning_state
                    .map(|state| serde_json::to_vec(&state))
                    .transpose()
                    .map_err(|error| internal_status("Failed to encode reasoning state", error))?
                    .unwrap_or_default();
                Ok::<_, Status>(proto::PartialStream {
                    message_id: partial.message_id.to_string(),
                    accumulated: partial.accumulated,
                    reasoning_state_json,
                    attempt_settled: partial.attempt_settled,
                })
            })
            .transpose()?;
        Ok(Response::new(proto::GetPartialStreamResponse { partial }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worker_link::grpc_service::tests::{create_grpc_test_session, test_worker_service};
    use everruns_contracts::typed_id::{MessageId, SessionId, TurnId};
    use everruns_core::events::{
        EventContext, OutputMessageCompletedData, OutputMessageStartedData, ReasonCompletedData,
    };

    async fn emit(
        service: &WorkerServiceImpl,
        session_id: SessionId,
        turn_id: TurnId,
        data: impl Into<everruns_core::events::EventData>,
    ) {
        let context = EventContext {
            turn_id: Some(turn_id),
            ..EventContext::empty()
        };
        service
            .event_service
            .emit(everruns_core::EventRequest::new(session_id, context, data))
            .await
            .expect("emit event");
    }

    async fn started(
        service: &WorkerServiceImpl,
        session_id: SessionId,
        turn_id: TurnId,
    ) -> MessageId {
        let message_id = MessageId::new();
        emit(
            service,
            session_id,
            turn_id,
            OutputMessageStartedData {
                reasoning_state: None,
                turn_id,
                message_id,
                model: None,
                iteration: Some(1),
                phase: None,
            },
        )
        .await;
        message_id
    }

    async fn lookup(
        service: &WorkerServiceImpl,
        session_id: SessionId,
        turn_id: TurnId,
    ) -> Option<proto::PartialStream> {
        service
            .handle_get_partial_stream(Request::new(proto::GetPartialStreamRequest {
                session_id: Some(everruns_internal_protocol::uuid_to_proto_uuid(
                    session_id.uuid(),
                )),
                turn_id: turn_id.to_string(),
            }))
            .await
            .expect("partial-stream lookup")
            .into_inner()
            .partial
    }

    #[tokio::test]
    async fn reports_the_stream_a_lost_attempt_left_open_and_nothing_once_it_completes() {
        let service = test_worker_service().await;
        let (session_id, _) = create_grpc_test_session(&service).await;
        let turn_id = TurnId::new();
        assert!(lookup(&service, session_id, turn_id).await.is_none());

        let first = started(&service, session_id, turn_id).await;
        let message = everruns_core::RuntimeMessage::assistant("checking").with_id(first);
        emit(
            &service,
            session_id,
            turn_id,
            OutputMessageCompletedData::new(message),
        )
        .await;
        assert!(
            lookup(&service, session_id, turn_id).await.is_none(),
            "a completed stream is not partial"
        );

        let open = started(&service, session_id, turn_id).await;
        let partial = lookup(&service, session_id, turn_id)
            .await
            .expect("the latest stream is open");
        assert_eq!(partial.message_id, open.to_string());
        assert!(partial.accumulated.is_empty());
        assert!(partial.reasoning_state_json.is_empty());
        assert!(!partial.attempt_settled);
        assert!(
            lookup(&service, session_id, TurnId::new()).await.is_none(),
            "another turn has no partial"
        );

        emit(
            &service,
            session_id,
            turn_id,
            ReasonCompletedData::failure("transient".into(), None),
        )
        .await;
        let partial = lookup(&service, session_id, turn_id)
            .await
            .expect("a failed attempt leaves its stream open");
        assert_eq!(partial.message_id, open.to_string());
        assert!(partial.attempt_settled);
    }

    #[tokio::test]
    async fn a_request_without_a_turn_is_invalid() {
        let service = test_worker_service().await;
        let (session_id, _) = create_grpc_test_session(&service).await;
        let status = service
            .handle_get_partial_stream(Request::new(proto::GetPartialStreamRequest {
                session_id: Some(everruns_internal_protocol::uuid_to_proto_uuid(
                    session_id.uuid(),
                )),
                turn_id: String::new(),
            }))
            .await
            .expect_err("missing turn id");
        assert_eq!(status.code(), tonic::Code::InvalidArgument);
    }
}

//! `GetTurnContext` with `omit_messages_and_model`: a worker executing a phase
//! assembles history and the model itself, so the server skips loading both.
//!
//! Its own file because `tests.rs` is on the size ratchet's debt list.

use super::tests::{create_grpc_test_session, test_worker_service};
use super::*;

async fn turn_context(
    service: &WorkerServiceImpl,
    session_id: everruns_contracts::typed_id::SessionId,
    omit: Option<bool>,
) -> GetTurnContextResponse {
    service
        .handle_get_turn_context(Request::new(GetTurnContextRequest {
            session_id: Some(everruns_internal_protocol::uuid_to_proto_uuid(
                session_id.uuid(),
            )),
            org_id: everruns_core::DEFAULT_ORG_ID,
            message_limit: None,
            input_message_id: None,
            omit_messages_and_model: omit,
        }))
        .await
        .expect("turn context")
        .into_inner()
}

#[tokio::test]
async fn omitting_messages_and_model_keeps_the_rest_of_the_context() {
    let service = test_worker_service().await;
    let (session_id, harness_id) = create_grpc_test_session(&service).await;
    service
        .event_service
        .emit(everruns_core::EventRequest::new(
            session_id,
            everruns_core::events::EventContext::empty(),
            everruns_core::events::InputMessageData::new(everruns_core::RuntimeMessage::user(
                "hello",
            )),
        ))
        .await
        .expect("emit input message");

    let full = turn_context(&service, session_id, None).await;
    assert_eq!(full.messages.len(), 1, "default still loads history");

    let lean = turn_context(&service, session_id, Some(true)).await;
    assert!(lean.messages.is_empty());
    assert!(lean.model.is_none());
    assert!(!lean.messages_truncated);
    // Session and harness, which the worker does use, are unchanged.
    let session = lean.session.expect("session");
    assert_eq!(session.id, full.session.expect("session").id);
    assert_eq!(
        lean.harness.and_then(|harness| harness.id),
        Some(everruns_internal_protocol::uuid_to_proto_uuid(
            harness_id.uuid()
        ))
    );
}

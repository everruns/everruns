//! EVE-1235: a worker still emitting for a session whose row is gone, renewing
//! its actor lease on it, or starting a turn queued before it went, must get
//! "session not found" naming the session on both transports, never an
//! internal store error or a plain lost lease.
//!
//! A session's row goes only when its user or organization is erased; the
//! tests reproduce that with `purge_session_for_test`. Deleting a session
//! keeps the row (soft delete): no new turn starts on it, and what a worker
//! still writes is stored. The last tests here cover that.

use super::test_adapters;
use crate::services::EventService;
use crate::storage::test_database::test_session_row;
use everruns_contracts::error::AgentLoopError;
use everruns_contracts::typed_id::SessionId;
use everruns_core::events::{EventContext, InputMessageData, ToolCompletedData};
use everruns_core::{DEFAULT_ORG_ID, EventRequest, RuntimeMessage};
use everruns_worker::worker_adapters::WorkerAdapters;

fn late_event(session_id: SessionId) -> EventRequest {
    EventRequest::new(
        session_id,
        EventContext::empty(),
        InputMessageData::new(RuntimeMessage::user("late")),
    )
}

/// A session that existed, held events, and whose row was then erased.
async fn deleted_session(adapters: &super::DirectWorkerAdapters) -> SessionId {
    let session = adapters
        .db
        .create_session(test_session_row(DEFAULT_ORG_ID))
        .await
        .expect("create session");
    adapters
        .event_service
        .emit(late_event(session.id))
        .await
        .expect("emit before delete");
    assert!(
        adapters
            .db
            .purge_session_for_test(DEFAULT_ORG_ID, session.id)
            .await
            .expect("erase session")
    );
    session.id
}

#[tokio::test]
async fn direct_adapter_reports_deleted_session_as_not_found() {
    let adapters = test_adapters();
    let session_id = deleted_session(&adapters).await;

    let error = adapters
        .emit_event(late_event(session_id))
        .await
        .expect_err("emit for a deleted session must fail");
    assert!(
        matches!(error, AgentLoopError::SessionNotFound(id) if id == session_id),
        "expected SessionNotFound, got {error:?}"
    );
    assert!(error.is_non_retryable());
}

fn grpc_service(
    adapters: &super::DirectWorkerAdapters,
) -> crate::worker_link::grpc_service::WorkerServiceImpl {
    crate::worker_link::grpc_service::WorkerServiceImpl::new(
        adapters.event_service.as_ref().clone(),
        adapters.db.clone(),
        None,
        None,
        crate::oss_host_composition_for_grade(everruns_core::DeploymentGrade::Dev),
    )
}

#[tokio::test]
async fn grpc_emit_reports_deleted_session_as_not_found() {
    let adapters = test_adapters();
    let session_id = deleted_session(&adapters).await;
    let grpc_service = grpc_service(&adapters);

    let status = grpc_service
        .handle_emit_event(tonic::Request::new(
            everruns_internal_protocol::proto::EmitEventRequest {
                event: Some(everruns_internal_protocol::schema_event_request_to_proto(
                    &late_event(session_id),
                )),
                actor_fence: None,
                return_events: false,
            },
        ))
        .await
        .expect_err("emit for a deleted session must fail");
    assert_eq!(status.code(), tonic::Code::NotFound, "{status:?}");
    assert_eq!(status.message(), format!("Session not found: {session_id}"));
}

/// The batched insert behind `EmitEventStream` (and the worker's write-behind
/// queue) fails on the `event_sequences` FK rather than the `events` one.
#[tokio::test]
async fn batched_emit_reports_deleted_session() {
    let adapters = test_adapters();
    let session_id = deleted_session(&adapters).await;

    let error = adapters
        .event_service
        .emit_batch_returning_last(vec![late_event(session_id), late_event(session_id)])
        .await
        .expect_err("batch for a deleted session must fail");
    assert_eq!(EventService::deleted_session(&error), Some(session_id));
    assert!(matches!(
        EventService::worker_emit_error(error),
        AgentLoopError::SessionNotFound(id) if id == session_id
    ));
}

/// A service-MCP tool event reads the session before the insert; a deleted
/// session there is the same "session gone", not a provenance failure.
#[tokio::test]
async fn service_mcp_tool_event_reports_deleted_session() {
    let adapters = test_adapters();
    let session_id = deleted_session(&adapters).await;

    let error = adapters
        .emit_event(EventRequest::new(
            session_id,
            EventContext::empty(),
            ToolCompletedData::success(
                "call-1".to_string(),
                "mcp_linear__create_issue".to_string(),
                Vec::new(),
                None,
            ),
        ))
        .await
        .expect_err("emit for a deleted session must fail");
    assert!(
        matches!(error, AgentLoopError::SessionNotFound(id) if id == session_id),
        "expected SessionNotFound, got {error:?}"
    );
}

/// A turn queued before the delete loads its context first; both transports
/// name the missing session so the worker can stop the turn quietly.
#[tokio::test]
async fn turn_context_for_deleted_session_is_session_not_found() {
    let adapters = test_adapters();
    let session_id = deleted_session(&adapters).await;

    let error = match adapters
        .load_turn_context(DEFAULT_ORG_ID, session_id.uuid())
        .await
    {
        Ok(_) => panic!("turn context for a deleted session must fail"),
        Err(error) => error,
    };
    assert!(
        matches!(error, AgentLoopError::SessionNotFound(id) if id == session_id),
        "expected SessionNotFound, got {error:?}"
    );

    let status = grpc_service(&adapters)
        .handle_get_turn_context(tonic::Request::new(
            everruns_internal_protocol::proto::GetTurnContextRequest {
                session_id: Some(everruns_internal_protocol::proto::Uuid {
                    value: session_id.uuid().to_string(),
                }),
                org_id: DEFAULT_ORG_ID,
                ..Default::default()
            },
        ))
        .await
        .expect_err("turn context for a deleted session must fail");
    assert_eq!(status.code(), tonic::Code::NotFound, "{status:?}");
    assert_eq!(status.message(), format!("Session not found: {session_id}"));
}

const ACTOR_TTL: std::time::Duration = std::time::Duration::from_secs(10);

/// A session leased to a worker's actor, then deleted under it.
async fn leased_then_deleted(adapters: &super::DirectWorkerAdapters) -> (SessionId, u64) {
    let session_id = adapters.db.create_test_session().await;
    adapters
        .db
        .mark_session_actor_runnable(session_id, DEFAULT_ORG_ID, 1)
        .await
        .expect("mark runnable");
    let leased = adapters
        .actor_lease_store()
        .lease_actors("worker-a", 10, ACTOR_TTL, None)
        .await
        .expect("lease");
    let actor = leased
        .iter()
        .find(|actor| actor.session_id == session_id)
        .expect("leased the session");
    assert!(
        adapters
            .db
            .purge_session_for_test(DEFAULT_ORG_ID, session_id)
            .await
            .expect("erase session")
    );
    (session_id, actor.fence)
}

/// The actor row goes with its session, so the holder's renew finds no lease.
/// That is a deleted session, not another holder's lease: both transports
/// name it, so the actor runtime stops the turn quietly.
#[tokio::test]
async fn actor_renew_for_a_deleted_session_is_session_not_found() {
    let adapters = test_adapters();
    let (session_id, fence) = leased_then_deleted(&adapters).await;

    let error = adapters
        .actor_lease_store()
        .renew_actor_lease(session_id, "worker-a", fence, ACTOR_TTL, 0)
        .await
        .expect_err("renew for a deleted session must fail");
    assert!(
        matches!(error, AgentLoopError::SessionNotFound(id) if id == session_id),
        "expected SessionNotFound, got {error:?}"
    );

    let status = grpc_service(&adapters)
        .handle_renew_actor_lease(tonic::Request::new(
            everruns_internal_protocol::proto::RenewActorLeaseRequest {
                session_id: Some(everruns_internal_protocol::uuid_to_proto_uuid(
                    session_id.uuid(),
                )),
                holder: "worker-a".to_string(),
                fence: i64::try_from(fence).unwrap(),
                ttl_ms: 0,
                consumed_seq: 0,
            },
        ))
        .await
        .expect_err("renew for a deleted session must fail");
    assert_eq!(status.code(), tonic::Code::NotFound, "{status:?}");
    assert_eq!(status.message(), format!("Session not found: {session_id}"));
}

/// A lease lost to another holder stays a plain lost lease.
#[tokio::test]
async fn actor_renew_with_a_stale_fence_is_still_a_lost_lease() {
    let adapters = test_adapters();
    let session_id = adapters.db.create_test_session().await;
    adapters
        .db
        .mark_session_actor_runnable(session_id, DEFAULT_ORG_ID, 1)
        .await
        .expect("mark runnable");
    let store = adapters.actor_lease_store();
    let leased = store
        .lease_actors("worker-a", 10, ACTOR_TTL, None)
        .await
        .expect("lease");
    let actor = leased
        .iter()
        .find(|actor| actor.session_id == session_id)
        .expect("leased the session");
    assert!(
        store
            .renew_actor_lease(session_id, "worker-b", actor.fence, ACTOR_TTL, 0)
            .await
            .expect("renew")
            .is_none()
    );
}

/// The holder's fenced appends (every platform turn's event writes) fail
/// the session FK the same way an unfenced append does.
#[tokio::test]
async fn fenced_emit_for_a_deleted_session_is_session_not_found() {
    let adapters = test_adapters();
    let (session_id, fence) = leased_then_deleted(&adapters).await;

    let error = adapters
        .emit_events_fenced(
            vec![late_event(session_id)],
            &everruns_worker::ActorFence::new(session_id, "worker-a", fence),
        )
        .await
        .expect_err("fenced emit for a deleted session must fail");
    assert!(
        matches!(error, AgentLoopError::SessionNotFound(id) if id == session_id),
        "expected SessionNotFound, got {error:?}"
    );

    let status = grpc_service(&adapters)
        .handle_emit_event(tonic::Request::new(
            everruns_internal_protocol::proto::EmitEventRequest {
                event: Some(everruns_internal_protocol::schema_event_request_to_proto(
                    &late_event(session_id),
                )),
                actor_fence: Some(everruns_internal_protocol::proto::ActorFence {
                    holder: "worker-a".to_string(),
                    fence: i64::try_from(fence).unwrap(),
                    detached_tool_call_id: None,
                }),
                return_events: false,
            },
        ))
        .await
        .expect_err("fenced emit for a deleted session must fail");
    assert_eq!(status.code(), tonic::Code::NotFound, "{status:?}");
    assert_eq!(status.message(), format!("Session not found: {session_id}"));
}

/// Deleting a session keeps its row, so a worker finishing the turn still
/// stores its events; they stay with the session's history.
#[tokio::test]
async fn events_for_a_soft_deleted_session_are_stored() {
    let adapters = test_adapters();
    let session_id = adapters.db.create_test_session().await;
    assert!(
        adapters
            .db
            .delete_session(DEFAULT_ORG_ID, session_id)
            .await
            .expect("delete session")
    );

    adapters
        .emit_event(late_event(session_id))
        .await
        .expect("a late event for a deleted session is stored");
    let stored: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM events WHERE session_id = $1")
        .bind(session_id)
        .fetch_one(adapters.db.pool())
        .await
        .expect("count events");
    assert_eq!(stored, 1);
}

/// No turn starts on a deleted session: its context reads as not found, the
/// signal a worker stops a queued turn on.
#[tokio::test]
async fn turn_context_for_a_soft_deleted_session_is_session_not_found() {
    let adapters = test_adapters();
    let session_id = adapters.db.create_test_session().await;
    assert!(
        adapters
            .db
            .delete_session(DEFAULT_ORG_ID, session_id)
            .await
            .expect("delete session")
    );

    let error = match adapters
        .load_turn_context(DEFAULT_ORG_ID, session_id.uuid())
        .await
    {
        Ok(_) => panic!("turn context for a deleted session must fail"),
        Err(error) => error,
    };
    assert!(
        matches!(error, AgentLoopError::SessionNotFound(id) if id == session_id),
        "expected SessionNotFound, got {error:?}"
    );
}

//! The session actor RPCs round-trip through the runnable index.

use super::*;

fn proto_uuid(id: everruns_contracts::typed_id::SessionId) -> proto::Uuid {
    everruns_internal_protocol::uuid_to_proto_uuid(id.uuid())
}

#[tokio::test]
async fn actor_rpcs_lease_renew_read_and_release() {
    let svc = test_worker_service().await;
    let db = svc.db.clone();
    let session_id = db.create_test_session().await;
    db.mark_session_actor_runnable(session_id, everruns_core::DEFAULT_ORG_ID, 3)
        .await
        .unwrap();

    let leased = svc
        .lease_actors(Request::new(proto::LeaseActorsRequest {
            holder: "worker-a".to_string(),
            limit: 10,
            ttl_ms: 0,
            max_leased_per_org: 0,
        }))
        .await
        .unwrap()
        .into_inner()
        .actors;
    assert_eq!(leased.len(), 1);
    let actor = &leased[0];
    assert_eq!(actor.session_id, Some(proto_uuid(session_id)));
    assert_eq!(actor.org_id, everruns_core::DEFAULT_ORG_ID);
    assert_eq!(actor.inbox_seq, 3);

    // A message lands mid-turn; renew reports it and a cancel.
    db.mark_session_actor_runnable(session_id, everruns_core::DEFAULT_ORG_ID, 5)
        .await
        .unwrap();
    assert_eq!(
        db.request_session_actor_cancel(session_id).await.unwrap(),
        crate::storage::ActorCancel::Requested
    );
    let renewal = svc
        .renew_actor_lease(Request::new(proto::RenewActorLeaseRequest {
            session_id: Some(proto_uuid(session_id)),
            holder: "worker-a".to_string(),
            fence: actor.fence,
            ttl_ms: 5_000,
            consumed_seq: 3,
        }))
        .await
        .unwrap()
        .into_inner();
    assert!(renewal.held);
    assert_eq!(renewal.inbox_seq, 5);
    assert!(renewal.cancel_requested);

    let journal = svc
        .read_actor_journal(Request::new(proto::ReadActorJournalRequest {
            session_id: Some(proto_uuid(session_id)),
            after_sequence: None,
            limit: 0,
        }))
        .await
        .unwrap()
        .into_inner();
    assert!(!journal.has_more);
    assert_eq!(
        journal.head_sequence,
        db.read_session_actor_journal(session_id, None, 0)
            .await
            .unwrap()
            .head_sequence
    );

    let released = svc
        .release_actor(Request::new(proto::ReleaseActorRequest {
            session_id: Some(proto_uuid(session_id)),
            holder: "worker-a".to_string(),
            fence: actor.fence,
            consumed_inbox_seq: 3,
        }))
        .await
        .unwrap()
        .into_inner();
    assert!(released.released);
    assert!(released.still_runnable, "input 5 starts the next turn");

    let stale = svc
        .renew_actor_lease(Request::new(proto::RenewActorLeaseRequest {
            session_id: Some(proto_uuid(session_id)),
            holder: "worker-a".to_string(),
            fence: actor.fence,
            ttl_ms: 0,
            consumed_seq: 0,
        }))
        .await
        .unwrap()
        .into_inner();
    assert!(!stale.held);
}

#[tokio::test]
async fn actor_rpcs_reject_a_missing_holder() {
    let svc = test_worker_service().await;
    let status = svc
        .lease_actors(Request::new(proto::LeaseActorsRequest {
            holder: " ".to_string(),
            limit: 1,
            ttl_ms: 0,
            max_leased_per_org: 0,
        }))
        .await
        .unwrap_err();
    assert_eq!(status.code(), tonic::Code::InvalidArgument);
}

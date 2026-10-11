//! The in-process worker's session actor calls round-trip through the
//! runnable index.

use super::*;
use everruns_worker::actor_lease_store::ActorRelease;

const TTL: std::time::Duration = std::time::Duration::from_secs(10);

#[tokio::test]
async fn direct_adapter_leases_renews_reads_and_releases_an_actor() {
    let adapters = test_adapters();
    let session_id = adapters.db.create_test_session().await;
    let input = adapters
        .db
        .create_event(crate::storage::CreateEventRow {
            session_id,
            event_type: "turn.started".to_string(),
            ts: chrono::Utc::now(),
            context: serde_json::json!({}),
            data: serde_json::json!({}),
            metadata: None,
            tags: None,
        })
        .await
        .unwrap()
        .sequence;
    adapters
        .db
        .mark_session_actor_runnable(session_id, everruns_core::DEFAULT_ORG_ID, i64::from(input))
        .await
        .unwrap();

    let store = adapters.actor_lease_store();
    let leased = store.lease_actors("worker-a", 10, TTL, None).await.unwrap();
    assert_eq!(leased.len(), 1);
    let actor = &leased[0];
    assert_eq!(actor.session_id, session_id);
    assert_eq!(actor.org_id, everruns_core::DEFAULT_ORG_ID);
    assert_eq!(actor.inbox_seq, i64::from(input));
    assert!(
        store
            .lease_actors("worker-b", 10, TTL, None)
            .await
            .unwrap()
            .is_empty()
    );

    let renewal = store
        .renew_actor_lease(session_id, "worker-a", actor.fence, TTL)
        .await
        .unwrap()
        .expect("still held");
    assert!(!renewal.cancel_requested);
    assert!(
        store
            .renew_actor_lease(session_id, "worker-b", actor.fence, TTL)
            .await
            .unwrap()
            .is_none()
    );

    let journal = store.read_actor_journal(session_id, None, 0).await.unwrap();
    assert_eq!(journal.events.len(), 1);
    assert_eq!(journal.events[0].event_type, "turn.started");
    assert_eq!(journal.head_sequence, Some(input));

    assert_eq!(
        store
            .release_actor(session_id, "worker-a", actor.fence, i64::from(input))
            .await
            .unwrap(),
        ActorRelease::Released {
            still_runnable: false
        }
    );
    assert_eq!(
        store
            .release_actor(session_id, "worker-a", actor.fence, i64::from(input))
            .await
            .unwrap(),
        ActorRelease::LeaseLost
    );
}

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
        .renew_actor_lease(session_id, "worker-a", actor.fence, TTL, 0)
        .await
        .unwrap()
        .expect("still held");
    assert!(!renewal.cancel_requested);
    assert!(
        store
            .renew_actor_lease(session_id, "worker-b", actor.fence, TTL, 0)
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

fn assistant_event(session_id: SessionId, text: &str) -> EventRequest {
    EventRequest::new(
        session_id,
        everruns_core::events::EventContext::empty(),
        everruns_core::events::OutputMessageCompletedData::new(
            everruns_core::RuntimeMessage::assistant(text),
        ),
    )
}

async fn log_types(adapters: &DirectWorkerAdapters, session_id: SessionId) -> Vec<String> {
    adapters
        .db
        .read_session_actor_journal(session_id, Some(0), 0)
        .await
        .unwrap()
        .events
        .into_iter()
        .map(|row| row.event_type)
        .collect()
}

/// Every write of a leased turn carries its fence; a holder that lost its
/// lease cannot land an in-flight write, and its session adapter flips the
/// lease lost so the turn stops.
#[tokio::test]
async fn direct_adapter_appends_only_under_the_live_lease() {
    let adapters = test_adapters();
    let session_id = adapters.db.create_test_session().await;
    adapters
        .db
        .mark_session_actor_runnable(session_id, everruns_core::DEFAULT_ORG_ID, 1)
        .await
        .unwrap();
    let store = adapters.actor_lease_store();
    let first = store.lease_actors("worker-a", 1, TTL, None).await.unwrap();
    let current = everruns_worker::ActorFence::new(session_id, "worker-a", first[0].fence);

    let stored = adapters
        .emit_events_fenced(
            vec![
                assistant_event(session_id, "one"),
                assistant_event(session_id, "two"),
            ],
            &current,
        )
        .await
        .expect("the holder's batch lands")
        .expect("the last event comes back");
    assert!(stored.sequence.is_some());
    assert_eq!(log_types(&adapters, session_id).await.len(), 2);

    // The lease is lost to another worker while a write is in flight.
    sqlx::query(
        "UPDATE session_actors SET lease_expires_at = NOW() - INTERVAL '1 second' \
         WHERE session_id = $1",
    )
    .bind(session_id)
    .execute(adapters.db.database().pool())
    .await
    .unwrap();
    let second = store.lease_actors("worker-b", 1, TTL, None).await.unwrap();
    assert_eq!(second.len(), 1);

    let error = adapters
        .emit_events_fenced(vec![assistant_event(session_id, "late")], &current)
        .await
        .expect_err("a stale fence writes nothing");
    assert!(everruns_worker::is_actor_lease_lost(&error), "{error}");
    assert_eq!(log_types(&adapters, session_id).await.len(), 2);

    // Through the session adapter a leased host emits with: the refusal flips
    // the lease lost.
    let (lost_tx, lost) = tokio::sync::watch::channel(false);
    let emitter = everruns_worker::SessionAdapter::new(adapters.clone()).with_actor_fence(Some(
        current
            .clone()
            .with_lost_signal(std::sync::Arc::new(lost_tx)),
    ));
    let error = everruns_core::event_emitter::EventEmitter::emit(
        &emitter,
        assistant_event(session_id, "later"),
    )
    .await
    .expect_err("refused");
    assert!(everruns_worker::is_actor_lease_lost(&error), "{error}");
    assert!(*lost.borrow(), "the refusal marks the lease lost");
    assert_eq!(log_types(&adapters, session_id).await.len(), 2);

    // The new holder's write lands.
    let next = everruns_worker::ActorFence::new(session_id, "worker-b", second[0].fence);
    adapters
        .emit_events_fenced(vec![assistant_event(session_id, "three")], &next)
        .await
        .expect("the new holder writes");
    assert_eq!(log_types(&adapters, session_id).await.len(), 3);
}

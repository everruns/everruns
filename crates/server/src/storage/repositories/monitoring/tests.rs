use super::*;
use crate::storage::{CreateEventRow, StorageBackend, test_database::test_session_row};
use everruns_contracts::typed_id::SessionId;
use std::time::Duration;

async fn runnable(db: &StorageBackend, org: i64) -> SessionId {
    let id = db.create_session(test_session_row(org)).await.unwrap().id;
    db.mark_session_actor_runnable(id, org, 3).await.unwrap();
    id
}
async fn phase(db: &StorageBackend, id: SessionId, name: &str) {
    db.create_event(CreateEventRow {
        session_id: id,
        event_type: name.into(),
        ts: Utc::now(),
        context: serde_json::json!({}),
        data: serde_json::json!({"private": "must-not-leave"}),
        metadata: None,
        tags: None,
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn counts_phases_by_live_ownership_and_never_assigns_expired_work() {
    let db = StorageBackend::test_database();
    let a = runnable(&db, 2).await;
    let b = runnable(&db, 3).await;
    let leased = db
        .lease_session_actors("worker-a", 10, Duration::from_secs(10), 32)
        .await
        .unwrap();
    assert_eq!(leased.len(), 2);
    phase(&db, a, "reason.started").await;
    phase(&db, b, "act.started").await;
    let now = Utc::now();
    let all = db.monitoring_totals(None, now).await.unwrap();
    assert_eq!((all.actors, all.leased, all.organizations), (2, 2, 2));
    let org = db.monitoring_cohorts(Some(2), now).await.unwrap();
    assert_eq!(org.len(), 1);
    assert_eq!(
        (&*org[0].phase, org[0].holder.as_deref()),
        ("model", Some("worker-a"))
    );
    assert!(
        db.monitoring_actor(Some(2), now, b.uuid())
            .await
            .unwrap()
            .is_none()
    );
    sqlx::query("UPDATE session_actors SET lease_expires_at = now() - interval '1 second' WHERE session_id = $1")
        .bind(a).execute(db.pool()).await.unwrap();
    let org = db.monitoring_cohorts(Some(2), Utc::now()).await.unwrap();
    assert_eq!(
        (&*org[0].phase, org[0].leased, org[0].expired),
        ("runnable", 0, 1)
    );
    assert!(org[0].holder.is_none());
}

#[tokio::test]
async fn renewal_does_not_create_ownership_noise_but_reacquisition_does() {
    let db = StorageBackend::test_database();
    let id = runnable(&db, 2).await;
    let held = db
        .lease_session_actors("worker-old", 1, Duration::from_secs(10), 32)
        .await
        .unwrap()
        .remove(0);
    let before = db
        .monitoring_actor(Some(2), Utc::now(), id.uuid())
        .await
        .unwrap()
        .unwrap();
    db.renew_session_actor_lease(id, "worker-old", held.fence, Duration::from_secs(10), 3)
        .await
        .unwrap();
    let after = db
        .monitoring_actor(Some(2), Utc::now(), id.uuid())
        .await
        .unwrap()
        .unwrap();
    assert!(after.lease_renewed_at >= before.lease_renewed_at);
    assert_eq!(
        db.monitoring_lease_events(Some(2), id.uuid())
            .await
            .unwrap()
            .len(),
        1
    );
    sqlx::query("UPDATE session_actors SET lease_expires_at = now() - interval '1 second' WHERE session_id = $1")
        .bind(id).execute(db.pool()).await.unwrap();
    let new = db
        .lease_session_actors("worker-new", 1, Duration::from_secs(10), 32)
        .await
        .unwrap()
        .remove(0);
    let history = db
        .monitoring_lease_events(Some(2), id.uuid())
        .await
        .unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(
        (&*history[0].kind, &*history[0].holder),
        ("reacquired", "worker-new")
    );
    assert_eq!(history[0].previous_holder.as_deref(), Some("worker-old"));
    assert!(new.fence > held.fence);
    assert!(
        db.monitoring_lease_events(Some(3), id.uuid())
            .await
            .unwrap()
            .is_empty()
    );
    db.release_session_actor(id, "worker-new", new.fence, 3)
        .await
        .unwrap();
    let idle = db
        .monitoring_actor(Some(2), Utc::now(), id.uuid())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(idle.phase, "idle");
    assert_eq!(idle.lease_holder, None);
    assert_eq!(
        db.monitoring_lease_events(Some(2), id.uuid())
            .await
            .unwrap()[0]
            .kind,
        "released"
    );
}

#[tokio::test]
async fn historical_events_do_not_enter_window_and_actor_pages_are_bounded() {
    let db = StorageBackend::test_database();
    for _ in 0..205 {
        runnable(&db, 2).await;
    }
    let page = db
        .monitoring_actors(Some(2), Utc::now(), 0, None, None)
        .await
        .unwrap();
    assert_eq!(page.len(), 201);
    let id = SessionId::from_uuid(page[0].session_id);
    phase(&db, id, "turn.completed").await;
    let rows = db.monitoring_rates(Some(2), Utc::now()).await.unwrap();
    assert_eq!(
        rows.iter()
            .find(|r| r.event_type == "turn.completed")
            .unwrap()
            .count,
        1
    );
    assert!(
        db.monitoring_rates(Some(2), Utc::now() + chrono::Duration::seconds(120))
            .await
            .unwrap()
            .is_empty()
    );
}

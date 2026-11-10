//! The Engine's session authority over session actors, on a real database.

use std::time::Duration;

use everruns_contracts::typed_id::SessionId;
use serde_json::json;
use uuid::Uuid;

use crate::storage::test_database::test_session_row;
use crate::storage::{CreateEventRow, StorageBackend};

const ORG: i64 = 2;
const TTL: Duration = Duration::from_secs(10);
const HOLDER: &str = "worker-a";

/// A runnable session leased to [`HOLDER`]; its fence.
async fn leased(db: &StorageBackend) -> (SessionId, i64) {
    let id = db
        .create_session(test_session_row(ORG))
        .await
        .expect("create session")
        .id;
    db.mark_session_actor_runnable(id, ORG, 1)
        .await
        .expect("mark runnable");
    let leases = db
        .lease_session_actors(HOLDER, 10, TTL, 100)
        .await
        .expect("lease");
    let lease = leases
        .into_iter()
        .find(|row| row.session_id == id)
        .expect("leased");
    (id, lease.fence)
}

async fn event(db: &StorageBackend, session_id: SessionId, event_type: &str) {
    db.create_event(CreateEventRow {
        session_id,
        event_type: event_type.to_string(),
        ts: chrono::Utc::now(),
        context: json!({}),
        data: json!({}),
        metadata: None,
        tags: None,
    })
    .await
    .expect("create event");
}

/// A key with `revisions` published, the last one current.
async fn key(db: &StorageBackend, revisions: usize) -> String {
    let key = db
        .database()
        .engine_agent_key(ORG, Uuid::new_v4(), None)
        .await
        .expect("key");
    for n in 0..revisions {
        db.database()
            .publish_engine_agent(
                &key,
                &json!({"format": 1, "agent": {"n": n}}),
                &n.to_string(),
            )
            .await
            .expect("publish");
    }
    key
}

async fn release(db: &StorageBackend, id: SessionId, fence: i64) {
    db.release_session_actor(id, HOLDER, fence, 1)
        .await
        .expect("release");
}

/// A session binds once, under its lease, and only between turns.
#[tokio::test]
async fn a_session_binds_between_turns_and_keeps_its_binding() {
    let db = StorageBackend::test_database();
    let pg = db.database();
    let key = key(&db, 1).await;
    let (id, fence) = leased(&db).await;
    assert_eq!(pg.engine_session_binding(id).await.unwrap(), None);

    event(&db, id, "turn.started").await;
    let open = pg
        .bind_engine_session(id, HOLDER, fence, &key, "r1", 1)
        .await
        .unwrap();
    assert_eq!(
        open, None,
        "a turn is open: it finishes on the worker's host"
    );

    event(&db, id, "turn.completed").await;
    let lost = pg
        .bind_engine_session(id, "worker-b", fence, &key, "r1", 1)
        .await
        .unwrap();
    assert_eq!(lost, None, "another holder binds nothing");
    let bound = pg
        .bind_engine_session(id, HOLDER, fence, &key, "r1", 1)
        .await
        .unwrap()
        .expect("bound");
    assert_eq!(
        (bound.agent_key.as_str(), bound.revision.as_str()),
        (key.as_str(), "r1")
    );

    let again = pg
        .bind_engine_session(id, HOLDER, fence, "ak_other", "r9", 1)
        .await
        .unwrap()
        .expect("still bound");
    assert_eq!(again, bound, "a bound session keeps its binding");
}

/// A claim moves the session to its key's current revision, except while a
/// turn is open.
#[tokio::test]
async fn a_claim_advances_only_between_turns() {
    let db = StorageBackend::test_database();
    let pg = db.database();
    let key = key(&db, 1).await;
    let (id, fence) = leased(&db).await;
    pg.bind_engine_session(id, HOLDER, fence, &key, "r1", 1)
        .await
        .unwrap()
        .expect("bound");
    pg.publish_engine_agent(&key, &json!({"format": 1, "agent": {"n": 9}}), "9")
        .await
        .unwrap();

    event(&db, id, "turn.started").await;
    let in_turn = pg
        .claim_engine_binding(id, HOLDER, fence)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(in_turn.revision, "r1");

    event(&db, id, "turn.failed").await;
    let between = pg
        .claim_engine_binding(id, HOLDER, fence)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(between.revision, "r2");
    assert_eq!(
        pg.claim_engine_binding(id, "worker-b", fence)
            .await
            .unwrap(),
        None,
        "a lost lease claims nothing"
    );
}

/// Receipts are written only under the lease; `absent` keeps the first.
#[tokio::test]
async fn receipts_are_fenced_and_absent_keeps_the_first() {
    let db = StorageBackend::test_database();
    let pg = db.database();
    let (id, fence) = leased(&db).await;

    let written = pg
        .commit_engine_session_record(id, HOLDER, fence, "assignment/a", &json!(1), true)
        .await
        .unwrap();
    assert_eq!(written, Some(true));
    let kept = pg
        .commit_engine_session_record(id, HOLDER, fence, "assignment/a", &json!(2), true)
        .await
        .unwrap();
    assert_eq!(kept, Some(false));
    pg.commit_engine_session_record(id, HOLDER, fence, "outcome/a", &json!(3), false)
        .await
        .unwrap();
    pg.commit_engine_session_record(id, HOLDER, fence, "outcome/a", &json!(4), false)
        .await
        .unwrap();
    let lost = pg
        .commit_engine_session_record(id, HOLDER, fence + 1, "assignment/b", &json!(5), true)
        .await
        .unwrap();
    assert_eq!(lost, None, "a stale fence writes nothing");

    assert_eq!(
        pg.engine_session_record(id, "assignment/a").await.unwrap(),
        Some(json!(1))
    );
    assert_eq!(
        pg.engine_session_records(id, "outcome/", 10).await.unwrap(),
        vec![("outcome/a".to_string(), json!(4))]
    );
    assert_eq!(
        pg.engine_session_record(id, "assignment/b").await.unwrap(),
        None
    );
}

/// A suspended session is not leased until it is resumed.
#[tokio::test]
async fn a_suspended_session_waits_for_a_resume() {
    let db = StorageBackend::test_database();
    let pg = db.database();
    let key = key(&db, 1).await;
    let (id, fence) = leased(&db).await;
    pg.bind_engine_session(id, HOLDER, fence, &key, "r1", 1)
        .await
        .unwrap()
        .expect("bound");
    assert!(
        pg.suspend_engine_session(id, HOLDER, fence, "missing_build")
            .await
            .unwrap()
    );
    db.mark_session_actor_runnable(id, ORG, 5).await.unwrap();
    release(&db, id, fence).await;

    let leases = db.lease_session_actors(HOLDER, 10, TTL, 100).await.unwrap();
    assert!(leases.iter().all(|row| row.session_id != id));
    let binding = pg.engine_session_binding(id).await.unwrap().unwrap();
    assert_eq!(binding.suspended_reason.as_deref(), Some("missing_build"));

    assert!(pg.resume_engine_session(id).await.unwrap());
    assert!(!pg.resume_engine_session(id).await.unwrap());
    let leases = db.lease_session_actors(HOLDER, 10, TTL, 100).await.unwrap();
    let lease = leases
        .iter()
        .find(|row| row.session_id == id)
        .expect("leased");
    assert!(lease.engine, "a bound session runs on the Engine");
}

/// A due timer makes an idle actor runnable; the claim that runs it clears it.
#[tokio::test]
async fn a_due_timer_wakes_the_session() {
    let db = StorageBackend::test_database();
    let pg = db.database();
    let key = key(&db, 1).await;
    let (id, fence) = leased(&db).await;
    pg.bind_engine_session(id, HOLDER, fence, &key, "r1", 1)
        .await
        .unwrap()
        .expect("bound");
    let due = chrono::Utc::now() - chrono::Duration::seconds(1);
    assert!(
        pg.schedule_engine_session(id, HOLDER, fence, Some(due))
            .await
            .unwrap()
    );
    assert!(
        !pg.schedule_engine_session(id, "worker-b", fence, None)
            .await
            .unwrap()
    );
    release(&db, id, fence).await;

    let leases = db.lease_session_actors(HOLDER, 10, TTL, 100).await.unwrap();
    let lease = leases
        .iter()
        .find(|row| row.session_id == id)
        .expect("woken");
    let claimed = pg
        .claim_engine_binding(id, HOLDER, lease.fence)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(claimed.wake_at, None, "the claim takes the due timer");
}

/// An unbound session of an org enrolled in Engine hosting is routed to the
/// Engine; other orgs' sessions are not.
#[tokio::test]
async fn enrolment_routes_unbound_sessions_to_the_engine() {
    let db = StorageBackend::test_database();
    let (id, _) = leased(&db).await;
    let _ = id;
    let other = db.create_session(test_session_row(ORG)).await.unwrap().id;
    db.mark_session_actor_runnable(other, ORG, 1).await.unwrap();
    let leases = db.lease_session_actors(HOLDER, 10, TTL, 100).await.unwrap();
    assert!(
        !leases
            .iter()
            .find(|row| row.session_id == other)
            .unwrap()
            .engine
    );

    sqlx::query("INSERT INTO org_feature_flags (org_id, flag_name, enabled) VALUES ($1, $2, true)")
        .bind(ORG)
        .bind(super::super::session_actors::ENGINE_HOSTING_FLAG)
        .execute(&db.database().pool)
        .await
        .unwrap();
    let third = db.create_session(test_session_row(ORG)).await.unwrap().id;
    db.mark_session_actor_runnable(third, ORG, 1).await.unwrap();
    let leases = db.lease_session_actors(HOLDER, 10, TTL, 100).await.unwrap();
    assert!(
        leases
            .iter()
            .find(|row| row.session_id == third)
            .unwrap()
            .engine
    );
}

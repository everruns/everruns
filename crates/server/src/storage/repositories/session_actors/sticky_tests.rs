//! Sticky sessions on a real database: the worker that released an actor
//! gets it back first, and nobody waits on a worker that cannot take it.

use super::*;
use crate::storage::StorageBackend;
use crate::storage::test_database::test_session_row;

const TTL: Duration = Duration::from_secs(10);
const ORG: i64 = 2;

async fn runnable(db: &StorageBackend, inbox_seq: i64) -> SessionId {
    let id = db
        .create_session(test_session_row(ORG))
        .await
        .expect("create session")
        .id;
    db.mark_session_actor_runnable(id, ORG, inbox_seq)
        .await
        .expect("mark runnable");
    id
}

/// How a worker looks in the registry.
#[derive(Clone, Copy)]
enum Worker {
    Ready,
    Full,
    Draining,
    Silent,
}

async fn register(db: &StorageBackend, id: &str, worker: Worker) {
    let (status, load, heartbeat_age_secs) = match worker {
        Worker::Ready => ("active", 3, 1),
        Worker::Full => ("active", 20, 1),
        Worker::Draining => ("draining", 3, 1),
        Worker::Silent => ("active", 3, 120),
    };
    sqlx::query(
        "INSERT INTO durable_workers \
           (id, worker_group, activity_types, max_concurrency, current_load, status, \
            last_heartbeat_at, accepting_tasks) \
         VALUES ($1, 'default', '{}', 20, $2, $3, \
                 NOW() - $4::float8 * INTERVAL '1 second', $3 = 'active')",
    )
    .bind(id)
    .bind(load)
    .bind(status)
    .bind(heartbeat_age_secs as f64)
    .execute(&db.database().pool)
    .await
    .expect("register worker");
}

async fn lease(db: &StorageBackend, holder: &str) -> Vec<LeasedActorRow> {
    db.lease_session_actors(holder, 10, TTL, DEFAULT_ACTOR_LEASES_PER_ORG)
        .await
        .expect("lease")
}

/// Run one turn of `id` on `holder` and release it with the next input
/// (`next_seq`) already waiting, so the actor stays runnable.
async fn run_turn_with_input_waiting(
    db: &StorageBackend,
    id: SessionId,
    holder: &str,
    next_seq: i64,
) -> i64 {
    let leased = lease(db, holder).await;
    assert_eq!(leased.len(), 1, "{holder} leases the session");
    let fence = leased[0].fence;
    db.mark_session_actor_runnable(id, ORG, next_seq)
        .await
        .unwrap();
    assert_eq!(
        db.release_session_actor(id, holder, fence, leased[0].inbox_seq)
            .await
            .unwrap(),
        Some(true),
        "the next input keeps the actor runnable"
    );
    fence
}

/// Age the actor's place in line past the grace.
async fn past_the_grace(db: &StorageBackend, id: SessionId) {
    sqlx::query(
        "UPDATE session_actors SET runnable_since = NOW() - INTERVAL '5 seconds' \
         WHERE session_id = $1",
    )
    .bind(id)
    .execute(&db.database().pool)
    .await
    .unwrap();
}

#[tokio::test]
async fn release_records_the_worker_and_the_wake_up_names_it() {
    let db = StorageBackend::test_database();
    register(&db, "sticky-a", Worker::Ready).await;
    let id = runnable(&db, 1).await;
    let leased = lease(&db, "sticky-a").await;
    db.release_session_actor(id, "sticky-a", leased[0].fence, 1)
        .await
        .unwrap();

    let row = db.get_session_actor(id).await.unwrap().unwrap();
    assert_eq!(row.preferred_worker.as_deref(), Some("sticky-a"));
    assert_eq!(
        db.mark_session_actor_runnable(id, ORG, 2).await.unwrap(),
        Some("sticky-a".to_string()),
        "the next message's wake-up goes to the worker that ran the last turn"
    );
}

#[tokio::test]
async fn a_new_session_wakes_every_worker() {
    let db = StorageBackend::test_database();
    let id = db.create_session(test_session_row(ORG)).await.unwrap().id;
    assert_eq!(
        db.mark_session_actor_runnable(id, ORG, 1).await.unwrap(),
        None
    );
}

#[tokio::test]
async fn the_preferred_worker_gets_its_session_back_and_others_wait_the_grace() {
    let db = StorageBackend::test_database();
    register(&db, "sticky-a", Worker::Ready).await;
    register(&db, "sticky-b", Worker::Ready).await;
    let id = runnable(&db, 1).await;
    run_turn_with_input_waiting(&db, id, "sticky-a", 2).await;

    assert!(
        lease(&db, "sticky-b").await.is_empty(),
        "another worker leaves it to its preferred worker within the grace"
    );
    let back = lease(&db, "sticky-a").await;
    assert_eq!(back.len(), 1);
    assert_eq!(back[0].session_id, id);
    assert_eq!(back[0].inbox_seq, 2);
}

#[tokio::test]
async fn past_the_grace_any_worker_takes_it() {
    let db = StorageBackend::test_database();
    register(&db, "sticky-a", Worker::Ready).await;
    register(&db, "sticky-b", Worker::Ready).await;
    let id = runnable(&db, 1).await;
    let fence = run_turn_with_input_waiting(&db, id, "sticky-a", 2).await;
    past_the_grace(&db, id).await;

    let taken = lease(&db, "sticky-b").await;
    assert_eq!(
        taken.len(),
        1,
        "the preferred worker did not come: anyone may"
    );
    assert!(taken[0].fence > fence);
}

#[tokio::test]
async fn nobody_waits_on_a_preferred_worker_that_cannot_take_the_turn() {
    for (name, worker) in [
        ("full", Some(Worker::Full)),
        ("draining", Some(Worker::Draining)),
        ("silent", Some(Worker::Silent)),
        ("unregistered", None),
    ] {
        let db = StorageBackend::test_database();
        if let Some(worker) = worker {
            register(&db, "sticky-a", worker).await;
        }
        let id = runnable(&db, 1).await;
        let fence = run_turn_with_input_waiting(&db, id, "sticky-a", 2).await;

        let taken = lease(&db, "sticky-b").await;
        assert_eq!(
            taken.len(),
            1,
            "a {name} preferred worker holds nothing back"
        );
        assert!(taken[0].fence > fence);
        assert_eq!(
            db.mark_session_actor_runnable(id, ORG, 3).await.unwrap(),
            None,
            "the wake-up for a {name} preferred worker goes to everyone"
        );
    }
}

#[tokio::test]
async fn a_takeover_fences_the_preferred_worker_out() {
    let db = StorageBackend::test_database();
    register(&db, "sticky-a", Worker::Ready).await;
    let id = runnable(&db, 1).await;
    run_turn_with_input_waiting(&db, id, "sticky-a", 2).await;

    // sticky-a takes the turn back, then stalls past its lease.
    let stalled = lease(&db, "sticky-a").await.remove(0);
    sqlx::query(
        "UPDATE session_actors SET lease_expires_at = NOW() - INTERVAL '1 second', \
             runnable_since = NOW() - INTERVAL '5 seconds' \
         WHERE session_id = $1",
    )
    .bind(id)
    .execute(&db.database().pool)
    .await
    .unwrap();

    let taken = lease(&db, "sticky-b").await.remove(0);
    assert!(taken.fence > stalled.fence);
    assert_eq!(
        db.renew_session_actor_lease(id, "sticky-a", stalled.fence, TTL, 2)
            .await
            .unwrap(),
        None,
        "the old holder's renew fails on the fence"
    );
    assert_eq!(
        db.release_session_actor(id, "sticky-a", stalled.fence, 2)
            .await
            .unwrap(),
        None,
        "and so does its release, which cannot move the preference either"
    );
    let row = db.get_session_actor(id).await.unwrap().unwrap();
    assert_eq!(row.lease_holder.as_deref(), Some("sticky-b"));
    assert_eq!(row.preferred_worker.as_deref(), Some("sticky-a"));
}

#[tokio::test]
async fn a_hand_off_clears_the_preference() {
    let db = StorageBackend::test_database();
    register(&db, "sticky-a", Worker::Ready).await;
    register(&db, "sticky-b", Worker::Ready).await;
    let id = runnable(&db, 1).await;
    run_turn_with_input_waiting(&db, id, "sticky-a", 2).await;

    // sticky-a leases the next turn and drains mid-turn.
    let leased = lease(&db, "sticky-a").await.remove(0);
    db.hand_off_session_actor(id, "sticky-a", leased.fence, 1)
        .await
        .unwrap();
    let row = db.get_session_actor(id).await.unwrap().unwrap();
    assert_eq!(row.preferred_worker, None);
    assert_eq!(
        lease(&db, "sticky-b").await.len(),
        1,
        "another worker resumes it at once"
    );
}

#[tokio::test]
async fn a_worker_takes_its_own_sessions_before_older_ones() {
    let db = StorageBackend::test_database();
    register(&db, "sticky-a", Worker::Ready).await;
    let mine = runnable(&db, 1).await;
    run_turn_with_input_waiting(&db, mine, "sticky-a", 2).await;
    let older = runnable(&db, 1).await;
    for (id, age_secs) in [(older, 5.0), (mine, 1.0)] {
        sqlx::query(
            "UPDATE session_actors \
             SET runnable_since = NOW() - $2::float8 * INTERVAL '1 second' \
             WHERE session_id = $1",
        )
        .bind(id)
        .bind(age_secs)
        .execute(&db.database().pool)
        .await
        .unwrap();
    }

    let first = db
        .lease_session_actors("sticky-a", 1, TTL, DEFAULT_ACTOR_LEASES_PER_ORG)
        .await
        .unwrap();
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].session_id, mine, "its own session goes first");
}

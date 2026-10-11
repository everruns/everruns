//! The runnable index and lease protocol on a real database.

use super::*;
use crate::storage::test_database::test_session_row;
use crate::storage::{CreateEventRow, StorageBackend};
use std::collections::HashSet;

const TTL: Duration = Duration::from_secs(10);
const ORG_A: i64 = 2;
const ORG_B: i64 = 3;

async fn session(db: &StorageBackend, org_id: i64) -> SessionId {
    db.create_session(test_session_row(org_id))
        .await
        .expect("create session")
        .id
}

async fn runnable(db: &StorageBackend, org_id: i64, inbox_seq: i64) -> SessionId {
    let id = session(db, org_id).await;
    db.mark_session_actor_runnable(id, org_id, inbox_seq)
        .await
        .expect("mark runnable");
    id
}

/// Move `session_id`'s lease expiry into the past, as a stalled holder's would.
async fn expire_lease(db: &StorageBackend, session_id: SessionId) {
    sqlx::query(
        "UPDATE session_actors SET lease_expires_at = NOW() - INTERVAL '1 second' \
         WHERE session_id = $1",
    )
    .bind(session_id)
    .execute(&db.database().pool)
    .await
    .expect("expire lease");
}

async fn event(db: &StorageBackend, session_id: SessionId, event_type: &str) -> i32 {
    db.create_event(CreateEventRow {
        session_id,
        event_type: event_type.to_string(),
        ts: chrono::Utc::now(),
        context: serde_json::json!({}),
        data: serde_json::json!({}),
        metadata: None,
        tags: None,
    })
    .await
    .expect("create event")
    .sequence
}

fn ids(rows: &[LeasedActorRow]) -> HashSet<SessionId> {
    rows.iter().map(|row| row.session_id).collect()
}

#[tokio::test]
async fn mark_runnable_is_an_upsert_whose_inbox_only_moves_forward() {
    let db = StorageBackend::test_database();
    let id = runnable(&db, ORG_A, 5).await;
    let first = db.get_session_actor(id).await.unwrap().unwrap();

    db.mark_session_actor_runnable(id, ORG_A, 3).await.unwrap();
    let again = db.get_session_actor(id).await.unwrap().unwrap();
    assert_eq!(
        again.inbox_seq, 5,
        "an older input never moves the inbox back"
    );
    assert_eq!(
        again.runnable_since, first.runnable_since,
        "a runnable actor keeps its place in line"
    );

    db.mark_session_actor_runnable(id, ORG_A, 9).await.unwrap();
    assert_eq!(
        db.get_session_actor(id).await.unwrap().unwrap().inbox_seq,
        9
    );
}

#[tokio::test]
async fn mark_runnable_commits_with_the_callers_transaction() {
    let db = StorageBackend::test_database();
    let id = session(&db, ORG_A).await;

    let mut tx = db.database().pool.begin().await.unwrap();
    mark_session_actor_runnable(&mut *tx, id, ORG_A, 1)
        .await
        .unwrap();
    drop(tx);
    assert!(
        db.get_session_actor(id).await.unwrap().is_none(),
        "a rolled-back input leaves no runnable marker"
    );

    let mut tx = db.database().pool.begin().await.unwrap();
    mark_session_actor_runnable(&mut *tx, id, ORG_A, 1)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert!(db.get_session_actor(id).await.unwrap().is_some());
}

#[tokio::test]
async fn two_holders_cannot_lease_the_same_actor() {
    let db = StorageBackend::test_database();
    let id = runnable(&db, ORG_A, 1).await;

    let first = db
        .lease_session_actors("worker-a", 10, TTL, DEFAULT_ACTOR_LEASES_PER_ORG)
        .await
        .unwrap();
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].session_id, id);
    assert_eq!(first[0].org_id, ORG_A);
    assert_eq!(first[0].fence, 1);
    assert_eq!(first[0].inbox_seq, 1);
    assert!(!first[0].cancel_requested);

    let second = db
        .lease_session_actors("worker-b", 10, TTL, DEFAULT_ACTOR_LEASES_PER_ORG)
        .await
        .unwrap();
    assert!(second.is_empty(), "a live lease is not taken");
}

#[tokio::test]
async fn an_expired_lease_is_taken_over_with_a_higher_fence() {
    let db = StorageBackend::test_database();
    let id = runnable(&db, ORG_A, 1).await;
    let old = db
        .lease_session_actors("worker-a", 1, TTL, DEFAULT_ACTOR_LEASES_PER_ORG)
        .await
        .unwrap()
        .remove(0);

    expire_lease(&db, id).await;
    let new = db
        .lease_session_actors("worker-b", 1, TTL, DEFAULT_ACTOR_LEASES_PER_ORG)
        .await
        .unwrap()
        .remove(0);
    assert_eq!(new.session_id, id);
    assert!(new.fence > old.fence);

    assert_eq!(
        db.renew_session_actor_lease(id, "worker-a", old.fence, TTL, 0)
            .await
            .unwrap(),
        None,
        "the stalled holder lost the lease"
    );
    assert_eq!(
        db.release_session_actor(id, "worker-a", old.fence, 1)
            .await
            .unwrap(),
        None,
        "the stalled holder cannot release the new holder's lease"
    );
    assert!(
        db.renew_session_actor_lease(id, "worker-b", new.fence, TTL, 0)
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn an_expired_lease_nobody_took_is_still_renewable() {
    let db = StorageBackend::test_database();
    let id = runnable(&db, ORG_A, 1).await;
    let lease = db
        .lease_session_actors("worker-a", 1, TTL, DEFAULT_ACTOR_LEASES_PER_ORG)
        .await
        .unwrap()
        .remove(0);
    expire_lease(&db, id).await;
    assert!(
        db.renew_session_actor_lease(id, "worker-a", lease.fence, TTL, 0)
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn release_with_a_moved_inbox_keeps_the_actor_runnable() {
    let db = StorageBackend::test_database();
    let id = runnable(&db, ORG_A, 4).await;
    let lease = db
        .lease_session_actors("worker-a", 1, TTL, DEFAULT_ACTOR_LEASES_PER_ORG)
        .await
        .unwrap()
        .remove(0);

    // A message lands while the turn for input 4 finishes.
    db.mark_session_actor_runnable(id, ORG_A, 7).await.unwrap();
    let renewal = db
        .renew_session_actor_lease(id, "worker-a", lease.fence, TTL, 0)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(renewal.inbox_seq, 7, "renew reports the new input");

    assert_eq!(
        db.release_session_actor(id, "worker-a", lease.fence, 4)
            .await
            .unwrap(),
        Some(true)
    );
    let next = db
        .lease_session_actors("worker-b", 1, TTL, DEFAULT_ACTOR_LEASES_PER_ORG)
        .await
        .unwrap();
    assert_eq!(ids(&next), HashSet::from([id]), "the next turn starts");
    assert_eq!(next[0].inbox_seq, 7);
}

#[tokio::test]
async fn release_with_nothing_new_clears_the_runnable_marker() {
    let db = StorageBackend::test_database();
    let id = runnable(&db, ORG_A, 4).await;
    let lease = db
        .lease_session_actors("worker-a", 1, TTL, DEFAULT_ACTOR_LEASES_PER_ORG)
        .await
        .unwrap()
        .remove(0);

    assert_eq!(
        db.release_session_actor(id, "worker-a", lease.fence, 4)
            .await
            .unwrap(),
        Some(false)
    );
    let row = db.get_session_actor(id).await.unwrap().unwrap();
    assert!(row.runnable_since.is_none());
    assert!(row.lease_holder.is_none());
    assert_eq!(row.lease_fence, lease.fence, "the fence survives release");
    assert!(
        db.lease_session_actors("worker-b", 1, TTL, DEFAULT_ACTOR_LEASES_PER_ORG)
            .await
            .unwrap()
            .is_empty()
    );

    // The next input leases with a fence above every earlier one.
    db.mark_session_actor_runnable(id, ORG_A, 5).await.unwrap();
    let next = db
        .lease_session_actors("worker-b", 1, TTL, DEFAULT_ACTOR_LEASES_PER_ORG)
        .await
        .unwrap()
        .remove(0);
    assert!(next.fence > lease.fence);
}

#[tokio::test]
async fn renew_reports_a_cancel_and_release_consumes_it() {
    let db = StorageBackend::test_database();
    let id = runnable(&db, ORG_A, 1).await;
    let lease = db
        .lease_session_actors("worker-a", 1, TTL, DEFAULT_ACTOR_LEASES_PER_ORG)
        .await
        .unwrap()
        .remove(0);

    assert_eq!(
        db.request_session_actor_cancel(id).await.unwrap(),
        ActorCancel::Requested
    );
    let renewal = db
        .renew_session_actor_lease(id, "worker-a", lease.fence, TTL, 0)
        .await
        .unwrap()
        .unwrap();
    assert!(renewal.cancel_requested);

    assert_eq!(
        db.release_session_actor(id, "worker-a", lease.fence, 1)
            .await
            .unwrap(),
        Some(false)
    );
    let row = db.get_session_actor(id).await.unwrap().unwrap();
    assert!(
        row.cancel_requested_at.is_none(),
        "the reported cancel is done"
    );
    assert_eq!(
        db.request_session_actor_cancel(id).await.unwrap(),
        ActorCancel::Idle,
        "an idle actor has no turn to cancel"
    );
}

#[tokio::test]
async fn a_cancel_the_holder_never_saw_survives_release() {
    let db = StorageBackend::test_database();
    let id = runnable(&db, ORG_A, 1).await;
    let lease = db
        .lease_session_actors("worker-a", 1, TTL, DEFAULT_ACTOR_LEASES_PER_ORG)
        .await
        .unwrap()
        .remove(0);

    // Lands after the holder's last renew.
    assert_eq!(
        db.request_session_actor_cancel(id).await.unwrap(),
        ActorCancel::Requested
    );
    assert_eq!(
        db.release_session_actor(id, "worker-a", lease.fence, 1)
            .await
            .unwrap(),
        Some(true)
    );
    let next = db
        .lease_session_actors("worker-b", 1, TTL, DEFAULT_ACTOR_LEASES_PER_ORG)
        .await
        .unwrap()
        .remove(0);
    assert!(next.cancel_requested, "the next holder hears the cancel");
    assert_eq!(
        db.release_session_actor(id, "worker-b", next.fence, 1)
            .await
            .unwrap(),
        Some(false)
    );
}

#[tokio::test]
async fn a_cancel_on_a_waiting_actor_withdraws_it() {
    let db = StorageBackend::test_database();
    let id = runnable(&db, ORG_A, 4).await;
    assert_eq!(
        db.request_session_actor_cancel(id).await.unwrap(),
        ActorCancel::Withdrawn,
        "nothing runs the turn, so the caller records the cancel"
    );
    let row = db.get_session_actor(id).await.unwrap().unwrap();
    assert!(row.runnable_since.is_none());
    assert_eq!(row.consumed_seq, 4, "the pending input counts as consumed");
    assert!(row.cancel_requested_at.is_none());
    assert!(
        db.lease_session_actors("worker-a", 1, TTL, DEFAULT_ACTOR_LEASES_PER_ORG)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn a_cancel_on_an_expired_lease_withdraws_it_from_the_stalled_holder() {
    let db = StorageBackend::test_database();
    let id = runnable(&db, ORG_A, 2).await;
    let lease = db
        .lease_session_actors("worker-a", 1, TTL, DEFAULT_ACTOR_LEASES_PER_ORG)
        .await
        .unwrap()
        .remove(0);
    expire_lease(&db, id).await;
    assert_eq!(
        db.request_session_actor_cancel(id).await.unwrap(),
        ActorCancel::Withdrawn
    );
    assert_eq!(
        db.renew_session_actor_lease(id, "worker-a", lease.fence, TTL, 2)
            .await
            .unwrap(),
        None,
        "the stalled holder lost the lease and stops without writing"
    );
}

#[tokio::test]
async fn a_fresh_turn_starts_at_the_first_input_marked_after_the_cursor() {
    let db = StorageBackend::test_database();
    let id = runnable(&db, ORG_A, 4).await;
    let row = db.get_session_actor(id).await.unwrap().unwrap();
    assert_eq!((row.consumed_seq, row.next_input_seq), (0, 4));

    // A second message queues behind the first: the start does not move.
    db.mark_session_actor_runnable(id, ORG_A, 6).await.unwrap();
    let lease = db
        .lease_session_actors("worker-a", 1, TTL, DEFAULT_ACTOR_LEASES_PER_ORG)
        .await
        .unwrap()
        .remove(0);
    assert_eq!(
        (lease.consumed_seq, lease.next_input_seq, lease.inbox_seq),
        (0, 4, 6)
    );

    // The turn consumed the first only; the second keeps the actor runnable.
    db.renew_session_actor_lease(id, "worker-a", lease.fence, TTL, 3)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        db.release_session_actor(id, "worker-a", lease.fence, 5)
            .await
            .unwrap(),
        Some(true)
    );
    let row = db.get_session_actor(id).await.unwrap().unwrap();
    assert_eq!(row.consumed_seq, 5, "the cursor only moves forward");

    // The next mark after the cursor moved names the next turn's input.
    db.mark_session_actor_runnable(id, ORG_A, 8).await.unwrap();
    let row = db.get_session_actor(id).await.unwrap().unwrap();
    assert_eq!(row.next_input_seq, 8);
}

#[tokio::test]
async fn a_resolution_mark_points_the_inbox_at_the_log_head() {
    let db = StorageBackend::test_database();
    let id = session(&db, ORG_A).await;
    event(&db, id, "turn.started").await;
    let head = event(&db, id, "tool.completed").await;
    db.mark_session_actor_resumable(id).await.unwrap();
    let row = db.get_session_actor(id).await.unwrap().unwrap();
    assert_eq!(row.inbox_seq, i64::from(head));
    assert_eq!(
        row.next_input_seq, 0,
        "a resolution continues the open turn"
    );
    assert!(row.runnable_since.is_some());
    assert_eq!(row.org_id, ORG_A);
}

#[tokio::test]
async fn attempts_count_leases_until_a_release() {
    let db = StorageBackend::test_database();
    let id = runnable(&db, ORG_A, 1).await;
    for attempt in 1..=3 {
        let lease = db
            .lease_session_actors("worker-a", 1, TTL, DEFAULT_ACTOR_LEASES_PER_ORG)
            .await
            .unwrap()
            .remove(0);
        assert_eq!(lease.attempts, attempt);
        expire_lease(&db, id).await;
    }
    let lease = db
        .lease_session_actors("worker-a", 1, TTL, DEFAULT_ACTOR_LEASES_PER_ORG)
        .await
        .unwrap()
        .remove(0);
    db.release_session_actor(id, "worker-a", lease.fence, 1)
        .await
        .unwrap();
    assert_eq!(db.get_session_actor(id).await.unwrap().unwrap().attempts, 0);
}

#[tokio::test]
async fn leased_actors_are_counted() {
    let db = StorageBackend::test_database();
    let before = db.count_leased_session_actors().await.unwrap();
    runnable(&db, ORG_B, 1).await;
    let leased = db
        .lease_session_actors("worker-a", 100, TTL, DEFAULT_ACTOR_LEASES_PER_ORG)
        .await
        .unwrap();
    assert!(db.count_leased_session_actors().await.unwrap() >= before + leased.len() as i64);
}

#[tokio::test]
async fn the_per_org_cap_limits_a_batch() {
    let db = StorageBackend::test_database();
    let mut org_a = HashSet::new();
    for _ in 0..5 {
        org_a.insert(runnable(&db, ORG_A, 1).await);
    }
    let mut org_b = HashSet::new();
    for _ in 0..2 {
        org_b.insert(runnable(&db, ORG_B, 1).await);
    }

    let leased = db
        .lease_session_actors("worker-a", 10, TTL, 3)
        .await
        .unwrap();
    let from_a = leased.iter().filter(|row| row.org_id == ORG_A).count();
    let from_b = leased.iter().filter(|row| row.org_id == ORG_B).count();
    assert_eq!((from_a, from_b), (3, 2));

    // Held leases count toward the cap on the next call.
    assert!(
        db.lease_session_actors("worker-b", 10, TTL, 3)
            .await
            .unwrap()
            .is_empty()
    );
    // Releasing one frees one slot for the org.
    let done = leased.iter().find(|row| row.org_id == ORG_A).unwrap();
    db.release_session_actor(done.session_id, "worker-a", done.fence, 1)
        .await
        .unwrap();
    let more = db
        .lease_session_actors("worker-b", 10, TTL, 3)
        .await
        .unwrap();
    assert_eq!(more.len(), 1);
    assert!(org_a.contains(&more[0].session_id));
}

#[tokio::test]
async fn the_batch_limit_takes_the_oldest_first() {
    let db = StorageBackend::test_database();
    let first = runnable(&db, ORG_A, 1).await;
    let second = runnable(&db, ORG_A, 1).await;
    let _third = runnable(&db, ORG_A, 1).await;
    let leased = db
        .lease_session_actors("worker-a", 2, TTL, DEFAULT_ACTOR_LEASES_PER_ORG)
        .await
        .unwrap();
    assert_eq!(
        leased.iter().map(|row| row.session_id).collect::<Vec<_>>(),
        vec![first, second]
    );
}

#[tokio::test]
async fn lease_skips_rows_another_transaction_holds_locked() {
    let db = StorageBackend::test_database();
    let locked = runnable(&db, ORG_A, 1).await;
    let free = runnable(&db, ORG_A, 1).await;

    let mut tx = db.database().pool.begin().await.unwrap();
    sqlx::query("SELECT 1 FROM session_actors WHERE session_id = $1 FOR UPDATE")
        .bind(locked)
        .execute(&mut *tx)
        .await
        .unwrap();

    let leased = tokio::time::timeout(
        Duration::from_secs(5),
        db.lease_session_actors("worker-a", 10, TTL, DEFAULT_ACTOR_LEASES_PER_ORG),
    )
    .await
    .expect("lease never waits on a locked row")
    .unwrap();
    assert_eq!(ids(&leased), HashSet::from([free]));
    tx.rollback().await.unwrap();
}

#[tokio::test]
async fn concurrent_lease_calls_get_disjoint_sets() {
    let db = StorageBackend::test_database();
    let mut all = HashSet::new();
    for _ in 0..20 {
        all.insert(runnable(&db, ORG_A, 1).await);
    }

    let (a, b) = tokio::join!(
        db.lease_session_actors("worker-a", 12, TTL, 100),
        db.lease_session_actors("worker-b", 12, TTL, 100),
    );
    let (a, b) = (ids(&a.unwrap()), ids(&b.unwrap()));
    assert!(a.is_disjoint(&b), "no actor is leased twice");
    // A lease scans (and locks) more rows than it takes, for the per-org cap,
    // so the call that loses the race may come back short; it is never empty
    // handed for long, since the next poll takes what is left.
    assert!(a.len() + b.len() >= 12, "the winner fills its batch");
    let rest = ids(&db
        .lease_session_actors("worker-c", 20, TTL, 100)
        .await
        .unwrap());
    assert!(rest.is_disjoint(&a) && rest.is_disjoint(&b));
    assert_eq!(
        a.len() + b.len() + rest.len(),
        20,
        "every actor is leased once"
    );
    assert!(a.union(&b).chain(rest.iter()).all(|id| all.contains(id)));
}

#[tokio::test]
async fn the_journal_tail_starts_at_the_latest_turn() {
    let db = StorageBackend::test_database();
    let id = session(&db, ORG_A).await;

    let empty = db.read_session_actor_journal(id, None, 0).await.unwrap();
    assert!(empty.events.is_empty());
    assert_eq!(empty.head_sequence, None);

    let first_input = event(&db, id, "input.message").await;
    let before_turn = db.read_session_actor_journal(id, None, 0).await.unwrap();
    assert_eq!(
        before_turn
            .events
            .iter()
            .map(|e| e.sequence)
            .collect::<Vec<_>>(),
        vec![first_input],
        "with no turn yet the whole log is the tail"
    );

    event(&db, id, "turn.started").await;
    event(&db, id, "turn.completed").await;
    event(&db, id, "input.message").await;
    let started = event(&db, id, "turn.started").await;
    let reasoned = event(&db, id, "reason.completed").await;
    let head = event(&db, id, "output.message.completed").await;

    let tail = db.read_session_actor_journal(id, None, 0).await.unwrap();
    assert_eq!(
        tail.events.iter().map(|e| e.sequence).collect::<Vec<_>>(),
        vec![started, reasoned, head]
    );
    assert!(!tail.has_more);
    assert_eq!(tail.head_sequence, Some(head));

    let page = db.read_session_actor_journal(id, None, 2).await.unwrap();
    assert_eq!(page.events.len(), 2);
    assert!(page.has_more);
    let rest = db
        .read_session_actor_journal(id, Some(page.events[1].sequence), 2)
        .await
        .unwrap();
    assert_eq!(
        rest.events.iter().map(|e| e.sequence).collect::<Vec<_>>(),
        vec![head]
    );
    assert!(!rest.has_more);
}

#[tokio::test]
async fn deleting_the_session_removes_its_actor() {
    let db = StorageBackend::test_database();
    let id = runnable(&db, ORG_A, 1).await;
    sqlx::query("DELETE FROM sessions WHERE id = $1")
        .bind(id)
        .execute(&db.database().pool)
        .await
        .unwrap();
    assert!(db.get_session_actor(id).await.unwrap().is_none());
}

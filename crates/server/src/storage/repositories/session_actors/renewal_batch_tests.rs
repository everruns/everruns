//! Batched lease renewal on a real database: each lease of a batch comes back
//! as a single renewal would report it.

use super::*;
use crate::storage::StorageBackend;
use crate::storage::test_database::test_session_row;

const TTL: Duration = Duration::from_secs(10);
const ORG: i64 = 2;

async fn runnable(db: &StorageBackend) -> SessionId {
    let id = db
        .create_session(test_session_row(ORG))
        .await
        .expect("create session")
        .id;
    db.mark_session_actor_runnable(id, ORG, 1)
        .await
        .expect("mark runnable");
    id
}

/// Lease every runnable actor as `holder`; the fence per session.
async fn lease_all(db: &StorageBackend, holder: &str) -> Vec<(SessionId, i64)> {
    db.lease_session_actors(holder, 10, TTL, DEFAULT_ACTOR_LEASES_PER_ORG)
        .await
        .expect("lease")
        .into_iter()
        .map(|row| (row.session_id, row.fence))
        .collect()
}

fn item(session_id: SessionId, fence: i64, consumed_seq: i64) -> ActorLeaseRenewalItem {
    ActorLeaseRenewalItem {
        session_id,
        fence,
        consumed_seq,
    }
}

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

#[tokio::test]
async fn each_lease_of_a_batch_comes_back_as_one_renewal_would() {
    let db = StorageBackend::test_database();
    let held = runnable(&db).await;
    let lost = runnable(&db).await;
    let cancelled = runnable(&db).await;
    let deleted = runnable(&db).await;
    let fences: std::collections::HashMap<_, _> =
        lease_all(&db, "worker-a").await.into_iter().collect();
    assert_eq!(fences.len(), 4);

    // `lost` moves to worker-b; `cancelled` gets a cancel; `deleted` goes.
    expire_lease(&db, lost).await;
    let taken = lease_all(&db, "worker-b").await;
    assert_eq!(taken.iter().map(|(id, _)| *id).collect::<Vec<_>>(), [lost]);
    db.request_session_actor_cancel(cancelled).await.unwrap();
    sqlx::query("DELETE FROM sessions WHERE id = $1")
        .bind(deleted)
        .execute(&db.database().pool)
        .await
        .unwrap();
    expire_lease(&db, held).await;

    let outcomes = db
        .renew_session_actor_leases(
            "worker-a",
            TTL,
            &[
                item(held, fences[&held], 1),
                item(lost, fences[&lost], 1),
                item(cancelled, fences[&cancelled], 0),
                item(deleted, fences[&deleted], 0),
            ],
        )
        .await
        .unwrap();
    assert_eq!(
        outcomes,
        [
            ActorLeaseRenewalOutcome::Renewed(ActorRenewalRow {
                inbox_seq: 1,
                cancel_inbox_seq: None,
            }),
            ActorLeaseRenewalOutcome::Lost,
            ActorLeaseRenewalOutcome::Renewed(ActorRenewalRow {
                inbox_seq: 1,
                cancel_inbox_seq: Some(1),
            }),
            ActorLeaseRenewalOutcome::SessionGone,
        ]
    );

    let row = db.get_session_actor(held).await.unwrap().unwrap();
    assert_eq!(row.consumed_seq, 1, "the cursor commits");
    assert!(
        row.lease_expires_at.unwrap() > chrono::Utc::now(),
        "the expired lease is extended"
    );
    let row = db.get_session_actor(lost).await.unwrap().unwrap();
    assert_eq!(row.lease_holder.as_deref(), Some("worker-b"));
    assert_eq!(row.consumed_seq, 0, "a lost lease writes nothing");
    assert!(
        db.get_session_actor(cancelled)
            .await
            .unwrap()
            .unwrap()
            .cancel_reported
    );
}

#[tokio::test]
async fn a_lease_sent_twice_keeps_its_furthest_cursor() {
    let db = StorageBackend::test_database();
    let id = runnable(&db).await;
    let (_, fence) = lease_all(&db, "worker-a").await.remove(0);
    let outcomes = db
        .renew_session_actor_leases("worker-a", TTL, &[item(id, fence, 5), item(id, fence, 2)])
        .await
        .unwrap();
    assert!(
        outcomes
            .iter()
            .all(|outcome| matches!(outcome, ActorLeaseRenewalOutcome::Renewed(_)))
    );
    let row = db.get_session_actor(id).await.unwrap().unwrap();
    assert_eq!(row.consumed_seq, 5);
}

#[tokio::test]
async fn a_stale_fence_in_a_batch_is_lost() {
    let db = StorageBackend::test_database();
    let id = runnable(&db).await;
    let (_, fence) = lease_all(&db, "worker-a").await.remove(0);
    let outcomes = db
        .renew_session_actor_leases("worker-a", TTL, &[item(id, fence - 1, 3)])
        .await
        .unwrap();
    assert_eq!(outcomes, [ActorLeaseRenewalOutcome::Lost]);
    assert!(
        db.renew_session_actor_leases("worker-a", TTL, &[])
            .await
            .unwrap()
            .is_empty()
    );
}

/// An event append's fence check share-locks lease rows in session_id order
/// and holds them until its transaction commits (`events::group_commit`). A
/// renewal batch over the same rows has to lock in that order too: one that
/// took some rows first and then waited for a row the append holds, while the
/// append waited for one of those, would deadlock, and PostgreSQL would fail
/// one side (the stress bench lost hundreds of leases this way).
#[tokio::test]
async fn a_renewal_batch_and_an_append_fence_check_do_not_deadlock() {
    let db = StorageBackend::test_database();
    for _ in 0..20 {
        runnable(&db).await;
    }
    let mut leases = db
        .lease_session_actors("worker-a", 20, TTL, DEFAULT_ACTOR_LEASES_PER_ORG)
        .await
        .expect("lease")
        .into_iter()
        .map(|row| (row.session_id, row.fence))
        .collect::<Vec<_>>();
    assert_eq!(leases.len(), 20);
    leases.sort_by_key(|(id, _)| id.uuid());
    let (first, _) = leases[0];
    let rest: Vec<uuid::Uuid> = leases[1..].iter().map(|(id, _)| id.uuid()).collect();

    // The append's transaction holds the lowest row...
    let pool = db.database().pool.clone();
    let mut append = pool.begin().await.unwrap();
    sqlx::query("SELECT 1 FROM session_actors WHERE session_id = $1 FOR SHARE")
        .bind(first)
        .execute(&mut *append)
        .await
        .unwrap();

    // ...while a batch renews every lease, highest first.
    let items: Vec<_> = leases
        .iter()
        .rev()
        .map(|(id, fence)| item(*id, *fence, 1))
        .collect();
    let renewer = db.clone();
    let renewal = tokio::spawn(async move {
        renewer
            .renew_session_actor_leases("worker-a", TTL, &items)
            .await
    });
    tokio::time::sleep(Duration::from_millis(300)).await;

    // The append then locks the rest, in order, and commits.
    sqlx::query(
        "SELECT 1 FROM session_actors WHERE session_id = ANY($1) \
         ORDER BY session_id FOR SHARE",
    )
    .bind(&rest)
    .execute(&mut *append)
    .await
    .expect("the append's fence check must not deadlock");
    append.commit().await.unwrap();

    let outcomes = tokio::time::timeout(Duration::from_secs(10), renewal)
        .await
        .expect("the batch finishes once the append commits")
        .unwrap()
        .expect("the batch must not deadlock");
    assert!(
        outcomes
            .iter()
            .all(|outcome| matches!(outcome, ActorLeaseRenewalOutcome::Renewed(_)))
    );
}

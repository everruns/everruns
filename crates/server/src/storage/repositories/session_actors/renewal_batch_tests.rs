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

//! Test-only grant seeding; these identifiers are not credentials.
use super::super::assignment_scope::ActorJournalScope;
use super::*;
use everruns_contracts::typed_id::SessionId;

pub(super) async fn seed_scope(
    db: &StorageBackend,
    org_id: i64,
    session_id: SessionId,
    holder: &str,
    fence: i64,
) -> ActorJournalScope {
    seed_scope_for_executor(db, uuid::Uuid::new_v4(), org_id, session_id, holder, fence).await
}

pub(super) async fn seed_scope_for_executor(
    db: &StorageBackend,
    executor_id: uuid::Uuid,
    org_id: i64,
    session_id: SessionId,
    holder: &str,
    fence: i64,
) -> ActorJournalScope {
    let grant_id = uuid::Uuid::new_v4();
    sqlx::query(
        "INSERT INTO worker_journal_read_grants \
         (id, executor_id, org_id, session_id, lease_holder, lease_fence, expires_at) \
         SELECT $1, $2, $3, session_id, $5, $6, lease_expires_at \
         FROM session_actors WHERE session_id = $4",
    )
    .bind(grant_id)
    .bind(executor_id)
    .bind(org_id)
    .bind(session_id)
    .bind(holder)
    .bind(fence)
    .execute(db.database().pool())
    .await
    .unwrap();
    ActorJournalScope::new(
        grant_id,
        executor_id,
        org_id,
        session_id,
        holder.into(),
        fence,
    )
    .unwrap()
}

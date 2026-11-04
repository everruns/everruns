//! Admission requires a live grant independently of a live actor lease.
//! The test adapter asserts synthetic executor IDs; this is not identity proof.
use super::super::journal_grants::{seed_scope, seed_scope_for_executor};
use super::*;

async fn refused(server: &mut ScopedServer, session: SessionId) {
    assert_eq!(
        server
            .client
            .read_actor_journal(read(session))
            .await
            .unwrap_err()
            .code(),
        tonic::Code::PermissionDenied,
    );
}

#[tokio::test]
async fn revoked_grant_stays_refused_after_reconnect_and_regrant() {
    let service = Arc::new(test_worker_service().await);
    let own = journal(&service, 2, "assigned journal").await;
    let scope = leased_scope(&service, own, 2).await;
    let db = service.db.clone();
    let mut server = ScopedServer::start_shared(service.clone(), scope.clone()).await;
    assert_eq!(
        server
            .client
            .read_actor_journal(read(own))
            .await
            .unwrap()
            .into_inner()
            .events
            .len(),
        1
    );
    assert!(
        !db.revoke_worker_journal_read_grant(3, scope.grant_id)
            .await
            .unwrap()
    );
    assert!(
        !db.revoke_worker_journal_read_grant(2, uuid::Uuid::new_v4())
            .await
            .unwrap()
    );
    assert!(
        db.revoke_worker_journal_read_grant(2, scope.grant_id)
            .await
            .unwrap()
    );
    assert!(
        !db.revoke_worker_journal_read_grant(2, scope.grant_id)
            .await
            .unwrap()
    );
    refused(&mut server, own).await;
    server.client = WorkerServiceClient::connect(server.address.clone())
        .await
        .unwrap();
    refused(&mut server, own).await;
    // The lease and executor remain unchanged; replacing a revoked grant does
    // not resurrect a retained old reference.
    let fresh =
        seed_scope_for_executor(&db, scope.executor_id, 2, own, &scope.holder, scope.fence).await;
    let fresh_id = fresh.grant_id;
    let mut replacement = ScopedServer::start_shared(service, fresh).await;
    assert_eq!(
        replacement
            .client
            .read_actor_journal(read(own))
            .await
            .unwrap()
            .into_inner()
            .events
            .len(),
        1
    );
    refused(&mut server, own).await;
    let mut forged = read(own);
    forged
        .metadata_mut()
        .insert("x-everruns-grant-id", fresh_id.to_string().parse().unwrap());
    assert_eq!(
        server
            .client
            .read_actor_journal(forged)
            .await
            .unwrap_err()
            .code(),
        tonic::Code::PermissionDenied
    );
}

#[tokio::test]
async fn a_foreign_executor_cannot_redeem_a_tenant_grant_with_equal_holder_and_fence() {
    let service = Arc::new(test_worker_service().await);
    let own = journal(&service, 2, "tenant A").await;
    let foreign = journal(&service, 3, "tenant B secret").await;
    let a = leased_scope(&service, own, 2).await;
    let b = seed_scope(&service.db, 3, foreign, &a.holder, a.fence).await;
    let actor = service
        .db
        .get_session_actor(foreign)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(actor.lease_fence, a.fence);
    assert_ne!(a.executor_id, b.executor_id);
    let wrong_executor = ActorJournalScope::new(
        b.grant_id,
        a.executor_id,
        3,
        foreign,
        b.holder.clone(),
        b.fence,
    )
    .unwrap();
    let mut server = ScopedServer::start_shared(service, wrong_executor).await;
    // The session selector agrees with the asserted assignment; the grant's
    // executor binding must still deny A reading B's otherwise-live lease.
    refused(&mut server, foreign).await;
}

#[tokio::test]
async fn journal_read_requires_a_present_grant_record() {
    let service = Arc::new(test_worker_service().await);
    let own = journal(&service, 2, "assigned journal").await;
    let scope = leased_scope(&service, own, 2).await;
    let missing = ActorJournalScope::new(
        uuid::Uuid::new_v4(),
        scope.executor_id,
        2,
        own,
        scope.holder.clone(),
        scope.fence,
    )
    .unwrap();
    let db = service.db.clone();
    let mut absent = ScopedServer::start_shared(service.clone(), missing).await;
    refused(&mut absent, own).await;
    assert!(db.delete_session(2, own).await.unwrap());
    let mut deleted = ScopedServer::start_shared(service, scope).await;
    refused(&mut deleted, own).await;
}

async fn timed_scope(
    service: &WorkerServiceImpl,
    scope: &ActorJournalScope,
    created: i32,
    expires: i32,
) -> ActorJournalScope {
    let id = uuid::Uuid::new_v4();
    sqlx::query(
        "INSERT INTO worker_journal_read_grants \
         (id, executor_id, org_id, session_id, lease_holder, lease_fence, created_at, expires_at) \
         VALUES ($1, $2, $3, $4, $5, $6, clock_timestamp() + $7::integer * INTERVAL '1 second', \
                 clock_timestamp() + $8::integer * INTERVAL '1 second')",
    )
    .bind(id)
    .bind(scope.executor_id)
    .bind(scope.org_id)
    .bind(scope.session_id)
    .bind(&scope.holder)
    .bind(scope.fence)
    .bind(created)
    .bind(expires)
    .execute(service.db.database().pool())
    .await
    .unwrap();
    ActorJournalScope::new(
        id,
        scope.executor_id,
        scope.org_id,
        scope.session_id,
        scope.holder.clone(),
        scope.fence,
    )
    .unwrap()
}

#[tokio::test]
async fn expired_grant_does_not_inherit_a_live_lease() {
    let service = Arc::new(test_worker_service().await);
    let own = journal(&service, 2, "assigned journal").await;
    let live = leased_scope(&service, own, 2).await;
    let expired = timed_scope(&service, &live, -120, -60).await;
    let mut server = ScopedServer::start_shared(service, expired).await;
    refused(&mut server, own).await;
}

#[tokio::test]
async fn not_yet_valid_grant_does_not_inherit_a_live_lease() {
    let service = Arc::new(test_worker_service().await);
    let own = journal(&service, 2, "assigned journal").await;
    let live = leased_scope(&service, own, 2).await;
    let future = timed_scope(&service, &live, 60, 120).await;
    let mut server = ScopedServer::start_shared(service, future).await;
    refused(&mut server, own).await;
}

#[tokio::test]
async fn a_grant_cannot_be_retargeted_to_another_tenant_with_the_same_executor_and_fence() {
    let service = Arc::new(test_worker_service().await);
    let own = journal(&service, 2, "tenant A").await;
    let foreign = journal(&service, 3, "tenant B secret").await;
    let a = leased_scope(&service, own, 2).await;
    let actor = service
        .db
        .get_session_actor(foreign)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(actor.lease_fence, a.fence);
    let wrong_session = ActorJournalScope::new(
        a.grant_id,
        a.executor_id,
        3,
        foreign,
        a.holder.clone(),
        a.fence,
    )
    .unwrap();
    let mut server = ScopedServer::start_shared(service, wrong_session).await;
    refused(&mut server, foreign).await;
}

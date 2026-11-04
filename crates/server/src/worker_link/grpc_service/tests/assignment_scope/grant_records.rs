//! Stored grant invariants; these are authorization records, not credentials.
use super::super::journal_grants::seed_scope;
use super::*;

fn check_violation(error: sqlx::Error) {
    assert_eq!(
        error
            .as_database_error()
            .and_then(|error| error.code())
            .as_deref(),
        Some("23514")
    );
}

#[tokio::test]
async fn a_retained_grant_cannot_be_retargeted_extended_unrevoked_or_recreated() {
    let service = test_worker_service().await;
    let own = journal(&service, 2, "assigned journal").await;
    let scope = leased_scope(&service, own, 2).await;
    let db = &service.db;
    for change in [
        "id = uuidv7()",
        "executor_id = uuidv7()",
        "org_id = org_id + 1",
        "session_id = uuidv7()",
        "lease_holder = lease_holder || '-other'",
        "lease_fence = lease_fence + 1",
        "created_at = created_at - INTERVAL '1 second'",
        "expires_at = expires_at + INTERVAL '1 second'",
    ] {
        // The field expressions are a fixed test allowlist, never caller input.
        let mut query =
            sqlx::QueryBuilder::<sqlx::Postgres>::new("UPDATE worker_journal_read_grants SET ");
        query
            .push(change)
            .push(" WHERE id = ")
            .push_bind(scope.grant_id);
        check_violation(
            query
                .build()
                .execute(db.database().pool())
                .await
                .unwrap_err(),
        );
    }
    scope.read_journal(db, own, None, 1).await.unwrap();
    assert!(
        db.revoke_worker_journal_read_grant(2, scope.grant_id)
            .await
            .unwrap()
    );
    for change in [
        "revoked_at = NULL",
        "revoked_at = revoked_at + INTERVAL '1 second'",
    ] {
        let mut query =
            sqlx::QueryBuilder::<sqlx::Postgres>::new("UPDATE worker_journal_read_grants SET ");
        query
            .push(change)
            .push(" WHERE id = ")
            .push_bind(scope.grant_id);
        check_violation(
            query
                .build()
                .execute(db.database().pool())
                .await
                .unwrap_err(),
        );
    }
    check_violation(
        sqlx::query("DELETE FROM worker_journal_read_grants WHERE id = $1")
            .bind(scope.grant_id)
            .execute(db.database().pool())
            .await
            .unwrap_err(),
    );
    let duplicate = sqlx::query("INSERT INTO worker_journal_read_grants SELECT * FROM worker_journal_read_grants WHERE id = $1")
        .bind(scope.grant_id).execute(db.database().pool()).await.unwrap_err();
    assert_eq!(
        duplicate
            .as_database_error()
            .and_then(|error| error.code())
            .as_deref(),
        Some("23505")
    );
    assert_eq!(
        scope
            .read_journal(db, own, None, 1)
            .await
            .unwrap_err()
            .code(),
        tonic::Code::PermissionDenied
    );
}

#[tokio::test]
async fn journal_grants_have_finite_bounded_deadlines() {
    let service = test_worker_service().await;
    let own = journal(&service, 2, "assigned journal").await;
    let scope = leased_scope(&service, own, 2).await;
    for deadline in [
        "'infinity'::timestamptz",
        "clock_timestamp() + INTERVAL '6 minutes'",
        "clock_timestamp() - INTERVAL '1 second'",
    ] {
        let mut query = sqlx::QueryBuilder::<sqlx::Postgres>::new(
            "INSERT INTO worker_journal_read_grants \
             (id, executor_id, org_id, session_id, lease_holder, lease_fence, created_at, expires_at) SELECT ",
        );
        query
            .push_bind(uuid::Uuid::new_v4())
            .push(
                ", executor_id, org_id, session_id, lease_holder, lease_fence, clock_timestamp(), ",
            )
            .push(deadline)
            .push(" FROM worker_journal_read_grants WHERE id = ")
            .push_bind(scope.grant_id);
        check_violation(
            query
                .build()
                .execute(service.db.database().pool())
                .await
                .unwrap_err(),
        );
    }
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM worker_journal_read_grants WHERE session_id = $1")
            .bind(own)
            .fetch_one(service.db.database().pool())
            .await
            .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn revoking_one_org_cannot_revoke_another_orgs_grant() {
    let service = test_worker_service().await;
    let own = journal(&service, 2, "tenant A").await;
    let foreign = journal(&service, 3, "tenant B").await;
    let a = leased_scope(&service, own, 2).await;
    let b = seed_scope(&service.db, 3, foreign, &a.holder, a.fence).await;
    assert!(
        !service
            .db
            .revoke_worker_journal_read_grant(2, b.grant_id)
            .await
            .unwrap()
    );
    b.read_journal(&service.db, foreign, None, 1).await.unwrap();
    assert!(
        service
            .db
            .revoke_worker_journal_read_grant(3, b.grant_id)
            .await
            .unwrap()
    );
    assert_eq!(
        b.read_journal(&service.db, foreign, None, 1)
            .await
            .unwrap_err()
            .code(),
        tonic::Code::PermissionDenied
    );
    a.read_journal(&service.db, own, None, 1).await.unwrap();
}

#[tokio::test]
async fn retained_grant_tombstones_allow_parent_session_deletion() {
    let service = test_worker_service().await;
    let own = journal(&service, 2, "assigned journal").await;
    let scope = leased_scope(&service, own, 2).await;
    service
        .db
        .revoke_worker_journal_read_grant(2, scope.grant_id)
        .await
        .unwrap();
    assert!(service.db.delete_session(2, own).await.unwrap());
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM worker_journal_read_grants WHERE id = $1")
            .bind(scope.grant_id)
            .fetch_one(service.db.database().pool())
            .await
            .unwrap();
    assert_eq!(count, 0);
    assert_eq!(
        scope
            .read_journal(&service.db, own, None, 1)
            .await
            .unwrap_err()
            .code(),
        tonic::Code::PermissionDenied
    );
}

//! Deterministic lock ordering: no timing-only guesses about concurrent revokes.
use super::*;

async fn blocked(pool: &sqlx::PgPool, pid: i32) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let waiting: bool = sqlx::query_scalar("SELECT cardinality(pg_blocking_pids($1)) > 0")
                .bind(pid)
                .fetch_one(pool)
                .await
                .unwrap();
            if waiting {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("operation must wait on the source lock");
}

const REGISTER: &str = "INSERT INTO worker_journal_grant_dependencies \
    (grant_id,source_grant_id,source_resource_id,grant_revision,resource_revision,grant_epoch,resource_epoch) \
    VALUES ($1,$2,$3,1,1,1,1)";

#[tokio::test]
async fn revocation_racing_dependency_registration_either_aborts_registration_or_rolls_back_atomically()
 {
    for commit_revoke in [true, false] {
        let service = Arc::new(test_worker_service().await);
        let a = assignment(&service, 2, CLIENT_A, 0xa1).await;
        let refs = source_refs(&service.db, &a).await;
        let fresh = fresh_assignment(&service.db, &a, 0xc3, &[]).await;
        let pool = service.db.database().pool();
        let mut revoker = everruns_durable::begin_transaction(pool).await.unwrap();
        sqlx::query("UPDATE resource_grants SET revoked_at=clock_timestamp(),revision=revision+1 WHERE id=$1")
            .bind(refs.0).execute(&mut *revoker).await.unwrap();
        let db = service.db.clone();
        let id = fresh.grant;
        let (send, receive) = tokio::sync::oneshot::channel();
        let registration = tokio::spawn(async move {
            let mut tx = everruns_durable::begin_transaction(db.database().pool())
                .await
                .unwrap();
            let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
                .fetch_one(&mut *tx)
                .await
                .unwrap();
            send.send(pid).unwrap();
            let result = sqlx::query(REGISTER)
                .bind(id)
                .bind(refs.0)
                .bind(refs.1)
                .execute(&mut *tx)
                .await;
            if result.is_ok() {
                everruns_durable::commit_transaction(tx).await.unwrap();
            } else {
                tx.rollback().await.unwrap();
            }
            result.is_ok()
        });
        blocked(pool, receive.await.unwrap()).await;
        if commit_revoke {
            everruns_durable::commit_transaction(revoker).await.unwrap();
        } else {
            revoker.rollback().await.unwrap();
        }
        assert_eq!(registration.await.unwrap(), !commit_revoke);
        let server = Server::start(service.clone(), true, Some((CLIENT_A, KEY_A))).await;
        if commit_revoke {
            assert_eq!(
                server
                    .read(fresh.session, fresh.byte)
                    .await
                    .unwrap_err()
                    .code(),
                tonic::Code::PermissionDenied
            );
            assert_eq!(
                server.read(a.session, a.byte).await.unwrap_err().code(),
                tonic::Code::PermissionDenied
            );
        } else {
            server.read(fresh.session, fresh.byte).await.unwrap();
            server.read(a.session, a.byte).await.unwrap();
            assert!(!is_revoked(&service.db, a.grant).await);
        }
    }
}

#[tokio::test]
async fn registration_that_wins_the_source_lock_is_seen_by_the_waiting_revoker() {
    let service = Arc::new(test_worker_service().await);
    let a = assignment(&service, 2, CLIENT_A, 0xa1).await;
    let refs = source_refs(&service.db, &a).await;
    let fresh = fresh_assignment(&service.db, &a, 0xc3, &[]).await;
    let pool = service.db.database().pool();
    let mut registration = everruns_durable::begin_transaction(pool).await.unwrap();
    sqlx::query(REGISTER)
        .bind(fresh.grant)
        .bind(refs.0)
        .bind(refs.1)
        .execute(&mut *registration)
        .await
        .unwrap();
    let db = service.db.clone();
    let (send, receive) = tokio::sync::oneshot::channel();
    let revoker = tokio::spawn(async move {
        let mut tx = everruns_durable::begin_transaction(db.database().pool())
            .await
            .unwrap();
        let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *tx)
            .await
            .unwrap();
        send.send(pid).unwrap();
        sqlx::query("UPDATE resource_grants SET revoked_at=clock_timestamp(),revision=revision+1 WHERE id=$1")
            .bind(refs.0).execute(&mut *tx).await.unwrap();
        everruns_durable::commit_transaction(tx).await.unwrap();
    });
    blocked(pool, receive.await.unwrap()).await;
    everruns_durable::commit_transaction(registration)
        .await
        .unwrap();
    revoker.await.unwrap();
    assert!(is_revoked(&service.db, fresh.grant).await);
    let server = Server::start(service.clone(), true, Some((CLIENT_A, KEY_A))).await;
    assert_eq!(
        server
            .read(fresh.session, fresh.byte)
            .await
            .unwrap_err()
            .code(),
        tonic::Code::PermissionDenied
    );
}

#[tokio::test]
async fn assignment_creation_serializes_with_cancellation_and_is_revoked_at_commit() {
    let service = Arc::new(test_worker_service().await);
    let a = assignment(&service, 2, CLIENT_A, 0xa1).await;
    let pool = service.db.database().pool();
    let fresh = uuid::Uuid::new_v4();
    let mut creator = everruns_durable::begin_transaction(pool).await.unwrap();
    sqlx::query("INSERT INTO worker_journal_read_grants (id,executor_id,org_id,session_id,lease_holder,lease_fence,expires_at) VALUES ($1,$2,2,$3,'equal-holder',1,clock_timestamp()+INTERVAL '120 seconds')")
        .bind(fresh).bind(a.executor).bind(a.session).execute(&mut *creator).await.unwrap();
    let (send, receive) = tokio::sync::oneshot::channel();
    let db = service.db.clone();
    let session = a.session;
    let cancel = tokio::spawn(async move {
        let mut tx = everruns_durable::begin_transaction(db.database().pool())
            .await
            .unwrap();
        let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *tx)
            .await
            .unwrap();
        send.send(pid).unwrap();
        sqlx::query(
            "UPDATE session_actors SET cancel_requested_at=clock_timestamp() WHERE session_id=$1",
        )
        .bind(session)
        .execute(&mut *tx)
        .await
        .unwrap();
        everruns_durable::commit_transaction(tx).await.unwrap();
    });
    blocked(pool, receive.await.unwrap()).await;
    everruns_durable::commit_transaction(creator).await.unwrap();
    cancel.await.unwrap();
    assert!(is_revoked(&service.db, fresh).await);
    assert!(is_revoked(&service.db, a.grant).await);
    let server = Server::start(service.clone(), true, Some((CLIENT_A, KEY_A))).await;
    assert_eq!(
        server.read(a.session, a.byte).await.unwrap_err().code(),
        tonic::Code::PermissionDenied
    );
}

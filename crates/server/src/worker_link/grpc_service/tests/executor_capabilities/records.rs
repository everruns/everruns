use super::*;

#[tokio::test]
async fn identity_and_capability_bindings_cannot_be_retargeted_resurrected_or_recycled() {
    let service = test_worker_service().await;
    let a = assignment(&service, 2, CLIENT_A, 0xa1).await;
    let b = assignment(&service, 3, CLIENT_B, 0xb2).await;
    let pool = service.db.database().pool();
    // Fixed SQL fragments only. IDs/digests remain bound, never interpolated.
    for assignment in [
        "executor_id = $2",
        "expires_at = expires_at + INTERVAL '1 second'",
        "certificate_sha256 = $3",
    ] {
        let mut query = sqlx::QueryBuilder::new("UPDATE worker_executor_identities SET ");
        // Parameter positions are supplied via a fixed expression and binds.
        match assignment {
            "executor_id = $2" => {
                query.push("executor_id = ").push_bind(b.executor);
            }
            "certificate_sha256 = $3" => {
                query
                    .push("certificate_sha256 = ")
                    .push_bind(b.certificate.as_slice());
            }
            _ => {
                query.push("expires_at = expires_at + INTERVAL '1 second'");
            }
        }
        query
            .push(" WHERE certificate_sha256 = ")
            .push_bind(a.certificate.as_slice());
        assert!(query.build().execute(pool).await.is_err());
    }
    assert!(
        service
            .db
            .revoke_worker_executor_identity(a.executor, &a.certificate)
            .await
            .unwrap()
    );
    assert!(
        sqlx::query(
            "UPDATE worker_executor_identities SET revoked_at=NULL WHERE certificate_sha256=$1"
        )
        .bind(a.certificate.as_slice())
        .execute(pool)
        .await
        .is_err()
    );
    assert!(
        sqlx::query("DELETE FROM worker_executor_identities WHERE certificate_sha256=$1")
            .bind(a.certificate.as_slice())
            .execute(pool)
            .await
            .is_err()
    );

    assert!(
        sqlx::query(
            "UPDATE worker_journal_capabilities SET grant_id=$2 WHERE capability_sha256=$1"
        )
        .bind(a.digest.as_slice())
        .bind(b.grant)
        .execute(pool)
        .await
        .is_err()
    );
    assert!(sqlx::query("UPDATE worker_journal_capabilities SET expires_at=expires_at+INTERVAL '1 second' WHERE capability_sha256=$1").bind(a.digest.as_slice()).execute(pool).await.is_err());
    assert!(
        sqlx::query("DELETE FROM worker_journal_capabilities WHERE capability_sha256=$1")
            .bind(a.digest.as_slice())
            .execute(pool)
            .await
            .is_err()
    );
    assert!(
        service
            .db
            .revoke_worker_journal_capability(2, &a.digest)
            .await
            .unwrap()
    );
    assert!(
        sqlx::query(
            "UPDATE worker_journal_capabilities SET revoked_at=NULL WHERE capability_sha256=$1"
        )
        .bind(a.digest.as_slice())
        .execute(pool)
        .await
        .is_err()
    );
    assert!(
        sqlx::query(
            "INSERT INTO worker_journal_capabilities (capability_sha256,grant_id,expires_at) \
        SELECT $1,id,expires_at FROM worker_journal_read_grants WHERE id=$2"
        )
        .bind(a.digest.as_slice())
        .bind(b.grant)
        .execute(pool)
        .await
        .is_err()
    );

    assert!(
        sqlx::query(
            "UPDATE worker_journal_capabilities SET capability_sha256=$2 WHERE capability_sha256=$1"
        )
        .bind(a.digest.as_slice())
        .bind(journal_capability_digest(&[0xc3; 32]).as_slice())
        .execute(pool)
        .await
        .is_err()
    );

    // Parent purge is supported; neither identity nor proof can be recycled.
    service.db.delete_session(2, a.session).await.unwrap();
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM worker_journal_capabilities WHERE capability_sha256=$1",
    )
    .bind(a.digest.as_slice())
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(count, 1);
    let parent: Option<uuid::Uuid> = sqlx::query_scalar(
        "SELECT grant_id FROM worker_journal_capabilities WHERE capability_sha256=$1",
    )
    .bind(a.digest.as_slice())
    .fetch_one(pool)
    .await
    .unwrap();
    assert!(parent.is_none());
    assert!(
        sqlx::query(
            "UPDATE worker_journal_capabilities SET grant_id=$2 WHERE capability_sha256=$1"
        )
        .bind(a.digest.as_slice())
        .bind(b.grant)
        .execute(pool)
        .await
        .is_err()
    );
    assert!(
        sqlx::query("DELETE FROM worker_journal_capabilities WHERE capability_sha256=$1")
            .bind(a.digest.as_slice())
            .execute(pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("DELETE FROM worker_executor_identities WHERE certificate_sha256=$1")
            .bind(a.certificate.as_slice())
            .execute(pool)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn capability_validity_cannot_exceed_its_grant_and_digests_have_fixed_size() {
    let service = test_worker_service().await;
    let a = assignment(&service, 2, CLIENT_A, 0xa1).await;
    let pool = service.db.database().pool();
    assert!(
        sqlx::query(
            "INSERT INTO worker_journal_capabilities (capability_sha256,grant_id,expires_at) \
        SELECT $1,id,expires_at+INTERVAL '1 second' FROM worker_journal_read_grants WHERE id=$2"
        )
        .bind(journal_capability_digest(&[0xc3; 32]).as_slice())
        .bind(a.grant)
        .execute(pool)
        .await
        .is_err()
    );
    assert!(sqlx::query("INSERT INTO worker_journal_capabilities (capability_sha256,grant_id,created_at,expires_at) \
        SELECT $1,id,created_at-INTERVAL '1 second',expires_at FROM worker_journal_read_grants WHERE id=$2")
        .bind(journal_capability_digest(&[0xc3;32]).as_slice()).bind(a.grant).execute(pool).await.is_err());
    assert!(
        sqlx::query(
            "INSERT INTO worker_journal_capabilities (capability_sha256,grant_id,expires_at) \
        SELECT $1,id,expires_at FROM worker_journal_read_grants WHERE id=$2"
        )
        .bind([0_u8; 31].as_slice())
        .bind(a.grant)
        .execute(pool)
        .await
        .is_err()
    );
    assert!(
        sqlx::query(
            "INSERT INTO worker_executor_identities (certificate_sha256,executor_id,expires_at) \
        VALUES ($1,$2,clock_timestamp()+INTERVAL '1 minute')"
        )
        .bind([0_u8; 31].as_slice())
        .bind(uuid::Uuid::new_v4())
        .execute(pool)
        .await
        .is_err()
    );
}

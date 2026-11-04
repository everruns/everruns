//! Automatic lifecycle revocation uses real mTLS and actual source rows.
use super::*;

mod races;

async fn source_refs(db: &StorageBackend, a: &Assignment) -> (uuid::Uuid, uuid::Uuid) {
    sqlx::query_as("SELECT source_grant_id,source_resource_id FROM worker_journal_grant_dependencies WHERE grant_id=$1")
        .bind(a.grant).fetch_one(db.database().pool()).await.unwrap()
}

async fn assert_revoked(
    service: &WorkerServiceImpl,
    a: &Assignment,
    server: &mut Server,
    context: WorkerAuthority,
) {
    assert_eq!(
        retained_read(service, context, a.session)
            .await
            .unwrap_err()
            .code(),
        tonic::Code::PermissionDenied
    );
    assert_eq!(
        server.read(a.session, a.byte).await.unwrap_err().code(),
        tonic::Code::PermissionDenied
    );
    server.channel = Server::connect(&server.address, Some((CLIENT_A, KEY_A))).await;
    assert_eq!(
        server.read(a.session, a.byte).await.unwrap_err().code(),
        tonic::Code::PermissionDenied
    );
}

async fn is_revoked(db: &StorageBackend, id: uuid::Uuid) -> bool {
    sqlx::query_scalar("SELECT revoked_at IS NOT NULL FROM worker_journal_read_grants WHERE id=$1")
        .bind(id)
        .fetch_one(db.database().pool())
        .await
        .unwrap()
}

async fn fresh_assignment(
    db: &StorageBackend,
    old: &Assignment,
    byte: u8,
    deps: &[(uuid::Uuid, uuid::Uuid)],
) -> Assignment {
    let grant = uuid::Uuid::new_v4();
    sqlx::query("INSERT INTO worker_journal_read_grants \
        (id,executor_id,org_id,session_id,lease_holder,lease_fence,expires_at) \
        SELECT $1,g.executor_id,g.org_id,g.session_id,a.lease_holder,a.lease_fence, \
          clock_timestamp()+INTERVAL '120 seconds' \
        FROM worker_journal_read_grants g JOIN session_actors a ON a.session_id=g.session_id WHERE g.id=$2")
        .bind(grant).bind(old.grant).execute(db.database().pool()).await.unwrap();
    for (source, resource) in deps {
        register_dependency(db, grant, *source, *resource)
            .await
            .unwrap();
    }
    let digest = journal_capability_digest(&[byte; 32]);
    sqlx::query("INSERT INTO worker_journal_capabilities (capability_sha256,grant_id,expires_at) SELECT $1,id,expires_at FROM worker_journal_read_grants WHERE id=$2")
        .bind(digest.as_slice()).bind(grant).execute(db.database().pool()).await.unwrap();
    Assignment {
        session: old.session,
        grant,
        executor: old.executor,
        certificate: old.certificate,
        digest,
        byte,
    }
}

fn control(db: Arc<StorageBackend>, org: i64) -> crate::domains::common::Ctx {
    crate::domains::common::Ctx::minimal(
        everruns_core::Caller::internal(org),
        db,
        None,
        Arc::new(everruns_core::DefaultPermissionResolver),
    )
}

#[tokio::test]
async fn domain_grant_revocation_is_atomic_and_tenant_scoped_without_manual_worker_revocation() {
    let service = Arc::new(test_worker_service().await);
    let a = assignment(&service, 2, CLIENT_A, 0xa1).await;
    let b = assignment(&service, 3, CLIENT_B, 0xb2).await;
    let (source, resource) = source_refs(&service.db, &a).await;
    let mut server = Server::start(service.clone(), true, Some((CLIENT_A, KEY_A))).await;
    server.read(a.session, a.byte).await.unwrap();
    let context = server.context();
    assert!(
        crate::domains::credentials::management::revoke_grant(
            &control(service.db.clone(), 3),
            resource,
            source
        )
        .await
        .is_err()
    );
    server.read(a.session, a.byte).await.unwrap();
    crate::domains::credentials::management::revoke_grant(
        &control(service.db.clone(), 2),
        resource,
        source,
    )
    .await
    .unwrap();
    assert!(is_revoked(&service.db, a.grant).await);
    assert_revoked(&service, &a, &mut server, context).await;
    server.channel = Server::connect(&server.address, Some((CLIENT_B, KEY_B))).await;
    assert_eq!(
        server.read(b.session, b.byte).await.unwrap().events.len(),
        1
    );
    assert!(!is_revoked(&service.db, b.grant).await);
}

#[tokio::test]
async fn changing_and_restoring_resource_or_grant_state_never_revives_old_proofs() {
    for source_kind in ["grant", "resource"] {
        let service = Arc::new(test_worker_service().await);
        let a = assignment(&service, 2, CLIENT_A, 0xa1).await;
        let (source, resource) = source_refs(&service.db, &a).await;
        let mut server = Server::start(service.clone(), true, Some((CLIENT_A, KEY_A))).await;
        server.read(a.session, a.byte).await.unwrap();
        let context = server.context();
        if source_kind == "grant" {
            // No canonical revision bump: the server-owned epoch still changes.
            sqlx::query("UPDATE resource_grants SET constraints='{\"narrowed\":true}' WHERE id=$1")
                .bind(source)
                .execute(service.db.database().pool())
                .await
                .unwrap();
            sqlx::query("UPDATE resource_grants SET constraints=NULL,revision=1,revoked_at=NULL WHERE id=$1")
                .bind(source).execute(service.db.database().pool()).await.unwrap();
        } else {
            sqlx::query("UPDATE resource_access SET revoked_at=clock_timestamp(),revision=revision+1 WHERE id=$1")
                .bind(resource).execute(service.db.database().pool()).await.unwrap();
            sqlx::query("UPDATE resource_access SET revoked_at=NULL,revision=1 WHERE id=$1")
                .bind(resource)
                .execute(service.db.database().pool())
                .await
                .unwrap();
        }
        assert!(is_revoked(&service.db, a.grant).await);
        assert_revoked(&service, &a, &mut server, context.clone()).await;
        let stale = fresh_assignment(&service.db, &a, 0xd4, &[]).await;
        // Restored canonical revisions cannot enroll stale checked epochs.
        assert!(sqlx::query("INSERT INTO worker_journal_grant_dependencies \
            (grant_id,source_grant_id,source_resource_id,grant_revision,resource_revision,grant_epoch,resource_epoch) \
            VALUES ($1,$2,$3,1,1,1,1)")
            .bind(stale.grant).bind(source).bind(resource).execute(service.db.database().pool()).await.is_err());
        assert_eq!(
            server
                .read(stale.session, stale.byte)
                .await
                .unwrap_err()
                .code(),
            tonic::Code::PermissionDenied
        );
        let fresh = fresh_assignment(&service.db, &a, 0xc3, &[(source, resource)]).await;
        server.read(fresh.session, fresh.byte).await.unwrap();
        assert_revoked(&service, &a, &mut server, context).await;
    }
}

#[tokio::test]
async fn deleting_and_recreating_source_identifiers_leaves_a_permanent_dependency_tombstone() {
    for delete_resource in [false, true] {
        let service = Arc::new(test_worker_service().await);
        let a = assignment(&service, 2, CLIENT_A, 0xa1).await;
        let (source, resource) = source_refs(&service.db, &a).await;
        let mut server = Server::start(service.clone(), true, Some((CLIENT_A, KEY_A))).await;
        server.read(a.session, a.byte).await.unwrap();
        let context = server.context();
        if delete_resource {
            sqlx::query("DELETE FROM resource_access WHERE id=$1")
                .bind(resource)
                .execute(service.db.database().pool())
                .await
                .unwrap();
            sqlx::query("INSERT INTO resource_access (id,org_id,kind,scope,usage) VALUES ($1,2,'credential_binding','organization','service')")
                .bind(resource).execute(service.db.database().pool()).await.unwrap();
        } else {
            sqlx::query("DELETE FROM resource_grants WHERE id=$1")
                .bind(source)
                .execute(service.db.database().pool())
                .await
                .unwrap();
        }
        sqlx::query("INSERT INTO resource_grants (id,org_id,resource_id,consumer_scope,consumer_session_id,usage) VALUES ($1,2,$2,'session',$3,'service')")
            .bind(source).bind(resource).bind(a.session).execute(service.db.database().pool()).await.unwrap();
        let missing:bool=sqlx::query_scalar("SELECT source_grant_id IS NULL FROM worker_journal_grant_dependencies WHERE grant_id=$1")
            .bind(a.grant).fetch_one(service.db.database().pool()).await.unwrap();
        assert!(missing);
        assert!(
            sqlx::query(
                "UPDATE worker_journal_grant_dependencies SET source_grant_id=$2 WHERE grant_id=$1"
            )
            .bind(a.grant)
            .bind(source)
            .execute(service.db.database().pool())
            .await
            .is_err()
        );
        assert!(
            sqlx::query("DELETE FROM worker_journal_grant_dependencies WHERE grant_id=$1")
                .bind(a.grant)
                .execute(service.db.database().pool())
                .await
                .is_err()
        );
        assert_revoked(&service, &a, &mut server, context).await;
    }
}

#[tokio::test]
async fn every_dependency_must_remain_live_and_expiry_extension_requires_a_fresh_assignment() {
    let service = Arc::new(test_worker_service().await);
    let a = assignment(&service, 2, CLIENT_A, 0xa1).await;
    let first = source_refs(&service.db, &a).await;
    let source = uuid::Uuid::new_v4();
    let resource = uuid::Uuid::new_v4();
    sqlx::query("INSERT INTO resource_access (id,org_id,kind,scope,usage) VALUES ($1,2,'credential_binding','organization','service')")
        .bind(resource).execute(service.db.database().pool()).await.unwrap();
    sqlx::query("INSERT INTO resource_grants (id,org_id,resource_id,consumer_scope,consumer_session_id,usage,expires_at) VALUES ($1,2,$2,'session',$3,'service',clock_timestamp()+INTERVAL '10 seconds')")
        .bind(source).bind(resource).bind(a.session).execute(service.db.database().pool()).await.unwrap();
    register_dependency(&service.db, a.grant, source, resource)
        .await
        .unwrap();
    let mut server = Server::start(service.clone(), true, Some((CLIENT_A, KEY_A))).await;
    server.read(a.session, a.byte).await.unwrap();
    let context = server.context();
    tokio::time::sleep(Duration::from_secs(11)).await;
    assert!(
        !is_revoked(&service.db, a.grant).await,
        "expiry is enforced online without a polling job"
    );
    assert_revoked(&service, &a, &mut server, context.clone()).await;
    sqlx::query("UPDATE resource_grants SET expires_at=NULL WHERE id=$1")
        .bind(source)
        .execute(service.db.database().pool())
        .await
        .unwrap();
    assert!(is_revoked(&service.db, a.grant).await);
    let fresh = fresh_assignment(&service.db, &a, 0xc3, &[first, (source, resource)]).await;
    server.read(fresh.session, fresh.byte).await.unwrap();
    assert_revoked(&service, &a, &mut server, context).await;
}

#[tokio::test]
async fn cancellation_archive_and_cross_tenant_lifecycle_changes_fail_closed() {
    for transition in ["cancel", "archive", "tenant"] {
        let service = Arc::new(test_worker_service().await);
        let a = assignment(&service, 2, CLIENT_A, 0xa1).await;
        let mut server = Server::start(service.clone(), true, Some((CLIENT_A, KEY_A))).await;
        server.read(a.session, a.byte).await.unwrap();
        let context = server.context();
        match transition {
            "cancel" => {
                service
                    .db
                    .request_session_actor_cancel(a.session)
                    .await
                    .unwrap();
                sqlx::query("UPDATE session_actors SET cancel_requested_at=NULL,cancel_inbox_seq=NULL,cancel_reported=FALSE WHERE session_id=$1")
                    .bind(a.session).execute(service.db.database().pool()).await.unwrap();
            }
            "archive" => {
                sqlx::query("UPDATE sessions SET archived_at=clock_timestamp() WHERE id=$1")
                    .bind(a.session)
                    .execute(service.db.database().pool())
                    .await
                    .unwrap();
                sqlx::query("UPDATE sessions SET archived_at=NULL WHERE id=$1")
                    .bind(a.session)
                    .execute(service.db.database().pool())
                    .await
                    .unwrap();
            }
            _ => {
                // Canonical org/session grant FK already prevents moving this source.
                assert!(
                    sqlx::query("UPDATE sessions SET org_id=3 WHERE id=$1")
                        .bind(a.session)
                        .execute(service.db.database().pool())
                        .await
                        .is_err()
                );
                assert!(!is_revoked(&service.db, a.grant).await);
                server.read(a.session, a.byte).await.unwrap();
                continue;
            }
        }
        assert!(is_revoked(&service.db, a.grant).await);
        assert_revoked(&service, &a, &mut server, context).await;
    }
}

#[tokio::test]
async fn expired_lease_renewal_cannot_revive_a_proof_even_when_the_fence_is_unchanged() {
    let service = Arc::new(test_worker_service().await);
    let a = assignment(&service, 2, CLIENT_A, 0xa1).await;
    let refs = source_refs(&service.db, &a).await;
    let mut server = Server::start(service.clone(), true, Some((CLIENT_A, KEY_A))).await;
    sqlx::query("UPDATE session_actors SET lease_expires_at=clock_timestamp()+INTERVAL '10 seconds' WHERE session_id=$1")
        .bind(a.session).execute(service.db.database().pool()).await.unwrap();
    let fresh = fresh_assignment(&service.db, &a, 0xc3, &[refs]).await;
    server.read(fresh.session, fresh.byte).await.unwrap();
    let context = server.context();
    tokio::time::sleep(Duration::from_secs(11)).await;
    service
        .db
        .renew_session_actor_lease(a.session, "equal-holder", 1, Duration::from_secs(240), 0)
        .await
        .unwrap()
        .unwrap();
    let fence: i64 =
        sqlx::query_scalar("SELECT lease_fence FROM session_actors WHERE session_id=$1")
            .bind(a.session)
            .fetch_one(service.db.database().pool())
            .await
            .unwrap();
    assert_eq!(fence, 1);
    // Otherwise-live grant/proof deadlines cannot hide a renewed expired origin.
    let still_in_window: bool = sqlx::query_scalar(
        "SELECT expires_at > clock_timestamp() FROM worker_journal_read_grants WHERE id=$1",
    )
    .bind(fresh.grant)
    .fetch_one(service.db.database().pool())
    .await
    .unwrap();
    assert!(still_in_window);
    assert!(is_revoked(&service.db, fresh.grant).await);
    assert_revoked(&service, &fresh, &mut server, context).await;
}

#[tokio::test]
async fn forward_renewal_and_non_authorization_metadata_edits_preserve_a_live_assignment() {
    let service = Arc::new(test_worker_service().await);
    let a = assignment(&service, 2, CLIENT_A, 0xa1).await;
    let server = Server::start(service.clone(), true, Some((CLIENT_A, KEY_A))).await;
    server.read(a.session, a.byte).await.unwrap();
    service
        .db
        .renew_session_actor_lease(a.session, "equal-holder", 1, Duration::from_secs(300), 0)
        .await
        .unwrap()
        .unwrap();
    sqlx::query("UPDATE sessions SET title='renamed' WHERE id=$1")
        .bind(a.session)
        .execute(service.db.database().pool())
        .await
        .unwrap();
    assert!(!is_revoked(&service.db, a.grant).await);
    server.read(a.session, a.byte).await.unwrap();
}

#[tokio::test]
async fn missing_cross_tenant_cross_session_and_agent_consumer_dependencies_fail_closed() {
    let service = Arc::new(test_worker_service().await);
    let a = assignment(&service, 2, CLIENT_A, 0xa1).await;
    let b = assignment(&service, 3, CLIENT_B, 0xb2).await;
    let brefs = source_refs(&service.db, &b).await;
    let orphan = fresh_assignment(&service.db, &a, 0xc3, &[]).await;
    let server = Server::start(service.clone(), true, Some((CLIENT_A, KEY_A))).await;
    assert_eq!(
        server
            .read(orphan.session, orphan.byte)
            .await
            .unwrap_err()
            .code(),
        tonic::Code::PermissionDenied
    );
    assert!(
        register_dependency(&service.db, orphan.grant, brefs.0, brefs.1)
            .await
            .is_err()
    );
    let other = service
        .db
        .create_session(crate::storage::test_database::test_session_row(2))
        .await
        .unwrap()
        .id;
    let refs = seed_dependency_source(&service.db, other, 2).await;
    assert!(
        register_dependency(&service.db, orphan.grant, refs.0, refs.1)
            .await
            .is_err()
    );
    let agent = service.db.create_test_agent(2, uuid::Uuid::new_v4()).await;
    sqlx::query("UPDATE resource_grants SET consumer_scope='agent',consumer_agent_id=$2,consumer_session_id=NULL WHERE id=$1")
        .bind(refs.0).bind(agent).execute(service.db.database().pool()).await.unwrap();
    assert!(
        register_dependency(&service.db, orphan.grant, refs.0, refs.1)
            .await
            .is_err()
    );
    let owned = (uuid::Uuid::new_v4(), uuid::Uuid::new_v4());
    sqlx::query("INSERT INTO resource_access (id,org_id,kind,scope,owner_agent_id,usage) VALUES ($1,2,'credential_binding','agent',$2,'service')")
        .bind(owned.1).bind(agent).execute(service.db.database().pool()).await.unwrap();
    sqlx::query("INSERT INTO resource_grants (id,org_id,resource_id,consumer_scope,consumer_session_id,usage) VALUES ($1,2,$2,'session',$3,'service')")
        .bind(owned.0).bind(owned.1).bind(a.session).execute(service.db.database().pool()).await.unwrap();
    assert!(
        register_dependency(&service.db, orphan.grant, owned.0, owned.1)
            .await
            .is_err()
    );
    server.read(a.session, a.byte).await.unwrap();
}

async fn seed_dependency_source(
    db: &StorageBackend,
    session: SessionId,
    org: i64,
) -> (uuid::Uuid, uuid::Uuid) {
    let resource = uuid::Uuid::new_v4();
    let source = uuid::Uuid::new_v4();
    sqlx::query("INSERT INTO resource_access (id,org_id,kind,scope,usage) VALUES ($1,$2,'credential_binding','organization','service')")
        .bind(resource).bind(org).execute(db.database().pool()).await.unwrap();
    sqlx::query("INSERT INTO resource_grants (id,org_id,resource_id,consumer_scope,consumer_session_id,usage) VALUES ($1,$2,$3,'session',$4,'service')")
        .bind(source).bind(org).bind(resource).bind(session).execute(db.database().pool()).await.unwrap();
    (source, resource)
}

#[tokio::test]
async fn agent_archive_restore_and_session_agent_rebinding_end_the_old_assignment() {
    for transition in ["agent_archive", "session_agent"] {
        let service = Arc::new(test_worker_service().await);
        let a = assignment(&service, 2, CLIENT_A, 0xa1).await;
        let refs = source_refs(&service.db, &a).await;
        let agent = service.db.create_test_agent(2, uuid::Uuid::new_v4()).await;
        sqlx::query("UPDATE sessions SET agent_id=$2 WHERE id=$1")
            .bind(a.session)
            .bind(agent)
            .execute(service.db.database().pool())
            .await
            .unwrap();
        let fresh = fresh_assignment(&service.db, &a, 0xc3, &[refs]).await;
        let mut server = Server::start(service.clone(), true, Some((CLIENT_A, KEY_A))).await;
        server.read(fresh.session, fresh.byte).await.unwrap();
        let context = server.context();
        if transition == "agent_archive" {
            service.db.delete_agent(2, agent.into()).await.unwrap();
            sqlx::query("UPDATE agents SET status='active',archived_at=NULL WHERE id=$1")
                .bind(agent)
                .execute(service.db.database().pool())
                .await
                .unwrap();
        } else {
            sqlx::query("UPDATE sessions SET agent_id=NULL WHERE id=$1")
                .bind(a.session)
                .execute(service.db.database().pool())
                .await
                .unwrap();
            sqlx::query("UPDATE sessions SET agent_id=$2 WHERE id=$1")
                .bind(a.session)
                .bind(agent)
                .execute(service.db.database().pool())
                .await
                .unwrap();
        }
        assert!(is_revoked(&service.db, fresh.grant).await);
        assert_revoked(&service, &fresh, &mut server, context).await;
    }
}

#[tokio::test]
async fn release_and_actor_recreation_cannot_reuse_an_old_origin_or_proof() {
    for recreate in [false, true] {
        let service = Arc::new(test_worker_service().await);
        let a = assignment(&service, 2, CLIENT_A, 0xa1).await;
        let mut server = Server::start(service.clone(), true, Some((CLIENT_A, KEY_A))).await;
        server.read(a.session, a.byte).await.unwrap();
        let context = server.context();
        if recreate {
            sqlx::query("DELETE FROM session_actors WHERE session_id=$1")
                .bind(a.session)
                .execute(service.db.database().pool())
                .await
                .unwrap();
            sqlx::query("INSERT INTO session_actors (session_id,org_id,lease_holder,lease_fence,lease_expires_at) VALUES ($1,2,'equal-holder',1,clock_timestamp()+INTERVAL '240 seconds')")
                .bind(a.session).execute(service.db.database().pool()).await.unwrap();
        } else {
            service
                .db
                .release_session_actor(a.session, "equal-holder", 1, 0)
                .await
                .unwrap()
                .unwrap();
            // Deliberately restore even the original fence: monotonic origin still refuses.
            sqlx::query("UPDATE session_actors SET lease_holder='equal-holder',lease_expires_at=clock_timestamp()+INTERVAL '240 seconds' WHERE session_id=$1")
                .bind(a.session).execute(service.db.database().pool()).await.unwrap();
        }
        assert!(is_revoked(&service.db, a.grant).await);
        assert_revoked(&service, &a, &mut server, context).await;
    }
}

#[tokio::test]
async fn caller_cannot_rewrite_epochs_origins_or_dependency_versions() {
    let service = Arc::new(test_worker_service().await);
    let a = assignment(&service, 2, CLIENT_A, 0xa1).await;
    let refs = source_refs(&service.db, &a).await;
    for (query, value) in [
        (
            "UPDATE sessions SET worker_authorization_epoch=worker_authorization_epoch+1 WHERE id=$1",
            a.session.uuid(),
        ),
        (
            "UPDATE session_actors SET worker_authorization_epoch=worker_authorization_epoch+1 WHERE session_id=$1",
            a.session.uuid(),
        ),
        (
            "UPDATE resource_grants SET worker_authorization_epoch=worker_authorization_epoch+1 WHERE id=$1",
            refs.0,
        ),
        (
            "UPDATE resource_access SET worker_authorization_epoch=worker_authorization_epoch+1 WHERE id=$1",
            refs.1,
        ),
    ] {
        assert!(
            sqlx::query(query)
                .bind(value)
                .execute(service.db.database().pool())
                .await
                .is_err()
        );
    }
    assert!(sqlx::query("UPDATE worker_journal_read_grants SET actor_authorization_epoch=actor_authorization_epoch+1 WHERE id=$1")
        .bind(a.grant).execute(service.db.database().pool()).await.is_err());
    assert!(sqlx::query("UPDATE worker_journal_grant_dependencies SET grant_revision=grant_revision+1,grant_epoch=grant_epoch+1 WHERE grant_id=$1")
        .bind(a.grant).execute(service.db.database().pool()).await.is_err());
    let server = Server::start(service.clone(), true, Some((CLIENT_A, KEY_A))).await;
    server.read(a.session, a.byte).await.unwrap();
}

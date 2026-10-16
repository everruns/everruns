use super::*;
use crate::kernel_imports::DEFAULT_ORG_ID;
use std::sync::Arc;
fn identity(org: i64, realm: &str, subject: &str) -> VerifiedRuntimeIdentity {
    VerifiedRuntimeIdentity {
        org_id: org,
        provider: "oidc".into(),
        realm: realm.into(),
        subject: subject.into(),
        name: "Alice".into(),
        avatar_url: None,
        management_user_id: None,
    }
}

async fn wait_for_binding_inserts(pool: &sqlx::PgPool, count: i64) {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let waiting: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM pg_locks WHERE database=(SELECT oid FROM pg_database WHERE datname=current_database()) AND relation='virtual_user_bindings'::regclass AND mode='RowExclusiveLock' AND NOT granted",
            )
            .fetch_one(pool)
            .await
            .unwrap();
            if waiting == count {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("requests must reach binding insertion");
}
#[tokio::test]
async fn identity_namespace_is_scoped_by_org_and_verified_realm() {
    let db = StorageBackend::test_database();
    let org = db
        .create_organization(CreateOrganizationRow {
            public_id: "org_00000000000000000000000000000010".into(),
            name: "Other".into(),
            created_by: None,
        })
        .await
        .unwrap()
        .org_id;
    let a = db
        .resolve_runtime_identity(identity(DEFAULT_ORG_ID, "issuer-a", "alice"))
        .await
        .unwrap();
    let same = db
        .resolve_runtime_identity(identity(DEFAULT_ORG_ID, "issuer-a", "alice"))
        .await
        .unwrap();
    let realm = db
        .resolve_runtime_identity(identity(DEFAULT_ORG_ID, "issuer-b", "alice"))
        .await
        .unwrap();
    let tenant = db
        .resolve_runtime_identity(identity(org, "issuer-a", "alice"))
        .await
        .unwrap();
    assert_eq!(a.id, same.id);
    assert_ne!(a.id, realm.id);
    assert_ne!(a.id, tenant.id);
    assert!(db.get_virtual_user(org, a.id).await.unwrap().is_none());
    assert_eq!(a.usage, "end_user");
}
#[tokio::test]
async fn concurrent_first_use_has_one_binding_and_account() {
    let db = Arc::new(StorageBackend::test_database());
    let mut tasks = vec![];
    for _ in 0..32 {
        let db = db.clone();
        tasks.push(tokio::spawn(async move {
            db.resolve_runtime_identity(identity(DEFAULT_ORG_ID, "issuer", "alice"))
                .await
                .unwrap()
                .id
        }));
    }
    let mut ids = vec![];
    for task in tasks {
        ids.push(task.await.unwrap());
    }
    assert!(ids.iter().all(|id| *id == ids[0]));
    assert_eq!(
        db.list_virtual_user_bindings(DEFAULT_ORG_ID, ids[0])
            .await
            .unwrap()
            .len(),
        1
    );
}
#[tokio::test]
async fn concurrent_default_first_use_has_one_binding_and_account() {
    let db = Arc::new(StorageBackend::test_database());
    let pool = db.database().pool();
    for round in 0..16 {
        let human = db
            .create_user(CreateUserRow {
                email: format!("concurrent-{round}@example.com"),
                name: "Manager".into(),
                avatar_url: None,
                roles: vec![],
                password_hash: None,
                email_verified: true,
                auth_provider: None,
                auth_provider_id: None,
                external_id: None,
            })
            .await
            .unwrap()
            .id;
        db.add_organization_member(DEFAULT_ORG_ID, human, "member")
            .await
            .unwrap();

        // Hold both first-use requests at INSERT, after their absent-row reads.
        // This lock and the wait query are confined to this test's private DB.
        let mut blocker = pool.begin().await.unwrap();
        sqlx::query("LOCK TABLE virtual_user_bindings IN SHARE MODE")
            .execute(&mut *blocker)
            .await
            .unwrap();
        let tasks: Vec<_> = (0..2)
            .map(|_| {
                let db = db.clone();
                tokio::spawn(async move { db.default_virtual_user(DEFAULT_ORG_ID, human).await })
            })
            .collect();
        wait_for_binding_inserts(pool, 2).await;
        blocker.commit().await.unwrap();
        let mut ids = Vec::new();
        for task in tasks {
            ids.push(
                tokio::time::timeout(std::time::Duration::from_secs(10), task)
                    .await
                    .unwrap()
                    .unwrap()
                    .expect("concurrent default account resolution must succeed")
                    .id,
            );
        }
        assert_eq!(ids[0], ids[1]);
        let bindings = db
            .list_virtual_user_bindings(DEFAULT_ORG_ID, ids[0])
            .await
            .unwrap();
        assert_eq!(bindings.len(), 1);
        assert_eq!(bindings[0].management_user_id, Some(human));
    }
}
#[tokio::test]
async fn first_use_rechecks_revocation_after_losing_binding_insert() {
    let db = Arc::new(StorageBackend::test_database());
    let pool = db.database().pool();
    let mut input = identity(DEFAULT_ORG_ID, "issuer", "revoked-during-first-use");
    input.name = "Revoked while binding".into();
    let mut blocker = pool.begin().await.unwrap();
    sqlx::query("LOCK TABLE virtual_user_bindings IN SHARE MODE")
        .execute(&mut *blocker)
        .await
        .unwrap();
    let task = {
        let db = db.clone();
        let input = input.clone();
        tokio::spawn(async move { db.resolve_runtime_identity(input).await })
    };
    wait_for_binding_inserts(pool, 1).await;
    // Another request bound and revoked the same account after our absent read.
    let inserted = sqlx::query("INSERT INTO virtual_user_bindings(org_id,virtual_user_id,provider,realm,subject,status) SELECT org_id,id,$2,$3,$4,'revoked' FROM virtual_users WHERE org_id=$1 AND name=$5")
        .bind(input.org_id).bind(input.provider).bind(input.realm).bind(input.subject).bind(input.name)
        .execute(&mut *blocker).await.unwrap();
    assert_eq!(inserted.rows_affected(), 1);
    blocker.commit().await.unwrap();
    let error = tokio::time::timeout(std::time::Duration::from_secs(10), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert_eq!(error.to_string(), "Identity binding revoked");
}
#[tokio::test]
async fn unlink_is_a_tombstone_and_cannot_reclaim_state() {
    let db = StorageBackend::test_database();
    let input = identity(DEFAULT_ORG_ID, "issuer", "alice");
    let user = db.resolve_runtime_identity(input.clone()).await.unwrap();
    let binding = db
        .list_virtual_user_bindings(DEFAULT_ORG_ID, user.id)
        .await
        .unwrap()[0]
        .id;
    assert!(
        !db.revoke_virtual_user_binding(DEFAULT_ORG_ID, VirtualUserId::new(), binding)
            .await
            .unwrap()
    );
    assert!(
        db.revoke_virtual_user_binding(DEFAULT_ORG_ID, user.id, binding)
            .await
            .unwrap()
    );
    assert!(
        db.list_virtual_user_bindings(DEFAULT_ORG_ID, user.id)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(db.resolve_runtime_identity(input).await.is_err());
}
#[tokio::test]
async fn console_default_requires_current_membership_and_cannot_be_unlinked() {
    let db = StorageBackend::test_database();
    let human = db
        .create_user(CreateUserRow {
            email: "manager@example.com".into(),
            name: "Manager".into(),
            avatar_url: None,
            roles: vec![],
            password_hash: None,
            email_verified: true,
            auth_provider: None,
            auth_provider_id: None,
            external_id: None,
        })
        .await
        .unwrap()
        .id;
    db.add_organization_member(DEFAULT_ORG_ID, human, "member")
        .await
        .unwrap();
    let user = db
        .default_virtual_user(DEFAULT_ORG_ID, human)
        .await
        .unwrap();
    let binding = db
        .list_virtual_user_bindings(DEFAULT_ORG_ID, user.id)
        .await
        .unwrap()[0]
        .id;
    assert!(
        !db.revoke_virtual_user_binding(DEFAULT_ORG_ID, user.id, binding)
            .await
            .unwrap()
    );
    assert!(db.default_virtual_user(999, human).await.is_err());
    let mut conflicting_identity = identity(DEFAULT_ORG_ID, "another-issuer", "manager");
    conflicting_identity.management_user_id = Some(human);
    assert!(
        db.resolve_runtime_identity(conflicting_identity)
            .await
            .is_err()
    );
    assert_eq!(
        db.default_virtual_user(DEFAULT_ORG_ID, human)
            .await
            .unwrap()
            .id,
        user.id
    );
}
#[tokio::test]
async fn connection_rotation_preserves_id_and_private_owner() {
    let db = StorageBackend::test_database();
    let alice = db
        .resolve_runtime_identity(identity(DEFAULT_ORG_ID, "issuer", "alice"))
        .await
        .unwrap();
    let bob = db
        .resolve_runtime_identity(identity(DEFAULT_ORG_ID, "issuer", "bob"))
        .await
        .unwrap();
    let input = CreateVirtualUserConnectionRow {
        virtual_user_id: alice.id,
        provider: "example".into(),
        connection_type: "api_key".into(),
        provider_user_id: None,
        provider_username: None,
        access_token_encrypted: Some(vec![1]),
        refresh_token_encrypted: None,
        scopes: None,
        expires_at: None,
        installation_id: None,
        provider_metadata: None,
    };
    let first = db
        .upsert_virtual_user_connection(input.clone())
        .await
        .unwrap();
    let mut rotate = input;
    rotate.access_token_encrypted = Some(vec![2]);
    let second = db.upsert_virtual_user_connection(rotate).await.unwrap();
    assert_eq!(first.id, second.id);
    assert_eq!(first.created_at, second.created_at);
    assert!(
        db.get_virtual_user_connection(bob.id, "example")
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn stale_refresh_and_revocation_cannot_overwrite_reconnected_credentials() {
    let db = StorageBackend::test_database();
    let account = db
        .resolve_runtime_identity(identity(DEFAULT_ORG_ID, "issuer", "alice"))
        .await
        .unwrap();
    let row = db
        .upsert_virtual_user_connection(CreateVirtualUserConnectionRow {
            virtual_user_id: account.id,
            provider: "oauth-fixture".into(),
            connection_type: "oauth".into(),
            provider_user_id: None,
            provider_username: None,
            access_token_encrypted: Some(vec![1]),
            refresh_token_encrypted: Some(vec![2]),
            scopes: None,
            expires_at: None,
            installation_id: None,
            provider_metadata: None,
        })
        .await
        .unwrap();
    let updated = db
        .rotate_runtime_connection(
            &[1],
            UpdateOAuthConnectionTokens {
                connection_id: row.id,
                access_token_encrypted: vec![3],
                refresh_token_encrypted: vec![4],
                expires_at: None,
                scopes: None,
            },
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.id, updated.id);
    assert!(
        db.rotate_runtime_connection(
            &[1],
            UpdateOAuthConnectionTokens {
                connection_id: row.id,
                access_token_encrypted: vec![5],
                refresh_token_encrypted: vec![6],
                expires_at: None,
                scopes: None,
            }
        )
        .await
        .unwrap()
        .is_none()
    );
    assert!(
        !db.revoke_runtime_connection_if_unchanged(row.id, &[1])
            .await
            .unwrap()
    );
    let current = db
        .get_virtual_user_connection(account.id, "oauth-fixture")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(current.access_token_encrypted, Some(vec![3]));
    assert_eq!(current.refresh_token_encrypted, Some(vec![4]));
}

#[tokio::test]
async fn setup_state_is_hash_bound_single_use_and_provider_scoped() {
    let db = Arc::new(StorageBackend::test_database());
    db.register_connection_setup(
        "opaque",
        "github",
        &[1; 32],
        chrono::Utc::now() + chrono::Duration::minutes(10),
    )
    .await
    .unwrap();
    assert!(
        !db.consume_connection_setup("opaque", "other", &[1; 32])
            .await
            .unwrap()
    );
    assert!(
        !db.consume_connection_setup("opaque", "github", &[2; 32])
            .await
            .unwrap()
    );
    let mut tasks = vec![];
    for _ in 0..16 {
        let db = db.clone();
        tasks.push(tokio::spawn(async move {
            db.consume_connection_setup("opaque", "github", &[1; 32])
                .await
                .unwrap()
        }));
    }
    let mut wins = 0;
    for task in tasks {
        wins += usize::from(task.await.unwrap());
    }
    assert_eq!(wins, 1);
}

#[tokio::test]
async fn setup_state_enforces_its_server_expiry() {
    let db = StorageBackend::test_database();
    db.register_connection_setup(
        "expired",
        "slack_manager_connect",
        &[1; 32],
        chrono::Utc::now() - chrono::Duration::seconds(1),
    )
    .await
    .unwrap();
    assert!(
        !db.consume_connection_setup("expired", "slack_manager_connect", &[1; 32])
            .await
            .unwrap()
    );
    db.register_connection_setup(
        "pending",
        "slack_manager_connect",
        &[2; 32],
        chrono::Utc::now() + chrono::Duration::minutes(30),
    )
    .await
    .unwrap();
    // An earlier pending transaction remains valid beyond the old ten-minute TTL.
    sqlx::query("UPDATE connection_setup_states SET expires_at = expires_at - interval '11 minutes' WHERE state = 'pending'")
        .execute(db.database().pool()).await.unwrap();
    assert!(
        db.consume_connection_setup("pending", "slack_manager_connect", &[2; 32])
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn a_continued_invocation_keeps_the_sessions_latest_identity() {
    let db = StorageBackend::test_database();
    let person = db.create_test_user(Uuid::now_v7()).await;
    let subject = VirtualUserId::from_uuid(person);
    db.create_virtual_user(CreateVirtualUserRow {
        org_id: DEFAULT_ORG_ID,
        id: subject,
        usage: "end_user".to_string(),
        name: "Alice".to_string(),
        description: None,
        avatar_url: None,
        locale: None,
        timezone: None,
    })
    .await
    .unwrap();
    let session = db.create_test_session().await;

    // Nobody invoked the session yet: the wake runs anonymously.
    let first = Uuid::now_v7();
    db.record_continued_runtime_invocation(DEFAULT_ORG_ID, session, first, None)
        .await
        .unwrap();
    assert!(db.runtime_invocation_exists(session, first).await.unwrap());
    assert!(
        !db.runtime_invocation_has_subject(session, first)
            .await
            .unwrap()
    );

    let sent = Uuid::now_v7();
    db.record_runtime_invocation(DEFAULT_ORG_ID, session, sent, Some(subject), None, None)
        .await
        .unwrap();
    let wake = Uuid::now_v7();
    db.record_continued_runtime_invocation(DEFAULT_ORG_ID, session, wake, None)
        .await
        .unwrap();
    assert_eq!(
        db.runtime_invocation_subject(session, wake).await.unwrap(),
        Some(subject)
    );

    // Another org's session id is refused.
    assert!(
        db.record_continued_runtime_invocation(DEFAULT_ORG_ID + 1, session, Uuid::now_v7(), None)
            .await
            .is_err()
    );
}

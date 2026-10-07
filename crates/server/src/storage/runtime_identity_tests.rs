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
    db.register_connection_setup("opaque", "github", &[1; 32])
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

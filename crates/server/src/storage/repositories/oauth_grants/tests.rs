// Grant upsert, revoke and listing against a real migrated database.

use crate::storage::{
    CreateOAuthClientRow, CreateOAuthRefreshTokenRow, CreateUserRow, StorageBackend,
};
use chrono::{Duration, Utc};
use uuid::Uuid;

async fn user(db: &StorageBackend, label: &str) -> Uuid {
    db.create_user(CreateUserRow {
        email: format!("{label}-{}@example.com", Uuid::now_v7().simple()),
        name: label.to_string(),
        avatar_url: None,
        roles: vec!["user".to_string()],
        password_hash: None,
        email_verified: true,
        auth_provider: None,
        auth_provider_id: None,
        external_id: None,
    })
    .await
    .expect("create user")
    .id
}

async fn client(db: &StorageBackend, name: &str) -> String {
    let client_id = format!("mcp_client_{}", Uuid::now_v7().simple());
    db.create_oauth_client(CreateOAuthClientRow {
        client_id: client_id.clone(),
        client_secret_hash: "hash".to_string(),
        client_name: name.to_string(),
        redirect_uris: serde_json::json!(["https://claude.ai/api/mcp/auth_callback"]),
    })
    .await
    .expect("create client");
    client_id
}

async fn refresh_token(db: &StorageBackend, client_id: &str, user_id: Uuid, grant_id: Uuid) {
    db.create_oauth_refresh_token(CreateOAuthRefreshTokenRow {
        token_hash: Uuid::now_v7().simple().to_string(),
        client_id: client_id.to_string(),
        user_id,
        org_id: everruns_core::DEFAULT_ORG_ID,
        scope: "mcp".to_string(),
        expires_at: Utc::now() + Duration::days(30),
        grant_id,
    })
    .await
    .expect("create refresh token");
}

async fn refresh_token_count(db: &StorageBackend, client_id: &str, user_id: Uuid) -> i64 {
    sqlx::query_scalar(
        "SELECT COUNT(*) FROM oauth_refresh_tokens WHERE client_id = $1 AND user_id = $2",
    )
    .bind(client_id)
    .bind(user_id)
    .fetch_one(db.pool())
    .await
    .unwrap()
}

#[tokio::test]
async fn approving_twice_keeps_one_grant() {
    let db = StorageBackend::test_database();
    let user_id = user(&db, "alice").await;
    let client_id = client(&db, "Cursor").await;

    let first = db.approve_oauth_grant(&client_id, user_id).await.unwrap();
    let second = db.approve_oauth_grant(&client_id, user_id).await.unwrap();

    assert_eq!(first.id, second.id, "re-approval must not stack a grant");
    assert_eq!(second.access, "read_and_run");
    assert_eq!(second.allowed_org_ids, None, "new grants cover all orgs");
    assert!(second.revoked_at.is_none());
    assert_eq!(
        db.list_active_oauth_grants_for_user(user_id)
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn revoke_deletes_the_clients_refresh_tokens_for_that_user_only() {
    let db = StorageBackend::test_database();
    let alice = user(&db, "alice").await;
    let bob = user(&db, "bob").await;
    let client_id = client(&db, "Claude").await;
    let alice_grant = db.approve_oauth_grant(&client_id, alice).await.unwrap();
    let bob_grant = db.approve_oauth_grant(&client_id, bob).await.unwrap();
    refresh_token(&db, &client_id, alice, alice_grant.id).await;
    refresh_token(&db, &client_id, alice, alice_grant.id).await;
    refresh_token(&db, &client_id, bob, bob_grant.id).await;

    // Another user cannot revoke Alice's grant.
    assert!(
        db.revoke_oauth_grant(alice_grant.id, bob)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(refresh_token_count(&db, &client_id, alice).await, 2);

    let revoked = db
        .revoke_oauth_grant(alice_grant.id, alice)
        .await
        .unwrap()
        .expect("owner revokes");
    assert!(revoked.revoked_at.is_some());
    assert_eq!(refresh_token_count(&db, &client_id, alice).await, 0);
    assert_eq!(refresh_token_count(&db, &client_id, bob).await, 1);
    assert!(
        db.list_active_oauth_grants_for_user(alice)
            .await
            .unwrap()
            .is_empty()
    );

    // Revoking again is a no-op the API reports as not found.
    assert!(
        db.revoke_oauth_grant(alice_grant.id, alice)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn reapproving_after_revoke_creates_a_fresh_grant() {
    let db = StorageBackend::test_database();
    let user_id = user(&db, "alice").await;
    let client_id = client(&db, "ChatGPT").await;
    let old = db.approve_oauth_grant(&client_id, user_id).await.unwrap();
    db.revoke_oauth_grant(old.id, user_id).await.unwrap();

    // ensure_* (token endpoint) must not revive a revoked grant.
    let ensured = db.ensure_oauth_grant(&client_id, user_id).await.unwrap();
    assert_eq!(ensured.id, old.id);
    assert!(ensured.revoked_at.is_some());

    let fresh = db.approve_oauth_grant(&client_id, user_id).await.unwrap();
    assert_ne!(
        fresh.id, old.id,
        "old tokens name the old grant and stay dead"
    );
    assert!(fresh.revoked_at.is_none());
    assert!(db.get_oauth_grant(old.id).await.unwrap().is_none());
}

#[tokio::test]
async fn ensure_creates_a_grant_only_when_none_exists() {
    let db = StorageBackend::test_database();
    let user_id = user(&db, "alice").await;
    let client_id = client(&db, "Cursor").await;

    let created = db.ensure_oauth_grant(&client_id, user_id).await.unwrap();
    assert!(created.revoked_at.is_none());
    let again = db.ensure_oauth_grant(&client_id, user_id).await.unwrap();
    assert_eq!(created.id, again.id);
}

#[tokio::test]
async fn list_joins_the_client_and_touch_sets_last_used() {
    let db = StorageBackend::test_database();
    let user_id = user(&db, "alice").await;
    let client_id = client(&db, "Claude").await;
    let grant = db.approve_oauth_grant(&client_id, user_id).await.unwrap();
    db.touch_oauth_grant_last_used(grant.id).await.unwrap();

    let rows = db.list_active_oauth_grants_for_user(user_id).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].client_name, "Claude");
    assert_eq!(rows[0].client_id, client_id);
    assert!(rows[0].last_used_at.is_some());
}

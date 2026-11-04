// MCP OAuth refresh-token rotation with a reuse grace window, against a real
// migrated database. Policy: `REFRESH_REUSE_GRACE_SECS`.

use super::{REFRESH_REUSE_GRACE_SECS, SpentRefresh, spent_refresh};
use crate::storage::{
    CreateOAuthClientRow, CreateOAuthRefreshTokenRow, CreateUserRow, OAuthRefreshUse,
    StorageBackend,
};
use chrono::{Duration, Utc};
use uuid::Uuid;

#[test]
fn a_rotated_token_is_honoured_once_inside_the_window() {
    let t = Utc::now();
    let g = Duration::seconds(REFRESH_REUSE_GRACE_SECS);
    assert_eq!(spent_refresh(t, false, t), SpentRefresh::Grace);
    assert_eq!(spent_refresh(t, false, t + g), SpentRefresh::Grace);
    assert_eq!(
        spent_refresh(t, true, t + Duration::seconds(5)),
        SpentRefresh::Refuse
    );
    let late = t + g + Duration::seconds(1);
    assert_eq!(spent_refresh(t, false, late), SpentRefresh::Reuse);
    assert_eq!(spent_refresh(t, true, late), SpentRefresh::Reuse);
}

async fn token(db: &StorageBackend, family_id: Option<Uuid>) -> (String, Uuid) {
    let user_id = db
        .create_user(CreateUserRow {
            email: format!("rt-{}@example.com", Uuid::now_v7().simple()),
            name: "rt".to_string(),
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
        .id;
    let client_id = format!("mcp_client_{}", Uuid::now_v7().simple());
    db.create_oauth_client(CreateOAuthClientRow {
        client_id: client_id.clone(),
        client_secret_hash: "hash".to_string(),
        client_name: "Claude".to_string(),
        redirect_uris: serde_json::json!(["https://claude.ai/api/mcp/auth_callback"]),
    })
    .await
    .expect("create client");
    let grant = db.approve_oauth_grant(&client_id, user_id).await.unwrap();
    let hash = Uuid::now_v7().simple().to_string();
    let row = db
        .create_oauth_refresh_token(CreateOAuthRefreshTokenRow {
            token_hash: hash.clone(),
            client_id,
            user_id,
            org_id: everruns_core::DEFAULT_ORG_ID,
            scope: "mcp".to_string(),
            expires_at: Utc::now() + Duration::days(30),
            grant_id: grant.id,
            family_id,
        })
        .await
        .expect("create refresh token");
    (hash, row.id)
}

async fn age_rotation(db: &StorageBackend, hash: &str, secs: i64) {
    sqlx::query(
        "UPDATE oauth_refresh_tokens SET rotated_at = NOW() - make_interval(secs => $2) WHERE token_hash = $1",
    )
    .bind(hash)
    .bind(secs as f64)
    .execute(db.pool())
    .await
    .unwrap();
}

#[tokio::test]
async fn first_use_mints_then_one_replay_mints_then_refuses() {
    let db = StorageBackend::test_database();
    let (hash, id) = token(&db, None).await;

    let OAuthRefreshUse::Mint(row) = db.use_oauth_refresh_token(&hash).await.unwrap() else {
        panic!("first use must mint");
    };
    assert_eq!(row.family(), id, "a chain's first token names the family");
    assert!(matches!(
        db.use_oauth_refresh_token(&hash).await.unwrap(),
        OAuthRefreshUse::Mint(_)
    ));
    // A third request inside the window is refused, and revokes nothing.
    assert!(matches!(
        db.use_oauth_refresh_token(&hash).await.unwrap(),
        OAuthRefreshUse::Refused
    ));
    let (successor, _) = token(&db, Some(id)).await;
    assert!(matches!(
        db.use_oauth_refresh_token(&successor).await.unwrap(),
        OAuthRefreshUse::Mint(_)
    ));
}

#[tokio::test]
async fn replay_after_the_window_deletes_the_family() {
    let db = StorageBackend::test_database();
    let (hash, id) = token(&db, None).await;
    assert!(matches!(
        db.use_oauth_refresh_token(&hash).await.unwrap(),
        OAuthRefreshUse::Mint(_)
    ));
    let (successor, _) = token(&db, Some(id)).await;
    let (unrelated, _) = token(&db, None).await;
    age_rotation(&db, &hash, REFRESH_REUSE_GRACE_SECS + 5).await;

    assert!(matches!(
        db.use_oauth_refresh_token(&hash).await.unwrap(),
        OAuthRefreshUse::ReuseDetected
    ));
    // The leaked chain is dead, including the successor the thief may hold.
    assert!(matches!(
        db.use_oauth_refresh_token(&successor).await.unwrap(),
        OAuthRefreshUse::Refused
    ));
    // Other chains are untouched.
    assert!(matches!(
        db.use_oauth_refresh_token(&unrelated).await.unwrap(),
        OAuthRefreshUse::Mint(_)
    ));
}

#[tokio::test]
async fn unknown_token_is_refused() {
    let db = StorageBackend::test_database();
    assert!(matches!(
        db.use_oauth_refresh_token("nope").await.unwrap(),
        OAuthRefreshUse::Refused
    ));
}

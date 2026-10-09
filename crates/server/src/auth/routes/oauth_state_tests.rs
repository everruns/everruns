use super::*;

#[test]
fn test_oauth_state_length_and_hex() {
    // State must be 32 hex chars (16 random bytes) — sufficient entropy for CSRF
    let state = generate_oauth_state();
    assert_eq!(state.len(), 32);
    assert!(
        state.chars().all(|c| c.is_ascii_hexdigit()),
        "state must be hex-encoded"
    );
}

// ========================================================================
// Password reset + email verification
// ========================================================================

use crate::auth::backend::AuthBackend;
use crate::auth::config::AuthConfig;
use crate::records::email::{EmailMessage, EmailResult, EmailSender, SentEmail};
use crate::storage::StorageBackend;
use crate::storage::models::CreateUserRow;
use async_trait::async_trait;
use everruns_core::host::HostComposition;
use std::sync::Arc;
use std::sync::Mutex;

fn test_backend() -> BuiltinAuthBackend {
    BuiltinAuthBackend::new(
        AuthConfig::default(),
        Arc::new(StorageBackend::test_database()),
        Arc::new(crate::platform::oss_host_composition()),
    )
}

#[derive(Default)]
struct RecordingEmailSender {
    messages: Mutex<Vec<EmailMessage>>,
}

#[async_trait]
impl EmailSender for RecordingEmailSender {
    async fn send_email(&self, message: EmailMessage) -> EmailResult<SentEmail> {
        self.messages.lock().unwrap().push(message);
        Ok(SentEmail {
            provider: "recording",
            id: "recording".to_string(),
        })
    }
}

fn backend_with_email_sender(
    sender: Arc<dyn EmailSender>,
) -> (BuiltinAuthBackend, Arc<StorageBackend>) {
    let db = Arc::new(StorageBackend::test_database());
    let platform = HostComposition::builder().build();
    (
        BuiltinAuthBackend::new(AuthConfig::default(), db.clone(), Arc::new(platform))
            .with_email_sender(sender),
        db,
    )
}

#[derive(Debug)]
struct SlowEmailSender(std::time::Duration);

#[async_trait]
impl EmailSender for SlowEmailSender {
    async fn send_email(&self, _message: EmailMessage) -> EmailResult<SentEmail> {
        tokio::time::sleep(self.0).await;
        Ok(SentEmail {
            provider: "slow-test",
            id: "slow-test".to_string(),
        })
    }

    fn name(&self) -> &'static str {
        "SlowEmailSender"
    }
}

async fn wait_for_recorded_messages(sender: &RecordingEmailSender, expected: usize) {
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            if sender.messages.lock().unwrap().len() >= expected {
                return;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("background email delivery timed out");
}

#[test]
fn recovery_start_response_delay_enforces_minimum() {
    assert_eq!(
        recovery_start_response_delay(std::time::Duration::ZERO),
        RECOVERY_START_MIN_RESPONSE_TIME
    );
    assert_eq!(
        recovery_start_response_delay(RECOVERY_START_MIN_RESPONSE_TIME),
        std::time::Duration::ZERO
    );
    assert_eq!(
        recovery_start_response_delay(RECOVERY_START_MIN_RESPONSE_TIME * 2),
        std::time::Duration::ZERO
    );
}

// Full-mode backend so password registration is enabled.
fn full_mode_backend() -> BuiltinAuthBackend {
    let config = AuthConfig {
        mode: AuthMode::Full,
        ..Default::default()
    };
    BuiltinAuthBackend::new(
        config,
        Arc::new(StorageBackend::test_database()),
        Arc::new(crate::platform::oss_host_composition()),
    )
}

// Minimal signup: registering without a name derives the display name from
// the email local-part.
#[tokio::test]
async fn register_without_name_derives_display_name_from_email() {
    let state = full_mode_backend();
    let db = state.db.clone();
    let outcome = register(
        State(state.clone()),
        None,
        HeaderMap::new(),
        CookieJar::new(),
        Json(RegisterRequest {
            email: "eli.wong@example.com".to_string(),
            password: "password12345".to_string(),
            name: None,
            captcha_token: None,
        }),
    )
    .await
    .expect("register should succeed");
    let RegisterOutcome::Session(status, _jar, _json) = outcome else {
        panic!("default mode must return an instant session");
    };
    assert_eq!(status, StatusCode::CREATED);

    let user = db
        .get_user_by_email("eli.wong@example.com")
        .await
        .unwrap()
        .expect("user created");
    assert_eq!(user.name, "Eli Wong");
}

// An explicit name is preserved (not overridden by the email derivation).
#[tokio::test]
async fn register_with_name_keeps_supplied_name() {
    let state = full_mode_backend();
    let db = state.db.clone();
    let _ = register(
        State(state.clone()),
        None,
        HeaderMap::new(),
        CookieJar::new(),
        Json(RegisterRequest {
            email: "someone@example.com".to_string(),
            password: "password12345".to_string(),
            name: Some("Ada Lovelace".to_string()),
            captcha_token: None,
        }),
    )
    .await
    .expect("register should succeed");

    let user = db
        .get_user_by_email("someone@example.com")
        .await
        .unwrap()
        .expect("user created");
    assert_eq!(user.name, "Ada Lovelace");
}

// Multi-tenant safety: with auto_join_default_org off (the default), a fresh
// signup owns NO org, so zero-org onboarding creates the user's own org
// instead of dumping every tenant into the shared default organization.
#[tokio::test]
async fn register_does_not_join_default_org_by_default() {
    let state = full_mode_backend();
    let db = state.db.clone();
    let outcome = register(
        State(state.clone()),
        None,
        HeaderMap::new(),
        CookieJar::new(),
        Json(RegisterRequest {
            email: "solo@example.com".to_string(),
            password: "password12345".to_string(),
            name: None,
            captcha_token: None,
        }),
    )
    .await
    .expect("register should succeed");

    let RegisterOutcome::Session(status, jar, Json(tokens)) = outcome else {
        panic!("default mode must return an instant session");
    };
    assert_eq!(status, StatusCode::CREATED);
    assert!(
        jar.get(ORG_COOKIE_NAME).is_none(),
        "zero-org signup must not receive a synthetic org cookie"
    );

    let auth_user = state
        .validate_token(&tokens.access_token)
        .await
        .expect("issued access token should validate");
    assert!(
        auth_user.organizations.is_empty(),
        "issued token must preserve zero-org memberships, got {:?}",
        auth_user.organizations
    );

    let user = db
        .get_user_by_email("solo@example.com")
        .await
        .unwrap()
        .expect("user created");
    let orgs = db.list_user_organizations(user.id).await.unwrap();
    assert!(
        orgs.is_empty(),
        "fresh signup must have zero org memberships by default, got {orgs:?}"
    );
}

// Single-tenant opt-in: AUTH_AUTO_JOIN_DEFAULT_ORG=true restores the shared
// default-org membership for a single-binary / small self-host.
#[tokio::test]
async fn register_joins_default_org_when_opted_in() {
    use crate::storage::models::CreateOrganizationRow;
    let config = AuthConfig {
        mode: AuthMode::Full,
        auto_join_default_org: true,
        ..Default::default()
    };
    let db = Arc::new(StorageBackend::test_database());
    // Membership can only attach if the default org exists.
    db.create_organization_with_id(
        DEFAULT_ORG_ID,
        CreateOrganizationRow {
            public_id: everruns_core::DEFAULT_ORG_PUBLIC_ID.to_string(),
            name: "Default Organization".to_string(),
            created_by: None,
        },
    )
    .await
    .expect("seed default org");
    let state = BuiltinAuthBackend::new(
        config,
        db.clone(),
        Arc::new(crate::platform::oss_host_composition()),
    );
    register(
        State(state.clone()),
        None,
        HeaderMap::new(),
        CookieJar::new(),
        Json(RegisterRequest {
            email: "joiner@example.com".to_string(),
            password: "password12345".to_string(),
            name: None,
            captcha_token: None,
        }),
    )
    .await
    .expect("register should succeed");

    let user = db
        .get_user_by_email("joiner@example.com")
        .await
        .unwrap()
        .expect("user created");
    assert!(
        db.is_organization_member(DEFAULT_ORG_ID, user.id)
            .await
            .unwrap(),
        "opted-in signup should join the default org"
    );
}

async fn seed_local_user(db: &StorageBackend, email: &str, password: &str) -> Uuid {
    let user = db
        .create_user(CreateUserRow {
            email: email.to_string(),
            name: "Test User".to_string(),
            avatar_url: None,
            roles: vec!["user".to_string()],
            password_hash: Some(hash_password(password).unwrap()),
            email_verified: false,
            auth_provider: Some("local".to_string()),
            auth_provider_id: None,
            external_id: None,
        })
        .await
        .expect("create user");
    user.id
}

async fn seed_oauth_user(db: &StorageBackend, email: &str, verified: bool) -> Uuid {
    let user = db
        .create_user(CreateUserRow {
            email: email.to_string(),
            name: "OAuth User".to_string(),
            avatar_url: None,
            roles: vec!["user".to_string()],
            password_hash: None,
            email_verified: verified,
            auth_provider: Some("github".to_string()),
            auth_provider_id: Some(format!("github:{email}")),
            external_id: None,
        })
        .await
        .expect("create oauth user");
    user.id
}

#[tokio::test]
async fn admin_mode_register_is_disabled() {
    let config = AuthConfig {
        mode: AuthMode::Admin,
        admin: Some(super::super::config::AdminConfig {
            email: "admin@example.com".to_string(),
            password: "password12345".to_string(),
        }),
        ..Default::default()
    };
    let state = BuiltinAuthBackend::new(
        config,
        Arc::new(StorageBackend::test_database()),
        Arc::new(crate::platform::oss_host_composition()),
    );

    let err = register(
        State(state),
        None,
        HeaderMap::new(),
        CookieJar::new(),
        Json(RegisterRequest {
            email: "attacker@example.com".to_string(),
            password: "password12345".to_string(),
            name: None,
            captcha_token: None,
        }),
    )
    .await
    .expect_err("admin mode must not allow self-registration");
    assert_eq!(err.status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn admin_login_checks_both_credentials() {
    for (email, password, allowed) in [
        ("admin@example.com", "password12345", true),
        ("admin@example.com", "wrong-password", false),
        ("other@example.com", "password12345", false),
    ] {
        let state = BuiltinAuthBackend::new(
            AuthConfig {
                mode: AuthMode::Admin,
                admin: Some(super::super::config::AdminConfig {
                    email: "admin@example.com".into(),
                    password: "password12345".into(),
                }),
                ..Default::default()
            },
            Arc::new(StorageBackend::test_database()),
            Arc::new(crate::platform::oss_host_composition()),
        );
        let result = login(
            State(state),
            None,
            HeaderMap::new(),
            CookieJar::new(),
            Json(LoginRequest {
                email: email.into(),
                password: password.into(),
            }),
        )
        .await;
        if allowed {
            let (_, Json(tokens)) = result.expect("configured credentials should authenticate");
            assert!(!tokens.access_token.is_empty());
        } else {
            assert_eq!(result.unwrap_err().status, StatusCode::UNAUTHORIZED);
        }
    }
}

#[tokio::test]
async fn admin_login_rejects_non_admin_email_collision() {
    let config = AuthConfig {
        mode: AuthMode::Admin,
        admin: Some(super::super::config::AdminConfig {
            email: "admin@example.com".to_string(),
            password: "password12345".to_string(),
        }),
        ..Default::default()
    };
    let db = Arc::new(StorageBackend::test_database());
    let attacker_id = seed_local_user(&db, " ADMIN@example.com ", "attacker12345").await;
    let state = BuiltinAuthBackend::new(
        config,
        db.clone(),
        Arc::new(crate::platform::oss_host_composition()),
    );

    let err = login(
        State(state),
        None,
        HeaderMap::new(),
        CookieJar::new(),
        Json(LoginRequest {
            email: "admin@example.com".to_string(),
            password: "password12345".to_string(),
        }),
    )
    .await
    .expect_err("admin bootstrap must fail closed on non-admin collision");
    assert_eq!(err.status, StatusCode::UNAUTHORIZED);
    assert!(
        db.list_user_organizations(attacker_id)
            .await
            .unwrap()
            .is_empty(),
        "colliding user must not be promoted into any org"
    );
}

#[tokio::test]
async fn password_reset_token_create_consume_is_single_use() {
    let db = StorageBackend::test_database();
    let user_id = seed_local_user(&db, "reset@example.com", "password12345").await;
    let (raw, hash) = generate_recovery_token();
    db.create_password_reset_token(user_id, &hash, Utc::now() + Duration::hours(1))
        .await
        .unwrap();

    // Happy path: first consume returns the owner.
    let hash_again = crate::api::org_invitations::hash_invite_token(&raw);
    assert_eq!(
        db.consume_password_reset_token(&hash_again).await.unwrap(),
        Some(user_id)
    );
    // Single-use: second consume returns None.
    assert_eq!(
        db.consume_password_reset_token(&hash_again).await.unwrap(),
        None
    );
}

#[tokio::test]
async fn password_reset_token_expired_and_unknown_return_none() {
    let db = StorageBackend::test_database();
    let user_id = seed_local_user(&db, "exp@example.com", "password12345").await;
    let (raw, hash) = generate_recovery_token();
    // Already expired.
    db.create_password_reset_token(user_id, &hash, Utc::now() - Duration::minutes(1))
        .await
        .unwrap();
    let hash_again = crate::api::org_invitations::hash_invite_token(&raw);
    assert_eq!(
        db.consume_password_reset_token(&hash_again).await.unwrap(),
        None
    );
    // Unknown token.
    assert_eq!(
        db.consume_password_reset_token("deadbeef").await.unwrap(),
        None
    );
}

#[tokio::test]
async fn email_verification_token_create_consume_is_single_use() {
    let db = StorageBackend::test_database();
    let user_id = seed_local_user(&db, "verify@example.com", "password12345").await;
    let (raw, hash) = generate_recovery_token();
    db.create_email_verification_token(user_id, &hash, Utc::now() + Duration::hours(1))
        .await
        .unwrap();
    let hash_again = crate::api::org_invitations::hash_invite_token(&raw);
    assert_eq!(
        db.consume_email_verification_token(&hash_again)
            .await
            .unwrap(),
        Some(user_id)
    );
    assert_eq!(
        db.consume_email_verification_token(&hash_again)
            .await
            .unwrap(),
        None
    );
}

#[tokio::test]
async fn reset_password_updates_hash_and_revokes_refresh_tokens() {
    let state = test_backend();
    let db = state.db.clone();
    let user_id = seed_local_user(&db, "rp@example.com", "oldpassword").await;

    // A live refresh token that the reset must revoke.
    db.create_refresh_token(CreateRefreshTokenRow {
        user_id,
        token_hash: "some-refresh-hash".to_string(),
        expires_at: Utc::now() + Duration::days(30),
    })
    .await
    .unwrap();

    let (raw, hash) = generate_recovery_token();
    db.create_password_reset_token(user_id, &hash, Utc::now() + Duration::hours(1))
        .await
        .unwrap();

    let _ = reset_password(
        State(state.clone()),
        Json(ResetPasswordRequest {
            token: raw,
            password: "newpassword12".to_string(),
        }),
    )
    .await
    .expect("reset should succeed");

    let user = db.get_user(user_id).await.unwrap().unwrap();
    let stored = user.password_hash.unwrap();
    // Old password no longer verifies; new one does.
    assert!(!verify_password("oldpassword", &stored).unwrap());
    assert!(verify_password("newpassword12", &stored).unwrap());
    // Refresh tokens were revoked.
    assert_eq!(
        db.consume_refresh_token_by_hash("some-refresh-hash")
            .await
            .unwrap()
            .map(|t| t.user_id),
        None
    );
}

#[tokio::test]
async fn reset_password_rejects_invalid_token() {
    let state = test_backend();
    let err = reset_password(
        State(state),
        Json(ResetPasswordRequest {
            token: "nope".to_string(),
            password: "newpassword12".to_string(),
        }),
    )
    .await
    .expect_err("invalid token must be rejected");
    assert_eq!(err.status, StatusCode::BAD_REQUEST);
}

// Time-expired tokens (not just malformed ones) must be rejected with the
// same generic 400 as invalid/used tokens.
#[tokio::test]
async fn reset_password_rejects_expired_token() {
    let state = test_backend();
    let user_id = seed_local_user(&state.db, "expired@example.com", "password12345").await;
    let (raw, hash) = generate_recovery_token();
    state
        .db
        .create_password_reset_token(user_id, &hash, Utc::now() - Duration::minutes(1))
        .await
        .unwrap();
    let err = reset_password(
        State(state),
        Json(ResetPasswordRequest {
            token: raw,
            password: "newpassword123".to_string(),
        }),
    )
    .await
    .expect_err("expired token must be rejected");
    assert_eq!(err.status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn verify_email_rejects_expired_token() {
    let state = test_backend();
    let user_id = seed_local_user(&state.db, "expired2@example.com", "password12345").await;
    let (raw, hash) = generate_recovery_token();
    state
        .db
        .create_email_verification_token(user_id, &hash, Utc::now() - Duration::minutes(1))
        .await
        .unwrap();
    let err = verify_email(State(state), Json(VerifyEmailRequest { token: raw }))
        .await
        .expect_err("expired token must be rejected");
    assert_eq!(err.status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn reset_password_rejects_short_password() {
    let state = test_backend();
    let err = reset_password(
        State(state),
        Json(ResetPasswordRequest {
            token: "whatever".to_string(),
            password: "short".to_string(),
        }),
    )
    .await
    .expect_err("short password must be rejected");
    assert_eq!(err.status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn verify_email_sets_email_verified() {
    let state = test_backend();
    let db = state.db.clone();
    let user_id = seed_local_user(&db, "ve@example.com", "password12345").await;
    assert!(!db.get_user(user_id).await.unwrap().unwrap().email_verified);

    let (raw, hash) = generate_recovery_token();
    db.create_email_verification_token(user_id, &hash, Utc::now() + Duration::hours(1))
        .await
        .unwrap();

    let _ = verify_email(
        State(state.clone()),
        Json(VerifyEmailRequest { token: raw }),
    )
    .await
    .expect("verify should succeed");

    assert!(db.get_user(user_id).await.unwrap().unwrap().email_verified);
}

#[tokio::test]
async fn verify_email_rejects_invalid_token() {
    let state = test_backend();
    let err = verify_email(
        State(state),
        Json(VerifyEmailRequest {
            token: "bad".to_string(),
        }),
    )
    .await
    .expect_err("invalid token must be rejected");
    assert_eq!(err.status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn forgot_password_is_enumeration_safe_for_unknown_email() {
    let state = test_backend();
    // No user exists; must still return 200 ok without error.
    let resp = forgot_password(
        State(state),
        Json(EmailOnlyRequest {
            email: "ghost@example.com".to_string(),
            captcha_token: None,
        }),
    )
    .await
    .expect("enumeration-safe success");
    assert!(resp.0.ok);
}

#[tokio::test]
async fn forgot_password_sends_reset_email_for_oauth_only_account() {
    let sender = Arc::new(RecordingEmailSender::default());
    let (state, db) = backend_with_email_sender(sender.clone());
    seed_oauth_user(&db, "oauth-only@example.com", true).await;

    let resp = forgot_password(
        State(state),
        Json(EmailOnlyRequest {
            email: "oauth-only@example.com".to_string(),
            captcha_token: None,
        }),
    )
    .await
    .expect("enumeration-safe success");
    assert!(resp.0.ok);

    wait_for_recorded_messages(&sender, 1).await;
    let messages = sender.messages.lock().unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].subject, "Reset your Everruns password");
    assert_eq!(messages[0].to[0].email, "oauth-only@example.com");
}

#[tokio::test]
async fn resend_verification_is_enumeration_safe_for_unknown_email() {
    let state = test_backend();
    let resp = resend_verification(
        State(state),
        Json(EmailOnlyRequest {
            email: "ghost@example.com".to_string(),
            captcha_token: None,
        }),
    )
    .await
    .expect("enumeration-safe success");
    assert!(resp.0.ok);
}

#[tokio::test]
async fn forgot_password_does_not_wait_for_slow_email_delivery() {
    let (state, _) = backend_with_email_sender(Arc::new(SlowEmailSender(
        std::time::Duration::from_millis(250),
    )));
    seed_local_user(&state.db, "recover@example.com", "password12345").await;
    seed_oauth_user(&state.db, "oauth@example.com", false).await;

    for email in [
        "recover@example.com",
        "oauth@example.com",
        "ghost@example.com",
    ] {
        let started = tokio::time::Instant::now();
        let resp = forgot_password(
            State(state.clone()),
            Json(EmailOnlyRequest {
                email: email.to_string(),
                captcha_token: None,
            }),
        )
        .await
        .expect("always generic success");
        let elapsed = started.elapsed();
        assert!(resp.0.ok);
        assert!(
            elapsed >= RECOVERY_START_MIN_RESPONSE_TIME,
            "forgot-password response for {email} bypassed timing normalization"
        );
        assert!(
            elapsed < std::time::Duration::from_millis(200),
            "forgot-password response for {email} waited for slow email delivery"
        );
    }
}

#[tokio::test]
async fn resend_verification_does_not_wait_for_slow_email_delivery() {
    let (state, _) = backend_with_email_sender(Arc::new(SlowEmailSender(
        std::time::Duration::from_millis(250),
    )));
    seed_local_user(&state.db, "unverified@example.com", "password12345").await;
    let verified_id = seed_local_user(&state.db, "verified@example.com", "password12345").await;
    state
        .db
        .update_user(
            verified_id,
            crate::storage::models::UpdateUser {
                email_verified: Some(true),
                ..Default::default()
            },
        )
        .await
        .expect("mark verified");
    seed_oauth_user(&state.db, "oauth@example.com", false).await;

    for email in [
        "unverified@example.com",
        "verified@example.com",
        "oauth@example.com",
        "ghost@example.com",
    ] {
        let started = tokio::time::Instant::now();
        let resp = resend_verification(
            State(state.clone()),
            Json(EmailOnlyRequest {
                email: email.to_string(),
                captcha_token: None,
            }),
        )
        .await
        .expect("always generic success");
        let elapsed = started.elapsed();
        assert!(resp.0.ok);
        assert!(
            elapsed >= RECOVERY_START_MIN_RESPONSE_TIME,
            "resend-verification response for {email} bypassed timing normalization"
        );
        assert!(
            elapsed < std::time::Duration::from_millis(200),
            "resend-verification response for {email} waited for slow email delivery"
        );
    }
}

// --- Auth hardening tests (abuse limits, password cap, logout revoke,
// captcha gate, OAuth failure redirect) ---

#[tokio::test]
async fn register_rejects_oversized_password() {
    let state = full_mode_backend();
    let err = register(
        State(state),
        None,
        HeaderMap::new(),
        CookieJar::new(),
        Json(RegisterRequest {
            email: "big@example.com".to_string(),
            password: "x".repeat(PASSWORD_MAX_LENGTH + 1),
            name: None,
            captcha_token: None,
        }),
    )
    .await
    .expect_err("oversized password must be rejected");
    assert_eq!(err.status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn login_rejects_oversized_password_with_generic_error() {
    let state = full_mode_backend();
    seed_local_user(&state.db, "cap@example.com", "password12345").await;
    let err = login(
        State(state),
        None,
        HeaderMap::new(),
        CookieJar::new(),
        Json(LoginRequest {
            email: "cap@example.com".to_string(),
            password: "x".repeat(PASSWORD_MAX_BYTES + 1),
        }),
    )
    .await
    .expect_err("oversized password must fail");
    assert_eq!(err.status, StatusCode::UNAUTHORIZED);
    assert_eq!(err.error, "Invalid email or password");
}

#[tokio::test]
async fn reset_password_rejects_oversized_password() {
    let state = test_backend();
    let err = reset_password(
        State(state),
        Json(ResetPasswordRequest {
            token: "whatever".to_string(),
            password: "x".repeat(PASSWORD_MAX_LENGTH + 1),
        }),
    )
    .await
    .expect_err("oversized password must be rejected before token consume");
    assert_eq!(err.status, StatusCode::UNPROCESSABLE_ENTITY);
}

// Per-account throttle: after the cross-IP budget is exhausted for one
// email, login returns 429 regardless of credentials; other accounts are
// unaffected.
#[tokio::test]
async fn login_per_account_throttle_returns_429() {
    let state = full_mode_backend();
    seed_local_user(&state.db, "stuffed@example.com", "password12345").await;
    let mut last_status = None;
    for _ in 0..25 {
        let result = login(
            State(state.clone()),
            None,
            HeaderMap::new(),
            CookieJar::new(),
            Json(LoginRequest {
                email: "stuffed@example.com".to_string(),
                password: "wrong-password".to_string(),
            }),
        )
        .await;
        last_status = result.err().map(|e| e.status);
    }
    assert_eq!(
        last_status,
        Some(StatusCode::TOO_MANY_REQUESTS),
        "per-account budget must trip after repeated failures"
    );

    // A different account still gets the normal generic 401.
    seed_local_user(&state.db, "fresh@example.com", "password12345").await;
    let err = login(
        State(state),
        None,
        HeaderMap::new(),
        CookieJar::new(),
        Json(LoginRequest {
            email: "fresh@example.com".to_string(),
            password: "wrong-password".to_string(),
        }),
    )
    .await
    .expect_err("wrong password fails");
    assert_eq!(err.status, StatusCode::UNAUTHORIZED);
}

// Logout must revoke the refresh token server-side, not just clear the
// cookie (TM-AUTH-003).
#[tokio::test]
async fn logout_revokes_refresh_token_server_side() {
    let state = full_mode_backend();
    let db = state.db.clone();
    let RegisterOutcome::Session(_status, jar, _json) = register(
        State(state.clone()),
        None,
        HeaderMap::new(),
        CookieJar::new(),
        Json(RegisterRequest {
            email: "bye@example.com".to_string(),
            password: "password12345".to_string(),
            name: None,
            captcha_token: None,
        }),
    )
    .await
    .expect("register succeeds") else {
        panic!("default mode must return an instant session");
    };

    let refresh_cookie = jar
        .get("refresh_token")
        .expect("refresh cookie set")
        .value()
        .to_string();
    let token_hash = hash_token(&refresh_cookie);

    let _cleared = logout(State(state), jar).await;

    let consumed = db
        .consume_refresh_token_by_hash(&token_hash)
        .await
        .expect("storage reachable");
    assert!(
        consumed.is_none(),
        "refresh token must already be revoked by logout"
    );
}

// Email budget: the second forgot-password for the same address within a
// minute is silently skipped but still returns the generic success.
#[tokio::test]
async fn forgot_password_email_budget_stays_enumeration_safe() {
    let state = test_backend();
    for _ in 0..2 {
        let resp = forgot_password(
            State(state.clone()),
            Json(EmailOnlyRequest {
                email: "budget@example.com".to_string(),
                captcha_token: None,
            }),
        )
        .await
        .expect("always generic success");
        assert!(resp.0.ok);
    }
}

// Captcha gate: when Turnstile is configured, a missing token is a
// generic 403 before any account work (empty token short-circuits in the
// verifier without a network call).
#[tokio::test]
async fn register_requires_captcha_when_configured() {
    let config = AuthConfig {
        mode: AuthMode::Full,
        turnstile: Some(super::super::config::TurnstileAuthConfig {
            site_key: "site".to_string(),
            secret_key: "secret".to_string(),
        }),
        ..Default::default()
    };
    let state = BuiltinAuthBackend::new(
        config,
        Arc::new(StorageBackend::test_database()),
        Arc::new(crate::platform::oss_host_composition()),
    );
    let err = register(
        State(state),
        None,
        HeaderMap::new(),
        CookieJar::new(),
        Json(RegisterRequest {
            email: "bot@example.com".to_string(),
            password: "password12345".to_string(),
            name: None,
            captcha_token: None,
        }),
    )
    .await
    .expect_err("missing captcha token must be rejected");
    assert_eq!(err.status, StatusCode::FORBIDDEN);
}

// OAuth callback failures land on the login door with a coarse category,
// never a raw JSON error (browser-only endpoint).
#[tokio::test]
async fn oauth_callback_provider_error_redirects_to_login() {
    use axum::response::IntoResponse;
    let state = full_mode_backend();
    let frontend = state.config.frontend_url.trim_end_matches('/').to_string();
    let (_jar, redirect) = oauth_callback(
        State(state),
        None,
        HeaderMap::new(),
        Path("google".to_string()),
        Query(OAuthCallbackQuery {
            code: None,
            state: None,
            error: Some("access_denied".to_string()),
        }),
        CookieJar::new(),
    )
    .await;
    let response = redirect.into_response();
    let location = response
        .headers()
        .get(axum::http::header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    assert_eq!(location, format!("{frontend}/login?error=oauth_cancelled"));
}

#[tokio::test]
async fn oauth_callback_missing_params_redirects_to_login() {
    use axum::response::IntoResponse;
    let state = full_mode_backend();
    let frontend = state.config.frontend_url.trim_end_matches('/').to_string();
    let (_jar, redirect) = oauth_callback(
        State(state),
        None,
        HeaderMap::new(),
        Path("google".to_string()),
        Query(OAuthCallbackQuery {
            code: None,
            state: None,
            error: None,
        }),
        CookieJar::new(),
    )
    .await;
    let response = redirect.into_response();
    let location = response
        .headers()
        .get(axum::http::header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    assert_eq!(location, format!("{frontend}/login?error=oauth_failed"));
}

#[tokio::test]
async fn oauth_callback_failure_honors_configured_login_origin() {
    use axum::response::IntoResponse;
    let mut state = full_mode_backend();
    state.config.login_origin = Some("https://id.example.com".to_string());
    let (_jar, redirect) = oauth_callback(
        State(state),
        None,
        HeaderMap::new(),
        Path("google".to_string()),
        Query(OAuthCallbackQuery {
            code: None,
            state: None,
            error: Some("access_denied".to_string()),
        }),
        CookieJar::new(),
    )
    .await;
    let response = redirect.into_response();
    assert_eq!(
        response
            .headers()
            .get(axum::http::header::LOCATION)
            .and_then(|value| value.to_str().ok()),
        Some("https://id.example.com/login?error=oauth_cancelled")
    );
}

// TM-AUTH-007: the state/PKCE cookie is single-use — it must be cleared on
// the callback response so a captured callback URL can't be replayed to
// pass CSRF validation again.
#[tokio::test]
async fn oauth_callback_clears_state_cookie_on_provider_error() {
    let state = full_mode_backend();
    let jar = CookieJar::new().add(Cookie::new(OAUTH_STATE_COOKIE, "some-state-value"));
    let (returned_jar, _redirect) = oauth_callback(
        State(state),
        None,
        HeaderMap::new(),
        Path("google".to_string()),
        Query(OAuthCallbackQuery {
            code: None,
            state: None,
            error: Some("access_denied".to_string()),
        }),
        jar,
    )
    .await;

    assert!(
        returned_jar.get(OAUTH_STATE_COOKIE).is_none(),
        "state cookie must be cleared so it cannot be replayed"
    );
}

#[tokio::test]
async fn oauth_callback_state_mismatch_is_rejected_and_clears_cookie() {
    let mut state = full_mode_backend();
    state.config.google = Some(super::super::config::GoogleOAuthConfig {
        base: super::super::config::OAuthProviderConfig {
            client_id: "test-client-id".to_string(),
            client_secret: "test-client-secret".to_string(),
            redirect_uri: "https://example.com/v1/auth/callback/google".to_string(),
        },
        allowed_domains: None,
    });
    let frontend = state.config.frontend_url.trim_end_matches('/').to_string();

    let jar = CookieJar::new().add(Cookie::new(OAUTH_STATE_COOKIE, "expected-state"));
    let (returned_jar, redirect) = oauth_callback(
        State(state),
        None,
        HeaderMap::new(),
        Path("google".to_string()),
        Query(OAuthCallbackQuery {
            code: Some("some-code".to_string()),
            // Attacker-supplied state does not match the cookie: rejected.
            state: Some("attacker-state".to_string()),
            error: None,
        }),
        jar,
    )
    .await;

    use axum::response::IntoResponse;
    let response = redirect.into_response();
    let location = response
        .headers()
        .get(axum::http::header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    assert_eq!(location, format!("{frontend}/login?error=oauth_failed"));

    assert!(
        returned_jar.get(OAUTH_STATE_COOKIE).is_none(),
        "mismatched state must still clear the cookie, not leave it for a retry"
    );
}

// --- signup email-confirm mode (AUTH_SIGNUP_EMAIL_CONFIRM) ---

fn confirm_mode_backend() -> BuiltinAuthBackend {
    let config = AuthConfig {
        mode: AuthMode::Full,
        signup_email_confirm: true,
        ..Default::default()
    };
    BuiltinAuthBackend::new(
        config,
        Arc::new(StorageBackend::test_database()),
        Arc::new(crate::platform::oss_host_composition()),
    )
}

#[tokio::test]
async fn confirm_mode_register_creates_account_without_session() {
    let state = confirm_mode_backend();
    let db = state.db.clone();
    let outcome = register(
        State(state),
        None,
        HeaderMap::new(),
        CookieJar::new(),
        Json(RegisterRequest {
            email: "pending@example.com".to_string(),
            password: "password12345".to_string(),
            name: None,
            captcha_token: None,
        }),
    )
    .await
    .expect("register ok");
    assert!(
        matches!(outcome, RegisterOutcome::ConfirmationSent(_)),
        "confirm mode must not return a session"
    );
    let user = db
        .get_user_by_email("pending@example.com")
        .await
        .unwrap()
        .expect("account created");
    assert!(!user.email_verified);
}

// Existing address: identical generic outcome, no duplicate account, no
// on-screen enumeration signal.
#[tokio::test]
async fn confirm_mode_register_existing_email_is_indistinguishable() {
    let state = confirm_mode_backend();
    seed_local_user(&state.db, "taken@example.com", "password12345").await;
    let outcome = register(
        State(state.clone()),
        None,
        HeaderMap::new(),
        CookieJar::new(),
        Json(RegisterRequest {
            email: "taken@example.com".to_string(),
            password: "password12345".to_string(),
            name: None,
            captcha_token: None,
        }),
    )
    .await
    .expect("must be generic success");
    assert!(matches!(outcome, RegisterOutcome::ConfirmationSent(_)));
}

#[tokio::test]
async fn register_rejects_password_without_digit() {
    let state = full_mode_backend();
    let err = register(
        State(state),
        None,
        HeaderMap::new(),
        CookieJar::new(),
        Json(RegisterRequest {
            email: "nodigit@example.com".to_string(),
            password: "longenoughpassword".to_string(),
            name: None,
            captcha_token: None,
        }),
    )
    .await
    .expect_err("digit-less password must be rejected");
    assert_eq!(err.status, StatusCode::UNPROCESSABLE_ENTITY);
}

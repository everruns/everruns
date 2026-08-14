//! Auth integration tests for refresh token flows
//!
//! Tests the refresh token endpoint with both JSON body and HttpOnly cookie sources,
//! verifying the fix for cookie path (/ vs /v1/auth) and the new cookie-based refresh flow.
//!
//! Run with: cargo test -p everruns-server --test auth_integration_test

use everruns_host::HostComposition;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

use everruns_server::auth::config::{AuthConfig, AuthMode, JwtConfig};
use everruns_server::auth::{self, BuiltinAuthBackend};
use everruns_server::seed;
use everruns_server::storage::StorageBackend;

/// Build a mini router with auth routes backed by in-memory storage.
async fn auth_router() -> (Router, Arc<StorageBackend>) {
    let db = Arc::new(StorageBackend::in_memory());
    let grade = everruns_core::DeploymentGrade::from_env();
    seed::seed_all(&db, grade, &seed::SeedAuthContext::default())
        .await
        .expect("seed failed");

    let config = AuthConfig {
        mode: AuthMode::Full,
        jwt: JwtConfig {
            secret: "test-secret-for-auth-integration-tests".to_string(),
            access_token_lifetime: Duration::from_secs(900),
            refresh_token_lifetime: Duration::from_secs(86400),
        },
        ..Default::default()
    };

    let backend = BuiltinAuthBackend::new(
        config,
        db.clone(),
        std::sync::Arc::new(everruns_server::platform::oss_host_composition()),
    );
    let router = auth::routes(backend);
    (router, db)
}

/// Helper: issue a request and return (status, body_json, set-cookie headers).
async fn send(
    router: &Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
    cookies: Option<&str>,
) -> (StatusCode, Value, Vec<String>) {
    let mut builder = Request::builder().method(method).uri(uri);

    if body.is_some() {
        builder = builder.header("content-type", "application/json");
    }
    if let Some(c) = cookies {
        builder = builder.header("cookie", c);
    }

    let request = if let Some(b) = body {
        builder
            .body(Body::from(serde_json::to_string(&b).unwrap()))
            .unwrap()
    } else {
        builder.body(Body::empty()).unwrap()
    };

    let response = router.clone().oneshot(request).await.unwrap();
    let status = response.status();

    let set_cookies: Vec<String> = response
        .headers()
        .get_all("set-cookie")
        .iter()
        .map(|v| v.to_str().unwrap().to_string())
        .collect();

    let bytes = response.into_body().collect().await.unwrap().to_bytes();

    let json: Value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };

    (status, json, set_cookies)
}

/// Extract a named cookie value from Set-Cookie headers.
fn extract_cookie_value(set_cookies: &[String], name: &str) -> Option<String> {
    for header in set_cookies {
        if header.starts_with(&format!("{name}=")) {
            let value = header
                .split(';')
                .next()
                .unwrap()
                .trim_start_matches(&format!("{name}="));
            return Some(value.to_string());
        }
    }
    None
}

/// Register a test user and return (access_token, refresh_token, set_cookies).
async fn register_user(
    router: &Router,
    email: &str,
    password: &str,
) -> (String, String, Vec<String>) {
    let (status, body, cookies) = send(
        router,
        "POST",
        "/v1/auth/register",
        Some(json!({
            "email": email,
            "password": password,
            "name": "Test User"
        })),
        None,
    )
    .await;

    assert_eq!(status, StatusCode::CREATED, "register failed: {body}");
    let access_token = body["access_token"].as_str().unwrap().to_string();
    let refresh_token = body["refresh_token"].as_str().unwrap().to_string();
    (access_token, refresh_token, cookies)
}

// ============================================
// Registration validation tests (EVE-453)
// ============================================

// EVE-453 / TM-AUTH-004: direct API calls must not be able to bypass the UI's
// `minLength={8}` and create weak-password accounts.
#[tokio::test]
async fn test_register_rejects_short_password_via_api() {
    let (router, _db) = auth_router().await;

    let (status, body, _cookies) = send(
        &router,
        "POST",
        "/v1/auth/register",
        Some(json!({
            "email": "weak@example.com",
            "password": "short",
            "name": "Weak Pw User"
        })),
        None,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "short password must be rejected: {body}"
    );

    // Acceptable password works.
    let (status_ok, _body_ok, _) = send(
        &router,
        "POST",
        "/v1/auth/register",
        Some(json!({
            "email": "weak@example.com",
            "password": "longenough123",
            "name": "Weak Pw User"
        })),
        None,
    )
    .await;
    assert_eq!(status_ok, StatusCode::CREATED);
}

// EVE-453: rejection happens before any account-existence check, so the
// short-password error is identical whether or not the email already exists.
// Prevents using the new validation as an enumeration oracle.
#[tokio::test]
async fn test_register_short_password_does_not_leak_account_existence() {
    let (router, _db) = auth_router().await;

    // Pre-register the email with a valid password.
    register_user(&router, "exists@example.com", "validpassword1").await;

    let (status_a, body_a, _) = send(
        &router,
        "POST",
        "/v1/auth/register",
        Some(json!({
            "email": "exists@example.com",
            "password": "short",
            "name": "X"
        })),
        None,
    )
    .await;
    let (status_b, body_b, _) = send(
        &router,
        "POST",
        "/v1/auth/register",
        Some(json!({
            "email": "fresh@example.com",
            "password": "short",
            "name": "Y"
        })),
        None,
    )
    .await;
    assert_eq!(status_a, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(status_b, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        body_a, body_b,
        "short-password rejection must not depend on account existence"
    );
}

// ============================================
// Positive path tests
// ============================================

#[tokio::test]
async fn test_refresh_via_json_body() {
    let (router, _db) = auth_router().await;

    let (_access, refresh, _cookies) =
        register_user(&router, "body@example.com", "password12345").await;

    // Refresh using JSON body (the original flow)
    let (status, body, new_cookies) = send(
        &router,
        "POST",
        "/v1/auth/refresh",
        Some(json!({ "refresh_token": refresh })),
        None,
    )
    .await;

    assert_eq!(status, StatusCode::OK, "refresh via body failed: {body}");
    assert!(body["access_token"].is_string());
    assert!(body["refresh_token"].is_string());
    assert_eq!(body["token_type"], "Bearer");

    // New tokens should be different (token rotation)
    assert_ne!(body["refresh_token"].as_str().unwrap(), refresh);

    // Should also set cookies
    assert!(
        extract_cookie_value(&new_cookies, "access_token").is_some(),
        "access_token cookie missing"
    );
    assert!(
        extract_cookie_value(&new_cookies, "refresh_token").is_some(),
        "refresh_token cookie missing"
    );
}

#[tokio::test]
async fn test_refresh_via_cookie() {
    let (router, _db) = auth_router().await;

    let (_access, refresh, _cookies) =
        register_user(&router, "cookie@example.com", "password12345").await;

    // Refresh using cookie (no JSON body) — the new browser flow
    let cookie_header = format!("refresh_token={refresh}");
    let (status, body, _new_cookies) = send(
        &router,
        "POST",
        "/v1/auth/refresh",
        None,
        Some(&cookie_header),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "refresh via cookie failed: {body}");
    assert!(body["access_token"].is_string());
    assert!(body["refresh_token"].is_string());
    // Token rotation: new refresh token must differ from old one
    assert_ne!(body["refresh_token"].as_str().unwrap(), refresh);
}

#[tokio::test]
async fn test_refresh_body_takes_precedence_over_cookie() {
    let (router, _db) = auth_router().await;

    // Register two users
    let (_a1, refresh1, _c1) = register_user(&router, "user1@example.com", "password12345").await;
    let (_a2, refresh2, _c2) = register_user(&router, "user2@example.com", "password12345").await;

    // Send body with user1's token, cookie with user2's token
    // Body should take precedence
    let cookie_header = format!("refresh_token={refresh2}");
    let (status, body, _cookies) = send(
        &router,
        "POST",
        "/v1/auth/refresh",
        Some(json!({ "refresh_token": refresh1 })),
        Some(&cookie_header),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "refresh failed: {body}");

    // Verify the user2 token is still valid (wasn't consumed)
    let cookie_header2 = format!("refresh_token={refresh2}");
    let (status2, body2, _) = send(
        &router,
        "POST",
        "/v1/auth/refresh",
        None,
        Some(&cookie_header2),
    )
    .await;
    assert_eq!(
        status2,
        StatusCode::OK,
        "user2 refresh should still work: {body2}"
    );
}

#[tokio::test]
async fn test_refresh_sets_cookie_with_root_path() {
    let (router, _db) = auth_router().await;

    let (_access, refresh, _cookies) =
        register_user(&router, "path@example.com", "password12345").await;

    let (status, _body, new_cookies) = send(
        &router,
        "POST",
        "/v1/auth/refresh",
        Some(json!({ "refresh_token": refresh })),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Verify the refresh_token cookie has Path=/
    let refresh_cookie = new_cookies
        .iter()
        .find(|c| c.starts_with("refresh_token="))
        .expect("refresh_token cookie not set");
    assert!(
        refresh_cookie.contains("Path=/;") || refresh_cookie.ends_with("Path=/"),
        "refresh_token cookie should have Path=/, got: {refresh_cookie}"
    );

    // Verify it's HttpOnly
    let lower = refresh_cookie.to_lowercase();
    assert!(lower.contains("httponly"), "should be HttpOnly");
}

// ============================================
// Negative path tests
// ============================================

#[tokio::test]
async fn test_refresh_no_token_returns_401() {
    let (router, _db) = auth_router().await;

    // No body, no cookie
    let (status, body, _cookies) = send(&router, "POST", "/v1/auth/refresh", None, None).await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(
        body["detail"]
            .as_str()
            .unwrap()
            .contains("Missing refresh token")
    );
}

#[tokio::test]
async fn test_refresh_invalid_token_returns_401() {
    let (router, _db) = auth_router().await;

    let (status, body, _cookies) = send(
        &router,
        "POST",
        "/v1/auth/refresh",
        Some(json!({ "refresh_token": "this-is-not-a-valid-jwt" })),
        None,
    )
    .await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(
        body["detail"]
            .as_str()
            .unwrap()
            .contains("Invalid refresh token")
    );
}

#[tokio::test]
async fn test_refresh_invalid_cookie_returns_401() {
    let (router, _db) = auth_router().await;

    let (status, body, _cookies) = send(
        &router,
        "POST",
        "/v1/auth/refresh",
        None,
        Some("refresh_token=garbage-jwt-value"),
    )
    .await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(
        body["detail"]
            .as_str()
            .unwrap()
            .contains("Invalid refresh token")
    );
}

#[tokio::test]
async fn test_refresh_reuse_revoked_token_returns_401() {
    let (router, _db) = auth_router().await;

    let (_access, refresh, _cookies) =
        register_user(&router, "revoke@example.com", "password12345").await;

    // First refresh succeeds (consumes the token)
    let (status1, _body1, _) = send(
        &router,
        "POST",
        "/v1/auth/refresh",
        Some(json!({ "refresh_token": refresh })),
        None,
    )
    .await;
    assert_eq!(status1, StatusCode::OK);

    // Second refresh with the same token should fail (token was rotated/deleted)
    let (status2, body2, _) = send(
        &router,
        "POST",
        "/v1/auth/refresh",
        Some(json!({ "refresh_token": refresh })),
        None,
    )
    .await;
    assert_eq!(
        status2,
        StatusCode::UNAUTHORIZED,
        "reused token should be rejected: {body2}"
    );
}

// EVE-454: concurrent refresh requests using the same token must not both
// succeed. The previous get-then-delete pattern allowed a race where two
// requests passed the existence check before either delete committed,
// minting multiple replacement refresh tokens. The atomic
// `consume_refresh_token_by_hash` (DELETE … RETURNING) closes that window.
#[tokio::test]
async fn test_refresh_concurrent_requests_only_one_succeeds() {
    let (router, _db) = auth_router().await;

    let (_access, refresh, _cookies) =
        register_user(&router, "concurrent@example.com", "password12345").await;

    // Fire multiple concurrent refreshes with the same refresh token.
    let mut handles = Vec::new();
    for _ in 0..8 {
        let router = router.clone();
        let refresh = refresh.clone();
        handles.push(tokio::spawn(async move {
            let (status, _body, _cookies) = send(
                &router,
                "POST",
                "/v1/auth/refresh",
                Some(json!({ "refresh_token": refresh })),
                None,
            )
            .await;
            status
        }));
    }

    let mut ok = 0;
    let mut unauthorized = 0;
    for h in handles {
        match h.await.unwrap() {
            StatusCode::OK => ok += 1,
            StatusCode::UNAUTHORIZED => unauthorized += 1,
            other => panic!("unexpected status {other}"),
        }
    }

    assert_eq!(
        ok, 1,
        "exactly one concurrent refresh must succeed (got ok={ok}, unauthorized={unauthorized})"
    );
    assert_eq!(unauthorized, 7, "the other 7 must be rejected");
}

#[tokio::test]
async fn test_refresh_with_access_token_returns_401() {
    let (router, _db) = auth_router().await;

    let (access, _refresh, _cookies) =
        register_user(&router, "wrong-type@example.com", "password12345").await;

    // Try refreshing with an access token (wrong token type)
    let (status, body, _cookies) = send(
        &router,
        "POST",
        "/v1/auth/refresh",
        Some(json!({ "refresh_token": access })),
        None,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "access token should not work as refresh: {body}"
    );
}

// ============================================
// Login flow tests
// ============================================

// EVE-452 / TM-AUTH-019: an OAuth-only account (one without a password hash)
// must produce the same generic credential failure as an unknown email or a
// wrong password. Otherwise an attacker can enumerate which addresses are
// registered via OAuth.
#[tokio::test]
async fn test_login_oauth_only_account_returns_generic_error() {
    use everruns_server::storage::models::CreateUserRow;

    let (router, db) = auth_router().await;

    // Insert an OAuth-only account directly: `password_hash = None`.
    db.create_user(CreateUserRow {
        email: "oauth-only@example.com".to_string(),
        name: "OAuth Only".to_string(),
        avatar_url: None,
        roles: vec!["user".to_string()],
        password_hash: None,
        email_verified: true,
        auth_provider: Some("google".to_string()),
        auth_provider_id: Some("google-sub-1".to_string()),
        external_id: None,
    })
    .await
    .expect("failed to seed oauth-only user");

    // Attempt password login on the OAuth-only account.
    let (status_oauth, body_oauth, _) = send(
        &router,
        "POST",
        "/v1/auth/login",
        Some(json!({
            "email": "oauth-only@example.com",
            "password": "anypassword",
        })),
        None,
    )
    .await;

    // Attempt password login on a totally unknown email.
    let (status_unknown, body_unknown, _) = send(
        &router,
        "POST",
        "/v1/auth/login",
        Some(json!({
            "email": "ghost@example.com",
            "password": "anypassword",
        })),
        None,
    )
    .await;

    assert_eq!(status_oauth, StatusCode::UNAUTHORIZED);
    assert_eq!(status_unknown, StatusCode::UNAUTHORIZED);
    assert_eq!(
        body_oauth, body_unknown,
        "OAuth-only and unknown-email login failures must be indistinguishable"
    );
    // Belt-and-suspenders: the legacy distinguishing string is gone.
    let body_text = body_oauth.to_string();
    assert!(
        !body_text.contains("Password login not available"),
        "legacy enumeration message must not appear: {body_text}"
    );
}

#[tokio::test]
async fn test_login_sets_cookies_with_root_path() {
    let (router, _db) = auth_router().await;

    // Register first
    register_user(&router, "login@example.com", "password12345").await;

    // Login
    let (status, body, cookies) = send(
        &router,
        "POST",
        "/v1/auth/login",
        Some(json!({
            "email": "login@example.com",
            "password": "password12345"
        })),
        None,
    )
    .await;

    assert_eq!(status, StatusCode::OK, "login failed: {body}");

    // Verify refresh_token cookie has Path=/
    let refresh_cookie = cookies
        .iter()
        .find(|c| c.starts_with("refresh_token="))
        .expect("refresh_token cookie not set");
    assert!(
        refresh_cookie.contains("Path=/;") || refresh_cookie.ends_with("Path=/"),
        "refresh_token cookie should have Path=/, got: {refresh_cookie}"
    );
}

#[tokio::test]
async fn test_full_flow_register_login_refresh_cookie_refresh() {
    let (router, _db) = auth_router().await;

    // 1. Register
    let (_access, _refresh, _reg_cookies) =
        register_user(&router, "flow@example.com", "password12345").await;

    // 2. Login
    let (status, _body, login_cookies) = send(
        &router,
        "POST",
        "/v1/auth/login",
        Some(json!({
            "email": "flow@example.com",
            "password": "password12345"
        })),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // 3. Refresh via cookie (simulating browser behavior)
    let refresh_val = extract_cookie_value(&login_cookies, "refresh_token")
        .expect("no refresh cookie from login");
    let (status, body, refresh_cookies) = send(
        &router,
        "POST",
        "/v1/auth/refresh",
        None,
        Some(&format!("refresh_token={refresh_val}")),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "cookie refresh failed: {body}");

    // 4. Refresh again with the new cookie (chained rotation)
    let new_refresh_val = extract_cookie_value(&refresh_cookies, "refresh_token")
        .expect("no refresh cookie after refresh");
    let (status2, body2, _) = send(
        &router,
        "POST",
        "/v1/auth/refresh",
        None,
        Some(&format!("refresh_token={new_refresh_val}")),
    )
    .await;
    assert_eq!(
        status2,
        StatusCode::OK,
        "chained cookie refresh failed: {body2}"
    );

    // 5. Old registration refresh token should be revoked already (used during register)
    // Actually register returns a fresh token, and login creates a new one.
    // The register refresh_token in reg_cookies was never used for refresh,
    // but let's verify the old login token is consumed after step 3.
    let (status3, _, _) = send(
        &router,
        "POST",
        "/v1/auth/refresh",
        None,
        Some(&format!("refresh_token={refresh_val}")),
    )
    .await;
    assert_eq!(
        status3,
        StatusCode::UNAUTHORIZED,
        "old login refresh token should be consumed"
    );
}

// ============================================
// Auth config endpoint test
// ============================================

#[tokio::test]
async fn test_auth_config_returns_full_mode() {
    let (router, _db) = auth_router().await;

    let (status, body, _cookies) = send(&router, "GET", "/v1/auth/config", None, None).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["mode"], "full");
    assert_eq!(body["password_auth_enabled"], true);
    assert_eq!(body["signup_enabled"], true);
    assert!(body.get("login_origin").is_none());
}

// ============================================
// Default-org harness-seed safety net (EVE-390)
// ============================================
//
// Invariants under test:
// 1. A user registering before the async seed task provisions harnesses
//    for DEFAULT_ORG_ID still lands in an org that has the built-in
//    harnesses (the safety net fires).
// 2. The safety net drives from the operator-composed built-in harness set
//    (`BuiltinAuthBackend::with_built_in_harnesses`, EVE-881) rather than
//    `oss_built_in_harnesses()`. A custom composition that ships only a
//    single harness must NOT have OSS defaults re-added on signup. This
//    preserves the fix from PR #1462 (TM-AUTH-016).

use everruns_core::{CapabilityRegistry, DEFAULT_ORG_ID, DEFAULT_ORG_PUBLIC_ID};
use everruns_platform::{BuiltInHarnessDefinition, BuiltInHarnessRole};
use everruns_provider::driver_registry::DriverRegistry;
use everruns_server::storage::models::CreateOrganizationRow;

fn single_custom_harness(name: &str) -> BuiltInHarnessDefinition {
    BuiltInHarnessDefinition::new(
        name,
        "Custom",
        "custom harness used in tests",
        "custom system prompt",
    )
    // Mark this sole harness as both Default and Base so the org-settings
    // pointers the provisioner needs can be resolved.
    .with_roles([BuiltInHarnessRole::Default, BuiltInHarnessRole::Base])
}

async fn custom_platform_auth_router(
    built_in_harnesses: Vec<BuiltInHarnessDefinition>,
) -> (Router, Arc<StorageBackend>) {
    let host_composition =
        HostComposition::new(CapabilityRegistry::new(), DriverRegistry::default());
    let db = Arc::new(StorageBackend::in_memory());

    // Deliberately do NOT call `seed::seed_all` — this simulates the cold-boot
    // window before the async seed task has provisioned harnesses for
    // DEFAULT_ORG_ID. Only the org row itself is created so `register` can
    // add the new user to it.
    db.create_organization_with_id(
        DEFAULT_ORG_ID,
        CreateOrganizationRow {
            public_id: DEFAULT_ORG_PUBLIC_ID.to_string(),
            name: "Default Organization".to_string(),
            created_by: None,
        },
    )
    .await
    .expect("create default org");

    let config = AuthConfig {
        mode: AuthMode::Full,
        // These tests exercise the default-org membership + harness safety net,
        // which is now opt-in (single-tenant). Enable it so `register` joins the
        // default org and provisions its harnesses.
        auto_join_default_org: true,
        jwt: JwtConfig {
            secret: "test-secret-for-auth-integration-tests".to_string(),
            access_token_lifetime: Duration::from_secs(900),
            refresh_token_lifetime: Duration::from_secs(86400),
        },
        ..Default::default()
    };

    let backend = BuiltinAuthBackend::new(config, db.clone(), Arc::new(host_composition))
        .with_built_in_harnesses(Arc::new(built_in_harnesses));
    let router = auth::routes(backend);
    (router, db)
}

#[tokio::test]
async fn test_register_safety_net_uses_host_composition_not_oss_defaults() {
    let custom_name = "custom-safety-net-harness";
    let (router, db) = custom_platform_auth_router(vec![single_custom_harness(custom_name)]).await;

    // Pre-condition: no harnesses seeded for DEFAULT_ORG_ID.
    let pre = db
        .list_harnesses(DEFAULT_ORG_ID, None, false)
        .await
        .expect("list harnesses (pre)");
    assert!(pre.is_empty(), "default org must start with no harnesses");

    // Register a fresh user.
    let (status, _body, _cookies) = send(
        &router,
        "POST",
        "/v1/auth/register",
        Some(json!({
            "email": "new-user@example.com",
            "password": "super-secret-password1",
            "name": "New User",
        })),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "register should succeed");

    // Post-condition: default org now has exactly the platform-defined
    // harness. No OSS defaults (base/generic/etc) should have been added.
    let post = db
        .list_harnesses(DEFAULT_ORG_ID, None, false)
        .await
        .expect("list harnesses (post)");
    let names: Vec<String> = post.iter().map(|h| h.name.clone()).collect();
    assert_eq!(
        names,
        vec![custom_name.to_string()],
        "safety net must only provision the platform-defined harness set (got {:?})",
        names
    );
}

#[tokio::test]
async fn test_register_safety_net_is_idempotent_when_seed_already_ran() {
    // When the harness already exists (seed task completed first), registering
    // must not create duplicates — the upsert is idempotent.
    let custom_name = "custom-idempotent-harness";
    let (router, db) = custom_platform_auth_router(vec![single_custom_harness(custom_name)]).await;

    // Pre-provision harnesses to simulate the seed task finishing first.
    everruns_server::org_init::initialize_org_harnesses_with_definitions(
        &db,
        DEFAULT_ORG_ID,
        &[single_custom_harness(custom_name)],
    )
    .await
    .expect("pre-seed");

    let before = db
        .list_harnesses(DEFAULT_ORG_ID, None, false)
        .await
        .expect("list harnesses (before)");
    assert_eq!(before.len(), 1);
    let before_id = before[0].id;

    // Register — safety net must not create a second row.
    let (status, _body, _cookies) = send(
        &router,
        "POST",
        "/v1/auth/register",
        Some(json!({
            "email": "second-user@example.com",
            "password": "another-secret-password1",
            "name": "Second User",
        })),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let after = db
        .list_harnesses(DEFAULT_ORG_ID, None, false)
        .await
        .expect("list harnesses (after)");
    assert_eq!(
        after.len(),
        1,
        "safety net must not duplicate harnesses on pre-seeded orgs"
    );
    assert_eq!(
        after[0].id, before_id,
        "safety net must keep the same harness row identity (idempotent upsert)"
    );
}

// ============================================
// Org cookie persistence across login/refresh
// ============================================

// Helper: ensure default-org membership for the user (the mini auth router
// harness does not run the full register provisioning), then hit /v1/auth/me
// (which sets the org cookie when missing) and return the minted org id.
async fn fetch_org_cookie(router: &Router, db: &Arc<StorageBackend>, access_token: &str) -> String {
    let (status, body, _cookies) = send(
        router,
        "GET",
        "/v1/auth/me",
        None,
        Some(&format!("access_token={access_token}")),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "/v1/auth/me failed: {body}");
    let user_id = uuid::Uuid::parse_str(body["id"].as_str().unwrap()).unwrap();
    db.add_organization_member(everruns_core::DEFAULT_ORG_ID, user_id, "member")
        .await
        .expect("add_organization_member failed");

    let (status, body, cookies) = send(
        router,
        "GET",
        "/v1/auth/me",
        None,
        Some(&format!("access_token={access_token}")),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "/v1/auth/me failed: {body}");
    extract_cookie_value(&cookies, "everruns_org")
        .unwrap_or_else(|| panic!("/v1/auth/me must set the org cookie, body: {body}"))
}

// The `everruns_org` cookie must not be re-minted to the user's first org on
// every token mint: `generate_token_response` runs on login AND on each silent
// refresh (~every access-token lifetime), so unconditional re-minting silently
// reset the selected organization back to the first (alphabetical) org.
#[tokio::test]
async fn test_refresh_preserves_valid_org_cookie() {
    let (router, db) = auth_router().await;
    let (access_token, refresh_token, _cookies) =
        register_user(&router, "orgkeep@example.com", "correct-horse-battery-9").await;
    let org_id = fetch_org_cookie(&router, &db, &access_token).await;

    let (status, _body, cookies) = send(
        &router,
        "POST",
        "/v1/auth/refresh",
        None,
        Some(&format!(
            "refresh_token={refresh_token}; everruns_org={org_id}"
        )),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert!(
        extract_cookie_value(&cookies, "everruns_org").is_none(),
        "refresh must not re-mint a still-valid org selection, got: {cookies:?}"
    );
}

// An org cookie that no longer maps to one of the user's organizations must be
// replaced with a valid one, and the replacement must be persistent (Max-Age)
// so the selection survives a browser restart.
#[tokio::test]
async fn test_refresh_replaces_invalid_org_cookie_with_persistent_one() {
    let (router, db) = auth_router().await;
    let (access_token, refresh_token, _cookies) =
        register_user(&router, "orgreset@example.com", "correct-horse-battery-9").await;
    let valid_org = fetch_org_cookie(&router, &db, &access_token).await;

    let (status, _body, cookies) = send(
        &router,
        "POST",
        "/v1/auth/refresh",
        None,
        Some(&format!(
            "refresh_token={refresh_token}; everruns_org=org_00000000000000000000000000009999"
        )),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        extract_cookie_value(&cookies, "everruns_org").as_deref(),
        Some(valid_org.as_str()),
        "an org cookie outside the user's memberships must be replaced"
    );
    let org_header = cookies
        .iter()
        .find(|h| h.starts_with("everruns_org="))
        .unwrap();
    assert!(
        org_header.contains("Max-Age="),
        "org cookie must persist across browser restarts, got: {org_header}"
    );
}

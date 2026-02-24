//! Auth integration tests for refresh token flows
//!
//! Tests the refresh token endpoint with both JSON body and HttpOnly cookie sources,
//! verifying the fix for cookie path (/ vs /v1/auth) and the new cookie-based refresh flow.
//!
//! Run with: cargo test -p everruns-server --test auth_integration_test

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
    seed::seed_all(&db, grade).await.expect("seed failed");

    let config = AuthConfig {
        mode: AuthMode::Full,
        jwt: JwtConfig {
            secret: "test-secret-for-auth-integration-tests".to_string(),
            access_token_lifetime: Duration::from_secs(900),
            refresh_token_lifetime: Duration::from_secs(86400),
        },
        ..Default::default()
    };

    let backend = BuiltinAuthBackend::new(config, db.clone());
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

// --------------------------------------------
// Positive path tests
// --------------------------------------------

#[tokio::test]
async fn test_refresh_via_json_body() {
    let (router, _db) = auth_router().await;

    let (_access, refresh, _cookies) =
        register_user(&router, "body@example.com", "password123").await;

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
        register_user(&router, "cookie@example.com", "password123").await;

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
    let (_a1, refresh1, _c1) = register_user(&router, "user1@example.com", "password123").await;
    let (_a2, refresh2, _c2) = register_user(&router, "user2@example.com", "password123").await;

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
        register_user(&router, "path@example.com", "password123").await;

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

// --------------------------------------------
// Negative path tests
// --------------------------------------------

#[tokio::test]
async fn test_refresh_no_token_returns_401() {
    let (router, _db) = auth_router().await;

    // No body, no cookie
    let (status, body, _cookies) = send(&router, "POST", "/v1/auth/refresh", None, None).await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(
        body["error"]
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
        body["error"]
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
        body["error"]
            .as_str()
            .unwrap()
            .contains("Invalid refresh token")
    );
}

#[tokio::test]
async fn test_refresh_reuse_revoked_token_returns_401() {
    let (router, _db) = auth_router().await;

    let (_access, refresh, _cookies) =
        register_user(&router, "revoke@example.com", "password123").await;

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

#[tokio::test]
async fn test_refresh_with_access_token_returns_401() {
    let (router, _db) = auth_router().await;

    let (access, _refresh, _cookies) =
        register_user(&router, "wrong-type@example.com", "password123").await;

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

// --------------------------------------------
// Login flow tests
// --------------------------------------------

#[tokio::test]
async fn test_login_sets_cookies_with_root_path() {
    let (router, _db) = auth_router().await;

    // Register first
    register_user(&router, "login@example.com", "password123").await;

    // Login
    let (status, body, cookies) = send(
        &router,
        "POST",
        "/v1/auth/login",
        Some(json!({
            "email": "login@example.com",
            "password": "password123"
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
        register_user(&router, "flow@example.com", "password123").await;

    // 2. Login
    let (status, _body, login_cookies) = send(
        &router,
        "POST",
        "/v1/auth/login",
        Some(json!({
            "email": "flow@example.com",
            "password": "password123"
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

// --------------------------------------------
// Auth config endpoint test
// --------------------------------------------

#[tokio::test]
async fn test_auth_config_returns_full_mode() {
    let (router, _db) = auth_router().await;

    let (status, body, _cookies) = send(&router, "GET", "/v1/auth/config", None, None).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["mode"], "full");
    assert_eq!(body["password_auth_enabled"], true);
    assert_eq!(body["signup_enabled"], true);
}

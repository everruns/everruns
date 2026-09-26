//! Integration tests for CLI authentication endpoints.
//!
//! Runs entirely in-process against the `cli_auth` router (and, where a
//! logged-in user is needed, the shared `auth::routes` router) backed by an
//! in-memory `StorageBackend` — no TCP listener, no external server.
//!
//! Covers:
//! - POST /v1/auth/cli/start creates a pending session and returns a URL
//! - POST /v1/auth/cli/start issues a unique session/state each call
//! - GET /v1/auth/cli/callback rejects an unknown/invalid `state`
//! - POST /v1/auth/cli/exchange rejects an unknown/invalid `code`
//! - GET /cli/login-success renders the branded static success page
//!
//! Run with: cargo test -p everruns-server --test domain cli_auth_test:: -- --test-threads=1

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

use everruns_server::auth::cli_auth::{CliAuthState, cli_auth_public_routes, cli_auth_routes};
use everruns_server::auth::config::{AuthConfig, AuthMode, JwtConfig};
use everruns_server::auth::{self, AuthState, BuiltinAuthBackend};
use everruns_server::seed;
use everruns_server::storage::StorageBackend;

const FRONTEND_URL: &str = "http://localhost:3000";
const BASE_URL: &str = "http://localhost:9000/api";

/// Build the CLI auth router (protected `/v1/auth/cli/*` + public
/// `/cli/login-success`) merged with the standard `/v1/auth/*` router, so
/// tests can both drive CLI auth and log a real user in for it.
async fn build_router() -> (Router, Arc<StorageBackend>) {
    let db = Arc::new(StorageBackend::in_memory());
    let grade = everruns_core::DeploymentGrade::from_env();
    seed::seed_all(&db, grade, &seed::SeedAuthContext::default())
        .await
        .expect("seed failed");

    let config = AuthConfig {
        mode: AuthMode::Full,
        jwt: JwtConfig {
            secret: "test-secret-cli-auth-domain".to_string(),
            access_token_lifetime: Duration::from_secs(900),
            refresh_token_lifetime: Duration::from_secs(86400),
        },
        ..Default::default()
    };

    let host_composition = Arc::new(everruns_server::platform::oss_host_composition());
    let backend = BuiltinAuthBackend::new(config.clone(), db.clone(), host_composition);
    let auth_state = AuthState::new(config, Arc::new(backend.clone()));

    let cli_state = CliAuthState {
        db: db.clone(),
        auth: auth_state,
        frontend_url: FRONTEND_URL.to_string(),
        login_origin: None,
        base_url: BASE_URL.to_string(),
    };

    let router = auth::routes(backend)
        .merge(cli_auth_routes(cli_state.clone()))
        .merge(cli_auth_public_routes(cli_state));

    (router, db)
}

/// Register a user via the real `/v1/auth/register` endpoint and return the
/// bearer access token, so protected CLI routes (e.g. the callback) can be
/// exercised as a real logged-in user rather than faked.
async fn register_and_get_access_token(router: &Router, email: &str) -> String {
    let request = Request::builder()
        .method("POST")
        .uri("/v1/auth/register")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_string(&json!({
                "email": email,
                "password": "password12345",
                "name": "CLI Test User",
            }))
            .unwrap(),
        ))
        .unwrap();

    let response = router.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    assert_eq!(status, StatusCode::CREATED, "register failed: {body}");
    body["access_token"]
        .as_str()
        .expect("register response must include access_token")
        .to_string()
}

#[tokio::test]
async fn test_cli_auth_start() {
    let (router, _db) = build_router().await;

    let request = Request::builder()
        .method("POST")
        .uri("/v1/auth/cli/start")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_string(&json!({"redirect_port": 12345})).unwrap(),
        ))
        .unwrap();

    let response = router.oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body: Value = serde_json::from_slice(&bytes).unwrap();

    assert_eq!(status, StatusCode::OK, "cli/start should succeed: {body}");
    assert!(body["auth_url"].is_string(), "response must have auth_url");
    assert!(body["state"].is_string(), "response must have state");

    let auth_url = body["auth_url"].as_str().unwrap();
    assert!(
        auth_url.contains("/login"),
        "auth_url should point to login page: {auth_url}"
    );

    // Unified auth resume contract: CLI auth uses `return_to`, the single
    // public login-page parameter. `redirect_to` must not appear.
    assert!(
        auth_url.contains("return_to="),
        "auth_url must use return_to: {auth_url}"
    );
    assert!(
        !auth_url.contains("redirect_to"),
        "auth_url must not use legacy redirect_to: {auth_url}"
    );

    // The return_to value must be a relative path (no scheme leaked) so the
    // login page stays same-origin.
    let return_to_start = auth_url.find("return_to=").unwrap() + "return_to=".len();
    let return_to = &auth_url[return_to_start..];
    assert!(
        return_to.starts_with("%2F") || return_to.starts_with('/'),
        "return_to must be a relative path: {return_to}"
    );
    assert!(
        !return_to.contains("%3A%2F%2F") && !return_to.contains("://"),
        "return_to must not embed a full URL: {return_to}"
    );

    let state = body["state"].as_str().unwrap();
    assert_eq!(state.len(), 32, "state should be 32 hex chars");
    assert!(
        state.chars().all(|c| c.is_ascii_hexdigit()),
        "state should be hex"
    );
}

#[tokio::test]
async fn test_cli_auth_start_creates_unique_sessions() {
    let (router, _db) = build_router().await;

    let start = |port: u16, router: Router| async move {
        let request = Request::builder()
            .method("POST")
            .uri("/v1/auth/cli/start")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_string(&json!({"redirect_port": port})).unwrap(),
            ))
            .unwrap();
        let response = router.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        serde_json::from_slice::<Value>(&bytes).unwrap()
    };

    let body1 = start(10001, router.clone()).await;
    let body2 = start(10002, router).await;

    assert_ne!(
        body1["state"], body2["state"],
        "each session should have a unique state"
    );
}

#[tokio::test]
async fn test_cli_callback_invalid_state() {
    let (router, _db) = build_router().await;

    // Authenticate a real user so the callback reaches state validation
    // rather than failing earlier on the `AuthUser` extractor.
    let access_token =
        register_and_get_access_token(&router, "cli-callback-user@example.com").await;

    let request = Request::builder()
        .method("GET")
        .uri("/v1/auth/cli/callback?state=nonexistent_state_12345")
        .header("authorization", format!("Bearer {access_token}"))
        .body(Body::empty())
        .unwrap();

    let response = router.oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);

    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "callback with an unknown state must be rejected: {status} {body}"
    );
}

#[tokio::test]
async fn test_cli_callback_requires_authentication() {
    // Without any credentials, the `AuthUser` extractor rejects the request
    // before state is even looked at.
    let (router, _db) = build_router().await;

    let request = Request::builder()
        .method("GET")
        .uri("/v1/auth/cli/callback?state=nonexistent_state_12345")
        .body(Body::empty())
        .unwrap();

    let response = router.oneshot(request).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "callback without auth must be rejected"
    );
}

#[tokio::test]
async fn test_cli_exchange_invalid_code() {
    let (router, _db) = build_router().await;

    let request = Request::builder()
        .method("POST")
        .uri("/v1/auth/cli/exchange")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_string(&json!({
                "code": "invalid_code_that_does_not_exist",
                "hostname": "test-machine",
                "os": "linux",
            }))
            .unwrap(),
        ))
        .unwrap();

    let response = router.oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);

    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "exchange with an invalid code should return 401: {body}"
    );
}

#[tokio::test]
async fn test_cli_login_success_page() {
    let (router, _db) = build_router().await;

    let request = Request::builder()
        .method("GET")
        .uri("/cli/login-success")
        .body(Body::empty())
        .unwrap();

    let response = router.oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body = String::from_utf8(bytes.to_vec()).unwrap();

    assert_eq!(status, StatusCode::OK, "login-success page should be 200");
    assert!(
        body.contains("You're logged in"),
        "success page should contain login confirmation"
    );
    assert!(
        body.contains("terminal"),
        "success page should mention returning to terminal"
    );
    assert!(body.contains("<!DOCTYPE html>"), "should be HTML");
}

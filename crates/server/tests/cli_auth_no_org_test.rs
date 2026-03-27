//! Test: CLI exchange returns 422 when user has no organizations.
//!
//! Verifies the fix for EVE-195 — CLI login must not silently fall back to
//! DEFAULT_ORG_ID when the user has zero orgs.
//!
//! Run with: cargo test -p everruns-server --test cli_auth_no_org_test

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use chrono::{Duration as ChronoDuration, Utc};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

use everruns_server::auth::cli_auth::{CliAuthState, cli_auth_routes};
use everruns_server::auth::config::{AuthConfig, AuthMode, JwtConfig};
use everruns_server::auth::{AuthState, BuiltinAuthBackend};
use everruns_server::seed;
use everruns_server::storage::{CreateCliAuthSessionRow, CreateUserRow, StorageBackend};

#[tokio::test]
async fn test_cli_exchange_no_orgs_returns_422() {
    // 1. Set up in-memory DB and auth router
    let db = Arc::new(StorageBackend::in_memory());
    let grade = everruns_core::DeploymentGrade::from_env();
    seed::seed_all(&db, grade, &seed::SeedAuthContext::default())
        .await
        .expect("seed failed");

    let config = AuthConfig {
        mode: AuthMode::Full,
        jwt: JwtConfig {
            secret: "test-secret-cli-no-org".to_string(),
            access_token_lifetime: Duration::from_secs(900),
            refresh_token_lifetime: Duration::from_secs(86400),
        },
        ..Default::default()
    };

    let backend = BuiltinAuthBackend::new(config.clone(), db.clone());
    let auth_state = AuthState::new(config, Arc::new(backend));
    let cli_state = CliAuthState {
        db: db.clone(),
        auth: auth_state,
        frontend_url: "http://localhost:3000".to_string(),
        base_url: "http://localhost:9000".to_string(),
    };
    let router = cli_auth_routes(cli_state);

    // 2. Create a user directly in DB — no org membership
    let user = db
        .create_user(CreateUserRow {
            email: "no-org-user@example.com".to_string(),
            name: "No Org User".to_string(),
            avatar_url: None,
            roles: vec![],
            password_hash: Some("unused".to_string()),
            email_verified: true,
            auth_provider: None,
            auth_provider_id: None,
            external_id: None,
        })
        .await
        .expect("create_user failed");

    // 3. Create a CLI auth session and complete it with this user
    let exchange_code = "test-exchange-code-no-org";
    let session = db
        .create_cli_auth_session(CreateCliAuthSessionRow {
            state: "test-state-no-org".to_string(),
            exchange_code: exchange_code.to_string(),
            redirect_port: 12345,
            expires_at: Utc::now() + ChronoDuration::seconds(300),
        })
        .await
        .expect("create session failed");

    db.complete_cli_auth_session(session.id, user.id)
        .await
        .expect("complete session failed");

    // 4. POST /v1/auth/cli/exchange — should return 422
    let request = Request::builder()
        .method("POST")
        .uri("/v1/auth/cli/exchange")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_string(&json!({
                "code": exchange_code,
                "hostname": "test-machine",
                "os": "linux"
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
        StatusCode::UNPROCESSABLE_ENTITY,
        "Expected 422 for user with no orgs, got {status}: {body}"
    );
    assert!(
        body["error"]
            .as_str()
            .unwrap_or("")
            .contains("create an organization"),
        "Error should tell user to create an org: {body}"
    );
}

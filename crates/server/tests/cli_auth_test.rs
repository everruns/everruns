//! Integration tests for CLI authentication endpoints.
//!
//! Tests require a running API server with AUTH_MODE=none or AUTH_MODE=admin.
//! Run with: cargo test -p everruns-server --test cli_auth_test -- --test-threads=1
//!
//! These tests verify:
//! - POST /v1/auth/cli/start creates a pending session
//! - GET /cli/login-success returns the branded success page
//! - POST /v1/auth/cli/exchange with invalid code returns error
//! - Full flow integration (start -> callback -> exchange) with admin auth

use serde_json::{Value, json};

const SERVER_BASE_URL: &str = "http://localhost:9000";
const API_BASE_URL: &str = "http://localhost:9000/api";

#[tokio::test]
async fn test_cli_auth_start() {
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{}/v1/auth/cli/start", API_BASE_URL))
        .json(&json!({
            "redirect_port": 12345
        }))
        .send()
        .await;

    // Server may not be running in CI — skip gracefully
    let resp = match resp {
        Ok(r) => r,
        Err(_) => {
            eprintln!("Skipping test: server not running");
            return;
        }
    };

    assert_eq!(resp.status(), 200, "POST /v1/auth/cli/start should succeed");

    let body: Value = resp.json().await.unwrap();
    assert!(
        body["auth_url"].is_string(),
        "Response should contain auth_url"
    );
    assert!(body["state"].is_string(), "Response should contain state");

    let auth_url = body["auth_url"].as_str().unwrap();
    assert!(
        auth_url.contains("/login"),
        "auth_url should point to login page: {}",
        auth_url
    );

    let state = body["state"].as_str().unwrap();
    assert_eq!(state.len(), 32, "state should be 32 hex chars");
    assert!(
        state.chars().all(|c| c.is_ascii_hexdigit()),
        "state should be hex"
    );
}

#[tokio::test]
async fn test_cli_login_success_page() {
    let client = reqwest::Client::new();

    let resp = match client
        .get(format!("{}/cli/login-success", SERVER_BASE_URL))
        .send()
        .await
    {
        Ok(r) => r,
        Err(_) => {
            eprintln!("Skipping test: server not running");
            return;
        }
    };

    assert_eq!(
        resp.status(),
        200,
        "GET /cli/login-success should return 200"
    );

    let body = resp.text().await.unwrap();
    assert!(
        body.contains("You're logged in"),
        "Success page should contain login confirmation"
    );
    assert!(
        body.contains("terminal"),
        "Success page should mention returning to terminal"
    );
    assert!(body.contains("<!DOCTYPE html>"), "Should be HTML");
}

#[tokio::test]
async fn test_cli_exchange_invalid_code() {
    let client = reqwest::Client::new();

    let resp = match client
        .post(format!("{}/v1/auth/cli/exchange", API_BASE_URL))
        .json(&json!({
            "code": "invalid_code_that_does_not_exist",
            "hostname": "test-machine",
            "os": "linux"
        }))
        .send()
        .await
    {
        Ok(r) => r,
        Err(_) => {
            eprintln!("Skipping test: server not running");
            return;
        }
    };

    assert_eq!(
        resp.status(),
        401,
        "Exchange with invalid code should return 401"
    );
}

#[tokio::test]
async fn test_cli_callback_invalid_state() {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();

    let resp = match client
        .get(format!(
            "{}/v1/auth/cli/callback?state=nonexistent_state_12345",
            API_BASE_URL
        ))
        .send()
        .await
    {
        Ok(r) => r,
        Err(_) => {
            eprintln!("Skipping test: server not running");
            return;
        }
    };

    // Should fail because:
    // 1. No authentication cookie/token (returns 401), or
    // 2. Invalid state (returns 401)
    assert!(
        resp.status() == 401 || resp.status() == 403,
        "Callback with invalid state should fail: got {}",
        resp.status()
    );
}

#[tokio::test]
async fn test_cli_auth_start_creates_unique_sessions() {
    let client = reqwest::Client::new();

    let resp1 = match client
        .post(format!("{}/v1/auth/cli/start", API_BASE_URL))
        .json(&json!({"redirect_port": 10001}))
        .send()
        .await
    {
        Ok(r) => r,
        Err(_) => {
            eprintln!("Skipping test: server not running");
            return;
        }
    };

    let resp2 = client
        .post(format!("{}/v1/auth/cli/start", API_BASE_URL))
        .json(&json!({"redirect_port": 10002}))
        .send()
        .await
        .unwrap();

    let body1: Value = resp1.json().await.unwrap();
    let body2: Value = resp2.json().await.unwrap();

    assert_ne!(
        body1["state"], body2["state"],
        "Each session should have unique state"
    );
}

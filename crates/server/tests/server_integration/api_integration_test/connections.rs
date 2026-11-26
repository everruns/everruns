//! API integration tests: user connections.

use crate::test_harness;
use axum::http::StatusCode;
use serde_json::{Value, json};
use test_harness::TestServer;

#[tokio::test]
async fn test_verify_connection_no_connection_returns_404() {
    let server = TestServer::in_memory().await;

    server
        .post(
            "/v1/platform/user/connections/nonexistent/verify",
            json!({}),
        )
        .await
        .assert_status(StatusCode::NOT_FOUND);
}

// ============================================================================
// Virtual User Connection tests
// ============================================================================

#[tokio::test]
async fn test_identity_connections_not_found_for_missing_identity() {
    let server = TestServer::in_memory().await;

    server
        .get("/v1/platform/virtual-users/identity_019d166cd0147e638c72892ecb30ffff/connections")
        .await
        .assert_status(StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_identity_connections_delete_not_found() {
    let server = TestServer::in_memory().await;

    let identity: Value = server
        .post("/v1/platform/virtual-users", json!({"name": "ConnDel"}))
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let id = identity["id"].as_str().unwrap();

    server
        .delete(&format!(
            "/v1/platform/virtual-users/{id}/connections/nonexistent"
        ))
        .await
        .assert_status(StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_identity_connections_create_unknown_provider() {
    let server = TestServer::in_memory().await;

    let identity: Value = server
        .post("/v1/platform/virtual-users", json!({"name": "ConnProv"}))
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let id = identity["id"].as_str().unwrap();

    server
        .post(
            &format!("/v1/platform/virtual-users/{id}/connections/nonexistent"),
            json!({"api_key": "test"}),
        )
        .await
        .assert_status(StatusCode::NOT_FOUND);
}

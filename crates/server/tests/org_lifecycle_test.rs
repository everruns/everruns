//! Organization lifecycle integration tests
//!
//! Verifies that creating an organization makes it immediately visible
//! in the org list and switchable. Regression tests for the bug where
//! list_organizations and switch_org read from stale auth context instead
//! of querying the database.
//!
//! Uses in-memory backend (no PostgreSQL required).
//!
//! Run with: cargo test -p everruns-server --test org_lifecycle_test

mod test_harness;

use axum::http::StatusCode;
use axum::http::header::SET_COOKIE;
use serde_json::{Value, json};
use test_harness::TestServer;

// ============================================
// Bug regression: created org must appear in list
// ============================================

#[tokio::test]
async fn test_created_org_appears_in_list() {
    let server = TestServer::in_memory().await;

    // List orgs before — should have only the default org
    let before: Value = server
        .get("/v1/orgs")
        .await
        .assert_status(StatusCode::OK)
        .json();
    let before_items = before["data"].as_array().expect("data array");
    assert_eq!(before_items.len(), 1, "should start with one default org");

    // Create a new org
    let created: Value = server
        .post("/v1/orgs", json!({"name": "Acme Corp"}))
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let new_org_id = created["id"].as_str().expect("org id");
    assert_eq!(created["name"], "Acme Corp");

    // List orgs after — new org must be present
    let after: Value = server
        .get("/v1/orgs")
        .await
        .assert_status(StatusCode::OK)
        .json();
    let after_items = after["data"].as_array().expect("data array");
    assert_eq!(after_items.len(), 2, "should now have two orgs");

    let org_ids: Vec<&str> = after_items
        .iter()
        .map(|o| o["id"].as_str().unwrap())
        .collect();
    assert!(
        org_ids.contains(&new_org_id),
        "newly created org must appear in list"
    );
}

// ============================================
// Bug regression: can switch to created org
// ============================================

#[tokio::test]
async fn test_can_switch_to_created_org() {
    let server = TestServer::in_memory().await;

    // Create a new org
    let created: Value = server
        .post("/v1/orgs", json!({"name": "Switch Target"}))
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let new_org_id = created["id"].as_str().expect("org id");

    // Switch to new org — must succeed
    let switch_resp: Value = server
        .post("/v1/users/me/switch-org", json!({"org_id": new_org_id}))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(switch_resp["success"], true);
    assert_eq!(switch_resp["org_id"], new_org_id);
}

// ============================================
// Multiple orgs visible after sequential creation
// ============================================

#[tokio::test]
async fn test_multiple_created_orgs_all_visible() {
    let server = TestServer::in_memory().await;

    let mut created_ids = Vec::new();
    for name in ["Alpha", "Beta", "Gamma"] {
        let resp: Value = server
            .post("/v1/orgs", json!({"name": name}))
            .await
            .assert_status(StatusCode::CREATED)
            .json();
        created_ids.push(resp["id"].as_str().unwrap().to_string());
    }

    let list: Value = server
        .get("/v1/orgs")
        .await
        .assert_status(StatusCode::OK)
        .json();
    let items = list["data"].as_array().unwrap();
    // 1 default + 3 created
    assert_eq!(items.len(), 4);

    let listed_ids: Vec<&str> = items.iter().map(|o| o["id"].as_str().unwrap()).collect();
    for id in &created_ids {
        assert!(listed_ids.contains(&id.as_str()), "org {id} must be listed");
    }
}

// ============================================
// Bug regression: /v1/auth/me must return newly created org (auth=none)
// ============================================

#[tokio::test]
async fn test_auth_me_includes_newly_created_org() {
    let server = TestServer::in_memory().await;

    // Before creation, /v1/auth/me should show at least the default org
    let me_before: Value = server
        .get("/v1/auth/me")
        .await
        .assert_status(StatusCode::OK)
        .json();
    let orgs_before = me_before["organizations"]
        .as_array()
        .expect("organizations array");
    assert_eq!(orgs_before.len(), 1, "should start with one default org");

    // Create a new org
    let created: Value = server
        .post("/v1/orgs", json!({"name": "AuthMe Corp"}))
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let new_org_id = created["id"].as_str().expect("org id");

    // After creation, /v1/auth/me must include the new org
    let me_after: Value = server
        .get("/v1/auth/me")
        .await
        .assert_status(StatusCode::OK)
        .json();
    let orgs_after = me_after["organizations"]
        .as_array()
        .expect("organizations array");
    assert_eq!(
        orgs_after.len(),
        2,
        "auth/me should now show two orgs (default + new)"
    );

    let org_ids: Vec<&str> = orgs_after
        .iter()
        .map(|o| o["public_id"].as_str().unwrap())
        .collect();
    assert!(
        org_ids.contains(&new_org_id),
        "newly created org must appear in /v1/auth/me organizations"
    );
}

// ============================================
// Validation: empty name rejected
// ============================================

#[tokio::test]
async fn test_create_org_empty_name_rejected() {
    let server = TestServer::in_memory().await;

    server
        .post("/v1/orgs", json!({"name": ""}))
        .await
        .assert_status(StatusCode::BAD_REQUEST);
}

// ============================================
// Validation: name too long rejected
// ============================================

#[tokio::test]
async fn test_create_org_long_name_rejected() {
    let server = TestServer::in_memory().await;

    let long_name = "x".repeat(256);
    server
        .post("/v1/orgs", json!({"name": long_name}))
        .await
        .assert_status(StatusCode::BAD_REQUEST);
}

// ============================================
// Switch to non-member org rejected
// ============================================

#[tokio::test]
async fn test_switch_to_unknown_org_rejected() {
    let server = TestServer::in_memory().await;

    server
        .post(
            "/v1/users/me/switch-org",
            json!({"org_id": "org_ffffffffffffffffffffffffffffffff"}),
        )
        .await
        .assert_status(StatusCode::NOT_FOUND);
}

// ============================================
// Switch with invalid org ID format rejected
// ============================================

#[tokio::test]
async fn test_switch_invalid_org_id_format() {
    let server = TestServer::in_memory().await;

    server
        .post(
            "/v1/users/me/switch-org",
            json!({"org_id": "not-an-org-id"}),
        )
        .await
        .assert_status(StatusCode::BAD_REQUEST);
}

// ============================================
// Created org has correct settings in response
// ============================================

#[tokio::test]
async fn test_created_org_response_fields() {
    let server = TestServer::in_memory().await;

    let resp: Value = server
        .post("/v1/orgs", json!({"name": "Fields Test"}))
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    assert_eq!(resp["name"], "Fields Test");
    assert!(resp["id"].as_str().unwrap().starts_with("org_"));
    assert!(resp["created_at"].is_string());
    assert!(resp["updated_at"].is_string());
}

// ============================================
// Created org visible via GET /v1/orgs/:id
// ============================================

#[tokio::test]
async fn test_get_created_org_by_id() {
    let server = TestServer::in_memory().await;

    let created: Value = server
        .post("/v1/orgs", json!({"name": "Get By ID"}))
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let org_id = created["id"].as_str().unwrap();

    let fetched: Value = server
        .get(&format!("/v1/orgs/{org_id}"))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(fetched["name"], "Get By ID");
    assert_eq!(fetched["id"], org_id);
}

// ============================================
// Bug regression: switch-org cookie must have Secure flag
// ============================================

#[tokio::test]
async fn test_switch_org_cookie_has_secure_flag() {
    let server = TestServer::in_memory().await;

    // Create a new org to switch to
    let created: Value = server
        .post("/v1/orgs", json!({"name": "Secure Cookie Test"}))
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let new_org_id = created["id"].as_str().expect("org id");

    // Switch to new org and inspect Set-Cookie header
    let resp = server
        .post("/v1/users/me/switch-org", json!({"org_id": new_org_id}))
        .await
        .assert_status(StatusCode::OK);

    let set_cookie_values: Vec<&str> = resp
        .headers
        .get_all(SET_COOKIE)
        .iter()
        .map(|v| v.to_str().unwrap())
        .collect();

    let org_cookie = set_cookie_values
        .iter()
        .find(|c| c.contains("everruns_org="))
        .expect("everruns_org cookie must be set");

    assert!(
        org_cookie.contains("Secure"),
        "everruns_org cookie must have Secure flag, got: {org_cookie}"
    );
}

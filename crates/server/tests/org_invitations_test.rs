//! OSS-owned organization invitation HTTP integration tests (EVE-602).
//!
//! Exercises the create/list/revoke routes and token-validation error paths
//! through the assembled router. The core invite/accept logic (email-delivery
//! classification, expiry, wrong-email, role escalation) is unit-tested in
//! `api::org_invitations`; these tests cover routing, extractors, auth wiring,
//! and serialization.
//!
//! Uses in-memory backend (no PostgreSQL required). In the no-auth test harness
//! the caller is the anonymous owner of the default org, and no email provider
//! is configured, so creation reports `not_configured`.
//!
//! Run with: cargo test -p everruns-server --test org_invitations_test

mod test_harness;

use axum::http::StatusCode;
use serde_json::{Value, json};
use test_harness::TestServer;

const DEFAULT_ORG: &str = "org_00000000000000000000000000000001";

#[tokio::test]
async fn create_list_revoke_invite_flow() {
    let server = TestServer::in_memory().await;

    // Create an invite for a brand-new email.
    let created: Value = server
        .post(
            &format!("/v1/orgs/{DEFAULT_ORG}/invites"),
            json!({ "email": "Newperson@Example.com", "role": "admin" }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    assert_eq!(created["email"], "newperson@example.com");
    assert_eq!(created["role"], "admin");
    assert_eq!(created["status"], "pending");
    // No email provider configured in tests -> copy-link UX.
    assert_eq!(created["email_delivery"], "not_configured");
    let invite_url = created["invite_url"].as_str().expect("invite_url");
    assert!(
        invite_url.contains("/invite/evrinv_"),
        "unexpected invite_url: {invite_url}"
    );
    let invite_id = created["id"].as_str().expect("invite id").to_string();

    // It shows up in the pending list.
    let list: Value = server
        .get(&format!("/v1/orgs/{DEFAULT_ORG}/invites"))
        .await
        .assert_status(StatusCode::OK)
        .json();
    let items = list["data"].as_array().expect("data array");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["id"], invite_id);
    // The raw token / invite_url is never re-exposed after creation.
    assert!(items[0].get("invite_url").is_none());

    // Revoke it.
    server
        .delete(&format!("/v1/orgs/{DEFAULT_ORG}/invites/{invite_id}"))
        .await
        .assert_status(StatusCode::NO_CONTENT);

    // Pending list is empty again.
    let list_after: Value = server
        .get(&format!("/v1/orgs/{DEFAULT_ORG}/invites"))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(list_after["data"].as_array().expect("data").len(), 0);

    // Revoking a missing invite is a 404.
    server
        .delete(&format!("/v1/orgs/{DEFAULT_ORG}/invites/{invite_id}"))
        .await
        .assert_status(StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn create_invite_rejects_invalid_email() {
    let server = TestServer::in_memory().await;
    let resp = server
        .post(
            &format!("/v1/orgs/{DEFAULT_ORG}/invites"),
            json!({ "email": "not-an-email" }),
        )
        .await
        .assert_status(StatusCode::BAD_REQUEST);
    let body: Value = resp.json();
    assert_eq!(body["code"], "invalid_email");
}

#[tokio::test]
async fn accept_invalid_token_returns_not_found() {
    let server = TestServer::in_memory().await;
    let resp = server
        .post("/v1/invites/evrinv_deadbeef/accept", json!({}))
        .await
        .assert_status(StatusCode::NOT_FOUND);
    let body: Value = resp.json();
    assert_eq!(body["code"], "invite_invalid");
}

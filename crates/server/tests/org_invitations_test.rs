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
use chrono::{Duration, Utc};
use everruns_platform::{ANONYMOUS_USER_EMAIL, ANONYMOUS_USER_ID};
use everruns_server::storage::{CreateOrgInvitation, CreateOrganizationRow};
use serde_json::{Value, json};
use test_harness::TestServer;

const DEFAULT_ORG: &str = "org_00000000000000000000000000000001";

#[tokio::test]
async fn current_user_can_discover_and_accept_pending_invitation() {
    let server = TestServer::in_memory().await;

    let empty: Value = server
        .get("/v1/invites/pending")
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert!(empty["data"].as_array().expect("data").is_empty());

    let org = server
        .db
        .create_organization(CreateOrganizationRow {
            public_id: "org_00000000000000000000000000000983".to_string(),
            name: "Inviting Organization".to_string(),
            created_by: None,
        })
        .await
        .expect("create inviting organization");
    let active = server
        .db
        .create_org_invitation(CreateOrgInvitation {
            public_id: "orginv_00000000000000000000000000000983".to_string(),
            org_id: org.org_id,
            email: ANONYMOUS_USER_EMAIL.to_string(),
            role: "member".to_string(),
            invited_by: ANONYMOUS_USER_ID,
            token_hash: "active-pending-token-hash".to_string(),
            expires_at: Utc::now() + Duration::days(1),
        })
        .await
        .expect("create active invitation");
    server
        .db
        .create_org_invitation(CreateOrgInvitation {
            public_id: "orginv_00000000000000000000000000000982".to_string(),
            org_id: org.org_id,
            email: ANONYMOUS_USER_EMAIL.to_string(),
            role: "admin".to_string(),
            invited_by: ANONYMOUS_USER_ID,
            token_hash: "expired-pending-token-hash".to_string(),
            expires_at: Utc::now() - Duration::days(1),
        })
        .await
        .expect("create expired invitation");

    let pending: Value = server
        .get("/v1/invites/pending")
        .await
        .assert_status(StatusCode::OK)
        .json();
    let invitations = pending["data"].as_array().expect("data");
    assert_eq!(invitations.len(), 1);
    assert_eq!(invitations[0]["id"], active.public_id);
    assert_eq!(invitations[0]["org_id"], org.public_id);
    assert_eq!(invitations[0]["org_name"], "Inviting Organization");
    assert_eq!(invitations[0]["role"], "member");

    let accepted: Value = server
        .post(
            &format!("/v1/invites/pending/{}/accept", active.public_id),
            json!({}),
        )
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(accepted["org_id"], org.public_id);
    assert_eq!(accepted["role"], "member");
    assert!(
        server
            .db
            .is_organization_member(org.org_id, ANONYMOUS_USER_ID)
            .await
            .expect("membership lookup")
    );

    let after_accept: Value = server
        .get("/v1/invites/pending")
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert!(after_accept["data"].as_array().expect("data").is_empty());
}

#[tokio::test]
async fn pending_invitation_acceptance_rejects_a_different_email() {
    let server = TestServer::in_memory().await;
    let org = server
        .db
        .create_organization(CreateOrganizationRow {
            public_id: "org_00000000000000000000000000000984".to_string(),
            name: "Private Organization".to_string(),
            created_by: None,
        })
        .await
        .expect("create organization");
    let invitation = server
        .db
        .create_org_invitation(CreateOrgInvitation {
            public_id: "orginv_00000000000000000000000000000984".to_string(),
            org_id: org.org_id,
            email: "someone-else@example.com".to_string(),
            role: "member".to_string(),
            invited_by: ANONYMOUS_USER_ID,
            token_hash: "different-email-token-hash".to_string(),
            expires_at: Utc::now() + Duration::days(1),
        })
        .await
        .expect("create invitation");

    let response = server
        .post(
            &format!("/v1/invites/pending/{}/accept", invitation.public_id),
            json!({}),
        )
        .await
        .assert_status(StatusCode::FORBIDDEN);
    let body: Value = response.json();
    assert_eq!(body["code"], "invite_email_mismatch");
    assert!(
        !server
            .db
            .is_organization_member(org.org_id, ANONYMOUS_USER_ID)
            .await
            .expect("membership lookup")
    );
}

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

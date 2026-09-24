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
//! Run with: cargo test -p everruns-server --test domain org_invitations_test::

use crate::test_harness;

use axum::http::StatusCode;
use chrono::{Duration, Utc};
use everruns_platform::{ANONYMOUS_USER_EMAIL, ANONYMOUS_USER_ID};
use everruns_server::storage::{
    CreateOrgInvitation, CreateOrganizationRow, OrgInvitationRow, UpdateUser,
};
use serde_json::{Value, json};
use test_harness::TestServer;
use uuid::Uuid;

const DEFAULT_ORG: &str = "org_00000000000000000000000000000001";

async fn create_org(server: &TestServer, name: &str) -> everruns_server::storage::OrganizationRow {
    server
        .db
        .create_organization(CreateOrganizationRow {
            public_id: everruns_platform::generate_org_public_id(),
            name: name.to_string(),
            created_by: Some(ANONYMOUS_USER_ID),
        })
        .await
        .expect("create organization")
}

async fn seed_invitation(
    server: &TestServer,
    org_id: i64,
    email: &str,
    expires_at: chrono::DateTime<Utc>,
) -> OrgInvitationRow {
    let unique = Uuid::now_v7().simple().to_string();
    server
        .db
        .create_org_invitation(CreateOrgInvitation {
            public_id: format!("orginv_{unique}"),
            org_id,
            email: email.to_string(),
            role: "member".to_string(),
            invited_by: ANONYMOUS_USER_ID,
            token_hash: format!("hash-{unique}"),
            expires_at,
        })
        .await
        .expect("create invitation")
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

#[tokio::test]
async fn lists_only_actionable_invitations_for_verified_email() {
    let server = TestServer::in_memory().await;
    let actionable_org = create_org(&server, "Actionable Org").await;
    let other_org = create_org(&server, "Other Org").await;
    let now = Utc::now();

    let actionable = seed_invitation(
        &server,
        actionable_org.org_id,
        ANONYMOUS_USER_EMAIL,
        now + Duration::days(1),
    )
    .await;
    seed_invitation(
        &server,
        other_org.org_id,
        "other@example.com",
        now + Duration::days(1),
    )
    .await;
    seed_invitation(
        &server,
        other_org.org_id,
        ANONYMOUS_USER_EMAIL,
        now - Duration::days(1),
    )
    .await;
    let revoked = seed_invitation(
        &server,
        other_org.org_id,
        ANONYMOUS_USER_EMAIL,
        now + Duration::days(1),
    )
    .await;
    server
        .db
        .revoke_org_invitation(other_org.org_id, &revoked.public_id)
        .await
        .expect("revoke invitation");
    let accepted = seed_invitation(
        &server,
        other_org.org_id,
        ANONYMOUS_USER_EMAIL,
        now + Duration::days(1),
    )
    .await;
    server
        .db
        .accept_org_invitation(accepted.id, ANONYMOUS_USER_ID)
        .await
        .expect("accept invitation");

    let body: Value = server
        .get("/v1/me/invitations")
        .await
        .assert_status(StatusCode::OK)
        .json();
    let items = body["data"].as_array().expect("invitation data");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["id"], actionable.public_id);
    assert_eq!(items[0]["org_name"], "Actionable Org");
    assert_eq!(items[0]["role"], "member");
}

#[tokio::test]
async fn accepts_own_invitation_by_public_id() {
    let server = TestServer::in_memory().await;
    let org = create_org(&server, "Inviting Org").await;
    let invitation = seed_invitation(
        &server,
        org.org_id,
        ANONYMOUS_USER_EMAIL,
        Utc::now() + Duration::days(1),
    )
    .await;

    let body: Value = server
        .post(
            &format!("/v1/me/invitations/{}/accept", invitation.public_id),
            json!({}),
        )
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(body["org_id"], org.public_id);
    assert_eq!(body["role"], "member");
    assert!(
        server
            .db
            .is_organization_member(org.org_id, ANONYMOUS_USER_ID)
            .await
            .expect("check membership")
    );
    assert!(
        server
            .get("/v1/me/invitations")
            .await
            .assert_status(StatusCode::OK)
            .json::<Value>()["data"]
            .as_array()
            .expect("invitation data")
            .is_empty()
    );
}

#[tokio::test]
async fn denies_unverified_email_for_list_and_accept() {
    let server = TestServer::in_memory().await;
    let org = create_org(&server, "Inviting Org").await;
    let invitation = seed_invitation(
        &server,
        org.org_id,
        ANONYMOUS_USER_EMAIL,
        Utc::now() + Duration::days(1),
    )
    .await;
    server
        .db
        .update_user(
            ANONYMOUS_USER_ID,
            UpdateUser {
                email_verified: Some(false),
                ..Default::default()
            },
        )
        .await
        .expect("update user");

    let list: Value = server
        .get("/v1/me/invitations")
        .await
        .assert_status(StatusCode::FORBIDDEN)
        .json();
    assert_eq!(list["code"], "invite_email_unverified");
    let accept: Value = server
        .post(
            &format!("/v1/me/invitations/{}/accept", invitation.public_id),
            json!({}),
        )
        .await
        .assert_status(StatusCode::FORBIDDEN)
        .json();
    assert_eq!(accept["code"], "invite_email_unverified");
    let missing: Value = server
        .post(
            "/v1/me/invitations/orginv_00000000000000000000000000000000/accept",
            json!({}),
        )
        .await
        .assert_status(StatusCode::FORBIDDEN)
        .json();
    assert_eq!(missing, accept);
}

#[tokio::test]
async fn public_id_accept_hides_wrong_addressee_and_rejects_expired_invite() {
    let server = TestServer::in_memory().await;
    let org = create_org(&server, "Inviting Org").await;
    let wrong_addressee = seed_invitation(
        &server,
        org.org_id,
        "other@example.com",
        Utc::now() + Duration::days(1),
    )
    .await;
    let expired = seed_invitation(
        &server,
        org.org_id,
        ANONYMOUS_USER_EMAIL,
        Utc::now() - Duration::days(1),
    )
    .await;

    let wrong: Value = server
        .post(
            &format!("/v1/me/invitations/{}/accept", wrong_addressee.public_id),
            json!({}),
        )
        .await
        .assert_status(StatusCode::NOT_FOUND)
        .json();
    let missing: Value = server
        .post(
            "/v1/me/invitations/orginv_00000000000000000000000000000000/accept",
            json!({}),
        )
        .await
        .assert_status(StatusCode::NOT_FOUND)
        .json();
    assert_eq!(wrong, missing);
    assert_eq!(wrong["code"], "invite_invalid");
    let stale: Value = server
        .post(
            &format!("/v1/me/invitations/{}/accept", expired.public_id),
            json!({}),
        )
        .await
        .assert_status(StatusCode::GONE)
        .json();
    assert_eq!(stale["code"], "invite_expired");
}

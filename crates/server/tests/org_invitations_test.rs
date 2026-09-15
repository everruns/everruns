//! OSS-owned organization invitation HTTP integration tests (EVE-602).
//!
//! Exercises the create/list/revoke routes and token-validation error paths
//! through the assembled router. The core invite/accept logic (email-delivery
//! classification, expiry, wrong-email, role escalation) is unit-tested in
//! `api::org_invitations`; these tests cover routing, extractors, auth wiring,
//! and serialization.
//!
//! Route cases use the in-memory backend. The final-slot concurrency case uses
//! PostgreSQL to verify the transaction and organization lock. In the no-auth
//! harness the caller is the anonymous owner of the default org, and no email
//! provider is configured, so creation reports `not_configured`.
//!
//! Run with: cargo test -p everruns-server --test org_invitations_test

mod test_harness;

use axum::http::StatusCode;
use chrono::{Duration, Utc};
use everruns_platform::{ANONYMOUS_USER_EMAIL, ANONYMOUS_USER_ID};
use everruns_server::{
    api::org_invitations::accept_pending_invitation,
    storage::{
        CreateOrgInvitation, CreateOrganizationRow, CreateUserRow, OrgInvitationRow,
        OrganizationRow, UpdateUser,
    },
};
use serde_json::{Value, json};
use test_harness::TestServer;
use uuid::Uuid;

const DEFAULT_ORG: &str = "org_00000000000000000000000000000001";

async fn create_org(server: &TestServer, name: &str) -> OrganizationRow {
    server
        .db
        .create_organization(CreateOrganizationRow {
            public_id: format!("org_{}", Uuid::new_v4().simple()),
            name: name.to_string(),
            created_by: None,
        })
        .await
        .expect("create organization")
}

async fn create_invitation(
    server: &TestServer,
    org_id: i64,
    email: &str,
    expires_at: chrono::DateTime<Utc>,
) -> OrgInvitationRow {
    server
        .db
        .create_org_invitation(CreateOrgInvitation {
            public_id: format!("orginv_{}", Uuid::new_v4().simple()),
            org_id,
            email: email.to_string(),
            role: "member".to_string(),
            invited_by: ANONYMOUS_USER_ID,
            token_hash: Uuid::new_v4().simple().to_string(),
            expires_at,
        })
        .await
        .expect("create invitation")
}

async fn create_verified_user(server: &TestServer, email: &str) -> Uuid {
    server
        .db
        .create_user(CreateUserRow {
            email: email.to_string(),
            name: "Invitee".to_string(),
            avatar_url: None,
            roles: vec![],
            password_hash: None,
            email_verified: true,
            auth_provider: Some("local".to_string()),
            auth_provider_id: None,
            external_id: None,
        })
        .await
        .expect("create user")
        .id
}

async fn fill_organization(server: &TestServer, org_id: i64, count: usize) {
    for index in 0..count {
        let user = create_verified_user(
            server,
            &format!("member-{index}-{}@example.com", Uuid::new_v4()),
        )
        .await;
        server
            .db
            .add_organization_member(org_id, user, "member")
            .await
            .expect("add member");
    }
}

fn pending_accept_path(invitation: &OrgInvitationRow) -> String {
    format!("/v1/invites/pending/{}/accept", invitation.public_id)
}

fn assert_invite_error(response: test_harness::TestResponse, status: StatusCode, code: &str) {
    let body: Value = response.assert_status(status).json();
    assert_eq!(body["code"], code);
}

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
async fn pending_invitation_id_does_not_disclose_another_recipient_lifecycle() {
    let server = TestServer::in_memory().await;
    let org = create_org(&server, "Private Organization").await;
    let other_email = format!("other-{}@example.com", Uuid::new_v4());
    let other_user = create_verified_user(&server, &other_email).await;
    let active = create_invitation(
        &server,
        org.org_id,
        &other_email,
        Utc::now() + Duration::days(1),
    )
    .await;
    let expired = create_invitation(
        &server,
        org.org_id,
        &other_email,
        Utc::now() - Duration::seconds(1),
    )
    .await;
    let revoked = create_invitation(
        &server,
        org.org_id,
        &other_email,
        Utc::now() + Duration::days(1),
    )
    .await;
    server
        .db
        .revoke_org_invitation(org.org_id, &revoked.public_id)
        .await
        .expect("revoke invitation");
    let accepted = create_invitation(
        &server,
        org.org_id,
        &other_email,
        Utc::now() + Duration::days(1),
    )
    .await;
    accept_pending_invitation(&server.db, &accepted.public_id, other_user, 50)
        .await
        .expect("accept invitation as recipient");

    for invitation in [active, expired, revoked, accepted] {
        assert_invite_error(
            server
                .post(&pending_accept_path(&invitation), json!({}))
                .await,
            StatusCode::NOT_FOUND,
            "invite_invalid",
        );
    }
}

#[tokio::test]
async fn pending_invitation_id_requires_a_verified_recipient() {
    let server = TestServer::in_memory().await;
    let org = create_org(&server, "Verified Recipient Organization").await;
    let invitation = create_invitation(
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
        .expect("mark user unverified");

    assert_invite_error(
        server
            .post(&pending_accept_path(&invitation), json!({}))
            .await,
        StatusCode::FORBIDDEN,
        "invite_email_unverified",
    );
    assert!(
        !server
            .db
            .is_organization_member(org.org_id, ANONYMOUS_USER_ID)
            .await
            .expect("membership lookup")
    );
}

#[tokio::test]
async fn pending_invitation_id_reports_recipient_owned_lifecycle_states() {
    let server = TestServer::in_memory().await;
    let org = create_org(&server, "Lifecycle Organization").await;
    let expired = create_invitation(&server, org.org_id, ANONYMOUS_USER_EMAIL, Utc::now()).await;
    let revoked = create_invitation(
        &server,
        org.org_id,
        ANONYMOUS_USER_EMAIL,
        Utc::now() + Duration::days(1),
    )
    .await;
    server
        .db
        .revoke_org_invitation(org.org_id, &revoked.public_id)
        .await
        .expect("revoke invitation");
    let accepted = create_invitation(
        &server,
        org.org_id,
        ANONYMOUS_USER_EMAIL,
        Utc::now() + Duration::days(1),
    )
    .await;
    server
        .post(&pending_accept_path(&accepted), json!({}))
        .await
        .assert_status(StatusCode::OK);

    assert_invite_error(
        server.post(&pending_accept_path(&expired), json!({})).await,
        StatusCode::GONE,
        "invite_expired",
    );
    assert_invite_error(
        server.post(&pending_accept_path(&revoked), json!({})).await,
        StatusCode::CONFLICT,
        "invite_revoked",
    );
    assert_invite_error(
        server
            .post(&pending_accept_path(&accepted), json!({}))
            .await,
        StatusCode::CONFLICT,
        "invite_already_accepted",
    );
}

#[tokio::test]
async fn pending_invitation_id_accepts_the_final_member_slot() {
    let server = TestServer::in_memory().await;
    let org = create_org(&server, "Boundary Organization").await;
    fill_organization(&server, org.org_id, 49).await;
    let invitation = create_invitation(
        &server,
        org.org_id,
        ANONYMOUS_USER_EMAIL,
        Utc::now() + Duration::days(1),
    )
    .await;

    server
        .post(&pending_accept_path(&invitation), json!({}))
        .await
        .assert_status(StatusCode::OK);
    assert_eq!(
        server
            .db
            .count_organization_members(org.org_id)
            .await
            .expect("count members"),
        50
    );
}

#[tokio::test]
async fn pending_invitation_id_preserves_the_invite_when_capacity_is_full() {
    let server = TestServer::in_memory().await;
    let org = create_org(&server, "Full Organization").await;
    fill_organization(&server, org.org_id, 50).await;
    let invitation = create_invitation(
        &server,
        org.org_id,
        ANONYMOUS_USER_EMAIL,
        Utc::now() + Duration::days(1),
    )
    .await;

    assert_invite_error(
        server
            .post(&pending_accept_path(&invitation), json!({}))
            .await,
        StatusCode::CONFLICT,
        "member_limit_reached",
    );
    let current = server
        .db
        .get_org_invitation_by_public_id_and_email(&invitation.public_id, ANONYMOUS_USER_EMAIL)
        .await
        .expect("get invitation")
        .expect("invitation exists");
    assert!(current.accepted_at.is_none());
    assert!(
        !server
            .db
            .is_organization_member(org.org_id, ANONYMOUS_USER_ID)
            .await
            .expect("membership lookup")
    );
}

#[tokio::test]
async fn concurrent_pending_invitations_cannot_exceed_the_member_limit() {
    let server = TestServer::new().await;
    let org = create_org(&server, "Concurrent Capacity Organization").await;
    fill_organization(&server, org.org_id, 49).await;
    let first_email = format!("first-{}@example.com", Uuid::new_v4());
    let second_email = format!("second-{}@example.com", Uuid::new_v4());
    let first_user = create_verified_user(&server, &first_email).await;
    let second_user = create_verified_user(&server, &second_email).await;
    let first_invitation = create_invitation(
        &server,
        org.org_id,
        &first_email,
        Utc::now() + Duration::days(1),
    )
    .await;
    let second_invitation = create_invitation(
        &server,
        org.org_id,
        &second_email,
        Utc::now() + Duration::days(1),
    )
    .await;
    let first_db = server.db.clone();
    let second_db = server.db.clone();
    let first_id = first_invitation.public_id.clone();
    let second_id = second_invitation.public_id.clone();
    let (first_result, second_result) = tokio::join!(
        async move { accept_pending_invitation(&first_db, &first_id, first_user, 50).await },
        async move { accept_pending_invitation(&second_db, &second_id, second_user, 50).await },
    );

    assert_eq!(
        usize::from(first_result.is_ok()) + usize::from(second_result.is_ok()),
        1
    );
    let failure = first_result
        .err()
        .or_else(|| second_result.err())
        .expect("one failure");
    assert_eq!(failure.code, "member_limit_reached");
    assert_eq!(
        server
            .db
            .count_organization_members(org.org_id)
            .await
            .expect("count members"),
        50
    );

    let first_current = server
        .db
        .get_org_invitation_by_public_id_and_email(&first_invitation.public_id, &first_email)
        .await
        .expect("get first invitation")
        .expect("first invitation exists");
    let second_current = server
        .db
        .get_org_invitation_by_public_id_and_email(&second_invitation.public_id, &second_email)
        .await
        .expect("get second invitation")
        .expect("second invitation exists");
    assert_eq!(
        usize::from(first_current.accepted_at.is_some())
            + usize::from(second_current.accepted_at.is_some()),
        1
    );
    assert_eq!(
        usize::from(
            server
                .db
                .is_organization_member(org.org_id, first_user)
                .await
                .expect("first membership lookup")
        ) + usize::from(
            server
                .db
                .is_organization_member(org.org_id, second_user)
                .await
                .expect("second membership lookup")
        ),
        1
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

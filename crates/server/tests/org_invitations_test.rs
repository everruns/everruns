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
use everruns_core::DEFAULT_ORG_ID;
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
async fn direct_add_and_invitation_acceptance_share_the_member_limit() {
    const ADVISORY_LOCK_KEY: i64 = 9_833_603;

    let server = TestServer::new().await;
    let org = server
        .db
        .get_organization(DEFAULT_ORG_ID)
        .await
        .expect("get default organization")
        .expect("default organization exists");
    let initial_member_count = server
        .db
        .count_organization_members(org.org_id)
        .await
        .expect("count initial members");
    assert!(initial_member_count <= 49);
    let mut test_member_ids = Vec::new();
    for index in initial_member_count..49 {
        let user = create_verified_user(
            &server,
            &format!("mixed-member-{index}-{}@example.com", Uuid::new_v4()),
        )
        .await;
        server
            .db
            .add_organization_member(org.org_id, user, "member")
            .await
            .expect("add member");
        test_member_ids.push(user);
    }
    let direct_user =
        create_verified_user(&server, &format!("direct-{}@example.com", Uuid::new_v4())).await;
    let invitee_email = format!("invitee-{}@example.com", Uuid::new_v4());
    let invitee = create_verified_user(&server, &invitee_email).await;
    let invitation = create_invitation(
        &server,
        org.org_id,
        &invitee_email,
        Utc::now() + Duration::days(1),
    )
    .await;

    let suffix = Uuid::new_v4().simple().to_string();
    let function_name = format!("pause_direct_member_add_{suffix}");
    let trigger_name = format!("pause_direct_member_add_{suffix}");
    let trigger_sql = format!(
        r#"
        CREATE FUNCTION {function_name}() RETURNS trigger AS $$
        BEGIN
            IF NEW.org_id = {org_id} AND NEW.user_id = '{direct_user}'::uuid THEN
                PERFORM pg_advisory_lock({ADVISORY_LOCK_KEY});
                PERFORM pg_advisory_unlock({ADVISORY_LOCK_KEY});
            END IF;
            RETURN NEW;
        END;
        $$ LANGUAGE plpgsql;
        CREATE TRIGGER {trigger_name}
        BEFORE INSERT ON organization_members
        FOR EACH ROW EXECUTE FUNCTION {function_name}();
        "#,
        org_id = org.org_id,
    );
    sqlx::raw_sql(sqlx::AssertSqlSafe(trigger_sql.as_str()))
        .execute(&server.pool)
        .await
        .expect("create direct-add pause trigger");

    let mut lock_connection = server
        .pool
        .acquire()
        .await
        .expect("acquire lock connection");
    sqlx::query("SELECT pg_advisory_lock($1)")
        .bind(ADVISORY_LOCK_KEY)
        .execute(&mut *lock_connection)
        .await
        .expect("hold direct-add pause lock");

    let member_path = format!("/v1/orgs/{}/members", org.public_id);
    let direct_add = server.post(
        &member_path,
        json!({ "user_id": direct_user, "role": "member" }),
    );
    tokio::pin!(direct_add);
    let wait_for_direct_add = async {
        for _ in 0..100 {
            let waiting = sqlx::query_scalar::<_, bool>(
                r#"
                SELECT EXISTS(
                    SELECT 1
                    FROM pg_locks
                    WHERE locktype = 'advisory'
                      AND classid::bigint = 0
                      AND objid::bigint = $1
                      AND NOT granted
                )
                "#,
            )
            .bind(ADVISORY_LOCK_KEY)
            .fetch_one(&server.pool)
            .await
            .expect("check direct-add pause lock");
            if waiting {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("direct member add did not reach the pause trigger");
    };
    tokio::select! {
        response = direct_add.as_mut() => {
            panic!("direct member add completed before the pause: {}", response.status());
        }
        _ = wait_for_direct_add => {}
    }

    let acceptance = accept_pending_invitation(&server.db, &invitation.public_id, invitee, 50);
    tokio::pin!(acceptance);
    let early_acceptance = tokio::select! {
        result = acceptance.as_mut() => Some(result),
        _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => None,
    };

    sqlx::query("SELECT pg_advisory_unlock($1)")
        .bind(ADVISORY_LOCK_KEY)
        .execute(&mut *lock_connection)
        .await
        .expect("release direct-add pause lock");
    let direct_response = direct_add.await;
    let acceptance_result = match early_acceptance {
        Some(result) => result,
        None => acceptance.await,
    };

    let cleanup_sql = format!(
        "DROP TRIGGER {trigger_name} ON organization_members; DROP FUNCTION {function_name}();"
    );
    sqlx::raw_sql(sqlx::AssertSqlSafe(cleanup_sql.as_str()))
        .execute(&server.pool)
        .await
        .expect("remove direct-add pause trigger");

    let direct_succeeded = direct_response.status() == StatusCode::CREATED;
    let acceptance_succeeded = acceptance_result.is_ok();
    let final_member_count = server
        .db
        .count_organization_members(org.org_id)
        .await
        .expect("count members");
    let direct_is_member = server
        .db
        .is_organization_member(org.org_id, direct_user)
        .await
        .expect("direct membership lookup");
    let invitee_is_member = server
        .db
        .is_organization_member(org.org_id, invitee)
        .await
        .expect("invitee membership lookup");
    let current_invitation = server
        .db
        .get_org_invitation_by_public_id_and_email(&invitation.public_id, &invitee_email)
        .await
        .expect("get invitation")
        .expect("invitation exists");

    test_member_ids.extend([direct_user, invitee]);
    for user_id in test_member_ids {
        server
            .db
            .remove_organization_member(org.org_id, user_id)
            .await
            .expect("remove test member");
    }

    assert_eq!(
        usize::from(direct_succeeded) + usize::from(acceptance_succeeded),
        1
    );
    if !direct_succeeded {
        direct_response.assert_status(StatusCode::CONFLICT);
    }
    if let Err(error) = &acceptance_result {
        assert_eq!(error.code, "member_limit_reached");
    }
    assert_eq!(final_member_count, 50);
    assert_eq!(direct_is_member, direct_succeeded);
    assert_eq!(invitee_is_member, acceptance_succeeded);
    assert_eq!(
        current_invitation.accepted_at.is_some(),
        acceptance_succeeded
    );
    assert_eq!(
        current_invitation.accepted_by == Some(invitee),
        acceptance_succeeded
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

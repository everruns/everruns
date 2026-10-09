use super::*;
use crate::kernel_imports::DEFAULT_ORG_ID;
use crate::storage::CreateUserRow;

fn agent(subject: &str, owner: &str) -> AgentIdAgent {
    AgentIdAgent {
        subject: subject.into(),
        owner_sub: owner.into(),
        owner_email: None,
        display_name: format!("Agent {subject}"),
    }
}

async fn signed_in(db: &StorageBackend, agent: AgentIdAgent) -> VirtualUserRow {
    match db.agentid_sign_in(DEFAULT_ORG_ID, agent).await.unwrap() {
        AgentIdSignIn::SignedIn(user) => *user,
        AgentIdSignIn::OwnerCapReached => panic!("owner cap reached"),
    }
}

#[tokio::test]
async fn the_same_agent_gets_the_same_end_user_account() {
    let db = StorageBackend::test_database();
    let first = signed_in(&db, agent("agent-same", "owner-same")).await;
    let again = signed_in(&db, agent("agent-same", "owner-same")).await;
    assert_eq!(first.id, again.id);
    assert_eq!(first.usage, "end_user");
    assert_eq!(first.name, "Agent agent-same");
    let bindings = db
        .list_virtual_user_bindings(DEFAULT_ORG_ID, first.id)
        .await
        .unwrap();
    assert_eq!(bindings.len(), 1);
    assert_eq!(bindings[0].provider, "agentid");
    assert_eq!(bindings[0].realm, "https://auth.agentid.com");
    assert_eq!(bindings[0].subject, "agent-same");
}

#[tokio::test]
async fn an_owner_cannot_sign_in_more_agents_than_the_org_cap() {
    let db = StorageBackend::test_database();
    db.set_agentid_agents_per_owner(DEFAULT_ORG_ID, Some(2))
        .await
        .unwrap();
    signed_in(&db, agent("cap-a", "owner-cap")).await;
    signed_in(&db, agent("cap-b", "owner-cap")).await;
    assert!(matches!(
        db.agentid_sign_in(DEFAULT_ORG_ID, agent("cap-c", "owner-cap"))
            .await
            .unwrap(),
        AgentIdSignIn::OwnerCapReached
    ));
    // Agents that already have an account keep signing in, and other owners
    // have their own allowance.
    signed_in(&db, agent("cap-a", "owner-cap")).await;
    signed_in(&db, agent("cap-c", "owner-other")).await;
    db.set_agentid_agents_per_owner(DEFAULT_ORG_ID, None)
        .await
        .unwrap();
    assert_eq!(
        db.agentid_agents_per_owner(DEFAULT_ORG_ID).await.unwrap(),
        DEFAULT_AGENTS_PER_OWNER
    );
}

#[tokio::test]
async fn an_owner_email_matching_a_management_user_is_never_linked() {
    let db = StorageBackend::test_database();
    let member = db
        .create_user(CreateUserRow {
            email: "owner-link@example.com".into(),
            name: "Member".into(),
            avatar_url: None,
            roles: vec!["user".into()],
            password_hash: None,
            email_verified: true,
            auth_provider: None,
            auth_provider_id: None,
            external_id: None,
        })
        .await
        .unwrap();
    let mut linked = agent("agent-link", "owner-link");
    linked.owner_email = Some("owner-link@example.com".into());
    let user = signed_in(&db, linked).await;
    let bindings = db
        .list_virtual_user_bindings(DEFAULT_ORG_ID, user.id)
        .await
        .unwrap();
    assert_eq!(bindings[0].management_user_id, None);
    assert_ne!(Some(member.id), bindings[0].management_user_id);
}

#[tokio::test]
async fn a_sign_in_state_is_consumed_once_and_expires_closed() {
    let db = StorageBackend::test_database();
    let login = AgentIdLoginState {
        org_id: DEFAULT_ORG_ID,
        channel_id: "appchan_state".into(),
        code_verifier: "verifier".into(),
        nonce: "nonce".into(),
        login_hint: Some("hint".into()),
    };
    let hash = [7u8; 32];
    db.create_agentid_login_state(&hash, &login).await.unwrap();
    let found = db
        .consume_agentid_login_state(&hash)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(found.channel_id, "appchan_state");
    assert!(
        db.consume_agentid_login_state(&hash)
            .await
            .unwrap()
            .is_none()
    );

    let expired = [9u8; 32];
    db.create_agentid_login_state(&expired, &login)
        .await
        .unwrap();
    sqlx::query(
        "UPDATE agentid_login_states SET expires_at=now()-interval '1 second' WHERE state_hash=$1",
    )
    .bind(&expired[..])
    .execute(db.database().pool())
    .await
    .unwrap();
    assert!(
        db.consume_agentid_login_state(&expired)
            .await
            .unwrap()
            .is_none()
    );
}

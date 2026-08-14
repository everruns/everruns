//! Integration tests for agent schedule triggers (EVE-757).
//!
//! Mirrors `app_invocation_channels_integration_test.rs`: it drives the real
//! `invoke_agent_trigger` execution path and asserts a session is created on the
//! agent's own harness with the rendered message, and that SharedSession reuse
//! works across two fires.

mod test_harness;

use axum::http::StatusCode;
use serde_json::{Value, json};
use test_harness::TestServer;

use everruns_core::DEFAULT_ORG_ID;
use everruns_provider::typed_id::SessionId;
use everruns_server::domains::agent_triggers::invoke_agent_trigger;
use everruns_server::domains::messages::MessageService;
use everruns_server::domains::sessions::SessionService;
use everruns_server::event_delivery::EventDelivery;

async fn create_agent(server: &TestServer, name: &str) -> Value {
    server
        .post(
            "/v1/agents",
            json!({
                "name": name,
                "system_prompt": "you are a scheduled worker",
                "harness_id": server.seed_generic_harness_id.clone(),
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json()
}

async fn create_trigger(server: &TestServer, agent_id: &str, session_mode: &str) -> Value {
    server
        .post(
            &format!("/v1/agents/{agent_id}/triggers"),
            json!({
                "cron_expression": "0 * * * *",
                "timezone": "UTC",
                "session_mode": session_mode,
                "message": "wake {{agent.name}} {{invocation.source}}",
                "enabled": true,
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json()
}

async fn list_user_message_texts(server: &TestServer, session_id: &str) -> Vec<String> {
    let body: Value = server
        .get(&format!("/v1/sessions/{session_id}/messages"))
        .await
        .assert_status(StatusCode::OK)
        .json();
    body["data"]
        .as_array()
        .expect("messages array")
        .iter()
        .filter(|message| message["role"].as_str() == Some("user"))
        .filter_map(|message| {
            message["content"].as_array().and_then(|parts| {
                parts.iter().find_map(|part| {
                    (part["type"].as_str() == Some("text"))
                        .then(|| part["text"].as_str().map(str::to_owned))
                        .flatten()
                })
            })
        })
        .collect()
}

#[tokio::test]
async fn agent_trigger_binds_schedule_and_invokes_shared_session() {
    let server = TestServer::in_memory().await;
    let agent = create_agent(&server, "trigger-agent").await;
    let agent_id = agent["id"].as_str().unwrap();
    let harness_id = agent["harness_id"].as_str().unwrap().to_string();

    let trigger = create_trigger(&server, agent_id, "shared_session").await;
    let trigger_id = trigger["id"].as_str().unwrap();
    // Enabled trigger binds an enabled durable schedule.
    assert!(
        trigger["config"]["cron_expression"].as_str().is_some(),
        "config carries the normalized cron"
    );

    let session_service = SessionService::new(server.db.clone());
    let message_service = MessageService::new(
        server.db.clone(),
        server.runner.clone(),
        false,
        EventDelivery::in_memory(),
    );

    let first = invoke_agent_trigger(
        &server.db,
        &session_service,
        &message_service,
        DEFAULT_ORG_ID,
        agent_id,
        trigger_id,
    )
    .await
    .expect("first agent trigger invocation");
    let second = invoke_agent_trigger(
        &server.db,
        &session_service,
        &message_service,
        DEFAULT_ORG_ID,
        agent_id,
        trigger_id,
    )
    .await
    .expect("second agent trigger invocation");

    // SharedSession reuse: the second fire reuses the first session.
    assert!(first.created_session);
    assert!(!second.created_session);
    assert_eq!(first.session_id, second.session_id);

    // Session runs on the AGENT's harness (P1) and is hosted by the agent (P2).
    let session_id: SessionId = first.session_id;
    let session = server
        .db
        .get_session(DEFAULT_ORG_ID, session_id)
        .await
        .expect("get session")
        .expect("session exists");
    assert_eq!(
        session
            .harness_id
            .expect("session has a harness")
            .to_string(),
        harness_id,
        "session runs on the agent's harness"
    );
    assert!(session.agent_id.is_some(), "session is hosted by the agent");

    // Ownership (EVE-758): the trigger session is owned by the agent's own
    // lazily-created identity principal, and the agent row is now linked to it.
    let agent_row = server
        .db
        .get_agent(DEFAULT_ORG_ID, session.agent_id.expect("agent id"))
        .await
        .expect("get agent")
        .expect("agent exists");
    let identity_id = agent_row
        .agent_identity_id
        .expect("agent linked to a lazily-created identity on first fire");
    let owner = server
        .db
        .get_principal(DEFAULT_ORG_ID, session.owner_principal_id)
        .await
        .expect("get owner principal")
        .expect("owner principal exists");
    assert_eq!(
        owner.kind, "agent_identity",
        "trigger session is owned by the agent's identity principal"
    );
    assert_eq!(
        session.agent_identity_id,
        Some(identity_id),
        "session records the agent's identity"
    );

    // The second (shared) fire reuses the same identity and the same session.
    let agent_after_second = server
        .db
        .get_agent(DEFAULT_ORG_ID, agent_row.id)
        .await
        .expect("get agent")
        .expect("agent exists");
    assert_eq!(
        agent_after_second.agent_identity_id,
        Some(identity_id),
        "shared-mode reuse keeps the same identity (no re-link)"
    );

    // The rendered template reached the session as a user message, twice.
    let texts = list_user_message_texts(&server, &first.session_id.to_string()).await;
    assert!(
        texts
            .iter()
            .any(|text| text.contains("wake trigger-agent agent_trigger")),
        "rendered trigger message should appear, got: {texts:?}"
    );
    assert_eq!(
        texts.len(),
        2,
        "both fires dispatched into the shared session"
    );

    server
        .delete(&format!("/v1/agent-identities/{identity_id}"))
        .await
        .assert_status(StatusCode::CONFLICT);
}

#[tokio::test]
async fn agent_trigger_session_per_invocation_creates_distinct_sessions() {
    let server = TestServer::in_memory().await;
    let agent = create_agent(&server, "per-invocation-agent").await;
    let agent_id = agent["id"].as_str().unwrap();

    let trigger = create_trigger(&server, agent_id, "session_per_invocation").await;
    let trigger_id = trigger["id"].as_str().unwrap();

    let session_service = SessionService::new(server.db.clone());
    let message_service = MessageService::new(
        server.db.clone(),
        server.runner.clone(),
        false,
        EventDelivery::in_memory(),
    );

    let first = invoke_agent_trigger(
        &server.db,
        &session_service,
        &message_service,
        DEFAULT_ORG_ID,
        agent_id,
        trigger_id,
    )
    .await
    .expect("first invocation");
    let second = invoke_agent_trigger(
        &server.db,
        &session_service,
        &message_service,
        DEFAULT_ORG_ID,
        agent_id,
        trigger_id,
    )
    .await
    .expect("second invocation");

    assert!(first.created_session);
    assert!(second.created_session);
    assert_ne!(
        first.session_id, second.session_id,
        "session_per_invocation creates a fresh session each fire"
    );
}

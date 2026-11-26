//! Integration tests for agent schedule triggers (EVE-757).
//!
//! Mirrors `channel_invocation_integration_test.rs`: it drives the real
//! `invoke_agent_trigger` execution path and asserts a session is created on the
//! agent's own harness with the rendered message, and that SharedSession reuse
//! works across two fires.

use crate::test_harness;

use axum::http::{Method, StatusCode};
use serde_json::{Value, json};
use test_harness::TestServer;

use everruns_contracts::typed_id::SessionId;
use everruns_core::DEFAULT_ORG_ID;
use everruns_server::domains::agent_triggers::invoke_agent_trigger;
use everruns_server::domains::messages::MessageService;
use everruns_server::domains::sessions::SessionService;
use everruns_server::domains::sessions::record::SessionSource;
use everruns_server::live_updates::event_delivery::EventDelivery;

async fn create_agent(server: &TestServer, name: &str) -> Value {
    server
        .post(
            "/v1/platform/agents",
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
            &format!("/v1/platform/agents/{agent_id}/triggers"),
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

#[tokio::test]
async fn webhook_trigger_can_be_created() {
    let server = TestServer::in_memory().await;
    let agent = create_agent(&server, "webhook-trigger-agent").await;
    let agent_id = agent["id"].as_str().unwrap();

    let trigger: Value = server
        .post(
            &format!("/v1/platform/agents/{agent_id}/triggers"),
            json!({
                "trigger_type": "webhook",
                "token": "webhook-secret",
                "session_mode": "shared_session",
                "message": "payload={{payload}}",
                "enabled": true,
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    assert_eq!(trigger["trigger_type"], "webhook");
    assert_eq!(trigger["config"]["token_configured"], true);
    assert!(trigger["config"].get("token").is_none());
    let ingress_id = trigger["ingress_id"].as_str().unwrap();
    assert!(ingress_id.starts_with("appchan_"));
    invoke_webhook(
        &server,
        &format!("/v1/apps/app_not_a_migrated_alias/webhooks/{ingress_id}"),
        ("x-everruns-webhook-token", "webhook-secret"),
        "native-alias",
    )
    .await
    .assert_status(StatusCode::NOT_FOUND);

    let invoked: Value = server
        .request_raw(
            Method::POST,
            &format!("/v1/channels/{ingress_id}/webhook"),
            vec![
                ("content-type", "application/json"),
                ("x-everruns-webhook-token", "webhook-secret"),
            ],
            serde_json::to_vec(&json!({"native": true})).unwrap(),
        )
        .await
        .assert_status(StatusCode::ACCEPTED)
        .json();
    assert!(invoked["created_session"].as_bool().unwrap());
    let session_id: SessionId = invoked["session_id"].as_str().unwrap().parse().unwrap();
    let session = server
        .db
        .get_session(DEFAULT_ORG_ID, session_id)
        .await
        .expect("get webhook session")
        .expect("webhook session exists");
    assert_eq!(
        SessionSource::from(session.source.as_str()),
        SessionSource::Webhook
    );
}

async fn list_user_message_texts(server: &TestServer, session_id: &str) -> Vec<String> {
    let body: Value = server
        .get(&format!("/v1/platform/sessions/{session_id}/messages"))
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

async fn invoke_webhook(
    server: &TestServer,
    path: &str,
    token_header: (&str, &str),
    action: &str,
) -> test_harness::TestResponse {
    server
        .request_raw(
            Method::POST,
            path,
            vec![
                ("content-type", "application/json"),
                token_header,
                ("x-event", "push"),
            ],
            serde_json::to_vec(&json!({"action": action})).unwrap(),
        )
        .await
}

#[tokio::test]
async fn webhook_trigger_creation_rejects_unsupported_auth_and_bindings() {
    let server = TestServer::in_memory().await;
    let agent = create_agent(&server, "invalid-webhook-trigger-agent").await;
    let agent_id = agent["id"].as_str().unwrap();

    for binding in ["per_thread", "per_channel", "per_user"] {
        let response = server
            .post(
                &format!("/v1/platform/agents/{agent_id}/triggers"),
                json!({
                    "trigger_type": "webhook",
                    "token": "secret",
                    "session_mode": binding,
                    "message": "payload={{payload}}",
                }),
            )
            .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert!(response.text().contains("not valid for an agent trigger"));
    }

    let response = server
        .post(
            &format!("/v1/platform/agents/{agent_id}/triggers"),
            json!({
                "trigger_type": "webhook",
                "token": "secret",
                "session_mode": "shared_session",
                "message": "payload={{payload}}",
                "auth": {"mode": "none"},
            }),
        )
        .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(response.text().contains("do not support auth config"));
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
        EventDelivery::in_memory(),
    );

    let first = invoke_agent_trigger(
        &server.db,
        &session_service,
        &message_service,
        None,
        None,
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
        None,
        None,
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
        SessionSource::from(session.source.as_str()),
        SessionSource::Schedule
    );
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
        .virtual_user_id
        .expect("agent linked to a lazily-created identity on first fire");
    let owner = server
        .db
        .get_principal(DEFAULT_ORG_ID, session.owner_principal_id)
        .await
        .expect("get owner principal")
        .expect("owner principal exists");
    assert_eq!(
        owner.kind, "virtual_user",
        "trigger session is owned by the agent's identity principal"
    );
    assert_eq!(
        session.virtual_user_id,
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
        agent_after_second.virtual_user_id,
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
        .delete(&format!("/v1/platform/virtual-users/{identity_id}"))
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
        EventDelivery::in_memory(),
    );

    let first = invoke_agent_trigger(
        &server.db,
        &session_service,
        &message_service,
        None,
        None,
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
        None,
        None,
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

/// Pause agent stops schedules too, not only channels and event triggers.
#[tokio::test]
async fn paused_agent_schedule_does_not_run_until_resumed() {
    let server = TestServer::in_memory().await;
    let agent = create_agent(&server, "paused-schedule-agent").await;
    let agent_id = agent["id"].as_str().unwrap();
    let trigger = create_trigger(&server, agent_id, "shared_session").await;
    let trigger_id = trigger["id"].as_str().unwrap();

    let session_service = SessionService::new(server.db.clone());
    let message_service = MessageService::new(
        server.db.clone(),
        server.runner.clone(),
        EventDelivery::in_memory(),
    );

    server
        .post(&format!("/v1/platform/agents/{agent_id}/pause"), json!({}))
        .await
        .assert_status(StatusCode::OK);

    let error = invoke_agent_trigger(
        &server.db,
        &session_service,
        &message_service,
        None,
        None,
        DEFAULT_ORG_ID,
        agent_id,
        trigger_id,
    )
    .await
    .expect_err("a paused agent's schedule must not run");
    assert!(error.to_string().contains("paused"), "{error}");

    server
        .post(&format!("/v1/platform/agents/{agent_id}/resume"), json!({}))
        .await
        .assert_status(StatusCode::OK);
    invoke_agent_trigger(
        &server.db,
        &session_service,
        &message_service,
        None,
        None,
        DEFAULT_ORG_ID,
        agent_id,
        trigger_id,
    )
    .await
    .expect("a resumed agent's schedule runs");
}

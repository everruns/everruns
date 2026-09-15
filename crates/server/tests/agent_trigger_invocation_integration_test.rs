//! Integration tests for agent schedule triggers (EVE-757).
//!
//! Mirrors `app_invocation_channels_integration_test.rs`: it drives the real
//! `invoke_agent_trigger` execution path and asserts a session is created on the
//! agent's own harness with the rendered message, and that SharedSession reuse
//! works across two fires.

mod test_harness;

use axum::http::{Method, StatusCode};
use chrono::Utc;
use everruns_durable::UpdateField;
use serde_json::{Value, json};
use test_harness::TestServer;

use everruns_core::DEFAULT_ORG_ID;
use everruns_provider::typed_id::{
    AgentId, AgentIdentityId, AppChannelId, HarnessId, SessionId, TriggerId,
};
use everruns_server::domains::agent_triggers::invoke_agent_trigger;
use everruns_server::domains::messages::MessageService;
use everruns_server::domains::sessions::SessionService;
use everruns_server::event_delivery::EventDelivery;
use everruns_server::storage::models::{CreateAgentTriggerRow, UpdateApp};

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

#[tokio::test]
async fn webhook_trigger_can_be_created() {
    let server = TestServer::in_memory().await;
    let agent = create_agent(&server, "webhook-trigger-agent").await;
    let agent_id = agent["id"].as_str().unwrap();

    let trigger: Value = server
        .post(
            &format!("/v1/agents/{agent_id}/triggers"),
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

    let invoked: Value = server
        .request_raw(
            Method::POST,
            &format!("/v1/e/{ingress_id}/webhook"),
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
        trigger["ingress_id"]
            .as_str()
            .unwrap()
            .starts_with("appchan_")
    );
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

async fn create_migrated_webhook_trigger(
    server: &TestServer,
    name: &str,
    session_mode: &str,
    message: &str,
    rate_limit_per_minute: Option<u32>,
) -> (String, String, String) {
    let agent = create_agent(server, &format!("{name}-agent")).await;
    let agent_public_id = agent["id"].as_str().unwrap().to_string();
    let app: Value = server
        .post(
            "/v1/apps",
            json!({
                "name": name,
                "harness_id": server.seed_generic_harness_id.clone(),
                "agent_id": agent_public_id,
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let app_public_id = app["id"].as_str().unwrap().to_string();
    let app_row = server
        .db
        .get_app_by_public_id(DEFAULT_ORG_ID, &app_public_id)
        .await
        .expect("get migrated app")
        .expect("migrated app exists");
    server
        .db
        .update_app(
            DEFAULT_ORG_ID,
            app_row.id,
            UpdateApp {
                status: Some("published".to_string()),
                published_at: UpdateField::Set(Utc::now()),
                ..Default::default()
            },
        )
        .await
        .expect("publish migrated app fixture")
        .expect("migrated app update returns row");

    let ingress_id = AppChannelId::new().to_string();
    server
        .db
        .create_agent_trigger(CreateAgentTriggerRow {
            org_id: DEFAULT_ORG_ID,
            id: TriggerId::new(),
            agent_id: AgentId::from_uuid(app_row.agent_id.expect("app agent")),
            trigger_type: "webhook".to_string(),
            ingress_id: Some(ingress_id.clone()),
            config: json!({
                "token": "migrated-secret",
                "session_mode": session_mode,
                "message": message,
                "rate_limit_per_minute": rate_limit_per_minute,
            }),
            config_encrypted: None,
            enabled: true,
            durable_schedule_id: None,
            execution_harness_id: Some(HarnessId::from_uuid(app_row.harness_id)),
            execution_owner_principal_id: Some(app_row.owner_principal_id),
            execution_resolved_owner_user_id: app_row.resolved_owner_user_id,
            execution_agent_identity_id: app_row.agent_identity_id.map(AgentIdentityId::from_uuid),
            execution_app_id: Some(app_row.id),
        })
        .await
        .expect("seed migrated webhook trigger");

    (app_public_id, agent_public_id, ingress_id)
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
                &format!("/v1/agents/{agent_id}/triggers"),
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
            &format!("/v1/agents/{agent_id}/triggers"),
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
async fn migrated_webhook_trigger_preserves_routes_auth_templates_and_shared_session() {
    let server = TestServer::in_memory().await;
    let template = concat!(
        "app={{app.id}} app_name={{app.name}} channel={{channel.id}} ",
        "agent={{agent.id}} trigger={{trigger.id}} endpoint={{endpoint.id}} ",
        "payload={{payload.action}} body={{webhook.body}} ",
        "json={{webhook.json.action}} event={{webhook.headers.x-event}}"
    );
    let (app_id, agent_id, ingress_id) = create_migrated_webhook_trigger(
        &server,
        "migrated-webhook",
        "shared_session",
        template,
        None,
    )
    .await;

    for path in [
        format!("/v1/apps/{app_id}/webhooks/{ingress_id}"),
        format!("/v1/e/{ingress_id}/webhook"),
    ] {
        let response = invoke_webhook(
            &server,
            &path,
            ("x-everruns-webhook-token", "wrong"),
            "ignored",
        )
        .await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            response.json::<Value>()["detail"],
            "Invalid or missing webhook token"
        );
    }

    let first: Value = invoke_webhook(
        &server,
        &format!("/v1/apps/{app_id}/webhooks/{ingress_id}"),
        ("x-everruns-webhook-token", "migrated-secret"),
        "opened",
    )
    .await
    .assert_status(StatusCode::ACCEPTED)
    .json();
    let second: Value = invoke_webhook(
        &server,
        &format!("/v1/e/{ingress_id}/webhook"),
        ("authorization", "Bearer migrated-secret"),
        "synchronize",
    )
    .await
    .assert_status(StatusCode::ACCEPTED)
    .json();

    assert!(first["created_session"].as_bool().unwrap());
    assert!(!second["created_session"].as_bool().unwrap());
    assert_eq!(first["session_id"], second["session_id"]);
    let texts = list_user_message_texts(&server, first["session_id"].as_str().unwrap()).await;
    assert!(texts.iter().any(|text| {
        text.contains(&format!(
            "app={app_id} app_name=migrated-webhook channel={ingress_id}"
        )) && text.contains(&format!("agent={agent_id}"))
            && text.contains("trigger=trg_")
            && text.contains(&format!("endpoint={ingress_id}"))
            && text.contains("payload=opened")
            && text.contains(r#"body={"action":"opened"}"#)
            && text.contains("json=opened event=push")
    }));
}

#[tokio::test]
async fn migrated_webhook_trigger_ephemeral_sessions_and_rate_limit_are_preserved() {
    let server = TestServer::in_memory().await;
    let (app_id, _, ingress_id) = create_migrated_webhook_trigger(
        &server,
        "migrated-ephemeral-webhook",
        "session_per_invocation",
        "{{payload.action}}",
        Some(2),
    )
    .await;

    let first: Value = invoke_webhook(
        &server,
        &format!("/v1/apps/{app_id}/webhooks/{ingress_id}"),
        ("x-everruns-webhook-token", "migrated-secret"),
        "first",
    )
    .await
    .assert_status(StatusCode::ACCEPTED)
    .json();
    let second: Value = invoke_webhook(
        &server,
        &format!("/v1/e/{ingress_id}/webhook"),
        ("x-everruns-webhook-token", "migrated-secret"),
        "second",
    )
    .await
    .assert_status(StatusCode::ACCEPTED)
    .json();
    assert!(first["created_session"].as_bool().unwrap());
    assert!(second["created_session"].as_bool().unwrap());
    assert_ne!(first["session_id"], second["session_id"]);

    invoke_webhook(
        &server,
        &format!("/v1/e/{ingress_id}/webhook"),
        ("x-everruns-webhook-token", "migrated-secret"),
        "limited",
    )
    .await
    .assert_status(StatusCode::TOO_MANY_REQUESTS);
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

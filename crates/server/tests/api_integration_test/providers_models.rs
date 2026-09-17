//! API integration tests: providers models.

use crate::support::*;
use crate::test_harness;
use axum::http::StatusCode;
use everruns_platform::Agent;
use everruns_platform::Session;
use everruns_provider::model::Model;
use everruns_provider::provider::Provider;
use serde_json::{Value, json};
use test_harness::TestServer;

#[tokio::test]
async fn test_knowledge_index_create_enqueues_sync_and_rejects_chat_model() {
    let server = TestServer::in_memory().await;
    let models: Value = server
        .get("/v1/models")
        .await
        .assert_status(StatusCode::OK)
        .json();
    let models = models["data"].as_array().expect("models list");
    let embedding_model = models
        .iter()
        .find(|model| model["model_id"] == "text-embedding-3-small")
        .expect("seeded embedding model");
    assert_eq!(embedding_model["capabilities"], json!(["embeddings"]));

    let created: Value = server
        .post(
            "/v1/knowledge-indexes",
            json!({
                "name": "Bashkit knowledge test",
                "source_type": "github",
                "source_config": {
                    "repository": "everruns/bashkit",
                    "branch": "main",
                    "root_folder": "knowledge"
                },
                "embedding_model_id": embedding_model["id"]
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    assert_eq!(created["sync_status"], "pending");

    let chat_model = models
        .iter()
        .find(|model| model["model_id"] == "claude-opus-4-7[1m]")
        .or_else(|| {
            models
                .iter()
                .find(|model| model["model_id"] == "claude-opus-4-7")
        })
        .expect("seeded Claude chat model");
    let rejected: Value = server
        .post(
            "/v1/knowledge-indexes",
            json!({
                "name": "Invalid embedding model",
                "source_type": "github",
                "source_config": {"repository": "everruns/bashkit"},
                "embedding_model_id": chat_model["id"]
            }),
        )
        .await
        .assert_status(StatusCode::BAD_REQUEST)
        .json();
    assert_eq!(
        rejected["detail"],
        "Embedding model is unavailable or does not support embeddings"
    );
}

// ============================================
// Health Endpoint Tests
// ============================================

#[tokio::test]
async fn test_health_endpoint() {
    let server = TestServer::new().await;

    let body: Value = server
        .get("/health")
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(body["status"], "ok");
}

// ============================================
// Feature Flags Endpoint Tests
// ============================================

#[tokio::test]
async fn test_feature_flags_endpoint() {
    let server = TestServer::new().await;

    let body: Value = server
        .get("/v1/feature-flags")
        .await
        .assert_status(StatusCode::OK)
        .json();

    // Should return a JSON object with boolean flags
    assert!(body.is_object());
    assert!(body.get("notifications").is_some());
    assert!(body.get("mcp_endpoint").is_none());
    assert_eq!(body["machine_payments"], Value::Bool(false));
    // In test env (DEV_MODE=true), experimental flags are enabled
    assert_eq!(body["notifications"], Value::Bool(false));
}

#[tokio::test]
async fn test_org_feature_flags_opt_in() {
    let server = TestServer::new().await;
    let org_id = "org_00000000000000000000000000000001";

    let settings: serde_json::Value = server
        .get(&format!("/v1/orgs/{org_id}/feature-flags/settings"))
        .await
        .assert_status(StatusCode::OK)
        .json();

    let flags = settings["flags"].as_array().expect("flags array");
    assert!(flags.iter().all(|flag| flag["name"] != "mcp_endpoint"));
    assert!(flags.iter().all(|flag| flag["label"] != "Platform Chat"));
    let notifications = flags
        .iter()
        .find(|f| f["name"] == "notifications")
        .expect("notifications flag");
    assert_eq!(
        notifications["system_enabled"],
        serde_json::Value::Bool(false)
    );
    assert_eq!(notifications["org_enabled"], serde_json::Value::Bool(false));

    let patched: serde_json::Value = server
        .patch(
            &format!("/v1/orgs/{org_id}/feature-flags"),
            serde_json::json!({ "flags": { "notifications": true } }),
        )
        .await
        .assert_status(StatusCode::BAD_REQUEST)
        .json();
    assert!(patched.get("detail").is_some());

    let effective: serde_json::Value = server
        .get(&format!("/v1/orgs/{org_id}/feature-flags"))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(effective["notifications"], serde_json::Value::Bool(false));
    assert!(effective.get("mcp_endpoint").is_none());
    assert_eq!(
        effective["machine_payments"],
        serde_json::Value::Bool(false)
    );

    server
        .patch(
            &format!("/v1/orgs/{org_id}/feature-flags"),
            serde_json::json!({ "flags": { "mcp_endpoint": true } }),
        )
        .await
        .assert_status(StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_notifications_routes_disabled_by_default() {
    let server = TestServer::new().await;

    server
        .get("/v1/notifications")
        .await
        .assert_status(StatusCode::NOT_FOUND);
}

// ============================================
// Agent CRUD Tests
// ============================================

#[tokio::test]
async fn test_create_user_message() {
    let server = TestServer::new().await;

    // Create agent and session
    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({
                "name": "message-test-agent",
                "display_name": "Message Test Agent",
                "system_prompt": "Test"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let session: Session = server
        .post(
            "/v1/sessions",
            json!({
                "harness_id": server.seed_base_harness_id,
                "agent_id": agent.public_id
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    // Create a user message
    let message: Value = server
        .post(
            &format!("/v1/sessions/{}/messages", session.id),
            json!({
                "message": {
                    "role": "user",
                    "content": [{"type": "text", "text": "Hello!"}]
                }
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    assert_eq!(message["role"], "user");
}

#[tokio::test]
async fn test_list_messages() {
    let server = TestServer::new().await;

    // Create agent and session
    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({
                "name": "list-messages-test-agent",
                "display_name": "List Messages Test Agent",
                "system_prompt": "Test"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let session: Session = server
        .post(
            "/v1/sessions",
            json!({
                "harness_id": server.seed_base_harness_id,
                "agent_id": agent.public_id
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    // Create a message
    let _: Value = server
        .post(
            &format!("/v1/sessions/{}/messages", session.id),
            json!({
                "message": {
                    "role": "user",
                    "content": [{"type": "text", "text": "Hello!"}]
                }
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    // List messages
    let data: Value = server
        .get(&format!("/v1/sessions/{}/messages", session.id))
        .await
        .assert_status(StatusCode::OK)
        .json();

    let messages = data["data"].as_array().expect("Expected array");
    assert_eq!(messages.len(), 1);
}

// ============================================
// Events Tests
// ============================================

#[tokio::test]
async fn test_list_events() {
    let server = TestServer::new().await;

    // Create agent and session
    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({
                "name": "events-test-agent",
                "display_name": "Events Test Agent",
                "system_prompt": "Test"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let session: Session = server
        .post(
            "/v1/sessions",
            json!({
                "harness_id": server.seed_base_harness_id,
                "agent_id": agent.public_id
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    // Create a message (which generates events)
    let _: Value = server
        .post(
            &format!("/v1/sessions/{}/messages", session.id),
            json!({
                "message": {
                    "role": "user",
                    "content": [{"type": "text", "text": "Hello!"}]
                }
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    // List events
    let data: Value = server
        .get(&format!("/v1/sessions/{}/events", session.id))
        .await
        .assert_status(StatusCode::OK)
        .json();

    // Events should be present (at least the user message event)
    assert!(data["data"].is_array());
}

/// Reasoning replay state is stored but never published (EVE-933).
///
/// `GET .../messages` strips `signature` / `encrypted` from reasoning parts via
/// `Message::into_public`. The events endpoint serves the same message inside
/// `output.message.completed`, so without its own projection it would republish
/// exactly what the message endpoint withholds. Writes the event straight to the
/// store so the assertion does not need a live provider.
#[tokio::test]
async fn test_events_do_not_publish_reasoning_replay_state() {
    let server = TestServer::in_memory().await;

    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({
                "name": "events-reasoning-projection-agent",
                "display_name": "Events Reasoning Projection",
                "description": "Agent for the events projection test",
                "system_prompt": "You are a helpful assistant"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let session: Session = server
        .post("/v1/sessions", json!({ "agent_id": agent.public_id }))
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let session_id = session.id;

    let mut message = everruns_core::Message::assistant("the answer");
    message.content.push(everruns_core::ContentPart::reasoning(
        everruns_provider::reasoning::ReasoningContentPart::opaque("anthropic")
            .with_item_id("rs_abc")
            .with_signature("sig-do-not-publish")
            .with_encrypted("enc-do-not-publish")
            .with_text(everruns_provider::reasoning::ReasoningText::Plain {
                text: "visible reasoning".to_string(),
            }),
    ));
    let data = everruns_core::EventData::OutputMessageCompleted(
        everruns_core::events::OutputMessageCompletedData::new(message),
    );

    server
        .db
        .create_event(everruns_server::storage::models::CreateEventRow {
            session_id,
            event_type: "output.message.completed".to_string(),
            ts: chrono::Utc::now(),
            context: serde_json::to_value(everruns_core::EventContext::empty()).unwrap(),
            data: serde_json::to_value(&data).unwrap(),
            metadata: None,
            tags: None,
        })
        .await
        .expect("store the event with its replay state intact");

    let body: Value = server
        .get(&format!("/v1/sessions/{}/events", session.id))
        .await
        .assert_status(StatusCode::OK)
        .json();

    let published = serde_json::to_string(&body).expect("serialize response");
    assert!(
        !published.contains("sig-do-not-publish"),
        "reasoning signature must not reach the events API: {published}"
    );
    assert!(
        !published.contains("enc-do-not-publish"),
        "encrypted reasoning payload must not reach the events API: {published}"
    );
    assert!(
        published.contains("visible reasoning"),
        "readable reasoning is content and must still be published: {published}"
    );
    assert!(
        published.contains("rs_abc"),
        "the provider-issued id is an identifier, not replay state: {published}"
    );
}

/// Tests the events endpoint with limit=1 returns only the last event
/// (used by CLI chat to efficiently snapshot before sending a message).
#[tokio::test]
async fn test_list_events_limit_one_returns_last_event() {
    let server = TestServer::in_memory().await;

    // Create agent and session
    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({
                "name": "events-limit-test-agent",
                "display_name": "Events Limit Test Agent",
                "system_prompt": "Test"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let session: Session = server
        .post(
            "/v1/sessions",
            json!({
                "harness_id": server.seed_base_harness_id,
                "agent_id": agent.public_id
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    // Create a message to generate events
    let _: Value = server
        .post(
            &format!("/v1/sessions/{}/messages", session.id),
            json!({
                "message": {
                    "role": "user",
                    "content": [{"type": "text", "text": "First message"}]
                }
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    // Get all events
    let all_events: Value = server
        .get(&format!("/v1/sessions/{}/events", session.id))
        .await
        .assert_status(StatusCode::OK)
        .json();
    let all_data = all_events["data"].as_array().unwrap();
    assert!(
        !all_data.is_empty(),
        "Should have at least one event after message creation"
    );
    let last_event_id = all_data.last().unwrap()["id"].as_str().unwrap();

    // Get events with limit=1 — should return only the last event
    let limited: Value = server
        .get(&format!("/v1/sessions/{}/events?limit=1", session.id))
        .await
        .assert_status(StatusCode::OK)
        .json();
    let limited_data = limited["data"].as_array().unwrap();
    assert_eq!(
        limited_data.len(),
        1,
        "limit=1 should return exactly one event"
    );
    assert_eq!(
        limited_data[0]["id"].as_str().unwrap(),
        last_event_id,
        "limit=1 should return the last event"
    );
}

/// Tests the events endpoint with since_id returns only events after the given ID
/// (used by CLI chat SSE streaming to avoid replaying old events).
#[tokio::test]
async fn test_list_events_since_id_filters_old_events() {
    let server = TestServer::in_memory().await;

    // Create agent and session
    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({
                "name": "events-sinceid-test-agent",
                "display_name": "Events SinceId Test Agent",
                "system_prompt": "Test"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let session: Session = server
        .post(
            "/v1/sessions",
            json!({
                "harness_id": server.seed_base_harness_id,
                "agent_id": agent.public_id
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    // Create first message
    let _: Value = server
        .post(
            &format!("/v1/sessions/{}/messages", session.id),
            json!({
                "message": {
                    "role": "user",
                    "content": [{"type": "text", "text": "First message"}]
                }
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    // Snapshot the last event ID
    let events_after_first: Value = server
        .get(&format!("/v1/sessions/{}/events?limit=1", session.id))
        .await
        .assert_status(StatusCode::OK)
        .json();
    let snapshot_id = events_after_first["data"][0]["id"].as_str().unwrap();

    // Get count of events before second message
    let all_before: Value = server
        .get(&format!("/v1/sessions/{}/events", session.id))
        .await
        .assert_status(StatusCode::OK)
        .json();
    let count_before = all_before["data"].as_array().unwrap().len();

    // Create second message
    let _: Value = server
        .post(
            &format!("/v1/sessions/{}/messages", session.id),
            json!({
                "message": {
                    "role": "user",
                    "content": [{"type": "text", "text": "Second message"}]
                }
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    // Get events since snapshot — should NOT include events from the first message
    let events_since: Value = server
        .get(&format!(
            "/v1/sessions/{}/events?since_id={}",
            session.id, snapshot_id
        ))
        .await
        .assert_status(StatusCode::OK)
        .json();
    let since_data = events_since["data"].as_array().unwrap();

    // Should have new events from the second message, but none from before the snapshot
    assert!(
        !since_data.is_empty(),
        "Should have events after second message"
    );

    // Verify no event ID matches the snapshot or any earlier event
    let all_before_ids: Vec<&str> = all_before["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["id"].as_str().unwrap())
        .collect();
    for event in since_data {
        let event_id = event["id"].as_str().unwrap();
        assert!(
            !all_before_ids.contains(&event_id),
            "since_id should exclude event {event_id} which existed before snapshot"
        );
    }

    // Total events should be: before + new since events
    let all_after: Value = server
        .get(&format!("/v1/sessions/{}/events", session.id))
        .await
        .assert_status(StatusCode::OK)
        .json();
    let count_after = all_after["data"].as_array().unwrap().len();
    assert_eq!(
        count_after,
        count_before + since_data.len(),
        "All events = events before snapshot + events since snapshot"
    );
}

// ============================================
// LLM Provider Tests
// ============================================

#[tokio::test]
async fn test_provider_crud() {
    let server = TestServer::new().await;

    // Create a provider
    let provider: Provider = server
        .post(
            "/v1/providers",
            json!({
                "name": "Test OpenAI Provider",
                "provider_type": "openai",
                "base_url": "https://api.openai.com/v1",
                "enabled": true
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    assert_eq!(provider.name, "Test OpenAI Provider");

    // List providers
    server
        .get("/v1/providers")
        .await
        .assert_status(StatusCode::OK);

    // Delete provider
    server
        .delete(&format!("/v1/providers/{}", provider.id))
        .await
        .assert_status(StatusCode::NO_CONTENT);
}

/// Credential checks must never persist anything and must reject unusable
/// input before any outbound request is made.
#[tokio::test]
async fn test_check_credentials_validates_input_without_persisting() {
    let server = TestServer::in_memory().await;

    // Seeded catalog providers exist from startup; the check must not add to
    // them, so compare against the baseline rather than zero.
    let before: serde_json::Value = server.get("/v1/providers").await.json();
    let baseline = before["data"].as_array().map(Vec::len);

    // Empty key: rejected up front, no provider row created.
    server
        .post(
            "/v1/providers/check-credentials",
            json!({"provider_type": "openai", "api_key": "   "}),
        )
        .await
        .assert_status(StatusCode::BAD_REQUEST);

    // SSRF guard: the same base-URL validation as create applies, since the
    // check issues a real outbound request.
    server
        .post(
            "/v1/providers/check-credentials",
            json!({
                "provider_type": "openai",
                "api_key": "sk-test",
                "base_url": "http://127.0.0.1:8080/v1"
            }),
        )
        .await
        .assert_status(StatusCode::BAD_REQUEST);

    // A driver with no upstream to ask reports "unsupported" rather than
    // failing the user's key.
    let body: serde_json::Value = server
        .post(
            "/v1/providers/check-credentials",
            json!({"provider_type": "llmsim", "api_key": "sk-test"}),
        )
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(body["status"], "unsupported");

    // Nothing was stored by any of the above.
    let after: serde_json::Value = server.get("/v1/providers").await.json();
    assert_eq!(after["data"].as_array().map(Vec::len), baseline);
}

/// Connection-level request options retain their non-secret shape across the
/// API, but header values must never be returned to provider-view callers.
#[tokio::test]
async fn test_provider_request_options_round_trip_and_validation() {
    let server = TestServer::in_memory().await;

    let provider: Provider = server
        .post(
            "/v1/providers",
            json!({
                "name": "Gateway Anthropic",
                "provider_type": "anthropic",
                "request_options": {
                    "headers": [{"name": "x-gateway-tenant", "value": "acme"}],
                    "cache_diagnostics": true
                }
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let options = provider
        .request_options
        .as_ref()
        .expect("request options returned on create");
    assert!(options.cache_diagnostics);
    assert_eq!(
        options.header_pairs(),
        vec![("x-gateway-tenant".to_string(), String::new())]
    );

    // Read back through GET: the options are stored, not just echoed.
    let fetched: Provider = server
        .get(&format!("/v1/providers/{}", provider.id))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(fetched.request_options, provider.request_options);

    // List responses use the same redaction boundary.
    let listed: serde_json::Value = server
        .get("/v1/providers")
        .await
        .assert_status(StatusCode::OK)
        .json();
    let listed_provider = listed["data"]
        .as_array()
        .expect("provider list data")
        .iter()
        .find(|item| item["id"] == provider.id.to_string())
        .expect("created provider in list");
    assert_eq!(
        listed_provider["request_options"]["headers"][0]["value"],
        ""
    );

    // A transport-owned header is refused with a message naming it.
    let rejected = server
        .patch(
            &format!("/v1/providers/{}", provider.id),
            json!({
                "request_options": {
                    "headers": [{"name": "Host", "value": "evil.example"}]
                }
            }),
        )
        .await;
    assert_eq!(rejected.status(), StatusCode::BAD_REQUEST);

    // Saving the form back after a redacted read keeps the header rather than
    // erasing it. `merge_request_options` covers the stored value itself; here
    // the point is that the round trip a UI actually performs is not lossy.
    let resaved: Provider = server
        .patch(
            &format!("/v1/providers/{}", provider.id),
            json!({
                "request_options": {
                    "headers": [{"name": "x-gateway-tenant", "value": ""}],
                    "cache_diagnostics": false
                }
            }),
        )
        .await
        .assert_status(StatusCode::OK)
        .json();
    let resaved_options = resaved
        .request_options
        .as_ref()
        .expect("header survives a redacted round trip");
    assert_eq!(
        resaved_options.header_pairs(),
        vec![("x-gateway-tenant".to_string(), String::new())]
    );
    assert!(!resaved_options.cache_diagnostics);

    // Update replaces the options wholesale, including clearing them.
    let cleared: Provider = server
        .patch(
            &format!("/v1/providers/{}", provider.id),
            json!({ "request_options": { "headers": [], "cache_diagnostics": false } }),
        )
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert!(cleared.request_options.is_none());
}

#[tokio::test]
async fn test_model_crud() {
    let server = TestServer::new().await;

    // Create a provider first
    let provider: Provider = server
        .post(
            "/v1/providers",
            json!({
                "name": "Model Test Provider",
                "provider_type": "openai"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    // Create a model
    let model: Model = server
        .post(
            &format!("/v1/providers/{}/models", provider.id),
            json!({
                "model_id": "gpt-4-test",
                "display_name": "GPT-4 Test",
                "capabilities": ["chat"],
                "enabled": true
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    assert_eq!(model.model_id, "gpt-4-test");

    // List all models
    server.get("/v1/models").await.assert_status(StatusCode::OK);

    // Cleanup
    server
        .delete(&format!("/v1/models/{}", model.id))
        .await
        .assert_status(StatusCode::NO_CONTENT);
    server
        .delete(&format!("/v1/providers/{}", provider.id))
        .await
        .assert_status(StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn test_create_model_missing_provider_returns_not_found() {
    let server = TestServer::in_memory().await;

    server
        .post(
            "/v1/providers/provider_019563a3000070008000000000000001/models",
            json!({
                "model_id": "missing-provider-model",
                "display_name": "Missing Provider Model",
                "capabilities": ["chat"],
                "enabled": true
            }),
        )
        .await
        .assert_status(StatusCode::NOT_FOUND);
}

// ============================================
// Session Model Inheritance Tests
// ============================================

#[tokio::test]
async fn test_post_message_wait_times_out_without_worker() {
    // No worker drains turn tasks in-process, so no turn can complete here: a
    // short wait budget must return 202 with a timeout status. The 200
    // completed path requires server+worker and is covered in workflow_test.rs.
    let server = TestServer::in_memory().await;
    let agent = create_llmsim_agent(&server, "msg-wait").await;
    let session: Session = server
        .post(
            "/v1/sessions",
            json!({
                "harness_id": server.seed_base_harness_id,
                "agent_id": agent.public_id
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let result: Value = server
        .post(
            &format!(
                "/v1/sessions/{}/messages?wait=true&timeout_ms=1500",
                session.id
            ),
            json!({
                "message": {
                    "role": "user",
                    "content": [{ "type": "text", "text": "Hello" }]
                }
            }),
        )
        .await
        .assert_status(StatusCode::ACCEPTED)
        .json();

    assert_eq!(result["status"], "timeout");
    assert!(
        result["message"]["id"].is_string(),
        "timeout returns the accepted message"
    );
}

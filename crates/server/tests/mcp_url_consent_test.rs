//! Consent for a URL mode elicitation raised by an MCP server Everruns calls.
//!
//! The turn pauses on a `confirm_url_elicitation` card; this is the endpoint the
//! card posts to. What matters here is that the decision is bound to what the
//! user was actually shown — the server, tool and domain come from the emitted
//! event, not from the request body — and that an accept leaves a consent the
//! MCP client can find on the retry.

mod test_harness;

use axum::http::StatusCode;
use everruns_mcp::{StoredConsent, consent_storage_key};
use everruns_platform::{Agent, Session};
use everruns_provider::typed_id::SessionId;
use serde_json::{Value, json};
use test_harness::TestServer;

const TEST_ORG_ID: i64 = 1;

async fn waiting_session(server: &TestServer) -> SessionId {
    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({
                "name": "consent-test-agent",
                "display_name": "Consent Test",
                "description": "Agent for the URL elicitation consent test",
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

    server
        .db
        .update_session(
            TEST_ORG_ID,
            session.id,
            everruns_server::storage::models::UpdateSession {
                status: Some("waiting_for_tool_results".to_string()),
                ..Default::default()
            },
        )
        .await
        .expect("update session status")
        .expect("session exists");

    session.id
}

/// Emit the card the engine emits when an MCP tool stops on an elicitation.
async fn emit_elicitation_card(server: &TestServer, session_id: SessionId, tool_call_id: &str) {
    server
        .db
        .create_event(everruns_server::storage::models::CreateEventRow {
            session_id,
            event_type: "tool.call_requested".to_string(),
            ts: chrono::Utc::now(),
            context: json!({}),
            data: json!({
                "tool_calls": [{
                    "id": tool_call_id,
                    "name": "confirm_url_elicitation",
                    "arguments": {
                        "server": "billing",
                        "tool": "charge",
                        "retry_tool": "mcp_billing_charge",
                        "message": "Authorize the charge",
                        "url": "https://pay.example.com/authorize/42",
                        "url_host": "pay.example.com",
                        "url_is_punycode": false,
                    }
                }]
            }),
            metadata: None,
            tags: None,
        })
        .await
        .expect("emit tool.call_requested");
}

async fn stored_consent(server: &TestServer, session_id: SessionId) -> Option<StoredConsent> {
    let row = server
        .db
        .get_session_key_value(session_id.uuid(), &consent_storage_key("billing", "charge"))
        .await
        .expect("read session storage")?;
    Some(serde_json::from_str(&row.value).expect("consent record parses"))
}

async fn post_consent(
    server: &TestServer,
    session_id: SessionId,
    body: Value,
) -> test_harness::TestResponse {
    server
        .post(
            &format!("/v1/sessions/{session_id}/mcp-elicitation-consent"),
            body,
        )
        .await
}

#[tokio::test]
async fn accepting_records_a_consent_the_retry_can_use() {
    let server = TestServer::in_memory().await;
    let session_id = waiting_session(&server).await;
    emit_elicitation_card(&server, session_id, "url_elicitation_1").await;

    let response = post_consent(
        &server,
        session_id,
        json!({ "tool_call_id": "url_elicitation_1", "action": "accept" }),
    )
    .await
    .assert_status(StatusCode::OK)
    .json_value();
    assert_eq!(response["host"], "pay.example.com");

    let consent = stored_consent(&server, session_id).await.expect("recorded");
    assert_eq!(consent.server, "billing");
    assert_eq!(consent.tool, "charge");
    // Bound to the domain the card showed, so a server that elicits somewhere
    // else on the retry gets nothing.
    assert_eq!(consent.host, "pay.example.com");
    assert!(consent.expires_at > chrono::Utc::now());
}

#[tokio::test]
async fn declining_records_nothing() {
    let server = TestServer::in_memory().await;
    let session_id = waiting_session(&server).await;
    emit_elicitation_card(&server, session_id, "url_elicitation_2").await;

    post_consent(
        &server,
        session_id,
        json!({ "tool_call_id": "url_elicitation_2", "action": "decline" }),
    )
    .await
    .assert_status(StatusCode::OK);

    assert!(
        stored_consent(&server, session_id).await.is_none(),
        "a refusal must not leave a usable consent behind"
    );
}

/// The decision has to reach the model, and a tool result cannot carry it: the
/// synthetic call was emitted by the engine, so nothing in the transcript claims
/// it and the lone result is dropped before the provider request is built.
#[tokio::test]
async fn the_decision_is_spoken_into_the_conversation() {
    let server = TestServer::in_memory().await;
    let session_id = waiting_session(&server).await;
    emit_elicitation_card(&server, session_id, "url_elicitation_6").await;

    post_consent(
        &server,
        session_id,
        json!({ "tool_call_id": "url_elicitation_6", "action": "accept" }),
    )
    .await
    .assert_status(StatusCode::OK);

    let events = server
        .db
        .list_events(
            session_id,
            None,
            None,
            &["input.message".to_string()],
            &[],
            None,
            Some(10),
        )
        .await
        .expect("list events");
    let spoken = events.last().expect("the decision was said out loud");
    let text = serde_json::to_string(&spoken.data).expect("serialize");
    assert!(text.contains("pay.example.com"), "unexpected: {text}");
    // No internal tool id: a person reads this line in the transcript.
    assert!(!text.contains("mcp_billing_charge"), "unexpected: {text}");
}

/// A turn can be pinned to a model per message. The decision continues that
/// same turn, so it has to inherit those controls — otherwise the resume runs
/// on whatever the org default resolves to, switching provider mid-conversation
/// while carrying the previous provider's response ids.
#[tokio::test]
async fn the_decision_inherits_the_runs_model() {
    let server = TestServer::in_memory().await;
    let session_id = waiting_session(&server).await;

    // The user's own message, pinned to a model.
    server
        .db
        .create_event(everruns_server::storage::models::CreateEventRow {
            session_id,
            event_type: "input.message".to_string(),
            ts: chrono::Utc::now(),
            context: json!({}),
            data: json!({
                "message": {
                    "id": "message_pinned",
                    "role": "user",
                    "content": [{ "type": "text", "text": "charge me" }],
                    "controls": { "model_id": "model_01933b5a00007000800000000000030c" },
                }
            }),
            metadata: None,
            tags: None,
        })
        .await
        .expect("emit input.message");
    emit_elicitation_card(&server, session_id, "url_elicitation_7").await;

    post_consent(
        &server,
        session_id,
        json!({ "tool_call_id": "url_elicitation_7", "action": "accept" }),
    )
    .await
    .assert_status(StatusCode::OK);

    let events = server
        .db
        .list_events(
            session_id,
            None,
            None,
            &["input.message".to_string()],
            &[],
            None,
            Some(10),
        )
        .await
        .expect("list events");
    let spoken = events.last().expect("the decision was said out loud");
    assert_eq!(
        spoken.data["message"]["controls"]["model_id"],
        "model_01933b5a00007000800000000000030c"
    );
}

#[tokio::test]
async fn the_decision_resumes_the_turn() {
    let server = TestServer::in_memory().await;
    let session_id = waiting_session(&server).await;
    emit_elicitation_card(&server, session_id, "url_elicitation_3").await;

    let response = post_consent(
        &server,
        session_id,
        json!({ "tool_call_id": "url_elicitation_3", "action": "accept" }),
    )
    .await
    .assert_status(StatusCode::OK)
    .json_value();
    assert_eq!(response["status"], "active");

    // The synthetic call is completed, which is what lets the paused act finish.
    let events = server
        .db
        .list_events(
            session_id,
            None,
            None,
            &["tool.completed".to_string()],
            &[],
            None,
            Some(10),
        )
        .await
        .expect("list events");
    let completed = events.last().expect("a tool.completed event");
    assert_eq!(completed.data["tool_call_id"], "url_elicitation_3");
}

#[tokio::test]
async fn an_unknown_tool_call_is_not_a_consent() {
    let server = TestServer::in_memory().await;
    let session_id = waiting_session(&server).await;
    emit_elicitation_card(&server, session_id, "url_elicitation_4").await;

    // Nothing was shown for this id, so there is nothing to consent to.
    post_consent(
        &server,
        session_id,
        json!({ "tool_call_id": "not_a_real_call", "action": "accept" }),
    )
    .await
    .assert_status(StatusCode::NOT_FOUND);

    assert!(stored_consent(&server, session_id).await.is_none());
}

#[tokio::test]
async fn a_session_that_is_not_paused_rejects_a_decision() {
    let server = TestServer::in_memory().await;
    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({
                "name": "consent-active-agent",
                "display_name": "Consent Active",
                "description": "Agent for the URL elicitation consent test",
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

    post_consent(
        &server,
        session.id,
        json!({ "tool_call_id": "url_elicitation_5", "action": "accept" }),
    )
    .await
    .assert_status(StatusCode::CONFLICT);
}

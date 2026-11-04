//! HTTP boundary canaries run in the affected-server PR contract suite.

use crate::test_harness::TestServer;
use axum::http::StatusCode;
use everruns_server::domains::agents::record::Agent;
use everruns_server::domains::sessions::record::Session;
use serde_json::{Value, json};
use std::time::Duration;

/// Reasoning replay state is stored but never published (EVE-933).
///
/// `GET .../messages` strips `signature` / `encrypted` from reasoning parts via
/// `Message::into_public`. The events endpoint serves the same message inside
/// `output.message.completed`, so without its own projection it would republish
/// exactly what the message endpoint withholds. Writes the event straight to the
/// store so the assertion does not need a live provider.
#[tokio::test]
async fn http_messages_events_exports_and_sse_hide_internal_state() {
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

    let mut message = everruns_core::RuntimeMessage::assistant("the answer");
    message.content.push(everruns_core::ContentPart::reasoning(
        everruns_contracts::reasoning::ReasoningContentPart::opaque("anthropic")
            .with_item_id("rs_abc")
            .with_signature("sig-do-not-publish")
            .with_encrypted("enc-do-not-publish")
            .with_text(everruns_contracts::reasoning::ReasoningText::Plain {
                text: "visible reasoning".to_string(),
            }),
    ));
    message.metadata = Some(serde_json::from_value(json!({"model":"test-model", "tool_changes":{"system_prompt":"METADATA-CANARY"}, "unknown":"METADATA-CANARY"})).unwrap());
    let data = everruns_core::EventData::OutputMessageCompleted(
        everruns_core::events::OutputMessageCompletedData::new(message),
    );

    server
        .db
        .create_event(everruns_server::storage::CreateEventRow {
            session_id,
            event_type: "output.message.completed".to_string(),
            ts: chrono::Utc::now(),
            context: serde_json::to_value(everruns_core::EventContext::empty()).unwrap(),
            data: serde_json::to_value(&data).unwrap(),
            metadata: Some(json!({"authority":"METADATA-CANARY"})),
            tags: None,
        })
        .await
        .expect("store the event with its replay state intact");

    let body: Value = server
        .get(&format!("/v1/sessions/{}/events", session.id))
        .await
        .assert_status(StatusCode::OK)
        .json();

    for path in [
        format!("/v1/sessions/{}/messages", session.id),
        format!("/v1/sessions/{}/export?format=jsonl", session.id),
    ] {
        let response = server.get(&path).await.assert_status(StatusCode::OK);
        assert!(!response.text().contains("METADATA-CANARY"));
        assert!(response.text().contains("test-model"));
    }
    let published = serde_json::to_string(&body).expect("serialize response");
    assert!(!published.contains("METADATA-CANARY"));
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

    let replay = server
        .get_stream_prefix(
            &format!("/v1/sessions/{}/sse?after_sequence=0", session.id),
            64 * 1024,
            Duration::from_secs(2),
        )
        .await;
    assert_public_stream(&replay);

    let events = server.event_service.clone();
    let writer = async move {
        tokio::time::sleep(Duration::from_millis(500)).await;
        events
            .emit(
                everruns_core::EventRequest::new(
                    session_id,
                    everruns_core::EventContext::empty(),
                    data,
                )
                .with_metadata(json!({"authority":"METADATA-CANARY"})),
            )
            .await
            .expect("publish live event");
    };
    let route = format!("/v1/sessions/{}/sse", session.id);
    let reader = server.get_stream_prefix(&route, 64 * 1024, Duration::from_secs(2));
    let (live, ()) = tokio::join!(reader, writer);
    assert_public_stream(&live);
}

fn assert_public_stream(stream: &str) {
    assert!(stream.contains("event: output.message.completed"));
    assert!(stream.contains("test-model"));
    assert!(stream.contains("visible reasoning"));
    for internal in [
        "METADATA-CANARY",
        "sig-do-not-publish",
        "enc-do-not-publish",
    ] {
        assert!(!stream.contains(internal), "stream leaked {internal}");
    }
}

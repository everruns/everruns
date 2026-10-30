use super::*;
use serde_json::{Value, json};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

async fn status_payload(state: ChannelSessionState) -> Value {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/agents.sessions.setStatus"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true})))
        .expect(1)
        .mount(&server)
        .await;
    let adapter = SlackDeliveryAdapter::with_api_base(server.uri());
    let context = everruns_core::channel::DeliveryTarget::new("D1", "1.0").context("xoxb-test");
    assert!(matches!(
        adapter.set_session_state(state, &context).await,
        ChannelDeliveryResult::Ok
    ));
    let requests = server.received_requests().await.unwrap();
    serde_json::from_slice(&requests[0].body).unwrap()
}

#[tokio::test]
async fn lifecycle_states_use_slacks_closed_vocabulary() {
    for (state, status) in [
        (ChannelSessionState::Processing, "processing"),
        (ChannelSessionState::Active, "active"),
        (ChannelSessionState::Suspended, "suspended"),
        (ChannelSessionState::Closed, "closed"),
    ] {
        let payload = status_payload(state).await;
        assert_eq!(payload["channel_id"], "D1");
        assert_eq!(payload["thread_ts"], "1.0");
        assert_eq!(payload["status"], status);
        assert!(payload.get("loading_message").is_none());
    }
}

#[tokio::test]
async fn narration_and_unbound_channels_never_issue_lifecycle_calls() {
    let server = MockServer::start().await;
    let adapter = SlackDeliveryAdapter::with_api_base(server.uri());
    let context = everruns_core::channel::DeliveryTarget::new("D1", "1.0").context("xoxb-test");
    for narration in ["is thinking...", "Running a tool", "", "closed"] {
        assert!(matches!(
            adapter.set_status(narration, &context).await,
            ChannelDeliveryResult::Ok
        ));
    }
    let unbound = everruns_core::channel::DeliveryTarget::new("C1", "").context("xoxb-test");
    adapter
        .set_session_state(ChannelSessionState::Processing, &unbound)
        .await;
    assert!(server.received_requests().await.unwrap().is_empty());
}

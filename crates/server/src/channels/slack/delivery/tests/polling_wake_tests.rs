//! Delivery without a notification source (EVERRUNS-2B).
//!
//! NATS deployments run no PostgreSQL event listener, so nothing sends the
//! dispatcher a wakeup. It must still deliver by polling active sessions.

use super::super::*;
use super::terminal_state_tests;

use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn polling_dispatcher_delivers_without_any_notification() {
    let db = Arc::new(StorageBackend::test_database());
    let session = terminal_state_tests::seed_session(&db).await;

    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat.postMessage"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({ "ok": true, "ts": "1.2" })),
        )
        .mount(&mock_server)
        .await;

    let dispatcher = SlackDeliveryDispatcher::start_with_adapter(
        db.clone(),
        DeliveryWake::Poll,
        "https://app.example.com".to_string(),
        Arc::new(SlackDeliveryAdapter::with_api_base(mock_server.uri())),
    );
    dispatcher
        .register(DeliveryRegistration {
            session_id: session.uuid(),
            input_message_id: "msg_turn_one".to_string(),
            bot_token: "xoxb-test-token".to_string(),
            channel: "C_POLL".to_string(),
            thread_ts: "1700000000.000100".to_string(),
            reply_mode: SlackReplyMode::AllMessages,
            surface: SlackSurface::Channel,
            recipient_user_id: None,
            recipient_team_id: None,
            tool_visibility: PublicToolVisibility::default(),
            generic_tool_text:
                crate::domains::agent_channels::record::DEFAULT_AG_UI_GENERIC_TOOL_TEXT.to_string(),
            approvals_enabled: true,
        })
        .await;

    terminal_state_tests::emit(
        &db,
        session,
        "output.message.completed",
        "msg_turn_one",
        serde_json::json!({
            "message": { "content": [{ "type": "text", "text": "polled reply" }] }
        }),
    )
    .await;
    terminal_state_tests::emit(
        &db,
        session,
        "turn.completed",
        "msg_turn_one",
        serde_json::json!({}),
    )
    .await;

    let posted = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let requests = mock_server.received_requests().await.unwrap_or_default();
            if let Some(request) = requests.first() {
                let body: serde_json::Value =
                    serde_json::from_slice(&request.body).expect("slack post body is json");
                return body["text"].as_str().unwrap_or_default().to_string();
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("a polling dispatcher must deliver without a notification");
    assert_eq!(posted, "polled reply");

    dispatcher.shutdown();
}

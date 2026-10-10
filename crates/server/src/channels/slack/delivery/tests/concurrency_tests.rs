use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::{Notify, broadcast};

use super::super::*;
use super::terminal_state_tests;
use crate::live_updates::event_notifications::EventNotificationPayload;

struct BlockingAdapter {
    blocked_channel: String,
    blocked: Arc<Notify>,
    release: Arc<Notify>,
    delivered: Arc<Notify>,
}

#[async_trait]
impl ChannelDeliveryAdapter for BlockingAdapter {
    fn platform(&self) -> &str {
        "blocking"
    }

    async fn deliver(
        &self,
        _message: &OutboundChannelMessage,
        context: &ChannelDeliveryContext,
    ) -> ChannelDeliveryResult {
        if context.channel_id == self.blocked_channel {
            self.blocked.notify_one();
            self.release.notified().await;
        } else {
            self.delivered.notify_one();
        }
        ChannelDeliveryResult::Ok
    }

    async fn send_ack(
        &self,
        _thread_ref: &str,
        _text: &str,
        _context: &ChannelDeliveryContext,
    ) -> ChannelDeliveryResult {
        ChannelDeliveryResult::Ok
    }
}

async fn register_test_delivery(
    dispatcher: &SlackDeliveryDispatcher,
    session_id: Uuid,
    channel: &str,
) {
    dispatcher
        .register(DeliveryRegistration {
            session_id,
            input_message_id: "msg_turn_one".to_string(),
            bot_token: "xoxb-test-token".to_string(),
            channel: channel.to_string(),
            thread_ts: "1700000000.000100".to_string(),
            surface: SlackSurface::Channel,
            recipient_user_id: None,
            recipient_team_id: None,
            tool_visibility: PublicToolVisibility::default(),
            generic_tool_text:
                crate::domains::agent_channels::record::DEFAULT_AG_UI_GENERIC_TOOL_TEXT.to_string(),
            approvals_enabled: true,
        })
        .await;
}

#[tokio::test]
async fn blocked_session_does_not_stall_unrelated_delivery() {
    let db = Arc::new(StorageBackend::test_database());
    let blocked_session = terminal_state_tests::seed_session(&db).await;
    let unrelated_session = terminal_state_tests::seed_session(&db).await;
    let blocked = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let delivered = Arc::new(Notify::new());
    let (tx, rx) = broadcast::channel::<EventNotificationPayload>(16);
    let dispatcher = SlackDeliveryDispatcher::start_with_adapter(
        db.clone(),
        rx,
        "https://app.example.com".to_string(),
        Arc::new(BlockingAdapter {
            blocked_channel: "C_BLOCKED".to_string(),
            blocked: blocked.clone(),
            release: release.clone(),
            delivered: delivered.clone(),
        }),
    );

    register_test_delivery(&dispatcher, blocked_session.uuid(), "C_BLOCKED").await;
    register_test_delivery(&dispatcher, unrelated_session.uuid(), "C_UNRELATED").await;
    for session in [blocked_session, unrelated_session] {
        terminal_state_tests::emit(
            &db,
            session,
            "output.message.completed",
            "msg_turn_one",
            serde_json::json!({
                "message": { "content": [{ "type": "text", "text": "reply" }] }
            }),
        )
        .await;
    }

    tx.send(EventNotificationPayload {
        session_id: blocked_session.uuid(),
    })
    .expect("notify blocked session");
    tokio::time::timeout(std::time::Duration::from_secs(1), blocked.notified())
        .await
        .expect("first delivery should enter its adapter");

    tx.send(EventNotificationPayload {
        session_id: unrelated_session.uuid(),
    })
    .expect("notify unrelated session");
    tokio::time::timeout(std::time::Duration::from_secs(1), delivered.notified())
        .await
        .expect("unrelated delivery must finish while the first is blocked");

    release.notify_one();
    dispatcher.shutdown();
}

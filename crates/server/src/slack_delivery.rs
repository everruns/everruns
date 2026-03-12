// Slack delivery dispatcher — event-driven Slack message posting
//
// Design Decision: Replaces the 120s polling deadline in wait_and_post_response()
// with an event-driven approach. Subscribes to EventNotificationBroadcaster and
// dispatches Slack messages as output.message.completed events arrive.
//
// Design Decision: No durable queue for the Slack HTTP call. Events are already
// durably stored — if the server restarts, startup recovery re-registers active
// Slack sessions and replays from the last known event. Retry on transient Slack
// API failures uses simple exponential backoff in-process.
//
// Design Decision: Delivery context is keyed by (session_id, input_message_id)
// to support concurrent turns in the same session.

use everruns_core::typed_id::{EventId, SessionId};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{RwLock, broadcast};
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::event_notifications::EventNotificationPayload;
use crate::storage::StorageBackend;

/// Context needed to deliver Slack messages for a turn.
#[derive(Debug, Clone)]
struct DeliveryContext {
    bot_token: String,
    channel: String,
    thread_ts: String,
    input_message_id: String,
    /// Last event ID we've processed (for cursor-based pagination).
    since_event_id: Option<EventId>,
}

/// Key for delivery context — a turn within a session.
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
struct DeliveryKey {
    session_id: Uuid,
    input_message_id: String,
}

/// Event-driven Slack delivery dispatcher.
///
/// Subscribes to the event notification broadcaster and delivers agent output
/// messages to Slack as they arrive, with no fixed deadline.
pub struct SlackDeliveryDispatcher {
    /// Active deliveries: (session_id, input_message_id) → context
    deliveries: Arc<RwLock<HashMap<DeliveryKey, DeliveryContext>>>,
    /// Set of session_ids that have at least one active delivery (for fast filtering)
    active_sessions: Arc<RwLock<std::collections::HashSet<Uuid>>>,
    db: Arc<StorageBackend>,
    shutdown_tx: tokio::sync::watch::Sender<bool>,
}

impl SlackDeliveryDispatcher {
    /// Create and start the dispatcher.
    ///
    /// `event_rx` is a broadcast receiver from EventNotificationBroadcaster.
    /// The dispatcher runs a background task that listens for event notifications.
    pub fn start(
        db: Arc<StorageBackend>,
        event_rx: broadcast::Receiver<EventNotificationPayload>,
    ) -> Arc<Self> {
        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

        let dispatcher = Arc::new(Self {
            deliveries: Arc::new(RwLock::new(HashMap::new())),
            active_sessions: Arc::new(RwLock::new(std::collections::HashSet::new())),
            db,
            shutdown_tx,
        });

        // Spawn the event processing loop
        let dispatcher_clone = dispatcher.clone();
        tokio::spawn(async move {
            dispatcher_clone.event_loop(event_rx, shutdown_rx).await;
        });

        dispatcher
    }

    /// Register a new delivery for a session turn.
    ///
    /// Called when a Slack message creates a new agent turn. The dispatcher will
    /// watch for output events and post them to Slack.
    pub async fn register(
        &self,
        session_id: Uuid,
        input_message_id: String,
        bot_token: String,
        channel: String,
        thread_ts: String,
    ) {
        let key = DeliveryKey {
            session_id,
            input_message_id: input_message_id.clone(),
        };

        let ctx = DeliveryContext {
            bot_token,
            channel,
            thread_ts,
            input_message_id,
            since_event_id: None,
        };

        info!(
            %session_id,
            input_message_id = %ctx.input_message_id,
            "Registered Slack delivery"
        );

        self.active_sessions.write().await.insert(session_id);
        self.deliveries.write().await.insert(key, ctx);
    }

    /// Shutdown the dispatcher.
    pub fn shutdown(&self) {
        let _ = self.shutdown_tx.send(true);
    }

    /// Main event loop: listens for event notifications and processes them.
    async fn event_loop(
        &self,
        mut event_rx: broadcast::Receiver<EventNotificationPayload>,
        mut shutdown_rx: tokio::sync::watch::Receiver<bool>,
    ) {
        info!("Slack delivery dispatcher started");

        loop {
            tokio::select! {
                result = event_rx.recv() => {
                    match result {
                        Ok(payload) => {
                            // Fast path: skip if no deliveries for this session
                            if !self.active_sessions.read().await.contains(&payload.session_id) {
                                continue;
                            }
                            self.process_session_events(payload.session_id).await;
                        }
                        Err(broadcast::error::RecvError::Lagged(n)) => {
                            warn!(
                                skipped = n,
                                "Slack delivery dispatcher lagged, processing all active sessions"
                            );
                            // On lag, process all active sessions to catch up
                            let sessions: Vec<Uuid> =
                                self.active_sessions.read().await.iter().copied().collect();
                            for session_id in sessions {
                                self.process_session_events(session_id).await;
                            }
                        }
                        Err(broadcast::error::RecvError::Closed) => {
                            info!("Event broadcaster closed, stopping Slack delivery dispatcher");
                            break;
                        }
                    }
                }
                _ = shutdown_rx.changed() => {
                    info!("Slack delivery dispatcher shutting down");
                    break;
                }
            }
        }
    }

    /// Process new events for a session, delivering messages to Slack.
    async fn process_session_events(&self, session_id: Uuid) {
        let session_id_typed = SessionId::from_uuid(session_id);
        let empty: Vec<String> = vec![];

        // Collect keys for this session
        let keys: Vec<DeliveryKey> = {
            let deliveries = self.deliveries.read().await;
            deliveries
                .keys()
                .filter(|k| k.session_id == session_id)
                .cloned()
                .collect()
        };

        for key in keys {
            // Get current context (clone to release lock)
            let ctx = {
                let deliveries = self.deliveries.read().await;
                match deliveries.get(&key) {
                    Some(ctx) => ctx.clone(),
                    None => continue,
                }
            };

            // Fetch events since last processed
            let events = match self
                .db
                .list_events(
                    session_id_typed,
                    None,
                    ctx.since_event_id,
                    &empty,
                    &empty,
                    None,
                    None,
                )
                .await
            {
                Ok(events) => events,
                Err(e) => {
                    error!(%session_id, error = %e, "Failed to list events for Slack delivery");
                    continue;
                }
            };

            let mut new_since_id = ctx.since_event_id;
            let mut should_unregister = false;

            for event in &events {
                new_since_id = Some(event.id);

                // Only consider events for our turn
                let event_input_msg = event
                    .context
                    .get("input_message_id")
                    .and_then(|v| v.as_str());
                let is_our_turn = event_input_msg == Some(&ctx.input_message_id);

                if !is_our_turn {
                    continue;
                }

                // Post output messages to Slack
                if event.event_type == "output.message.completed"
                    && let Some(text) = extract_response_text(&event.data)
                    && let Err(e) = post_to_slack_with_retry(
                        &ctx.bot_token,
                        &ctx.channel,
                        &ctx.thread_ts,
                        &text,
                    )
                    .await
                {
                    error!(
                        %session_id,
                        error = %e,
                        "Failed to post message to Slack after retries"
                    );
                }

                // Stop watching when turn ends
                if event.event_type == "turn.completed" || event.event_type == "turn.failed" {
                    debug!(
                        %session_id,
                        event_type = %event.event_type,
                        "Turn ended, unregistering Slack delivery"
                    );
                    should_unregister = true;
                    break;
                }
            }

            // Update cursor or unregister
            if should_unregister {
                self.unregister(&key).await;
            } else if new_since_id != ctx.since_event_id {
                // Update the cursor
                let mut deliveries = self.deliveries.write().await;
                if let Some(ctx) = deliveries.get_mut(&key) {
                    ctx.since_event_id = new_since_id;
                }
            }
        }
    }

    /// Remove a delivery registration.
    async fn unregister(&self, key: &DeliveryKey) {
        let session_id = key.session_id;
        self.deliveries.write().await.remove(key);

        // Check if any other deliveries exist for this session
        let has_others = self
            .deliveries
            .read()
            .await
            .keys()
            .any(|k| k.session_id == session_id);

        if !has_others {
            self.active_sessions.write().await.remove(&session_id);
        }

        info!(
            %session_id,
            input_message_id = %key.input_message_id,
            "Unregistered Slack delivery"
        );
    }

    /// Get the number of active deliveries (for monitoring).
    pub async fn active_delivery_count(&self) -> usize {
        self.deliveries.read().await.len()
    }

    /// Recover active Slack deliveries after server restart.
    ///
    /// Finds sessions in 'active' status with slack:* tags, looks up the
    /// corresponding app for bot_token, and re-registers deliveries for
    /// any turns that haven't completed yet.
    pub async fn recover(&self, app_service: &crate::services::AppService) {
        let sessions = match self.db.find_active_slack_sessions().await {
            Ok(sessions) => sessions,
            Err(e) => {
                error!(error = %e, "Failed to query active Slack sessions for recovery");
                return;
            }
        };

        if sessions.is_empty() {
            debug!("No active Slack sessions to recover");
            return;
        }

        info!(count = sessions.len(), "Recovering active Slack sessions");

        for session in &sessions {
            // Extract app_id from tags (slack:app:{app_id})
            let app_id = match session
                .tags
                .iter()
                .find_map(|t| t.strip_prefix("slack:app:"))
            {
                Some(id) => id.to_string(),
                None => continue,
            };

            // Look up the app to get bot_token
            let app = match app_service.get_by_public_id_unscoped(&app_id).await {
                Ok(Some(app)) => app,
                Ok(None) => {
                    warn!(app_id = %app_id, "App not found during Slack recovery");
                    continue;
                }
                Err(e) => {
                    warn!(app_id = %app_id, error = %e, "Failed to load app for Slack recovery");
                    continue;
                }
            };

            let slack_config = match app.slack_config() {
                Some(cfg) => cfg,
                None => continue,
            };

            // Find the last input.message event to determine delivery context
            let empty: Vec<String> = vec![];
            let filter_types = vec!["input.message".to_string()];
            let events = match self
                .db
                .list_events(session.id, None, None, &filter_types, &empty, None, None)
                .await
            {
                Ok(events) => events,
                Err(e) => {
                    warn!(
                        session_id = %session.id,
                        error = %e,
                        "Failed to list events for Slack recovery"
                    );
                    continue;
                }
            };

            // Get the last input message
            let last_input = match events.last() {
                Some(e) => e,
                None => continue,
            };

            let input_message_id = last_input
                .data
                .get("message")
                .and_then(|m| m.get("id"))
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();

            if input_message_id.is_empty() {
                continue;
            }

            // Extract channel and thread_ts from message metadata
            let metadata = last_input
                .data
                .get("message")
                .and_then(|m| m.get("metadata"));

            let channel = metadata
                .and_then(|m| m.get("slack_channel"))
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();

            let thread_ts = metadata
                .and_then(|m| m.get("slack_ts"))
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();

            // Check if this turn already completed
            let turn_events = vec!["turn.completed".to_string(), "turn.failed".to_string()];
            let completion_events = self
                .db
                .list_events(
                    session.id,
                    None,
                    Some(last_input.id),
                    &turn_events,
                    &empty,
                    None,
                    None,
                )
                .await
                .unwrap_or_default();

            let turn_done = completion_events.iter().any(|e| {
                e.context.get("input_message_id").and_then(|v| v.as_str())
                    == Some(&input_message_id)
            });

            if turn_done {
                debug!(
                    session_id = %session.id,
                    "Slack session turn already completed, skipping recovery"
                );
                continue;
            }

            if channel.is_empty() {
                warn!(
                    session_id = %session.id,
                    "Missing channel in Slack session recovery metadata"
                );
                continue;
            }

            info!(
                session_id = %session.id,
                input_message_id = %input_message_id,
                "Recovering Slack delivery"
            );

            self.register(
                session.id.uuid(),
                input_message_id,
                slack_config.bot_token.clone(),
                channel,
                thread_ts,
            )
            .await;
        }

        let count = self.deliveries.read().await.len();
        info!(recovered = count, "Slack delivery recovery complete");
    }
}

/// Extract text content from an output.message.completed event's data.
pub(crate) fn extract_response_text(data: &serde_json::Value) -> Option<String> {
    let message = data.get("message")?;
    let content = message.get("content")?.as_array()?;

    let mut text_parts = Vec::new();
    for part in content {
        if part.get("type")?.as_str()? == "text"
            && let Some(text) = part.get("text").and_then(|t| t.as_str())
        {
            text_parts.push(text.to_string());
        }
    }

    if text_parts.is_empty() {
        None
    } else {
        Some(text_parts.join("\n"))
    }
}

/// Post a message to Slack with exponential backoff retry.
///
/// Retries up to 3 times on transient failures (network errors, rate limits).
/// Non-retryable errors (invalid token, channel not found) fail immediately.
async fn post_to_slack_with_retry(
    bot_token: &str,
    channel: &str,
    thread_ts: &str,
    text: &str,
) -> anyhow::Result<()> {
    post_to_slack_with_retry_base(SLACK_API_BASE, bot_token, channel, thread_ts, text).await
}

async fn post_to_slack_with_retry_base(
    base_url: &str,
    bot_token: &str,
    channel: &str,
    thread_ts: &str,
    text: &str,
) -> anyhow::Result<()> {
    let max_attempts = 3;
    let mut delay = std::time::Duration::from_secs(1);

    for attempt in 1..=max_attempts {
        match post_to_slack_base(base_url, bot_token, channel, thread_ts, text).await {
            Ok(()) => return Ok(()),
            Err(e) => {
                let error_str = e.to_string();

                // Non-retryable Slack API errors
                if error_str.contains("channel_not_found")
                    || error_str.contains("not_authed")
                    || error_str.contains("invalid_auth")
                    || error_str.contains("token_revoked")
                    || error_str.contains("account_inactive")
                    || error_str.contains("no_text")
                {
                    return Err(e);
                }

                if attempt == max_attempts {
                    return Err(e);
                }

                warn!(
                    attempt,
                    max_attempts,
                    error = %e,
                    "Slack post failed, retrying"
                );
                tokio::time::sleep(delay).await;
                delay *= 2;
            }
        }
    }

    unreachable!()
}

const SLACK_API_BASE: &str = "https://slack.com/api";

/// Post a message to Slack using the Bot API.
pub(crate) async fn post_to_slack(
    bot_token: &str,
    channel: &str,
    thread_ts: &str,
    text: &str,
) -> anyhow::Result<()> {
    post_to_slack_base(SLACK_API_BASE, bot_token, channel, thread_ts, text).await
}

/// Post a message to Slack using the Bot API (with configurable base URL for testing).
pub(crate) async fn post_to_slack_base(
    base_url: &str,
    bot_token: &str,
    channel: &str,
    thread_ts: &str,
    text: &str,
) -> anyhow::Result<()> {
    let client = reqwest::Client::new();

    let mut payload = serde_json::json!({
        "channel": channel,
        "text": text,
    });

    if !thread_ts.is_empty() {
        payload["thread_ts"] = serde_json::Value::String(thread_ts.to_string());
    }

    let response = client
        .post(format!("{}/chat.postMessage", base_url))
        .header("Authorization", format!("Bearer {}", bot_token))
        .header("Content-Type", "application/json")
        .json(&payload)
        .send()
        .await?;

    let status = response.status();
    let body: serde_json::Value = response.json().await?;

    if !body.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
        let error = body
            .get("error")
            .and_then(|e| e.as_str())
            .unwrap_or("unknown");

        // Rate limited — treat as retryable
        if error == "ratelimited" {
            return Err(anyhow::anyhow!("Slack API rate limited"));
        }

        error!(
            channel = channel,
            error = error,
            status = %status,
            "Failed to post message to Slack"
        );
        return Err(anyhow::anyhow!("Slack API error: {}", error));
    }

    info!(channel = channel, "Posted response to Slack");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_response_text_valid() {
        let data = serde_json::json!({
            "message": {
                "content": [
                    {"type": "text", "text": "Hello from the agent!"},
                    {"type": "text", "text": "Second part."}
                ]
            }
        });
        let result = extract_response_text(&data);
        assert_eq!(
            result,
            Some("Hello from the agent!\nSecond part.".to_string())
        );
    }

    #[test]
    fn test_extract_response_text_single_part() {
        let data = serde_json::json!({
            "message": {
                "content": [
                    {"type": "text", "text": "Only one part."}
                ]
            }
        });
        assert_eq!(
            extract_response_text(&data),
            Some("Only one part.".to_string())
        );
    }

    #[test]
    fn test_extract_response_text_no_text() {
        let data = serde_json::json!({
            "message": {
                "content": [
                    {"type": "tool_use", "name": "search"}
                ]
            }
        });
        assert_eq!(extract_response_text(&data), None);
    }

    #[test]
    fn test_extract_response_text_empty_content() {
        let data = serde_json::json!({
            "message": {
                "content": []
            }
        });
        assert_eq!(extract_response_text(&data), None);
    }

    #[test]
    fn test_extract_response_text_missing_message() {
        let data = serde_json::json!({});
        assert_eq!(extract_response_text(&data), None);
    }

    #[test]
    fn test_extract_response_text_missing_content() {
        let data = serde_json::json!({
            "message": {}
        });
        assert_eq!(extract_response_text(&data), None);
    }

    #[test]
    fn test_extract_response_text_mixed_content() {
        let data = serde_json::json!({
            "message": {
                "content": [
                    {"type": "text", "text": "Part 1"},
                    {"type": "tool_use", "name": "search"},
                    {"type": "text", "text": "Part 2"}
                ]
            }
        });
        // tool_use part causes early return via `?` operator on type check
        // Only "Part 1" is extracted before the tool_use part returns None from the iterator
        let result = extract_response_text(&data);
        // The `?` in part.get("type")?.as_str()? causes the for loop to
        // short-circuit the entire function when a non-text part doesn't have
        // the expected structure. But tool_use does have "type", so it just
        // doesn't match "text" and the `&&` short-circuits. Both text parts
        // should be captured.
        assert_eq!(result, Some("Part 1\nPart 2".to_string()));
    }

    #[test]
    fn test_delivery_key_eq() {
        let k1 = DeliveryKey {
            session_id: Uuid::nil(),
            input_message_id: "msg_123".to_string(),
        };
        let k2 = DeliveryKey {
            session_id: Uuid::nil(),
            input_message_id: "msg_123".to_string(),
        };
        assert_eq!(k1, k2);
    }

    #[test]
    fn test_delivery_key_different_session() {
        let k1 = DeliveryKey {
            session_id: Uuid::nil(),
            input_message_id: "msg_123".to_string(),
        };
        let k2 = DeliveryKey {
            session_id: Uuid::from_u128(1),
            input_message_id: "msg_123".to_string(),
        };
        assert_ne!(k1, k2);
    }

    #[test]
    fn test_delivery_key_different_message() {
        let k1 = DeliveryKey {
            session_id: Uuid::nil(),
            input_message_id: "msg_123".to_string(),
        };
        let k2 = DeliveryKey {
            session_id: Uuid::nil(),
            input_message_id: "msg_456".to_string(),
        };
        assert_ne!(k1, k2);
    }

    #[test]
    fn test_delivery_key_hash_consistency() {
        use std::collections::HashMap;
        let key = DeliveryKey {
            session_id: Uuid::nil(),
            input_message_id: "msg_123".to_string(),
        };
        let mut map = HashMap::new();
        map.insert(key.clone(), "value");
        assert_eq!(map.get(&key), Some(&"value"));
    }

    // ==========================================
    // WireMock integration tests — post_to_slack
    // ==========================================

    mod wiremock_tests {
        use super::*;
        use wiremock::matchers::{header, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        #[tokio::test]
        async fn test_post_to_slack_success() {
            let mock_server = MockServer::start().await;

            Mock::given(method("POST"))
                .and(path("/chat.postMessage"))
                .and(header("Authorization", "Bearer xoxb-test-token"))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "ok": true,
                    "channel": "C123",
                    "ts": "1234567890.123456"
                })))
                .expect(1)
                .mount(&mock_server)
                .await;

            let result = post_to_slack_base(
                &mock_server.uri(),
                "xoxb-test-token",
                "C123",
                "1234567890.000000",
                "Hello from agent!",
            )
            .await;

            assert!(result.is_ok());
        }

        #[tokio::test]
        async fn test_post_to_slack_success_no_thread_ts() {
            let mock_server = MockServer::start().await;

            Mock::given(method("POST"))
                .and(path("/chat.postMessage"))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "ok": true,
                    "channel": "C123",
                    "ts": "1234567890.123456"
                })))
                .expect(1)
                .mount(&mock_server)
                .await;

            // Empty thread_ts should still succeed (message not in thread)
            let result =
                post_to_slack_base(&mock_server.uri(), "xoxb-test-token", "C123", "", "Hello!")
                    .await;

            assert!(result.is_ok());
        }

        #[tokio::test]
        async fn test_post_to_slack_channel_not_found() {
            let mock_server = MockServer::start().await;

            Mock::given(method("POST"))
                .and(path("/chat.postMessage"))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "ok": false,
                    "error": "channel_not_found"
                })))
                .expect(1)
                .mount(&mock_server)
                .await;

            let result = post_to_slack_base(
                &mock_server.uri(),
                "xoxb-test-token",
                "C_INVALID",
                "",
                "Hello!",
            )
            .await;

            assert!(result.is_err());
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("channel_not_found")
            );
        }

        #[tokio::test]
        async fn test_post_to_slack_invalid_auth() {
            let mock_server = MockServer::start().await;

            Mock::given(method("POST"))
                .and(path("/chat.postMessage"))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "ok": false,
                    "error": "invalid_auth"
                })))
                .expect(1)
                .mount(&mock_server)
                .await;

            let result =
                post_to_slack_base(&mock_server.uri(), "xoxb-bad-token", "C123", "", "Hello!")
                    .await;

            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("invalid_auth"));
        }

        #[tokio::test]
        async fn test_post_to_slack_rate_limited() {
            let mock_server = MockServer::start().await;

            Mock::given(method("POST"))
                .and(path("/chat.postMessage"))
                .respond_with(ResponseTemplate::new(429).set_body_json(serde_json::json!({
                    "ok": false,
                    "error": "ratelimited"
                })))
                .expect(1)
                .mount(&mock_server)
                .await;

            let result =
                post_to_slack_base(&mock_server.uri(), "xoxb-test-token", "C123", "", "Hello!")
                    .await;

            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("rate limited"));
        }

        #[tokio::test]
        async fn test_post_to_slack_token_revoked() {
            let mock_server = MockServer::start().await;

            Mock::given(method("POST"))
                .and(path("/chat.postMessage"))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "ok": false,
                    "error": "token_revoked"
                })))
                .expect(1)
                .mount(&mock_server)
                .await;

            let result =
                post_to_slack_base(&mock_server.uri(), "xoxb-revoked", "C123", "", "Hello!").await;

            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("token_revoked"));
        }

        #[tokio::test]
        async fn test_post_to_slack_unknown_error() {
            let mock_server = MockServer::start().await;

            Mock::given(method("POST"))
                .and(path("/chat.postMessage"))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "ok": false,
                    "error": "some_unexpected_error"
                })))
                .expect(1)
                .mount(&mock_server)
                .await;

            let result =
                post_to_slack_base(&mock_server.uri(), "xoxb-test-token", "C123", "", "Hello!")
                    .await;

            assert!(result.is_err());
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("some_unexpected_error")
            );
        }

        #[tokio::test]
        async fn test_post_to_slack_malformed_json() {
            let mock_server = MockServer::start().await;

            Mock::given(method("POST"))
                .and(path("/chat.postMessage"))
                .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
                .expect(1)
                .mount(&mock_server)
                .await;

            let result =
                post_to_slack_base(&mock_server.uri(), "xoxb-test-token", "C123", "", "Hello!")
                    .await;

            assert!(result.is_err());
        }

        #[tokio::test]
        async fn test_post_to_slack_server_error() {
            let mock_server = MockServer::start().await;

            Mock::given(method("POST"))
                .and(path("/chat.postMessage"))
                .respond_with(ResponseTemplate::new(500).set_body_string("Internal Server Error"))
                .expect(1)
                .mount(&mock_server)
                .await;

            let result =
                post_to_slack_base(&mock_server.uri(), "xoxb-test-token", "C123", "", "Hello!")
                    .await;

            // 500 with non-JSON body → reqwest JSON parse error
            assert!(result.is_err());
        }

        #[tokio::test]
        async fn test_post_to_slack_retry_non_retryable_errors_fail_immediately() {
            let mock_server = MockServer::start().await;

            // Non-retryable error should only be called once (no retry)
            Mock::given(method("POST"))
                .and(path("/chat.postMessage"))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "ok": false,
                    "error": "channel_not_found"
                })))
                .expect(1)
                .mount(&mock_server)
                .await;

            let result = post_to_slack_with_retry_base(
                &mock_server.uri(),
                "xoxb-test-token",
                "C_GONE",
                "",
                "Hello!",
            )
            .await;

            assert!(result.is_err());
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("channel_not_found")
            );
            // .expect(1) verifies exactly one call was made (no retries)
        }

        #[tokio::test]
        async fn test_post_to_slack_retry_success_on_first_attempt() {
            let mock_server = MockServer::start().await;

            Mock::given(method("POST"))
                .and(path("/chat.postMessage"))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "ok": true,
                    "channel": "C123",
                    "ts": "1234567890.123456"
                })))
                .expect(1)
                .mount(&mock_server)
                .await;

            let result = post_to_slack_with_retry_base(
                &mock_server.uri(),
                "xoxb-test-token",
                "C123",
                "1234.0000",
                "Hello!",
            )
            .await;

            assert!(result.is_ok());
        }

        #[tokio::test]
        async fn test_post_to_slack_retry_not_authed() {
            let mock_server = MockServer::start().await;

            Mock::given(method("POST"))
                .and(path("/chat.postMessage"))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "ok": false,
                    "error": "not_authed"
                })))
                .expect(1)
                .mount(&mock_server)
                .await;

            let result = post_to_slack_with_retry_base(
                &mock_server.uri(),
                "xoxb-test-token",
                "C123",
                "",
                "Hello!",
            )
            .await;

            assert!(result.is_err());
            // Should not retry — exactly 1 call
        }

        #[tokio::test]
        async fn test_post_to_slack_retry_account_inactive() {
            let mock_server = MockServer::start().await;

            Mock::given(method("POST"))
                .and(path("/chat.postMessage"))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "ok": false,
                    "error": "account_inactive"
                })))
                .expect(1)
                .mount(&mock_server)
                .await;

            let result = post_to_slack_with_retry_base(
                &mock_server.uri(),
                "xoxb-test-token",
                "C123",
                "",
                "Hello!",
            )
            .await;

            assert!(result.is_err());
        }

        #[tokio::test]
        async fn test_post_to_slack_retry_no_text() {
            let mock_server = MockServer::start().await;

            Mock::given(method("POST"))
                .and(path("/chat.postMessage"))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "ok": false,
                    "error": "no_text"
                })))
                .expect(1)
                .mount(&mock_server)
                .await;

            let result = post_to_slack_with_retry_base(
                &mock_server.uri(),
                "xoxb-test-token",
                "C123",
                "",
                "",
            )
            .await;

            assert!(result.is_err());
        }

        #[tokio::test]
        async fn test_post_to_slack_ok_false_no_error_field() {
            let mock_server = MockServer::start().await;

            Mock::given(method("POST"))
                .and(path("/chat.postMessage"))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "ok": false
                })))
                .expect(1)
                .mount(&mock_server)
                .await;

            let result =
                post_to_slack_base(&mock_server.uri(), "xoxb-test-token", "C123", "", "Hello!")
                    .await;

            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("unknown"));
        }
    }
}

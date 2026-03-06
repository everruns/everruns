// Slack ingestion API — app-scoped webhook endpoint
//
// Design Decision: Slack webhooks are app-scoped (POST /v1/apps/{app_id}/slack/events)
// because Slack is bound to an App which defines the agent, harness, signing secret,
// and session strategy. This endpoint is unauthenticated (no API key) — security
// comes from Slack signing secret verification (HMAC-SHA256).
//
// Design Decision: No auth middleware on this route. The app_id in the URL identifies
// the app; the signing secret in channel_config verifies the request origin.
//
// Design Decision: Session routing uses tags for lookup. Tags like
// "slack:thread:{thread_ts}" or "slack:channel:{channel}" let us find or create
// sessions based on the configured session strategy.
//
// Design Decision: Agent responses are posted back to Slack via a background task
// that polls for output.message.completed events on the session.

use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::post,
};
use everruns_core::{App, AppStatus, ChannelType, SessionStrategy, SlackChannelConfig};
use everruns_worker::AgentRunner;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::sync::Arc;

use crate::api::messages::{CreateMessageRequest, InputContentPart, InputMessage, MessageRole};
use crate::api::sessions::CreateSessionRequest;
use crate::services::{AppService, MessageService, SessionService};
use crate::storage::StorageBackend;

use super::common::ErrorResponse;

type HmacSha256 = Hmac<Sha256>;

/// Slack event wrapper (Events API envelope).
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct SlackEventEnvelope {
    /// Event type: "url_verification", "event_callback", etc.
    #[serde(rename = "type")]
    event_type: String,
    /// Challenge string for URL verification.
    #[serde(default)]
    challenge: Option<String>,
    /// The actual event payload.
    #[serde(default)]
    event: Option<SlackEvent>,
    /// Team ID.
    #[serde(default)]
    team_id: Option<String>,
}

/// Inner Slack event (message, app_mention, etc.).
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
struct SlackEvent {
    /// Event type: "message", "app_mention", etc.
    #[serde(rename = "type")]
    event_type: String,
    /// User who sent the message.
    #[serde(default)]
    user: Option<String>,
    /// Message text.
    #[serde(default)]
    text: Option<String>,
    /// Channel where the event occurred.
    #[serde(default)]
    channel: Option<String>,
    /// Thread timestamp (for threaded messages).
    #[serde(default)]
    thread_ts: Option<String>,
    /// Message timestamp.
    #[serde(default)]
    ts: Option<String>,
    /// Bot ID (present when message is from a bot — used to ignore own messages).
    #[serde(default)]
    bot_id: Option<String>,
    /// Subtype (e.g., "bot_message", "message_changed").
    #[serde(default)]
    subtype: Option<String>,
}

/// Response for URL verification challenge.
#[derive(Serialize)]
struct ChallengeResponse {
    challenge: String,
}

/// Acknowledgement response for event callbacks.
#[derive(Serialize)]
struct AckResponse {
    ok: bool,
}

/// App-scoped Slack state (no auth required).
#[derive(Clone)]
pub struct SlackState {
    pub db: Arc<StorageBackend>,
    pub app_service: Arc<AppService>,
    pub session_service: Arc<SessionService>,
    pub message_service: Arc<MessageService>,
}

impl SlackState {
    pub fn new(db: Arc<StorageBackend>, runner: Arc<dyn AgentRunner>) -> Self {
        Self {
            app_service: Arc::new(AppService::new(db.clone())),
            session_service: Arc::new(SessionService::new(db.clone())),
            message_service: Arc::new(MessageService::new(db.clone(), runner)),
            db,
        }
    }
}

/// Create Slack webhook routes (no auth middleware).
pub fn routes(state: SlackState) -> Router {
    Router::new()
        .route("/v1/apps/{app_id}/slack/events", post(handle_slack_event))
        .with_state(state)
}

/// POST /v1/apps/{app_id}/slack/events — Slack Events API webhook
///
/// This endpoint handles:
/// 1. URL verification challenges (Slack sends these when setting up the webhook)
/// 2. Event callbacks (messages, mentions, etc.)
///
/// Security: Verified via Slack signing secret (HMAC-SHA256), not API key auth.
async fn handle_slack_event(
    State(state): State<SlackState>,
    Path(app_id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, Json<ErrorResponse>)> {
    // 1. Look up app (unscoped — no org context for webhooks)
    let app = state
        .app_service
        .get_by_public_id_unscoped(&app_id)
        .await
        .map_err(|e| {
            tracing::error!(app_id = %app_id, error = %e, "Failed to lookup app for Slack webhook");
            ErrorResponse::new("Internal server error")
                .into_response(StatusCode::INTERNAL_SERVER_ERROR)
        })?
        .ok_or_else(|| ErrorResponse::new("App not found").into_response(StatusCode::NOT_FOUND))?;

    // 2. Verify app is published and is a Slack channel
    if app.status != AppStatus::Published {
        return Err(ErrorResponse::new("App is not published").into_response(StatusCode::FORBIDDEN));
    }
    if app.channel_type != ChannelType::Slack {
        return Err(
            ErrorResponse::new("App is not a Slack channel").into_response(StatusCode::BAD_REQUEST)
        );
    }

    // 3. Parse Slack channel config
    let slack_config: SlackChannelConfig = serde_json::from_value(app.channel_config.clone())
        .map_err(|e| {
            tracing::error!(app_id = %app_id, error = %e, "Invalid Slack channel config");
            ErrorResponse::new("Invalid Slack channel configuration")
                .into_response(StatusCode::INTERNAL_SERVER_ERROR)
        })?;

    // 4. Verify Slack signing secret
    verify_slack_signature(&headers, &body, &slack_config.signing_secret).map_err(|e| {
        tracing::warn!(app_id = %app_id, error = %e, "Slack signature verification failed");
        ErrorResponse::new("Invalid signature").into_response(StatusCode::UNAUTHORIZED)
    })?;

    // 5. Parse the event envelope
    let envelope: SlackEventEnvelope = serde_json::from_slice(&body).map_err(|e| {
        tracing::warn!(app_id = %app_id, error = %e, "Failed to parse Slack event");
        ErrorResponse::new("Invalid request body").into_response(StatusCode::BAD_REQUEST)
    })?;

    // 6. Handle based on event type
    match envelope.event_type.as_str() {
        "url_verification" => {
            let challenge = envelope.challenge.unwrap_or_default();
            tracing::info!(app_id = %app_id, "Slack URL verification challenge received");
            Ok((
                StatusCode::OK,
                Json(serde_json::to_value(ChallengeResponse { challenge }).unwrap()),
            ))
        }
        "event_callback" => {
            if let Some(event) = envelope.event {
                // Skip bot messages to avoid loops
                if event.bot_id.is_some() || event.subtype.as_deref() == Some("bot_message") {
                    tracing::debug!(app_id = %app_id, "Skipping bot message");
                    return Ok((StatusCode::OK, Json(ack_json())));
                }

                // Only handle "message" and "app_mention" events
                if event.event_type != "message" && event.event_type != "app_mention" {
                    tracing::debug!(app_id = %app_id, event_type = %event.event_type, "Ignoring non-message event");
                    return Ok((StatusCode::OK, Json(ack_json())));
                }

                let text = event.text.clone().unwrap_or_default();
                if text.is_empty() {
                    return Ok((StatusCode::OK, Json(ack_json())));
                }

                tracing::info!(
                    app_id = %app_id,
                    event_type = %event.event_type,
                    channel = ?event.channel,
                    user = ?event.user,
                    thread_ts = ?event.thread_ts,
                    "Slack message received"
                );

                // Process message in background (Slack requires 200 within 3 seconds)
                let state = state.clone();
                let app = app.clone();
                let slack_config = slack_config.clone();
                tokio::spawn(async move {
                    if let Err(e) = process_slack_message(&state, &app, &slack_config, &event).await
                    {
                        tracing::error!(
                            app_id = %app_id,
                            error = %e,
                            "Failed to process Slack message"
                        );
                    }
                });
            }

            Ok((StatusCode::OK, Json(ack_json())))
        }
        other => {
            tracing::debug!(app_id = %app_id, event_type = %other, "Unhandled Slack event type");
            Ok((StatusCode::OK, Json(ack_json())))
        }
    }
}

fn ack_json() -> serde_json::Value {
    serde_json::to_value(AckResponse { ok: true }).unwrap()
}

/// Process an incoming Slack message: find/create session, create message, wait for response.
async fn process_slack_message(
    state: &SlackState,
    app: &App,
    slack_config: &SlackChannelConfig,
    event: &SlackEvent,
) -> anyhow::Result<()> {
    let org_id = app.org_id;
    let text = event.text.clone().unwrap_or_default();

    // Resolve org_public_id
    let org_row = state
        .db
        .get_organization(org_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Organization not found for app"))?;
    let org_public_id = org_row.public_id;

    // Build session tags based on strategy
    let session_tags = build_session_tags(app, slack_config, event);

    // Find or create session
    let session = match state.db.find_session_by_tags(org_id, &session_tags).await? {
        Some(row) => {
            tracing::debug!(
                session_id = %row.id,
                tags = ?session_tags,
                "Found existing Slack session"
            );
            SessionService::row_to_session(row, &org_public_id)
        }
        None => {
            tracing::info!(
                tags = ?session_tags,
                "Creating new Slack session"
            );
            let title = build_session_title(slack_config, event);
            let req = CreateSessionRequest {
                harness_id: app.harness_id,
                agent_id: Some(app.agent_id),
                title: Some(title),
                tags: session_tags.clone(),
                model_id: None,
                capabilities: vec![],
                tools: vec![],
            };
            state
                .session_service
                .create(
                    org_id,
                    &org_public_id,
                    app.harness_id.uuid(),
                    Some(app.agent_id.uuid()),
                    Some(app.agent_id),
                    req,
                )
                .await?
        }
    };

    // Create user message (triggers agent workflow)
    let create_msg = CreateMessageRequest {
        message: InputMessage {
            role: MessageRole::User,
            content: vec![InputContentPart::text(text)],
        },
        controls: None,
        metadata: Some(
            [
                (
                    "slack_user".to_string(),
                    serde_json::Value::String(event.user.clone().unwrap_or_default()),
                ),
                (
                    "slack_channel".to_string(),
                    serde_json::Value::String(event.channel.clone().unwrap_or_default()),
                ),
                (
                    "slack_ts".to_string(),
                    serde_json::Value::String(event.ts.clone().unwrap_or_default()),
                ),
            ]
            .into_iter()
            .collect(),
        ),
        tags: None,
    };

    let message = state
        .message_service
        .create(
            org_id,
            app.harness_id.uuid(),
            Some(app.agent_id.uuid()),
            session.id.uuid(),
            create_msg,
        )
        .await?;

    tracing::info!(
        session_id = %session.id,
        message_id = %message.id,
        "Slack message routed to session"
    );

    // Wait for agent response and post back to Slack
    let bot_token = slack_config.bot_token.clone();
    let channel = event.channel.clone().unwrap_or_default();
    // Reply in thread: use thread_ts if available, otherwise the message ts
    let thread_ts = event
        .thread_ts
        .clone()
        .or_else(|| event.ts.clone())
        .unwrap_or_default();
    let session_id = session.id.uuid();
    let db = state.db.clone();

    tokio::spawn(async move {
        if let Err(e) =
            wait_and_post_response(&db, session_id, &bot_token, &channel, &thread_ts).await
        {
            tracing::error!(
                session_id = %session_id,
                error = %e,
                "Failed to post Slack response"
            );
        }
    });

    Ok(())
}

/// Build session tags for finding/creating sessions based on strategy.
fn build_session_tags(
    app: &App,
    slack_config: &SlackChannelConfig,
    event: &SlackEvent,
) -> Vec<String> {
    let mut tags = vec![format!("slack:app:{}", app.public_id)];

    match slack_config.session_strategy {
        SessionStrategy::PerThread => {
            // Use thread_ts if threaded, otherwise the message ts (new thread)
            let thread_id = event
                .thread_ts
                .as_deref()
                .or(event.ts.as_deref())
                .unwrap_or("unknown");
            tags.push(format!("slack:thread:{}", thread_id));
        }
        SessionStrategy::PerChannel => {
            let channel = event.channel.as_deref().unwrap_or("unknown");
            tags.push(format!("slack:channel:{}", channel));
        }
        SessionStrategy::PerUser => {
            let user = event.user.as_deref().unwrap_or("unknown");
            tags.push(format!("slack:user:{}", user));
        }
    }

    tags
}

/// Build a human-readable session title.
fn build_session_title(slack_config: &SlackChannelConfig, event: &SlackEvent) -> String {
    let channel = event.channel.as_deref().unwrap_or("unknown");
    match slack_config.session_strategy {
        SessionStrategy::PerThread => {
            let ts = event
                .thread_ts
                .as_deref()
                .or(event.ts.as_deref())
                .unwrap_or("?");
            format!("Slack thread {} in {}", ts, channel)
        }
        SessionStrategy::PerChannel => format!("Slack channel {}", channel),
        SessionStrategy::PerUser => {
            let user = event.user.as_deref().unwrap_or("unknown");
            format!("Slack user {} in {}", user, channel)
        }
    }
}

/// Wait for agent response events and post the result back to Slack.
///
/// Polls for output.message.completed events on the session, then sends
/// the agent's response text to the Slack channel/thread.
async fn wait_and_post_response(
    db: &StorageBackend,
    session_id: uuid::Uuid,
    bot_token: &str,
    channel: &str,
    thread_ts: &str,
) -> anyhow::Result<()> {
    use everruns_core::typed_id::{EventId, SessionId};

    let session_id_typed = SessionId::from_uuid(session_id);

    // Poll for output.message.completed events (max 120 seconds)
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(120);
    let mut since_id: Option<EventId> = None;
    let empty: Vec<String> = vec![];

    loop {
        if tokio::time::Instant::now() > deadline {
            tracing::warn!(session_id = %session_id, "Timed out waiting for agent response");
            break;
        }

        let events = db
            .list_events(session_id_typed, None, since_id, &empty, &empty)
            .await?;

        for event_row in &events {
            since_id = Some(event_row.id);

            if event_row.event_type == "output.message.completed" {
                // Extract the text from the event data
                if let Some(text) = extract_response_text(&event_row.data) {
                    post_to_slack(bot_token, channel, thread_ts, &text).await?;
                    return Ok(());
                }
            }

            // Also check for turn.completed or turn.failed as terminal states
            if event_row.event_type == "turn.completed" || event_row.event_type == "turn.failed" {
                tracing::debug!(
                    session_id = %session_id,
                    event_type = %event_row.event_type,
                    "Turn ended"
                );
                return Ok(());
            }
        }

        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }

    Ok(())
}

/// Extract text content from an output.message.completed event's data.
fn extract_response_text(data: &serde_json::Value) -> Option<String> {
    // The event data contains a message with content parts
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

/// Post a message to Slack using the Bot API.
async fn post_to_slack(
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

    // Reply in thread if we have a thread_ts
    if !thread_ts.is_empty() {
        payload["thread_ts"] = serde_json::Value::String(thread_ts.to_string());
    }

    let response = client
        .post("https://slack.com/api/chat.postMessage")
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
        tracing::error!(
            channel = channel,
            error = error,
            status = %status,
            "Failed to post message to Slack"
        );
        return Err(anyhow::anyhow!("Slack API error: {}", error));
    }

    tracing::info!(channel = channel, "Posted response to Slack");
    Ok(())
}

/// Verify Slack request signature using HMAC-SHA256.
///
/// Slack signs requests with:
///   sig_basestring = "v0:{timestamp}:{body}"
///   signature = "v0=" + HMAC-SHA256(signing_secret, sig_basestring)
///
/// Headers used:
///   - X-Slack-Request-Timestamp
///   - X-Slack-Signature
fn verify_slack_signature(
    headers: &HeaderMap,
    body: &[u8],
    signing_secret: &str,
) -> Result<(), String> {
    let timestamp = headers
        .get("X-Slack-Request-Timestamp")
        .and_then(|v| v.to_str().ok())
        .ok_or("Missing X-Slack-Request-Timestamp header")?;

    let signature = headers
        .get("X-Slack-Signature")
        .and_then(|v| v.to_str().ok())
        .ok_or("Missing X-Slack-Signature header")?;

    // Reject requests older than 5 minutes to prevent replay attacks
    if let Ok(ts) = timestamp.parse::<i64>() {
        let now = chrono::Utc::now().timestamp();
        if (now - ts).unsigned_abs() > 300 {
            return Err("Request timestamp too old".to_string());
        }
    }

    // Compute expected signature
    let sig_basestring = format!("v0:{}:{}", timestamp, String::from_utf8_lossy(body));
    let mut mac = HmacSha256::new_from_slice(signing_secret.as_bytes())
        .map_err(|e| format!("HMAC key error: {}", e))?;
    mac.update(sig_basestring.as_bytes());
    let expected = format!("v0={}", hex::encode(mac.finalize().into_bytes()));

    // Constant-time comparison
    if expected.len() != signature.len() {
        return Err("Signature mismatch".to_string());
    }
    let matches = expected
        .as_bytes()
        .iter()
        .zip(signature.as_bytes())
        .fold(0u8, |acc, (a, b)| acc | (a ^ b));
    if matches != 0 {
        return Err("Signature mismatch".to_string());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn make_signature(secret: &str, timestamp: &str, body: &str) -> String {
        let sig_basestring = format!("v0:{}:{}", timestamp, body);
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(sig_basestring.as_bytes());
        format!("v0={}", hex::encode(mac.finalize().into_bytes()))
    }

    #[test]
    fn test_verify_slack_signature_valid() {
        let secret = "test_signing_secret";
        let body = r#"{"type":"url_verification","challenge":"abc123"}"#;
        let timestamp = chrono::Utc::now().timestamp().to_string();
        let signature = make_signature(secret, &timestamp, body);

        let mut headers = HeaderMap::new();
        headers.insert(
            "X-Slack-Request-Timestamp",
            HeaderValue::from_str(&timestamp).unwrap(),
        );
        headers.insert(
            "X-Slack-Signature",
            HeaderValue::from_str(&signature).unwrap(),
        );

        assert!(verify_slack_signature(&headers, body.as_bytes(), secret).is_ok());
    }

    #[test]
    fn test_verify_slack_signature_invalid() {
        let secret = "test_signing_secret";
        let body = r#"{"type":"url_verification","challenge":"abc123"}"#;
        let timestamp = chrono::Utc::now().timestamp().to_string();

        let mut headers = HeaderMap::new();
        headers.insert(
            "X-Slack-Request-Timestamp",
            HeaderValue::from_str(&timestamp).unwrap(),
        );
        headers.insert("X-Slack-Signature", HeaderValue::from_static("v0=deadbeef"));

        assert!(verify_slack_signature(&headers, body.as_bytes(), secret).is_err());
    }

    #[test]
    fn test_verify_slack_signature_missing_headers() {
        let headers = HeaderMap::new();
        assert!(verify_slack_signature(&headers, b"body", "secret").is_err());
    }

    #[test]
    fn test_verify_slack_signature_old_timestamp() {
        let secret = "test_signing_secret";
        let body = "body";
        let old_timestamp = (chrono::Utc::now().timestamp() - 600).to_string();
        let signature = make_signature(secret, &old_timestamp, body);

        let mut headers = HeaderMap::new();
        headers.insert(
            "X-Slack-Request-Timestamp",
            HeaderValue::from_str(&old_timestamp).unwrap(),
        );
        headers.insert(
            "X-Slack-Signature",
            HeaderValue::from_str(&signature).unwrap(),
        );

        assert!(verify_slack_signature(&headers, body.as_bytes(), secret).is_err());
    }

    #[test]
    fn test_slack_event_envelope_url_verification() {
        let json = r#"{"type":"url_verification","challenge":"test_challenge_123"}"#;
        let envelope: SlackEventEnvelope = serde_json::from_str(json).unwrap();
        assert_eq!(envelope.event_type, "url_verification");
        assert_eq!(envelope.challenge.unwrap(), "test_challenge_123");
    }

    #[test]
    fn test_slack_event_envelope_event_callback() {
        let json = r#"{
            "type": "event_callback",
            "team_id": "T0123456789",
            "event": {
                "type": "message",
                "user": "U0123456789",
                "text": "Hello bot",
                "channel": "C0123456789",
                "ts": "1234567890.123456",
                "thread_ts": "1234567890.000000"
            }
        }"#;
        let envelope: SlackEventEnvelope = serde_json::from_str(json).unwrap();
        assert_eq!(envelope.event_type, "event_callback");
        let event = envelope.event.unwrap();
        assert_eq!(event.event_type, "message");
        assert_eq!(event.user.unwrap(), "U0123456789");
        assert_eq!(event.text.unwrap(), "Hello bot");
        assert_eq!(event.channel.unwrap(), "C0123456789");
        assert!(event.thread_ts.is_some());
    }

    #[test]
    fn test_slack_event_bot_message_detected() {
        let json = r#"{
            "type": "event_callback",
            "event": {
                "type": "message",
                "bot_id": "B0123456789",
                "text": "I am a bot",
                "channel": "C0123456789",
                "ts": "1234567890.123456"
            }
        }"#;
        let envelope: SlackEventEnvelope = serde_json::from_str(json).unwrap();
        let event = envelope.event.unwrap();
        assert!(event.bot_id.is_some());
    }

    #[test]
    fn test_slack_channel_config_deserialization() {
        let json = r#"{
            "signing_secret": "abc123",
            "bot_token": "xoxb-token",
            "channel_id": "C0123456789",
            "team_id": "T0123456789",
            "session_strategy": "per_thread"
        }"#;
        let config: SlackChannelConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.signing_secret, "abc123");
        assert_eq!(config.bot_token, "xoxb-token");
        assert_eq!(config.channel_id.unwrap(), "C0123456789");
        assert_eq!(config.session_strategy, SessionStrategy::PerThread);
    }

    #[test]
    fn test_slack_channel_config_defaults() {
        let json = r#"{"signing_secret": "sec", "bot_token": "tok"}"#;
        let config: SlackChannelConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.session_strategy, SessionStrategy::PerThread);
        assert!(config.channel_id.is_none());
        assert!(config.team_id.is_none());
    }

    #[test]
    fn test_build_session_tags_per_thread() {
        let app = test_app();
        let config = test_config(SessionStrategy::PerThread);
        let event = test_event("C123", Some("1234.5678"), Some("1234.0000"));

        let tags = build_session_tags(&app, &config, &event);
        assert_eq!(tags.len(), 2);
        assert!(tags[0].starts_with("slack:app:"));
        assert_eq!(tags[1], "slack:thread:1234.0000"); // uses thread_ts
    }

    #[test]
    fn test_build_session_tags_per_thread_no_thread_ts() {
        let app = test_app();
        let config = test_config(SessionStrategy::PerThread);
        let event = test_event("C123", Some("1234.5678"), None);

        let tags = build_session_tags(&app, &config, &event);
        assert_eq!(tags[1], "slack:thread:1234.5678"); // falls back to ts
    }

    #[test]
    fn test_build_session_tags_per_channel() {
        let app = test_app();
        let config = test_config(SessionStrategy::PerChannel);
        let event = test_event("C123", Some("1234.5678"), None);

        let tags = build_session_tags(&app, &config, &event);
        assert_eq!(tags[1], "slack:channel:C123");
    }

    #[test]
    fn test_build_session_tags_per_user() {
        let app = test_app();
        let config = test_config(SessionStrategy::PerUser);
        let mut event = test_event("C123", Some("1234.5678"), None);
        event.user = Some("U999".to_string());

        let tags = build_session_tags(&app, &config, &event);
        assert_eq!(tags[1], "slack:user:U999");
    }

    #[test]
    fn test_extract_response_text() {
        let data = serde_json::json!({
            "message": {
                "content": [
                    {"type": "text", "text": "Hello from the agent!"},
                    {"type": "text", "text": "More text."}
                ]
            }
        });
        let text = extract_response_text(&data);
        assert_eq!(text.unwrap(), "Hello from the agent!\nMore text.");
    }

    #[test]
    fn test_extract_response_text_no_message() {
        let data = serde_json::json!({});
        assert!(extract_response_text(&data).is_none());
    }

    #[test]
    fn test_extract_response_text_empty_content() {
        let data = serde_json::json!({"message": {"content": []}});
        assert!(extract_response_text(&data).is_none());
    }

    // Test helpers
    fn test_app() -> App {
        use everruns_core::typed_id::{AgentId, AppId, HarnessId};

        App {
            public_id: AppId::from_uuid(uuid::Uuid::nil()),
            internal_id: uuid::Uuid::nil(),
            org_id: 1,
            name: "Test App".to_string(),
            description: None,
            harness_id: HarnessId::from_uuid(uuid::Uuid::nil()),
            agent_id: AgentId::from_uuid(uuid::Uuid::nil()),
            channel_type: ChannelType::Slack,
            channel_config: serde_json::json!({}),
            status: AppStatus::Published,
            published_at: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    fn test_config(strategy: SessionStrategy) -> SlackChannelConfig {
        SlackChannelConfig {
            signing_secret: "secret".to_string(),
            bot_token: "xoxb-token".to_string(),
            channel_id: None,
            team_id: None,
            session_strategy: strategy,
        }
    }

    fn test_event(channel: &str, ts: Option<&str>, thread_ts: Option<&str>) -> SlackEvent {
        SlackEvent {
            event_type: "message".to_string(),
            user: None,
            text: Some("Hello".to_string()),
            channel: Some(channel.to_string()),
            thread_ts: thread_ts.map(String::from),
            ts: ts.map(String::from),
            bot_id: None,
            subtype: None,
        }
    }
}

// Slack ingestion API — app-scoped webhook endpoint
//
// Design Decision: Slack webhooks are app-scoped (POST /v1/apps/{app_id}/slack/events)
// because Slack is bound to an App which defines the agent, harness, signing secret,
// and session strategy. This endpoint is unauthenticated (no API key) — security
// comes from Slack signing secret verification (HMAC-SHA256).
//
// Design Decision: No auth middleware on this route. The app_id in the URL identifies
// the app; the signing secret in channel_config verifies the request origin.

use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::post,
};
use everruns_core::{AppStatus, ChannelType, SlackChannelConfig};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::sync::Arc;

use crate::services::AppService;
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
#[derive(Debug, Deserialize)]
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
    pub app_service: Arc<AppService>,
}

impl SlackState {
    pub fn new(db: Arc<StorageBackend>) -> Self {
        Self {
            app_service: Arc::new(AppService::new(db)),
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
            // Slack URL verification challenge — respond with the challenge token
            let challenge = envelope.challenge.unwrap_or_default();
            tracing::info!(app_id = %app_id, "Slack URL verification challenge received");
            Ok((
                StatusCode::OK,
                Json(serde_json::to_value(ChallengeResponse { challenge }).unwrap()),
            ))
        }
        "event_callback" => {
            // Acknowledge immediately (Slack requires 200 within 3 seconds)
            if let Some(event) = &envelope.event {
                // Skip bot messages to avoid loops
                if event.bot_id.is_some() || event.subtype.as_deref() == Some("bot_message") {
                    tracing::debug!(app_id = %app_id, "Skipping bot message");
                    return Ok((
                        StatusCode::OK,
                        Json(serde_json::to_value(AckResponse { ok: true }).unwrap()),
                    ));
                }

                tracing::info!(
                    app_id = %app_id,
                    event_type = %event.event_type,
                    channel = ?event.channel,
                    user = ?event.user,
                    thread_ts = ?event.thread_ts,
                    "Slack event received"
                );

                // TODO: Route to session based on session_strategy and create message
                // This will be implemented when we wire up session creation and message handling.
                // For now, we acknowledge the event to prevent Slack retries.
            }

            Ok((
                StatusCode::OK,
                Json(serde_json::to_value(AckResponse { ok: true }).unwrap()),
            ))
        }
        other => {
            tracing::debug!(app_id = %app_id, event_type = %other, "Unhandled Slack event type");
            Ok((
                StatusCode::OK,
                Json(serde_json::to_value(AckResponse { ok: true }).unwrap()),
            ))
        }
    }
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
        assert_eq!(
            config.session_strategy,
            everruns_core::SessionStrategy::PerThread
        );
    }

    #[test]
    fn test_slack_channel_config_defaults() {
        let json = r#"{"signing_secret": "sec", "bot_token": "tok"}"#;
        let config: SlackChannelConfig = serde_json::from_str(json).unwrap();
        assert_eq!(
            config.session_strategy,
            everruns_core::SessionStrategy::PerThread
        );
        assert!(config.channel_id.is_none());
        assert!(config.team_id.is_none());
    }
}

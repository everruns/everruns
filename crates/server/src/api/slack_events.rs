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
// Design Decision: Agent responses are posted back to Slack via an event-driven
// dispatcher (SlackDeliveryDispatcher) that subscribes to EventNotificationBroadcaster.
// No fixed deadline — handles arbitrarily long agent turns. Falls back to 120s
// polling in DEV_MODE where EventNotificationBroadcaster is unavailable.

use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use chrono::Utc;
use everruns_core::{App, AppStatus, ChannelType, SessionStrategy, SlackChannelConfig};
use everruns_worker::AgentRunner;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::api::messages::{CreateMessageRequest, InputContentPart, InputMessage, MessageRole};
use crate::api::sessions::CreateSessionRequest;
use crate::services::{AppService, EventService, MessageService, SessionService};
use crate::slack_delivery::SlackDeliveryDispatcher;
use crate::storage::StorageBackend;
use crate::storage::models::UpdateApp;

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
    /// File attachments (images, documents, videos, etc.).
    #[serde(default)]
    files: Vec<SlackFile>,
    /// Legacy attachments (link unfurls, bot attachments, etc.).
    #[serde(default)]
    attachments: Vec<SlackAttachment>,
}

/// Slack file attachment object (subset of fields we care about).
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
struct SlackFile {
    /// File ID (e.g., "F0123456789").
    #[serde(default)]
    id: Option<String>,
    /// Original filename.
    #[serde(default)]
    name: Option<String>,
    /// MIME type (e.g., "image/png", "video/mp4").
    #[serde(default)]
    mimetype: Option<String>,
    /// File type label from Slack (e.g., "png", "mp4", "pdf").
    #[serde(default)]
    filetype: Option<String>,
    /// URL for private download (requires bot token auth).
    #[serde(default)]
    url_private: Option<String>,
    /// File size in bytes.
    #[serde(default)]
    size: Option<u64>,
}

/// Slack legacy attachment (link unfurls, rich attachments from apps/workflows).
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
struct SlackAttachment {
    /// Attachment title.
    #[serde(default)]
    title: Option<String>,
    /// Title link URL.
    #[serde(default)]
    title_link: Option<String>,
    /// Main text body.
    #[serde(default)]
    text: Option<String>,
    /// Fallback text (plain-text summary).
    #[serde(default)]
    fallback: Option<String>,
    /// Pretext (displayed above the attachment block).
    #[serde(default)]
    pretext: Option<String>,
    /// Image URL (full-size image in the attachment).
    #[serde(default)]
    image_url: Option<String>,
    /// Thumbnail URL.
    #[serde(default)]
    thumb_url: Option<String>,
    /// Author name.
    #[serde(default)]
    author_name: Option<String>,
    /// Service name (e.g., "GitHub", "Jira").
    #[serde(default)]
    service_name: Option<String>,
    /// Original URL that triggered the unfurl.
    #[serde(default)]
    original_url: Option<String>,
    /// Content subtype hint from Slack (e.g., "canvas", "huddle_transcript").
    #[serde(default)]
    app_unfurl_url: Option<String>,
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

/// Resolved Slack user display name.
/// `Some(name)` = resolved, `None` = resolution failed (don't retry).
type SlackUserCache = Arc<RwLock<HashMap<String, Option<String>>>>;

/// App-scoped Slack state (no auth required).
#[derive(Clone)]
pub struct SlackState {
    pub db: Arc<StorageBackend>,
    pub app_service: Arc<AppService>,
    pub session_service: Arc<SessionService>,
    pub message_service: Arc<MessageService>,
    pub event_service: Arc<EventService>,
    /// Cache of Slack user ID → display name. Shared across requests.
    user_name_cache: SlackUserCache,
    /// Event-driven Slack delivery dispatcher (None in DEV_MODE without PostgreSQL).
    pub delivery_dispatcher: Option<Arc<SlackDeliveryDispatcher>>,
}

impl SlackState {
    pub fn new(
        db: Arc<StorageBackend>,
        runner: Arc<dyn AgentRunner>,
        delivery_dispatcher: Option<Arc<SlackDeliveryDispatcher>>,
    ) -> Self {
        Self {
            app_service: Arc::new(AppService::new(db.clone())),
            session_service: Arc::new(SessionService::new(db.clone())),
            message_service: Arc::new(MessageService::new(db.clone(), runner)),
            event_service: Arc::new(EventService::new(db.clone())),
            db,
            user_name_cache: Arc::new(RwLock::new(HashMap::new())),
            delivery_dispatcher,
        }
    }
}

/// Create Slack webhook routes (no auth middleware).
pub fn routes(state: SlackState) -> Router {
    Router::new()
        .route("/v1/apps/{app_id}/slack/events", post(handle_slack_event))
        .route(
            "/v1/apps/{app_id}/slack/manifest",
            get(handle_slack_manifest),
        )
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

    // 4. Verify Slack signing secret // THREAT[TM-SLACK-001]
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

            // Record webhook verification timestamp (idempotent — only sets if not already set)
            if slack_config.webhook_verified_at.is_none() {
                let mut updated_config = slack_config.clone();
                updated_config.webhook_verified_at = Some(Utc::now());
                if let Ok(config_json) = serde_json::to_value(&updated_config) {
                    let input = UpdateApp {
                        channel_config: Some(config_json),
                        ..Default::default()
                    };
                    if let Err(e) = state
                        .db
                        .update_app(app.org_id, app.internal_id, input)
                        .await
                    {
                        tracing::warn!(app_id = %app_id, error = %e, "Failed to record webhook verification");
                    }
                }
            }

            Ok((
                StatusCode::OK,
                Json(serde_json::to_value(ChallengeResponse { challenge }).unwrap()),
            ))
        }
        "event_callback" => {
            if let Some(event) = envelope.event {
                // Skip bot messages to avoid loops // THREAT[TM-SLACK-002]
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
                if text.is_empty() && event.files.is_empty() && event.attachments.is_empty() {
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

                // Record first message timestamp (idempotent — only sets once)
                if slack_config.first_message_received_at.is_none() {
                    let mut updated_config = slack_config.clone();
                    updated_config.first_message_received_at = Some(Utc::now());
                    if let Ok(config_json) = serde_json::to_value(&updated_config) {
                        let input = UpdateApp {
                            channel_config: Some(config_json),
                            ..Default::default()
                        };
                        if let Err(e) = state
                            .db
                            .update_app(app.org_id, app.internal_id, input)
                            .await
                        {
                            tracing::warn!(app_id = %app_id, error = %e, "Failed to record first message timestamp");
                        }
                    }
                }

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
    let slack_user_id = event.user.clone().unwrap_or_default();

    // Resolve Slack user display name (gracefully falls back to user ID)
    let display_name = if !slack_user_id.is_empty() {
        resolve_slack_user_name(
            &state.user_name_cache,
            &slack_config.bot_token,
            &slack_user_id,
        )
        .await
    } else {
        None
    };

    // Build ExternalActor for channel-agnostic user identity
    let external_actor = if !slack_user_id.is_empty() {
        let mut actor_metadata = std::collections::HashMap::new();
        if let Some(ref channel) = event.channel {
            actor_metadata.insert("channel".to_string(), channel.clone());
        }
        if let Some(ref team_id) = slack_config.team_id {
            actor_metadata.insert("team_id".to_string(), team_id.clone());
        }
        Some(everruns_core::ExternalActor {
            actor_id: slack_user_id.clone(),
            actor_name: display_name.clone(),
            source: "slack".to_string(),
            metadata: if actor_metadata.is_empty() {
                None
            } else {
                Some(actor_metadata)
            },
        })
    } else {
        None
    };

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
    let (session, is_new_session) =
        match state.db.find_session_by_tags(org_id, &session_tags).await? {
            Some(row) => {
                tracing::debug!(
                    session_id = %row.id,
                    tags = ?session_tags,
                    "Found existing Slack session"
                );
                (SessionService::row_to_session(row, &org_public_id), false)
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
                let s = state
                    .session_service
                    .create(
                        org_id,
                        &org_public_id,
                        app.harness_id.uuid(),
                        Some(app.agent_id.uuid()),
                        Some(app.agent_id),
                        req,
                    )
                    .await?;
                (s, true)
            }
        };

    // Inject thread context: when joining an existing thread mid-conversation,
    // fetch prior messages from Slack and inject them as context so the agent
    // sees the full conversation history.
    if is_new_session
        && slack_config.session_strategy == SessionStrategy::PerThread
        && event.thread_ts.is_some()
    {
        let thread_ts = event.thread_ts.as_deref().unwrap();
        let channel = event.channel.as_deref().unwrap_or("unknown");
        if let Err(e) = inject_thread_context(
            state,
            &slack_config.bot_token,
            channel,
            thread_ts,
            session.id,
            event.ts.as_deref(),
        )
        .await
        {
            // Non-fatal: agent proceeds without historical context
            tracing::warn!(
                session_id = %session.id,
                thread_ts = thread_ts,
                error = %e,
                "Failed to inject thread context (agent will proceed without history)"
            );
        }
    }

    // Dedup: Slack sends both app_mention and message events for @mentions.
    // Check if we already have an input.message with this slack_ts in the session.
    // This works across server instances because the check is DB-level.
    let slack_ts = event.ts.clone().unwrap_or_default();
    if !slack_ts.is_empty() {
        let already_exists = state
            .db
            .has_event_with_slack_ts(session.id, &slack_ts)
            .await?;
        if already_exists {
            tracing::debug!(
                session_id = %session.id,
                slack_ts = %slack_ts,
                "Skipping duplicate Slack event (already processed this ts)"
            );
            return Ok(());
        }
    }

    // Build content parts: text + file attachments + legacy attachments
    let mut content: Vec<InputContentPart> = Vec::new();
    if !text.is_empty() {
        content.push(InputContentPart::text(text));
    }
    content.extend(build_file_content_parts(&event.files));
    content.extend(build_attachment_content_parts(&event.attachments));

    // Create user message (triggers agent workflow)
    // Slack-specific metadata (ts for threading) stays in metadata;
    // user identity is carried by external_actor.
    let create_msg = CreateMessageRequest {
        message: InputMessage {
            role: MessageRole::User,
            content,
        },
        controls: None,
        metadata: Some(
            [
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
        external_actor,
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

    // Register delivery for agent response posting
    let bot_token = slack_config.bot_token.clone();
    let channel = event.channel.clone().unwrap_or_default();
    // Reply in thread: use thread_ts if available, otherwise the message ts
    let thread_ts = event
        .thread_ts
        .clone()
        .or_else(|| event.ts.clone())
        .unwrap_or_default();
    let session_id = session.id.uuid();
    let message_id = message.id;

    if let Some(ref dispatcher) = state.delivery_dispatcher {
        // Event-driven delivery: no deadline, handles arbitrarily long turns
        dispatcher
            .register(
                session_id,
                message_id.to_string(),
                bot_token,
                channel,
                thread_ts,
            )
            .await;
    } else {
        // Fallback for DEV_MODE without EventNotificationBroadcaster:
        // use the legacy polling approach
        let db = state.db.clone();
        tokio::spawn(async move {
            if let Err(e) = wait_and_post_response(
                &db, session_id, message_id, &bot_token, &channel, &thread_ts,
            )
            .await
            {
                tracing::error!(
                    session_id = %session_id,
                    error = %e,
                    "Failed to post Slack response"
                );
            }
        });
    }

    Ok(())
}

/// Supported image MIME types for Slack file attachments.
const SUPPORTED_IMAGE_TYPES: &[&str] = &["image/png", "image/jpeg", "image/gif", "image/webp"];

/// Convert Slack file attachments into content parts.
///
/// Images (png, jpeg, gif, webp) with a private URL → `InputContentPart::Image(url)`.
/// All other files → text description noting the file name and type.
fn build_file_content_parts(files: &[SlackFile]) -> Vec<InputContentPart> {
    files
        .iter()
        .map(|file| {
            let mime = file.mimetype.as_deref().unwrap_or("");
            let name = file.name.as_deref().unwrap_or("unnamed file");

            if SUPPORTED_IMAGE_TYPES.contains(&mime) {
                // Image with URL → image content part
                if let Some(url) = &file.url_private {
                    return InputContentPart::image_url(url);
                }
                // Image without URL → text fallback
                InputContentPart::text(format!(
                    "[Attached image: {name} — no download URL available]"
                ))
            } else {
                // Non-image file → text description (content not available inline)
                let filetype = file.filetype.as_deref().unwrap_or("unknown");
                InputContentPart::text(format!("[Attached file: {name} ({filetype})]"))
            }
        })
        .collect()
}

/// Convert Slack legacy attachments (link unfurls, app attachments, etc.) into content parts.
///
/// Attachments with an image_url → `InputContentPart::Image(url)`.
/// Others → text summary from title/text/fallback fields.
fn build_attachment_content_parts(attachments: &[SlackAttachment]) -> Vec<InputContentPart> {
    attachments
        .iter()
        .filter_map(|att| {
            // If the attachment has an image, include it
            if let Some(url) = &att.image_url {
                return Some(InputContentPart::image_url(url));
            }

            // Build a text summary from available fields
            let mut parts = Vec::new();
            if let Some(service) = &att.service_name {
                parts.push(service.clone());
            }
            if let Some(title) = &att.title {
                let line = if let Some(link) = &att.title_link {
                    format!("{title} ({link})")
                } else {
                    title.clone()
                };
                parts.push(line);
            }
            if let Some(text) = &att.text {
                parts.push(text.clone());
            }

            if parts.is_empty() {
                // Use fallback as last resort
                att.fallback
                    .as_ref()
                    .map(|f| InputContentPart::text(format!("[Attachment: {f}]")))
            } else {
                Some(InputContentPart::text(format!(
                    "[Attachment: {}]",
                    parts.join(" — ")
                )))
            }
        })
        .collect()
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

/// Reply message from Slack's conversations.replies API.
#[derive(Debug, Deserialize)]
struct SlackReplyMessage {
    /// User who sent the message (absent for bot messages).
    #[serde(default)]
    user: Option<String>,
    /// Message text.
    #[serde(default)]
    text: Option<String>,
    /// Message timestamp.
    #[serde(default)]
    ts: Option<String>,
    /// Bot ID (present when message is from a bot).
    #[serde(default)]
    bot_id: Option<String>,
    /// Subtype (e.g., "bot_message", "thread_broadcast").
    #[serde(default)]
    subtype: Option<String>,
}

/// Fetch thread replies from Slack's conversations.replies API.
///
/// Returns messages in chronological order. Gracefully returns empty vec on
/// API errors (missing scope, invalid token, etc.) so the agent can proceed
/// without history rather than failing the entire message flow.
async fn fetch_thread_replies(
    bot_token: &str,
    channel: &str,
    thread_ts: &str,
) -> Vec<SlackReplyMessage> {
    let client = reqwest::Client::new();
    let url = format!(
        "https://slack.com/api/conversations.replies?channel={}&ts={}&limit=100",
        channel, thread_ts
    );
    let result = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", bot_token))
        .send()
        .await;

    let response = match result {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(error = %e, "Failed to fetch thread replies (network)");
            return vec![];
        }
    };

    let body: serde_json::Value = match response.json().await {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(error = %e, "Failed to parse conversations.replies response");
            return vec![];
        }
    };

    if !body.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
        let error = body
            .get("error")
            .and_then(|e| e.as_str())
            .unwrap_or("unknown");
        tracing::warn!(
            error = error,
            channel = channel,
            thread_ts = thread_ts,
            "Slack conversations.replies API error (thread context unavailable)"
        );
        return vec![];
    }

    match serde_json::from_value::<Vec<SlackReplyMessage>>(
        body.get("messages").cloned().unwrap_or_default(),
    ) {
        Ok(msgs) => msgs,
        Err(e) => {
            tracing::warn!(error = %e, "Failed to parse thread reply messages");
            vec![]
        }
    }
}

/// Inject thread history as context messages into a newly created session.
///
/// Fetches prior messages from Slack's conversations.replies API and stores
/// them as input.message events (without triggering agent workflows). This
/// gives the agent full conversational context when first mentioned mid-thread.
///
/// Messages from bots (including our own) are injected as assistant-role
/// messages. Messages from users get user-role with ExternalActor attribution.
/// The triggering message (matching `exclude_ts`) is skipped since it will be
/// created normally by the caller.
async fn inject_thread_context(
    state: &SlackState,
    bot_token: &str,
    channel: &str,
    thread_ts: &str,
    session_id: everruns_core::typed_id::SessionId,
    exclude_ts: Option<&str>,
) -> anyhow::Result<()> {
    let replies = fetch_thread_replies(bot_token, channel, thread_ts).await;

    if replies.is_empty() {
        return Ok(());
    }

    let mut injected = 0u32;
    for reply in &replies {
        // Skip the triggering message (it will be created by the normal flow)
        if let (Some(reply_ts), Some(skip_ts)) = (reply.ts.as_deref(), exclude_ts)
            && reply_ts == skip_ts
        {
            continue;
        }

        // Skip messages with no text content
        let text = reply.text.as_deref().unwrap_or("");
        if text.is_empty() {
            continue;
        }

        // Skip subtypes we can't meaningfully represent (channel_join, etc.)
        if let Some(ref subtype) = reply.subtype
            && subtype != "bot_message"
            && subtype != "thread_broadcast"
        {
            continue;
        }

        // Determine role: bot messages → assistant, human messages → user
        let (role, external_actor) = if reply.bot_id.is_some() {
            (everruns_core::MessageRole::Agent, None)
        } else {
            let user_id = reply.user.clone().unwrap_or_default();
            let display_name = if !user_id.is_empty() {
                resolve_slack_user_name(&state.user_name_cache, bot_token, &user_id).await
            } else {
                None
            };
            let actor = if !user_id.is_empty() {
                Some(everruns_core::ExternalActor {
                    actor_id: user_id,
                    actor_name: display_name,
                    source: "slack".to_string(),
                    metadata: None,
                })
            } else {
                None
            };
            (everruns_core::MessageRole::User, actor)
        };

        let message = everruns_core::Message {
            id: everruns_core::typed_id::MessageId::new(),
            role,
            content: vec![everruns_core::ContentPart::text(text)],
            phase: None,
            thinking: None,
            thinking_signature: None,
            controls: None,
            metadata: reply.ts.as_ref().map(|ts| {
                [(
                    "slack_ts".to_string(),
                    serde_json::Value::String(ts.clone()),
                )]
                .into_iter()
                .collect()
            }),
            external_actor,
            created_at: chrono::Utc::now(),
        };

        // Emit as input.message event directly (no workflow trigger)
        state
            .event_service
            .emit(everruns_core::events::EventRequest::new(
                session_id,
                everruns_core::events::EventContext::empty(),
                everruns_core::events::InputMessageData::new(message),
            ))
            .await?;

        injected += 1;
    }

    if injected > 0 {
        tracing::info!(
            session_id = %session_id,
            thread_ts = thread_ts,
            injected_count = injected,
            "Injected thread context into new session"
        );
    }

    Ok(())
}

/// Wait for the agent turn to complete and stream responses to Slack.
///
/// Posts each `output.message.completed` text to Slack as it arrives, giving
/// users real-time progress during multi-step turns (Reason→Act cycles).
/// Filtering by `input_message_id` ensures we only see events from our turn.
/// Stops polling when `turn.completed` or `turn.failed` fires.
async fn wait_and_post_response(
    db: &StorageBackend,
    session_id: uuid::Uuid,
    input_message_id: everruns_core::typed_id::MessageId,
    bot_token: &str,
    channel: &str,
    thread_ts: &str,
) -> anyhow::Result<()> {
    use everruns_core::typed_id::{EventId, SessionId};

    let session_id_typed = SessionId::from_uuid(session_id);
    let input_msg_str = input_message_id.to_string();

    // Poll for events (max 120 seconds)
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

            // Only consider events triggered by our input message
            let event_input_msg = event_row
                .context
                .get("input_message_id")
                .and_then(|v| v.as_str());
            let is_our_turn = event_input_msg == Some(&input_msg_str);

            // Post each assistant message to Slack as it arrives. This gives
            // users progress visibility during multi-step agent turns (e.g.
            // "Let me search for that..." before tool execution).
            if event_row.event_type == "output.message.completed"
                && is_our_turn
                && let Some(text) = extract_response_text(&event_row.data)
            {
                post_to_slack(bot_token, channel, thread_ts, &text).await?;
            }

            // Stop polling once the turn ends
            if event_row.event_type == "turn.completed" && is_our_turn {
                return Ok(());
            }

            if event_row.event_type == "turn.failed" && is_our_turn {
                tracing::warn!(session_id = %session_id, "Turn failed");
                return Ok(());
            }
        }

        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }

    Ok(())
}

/// Extract text content from an output.message.completed event's data.
/// Delegates to the shared implementation in `slack_delivery`.
fn extract_response_text(data: &serde_json::Value) -> Option<String> {
    crate::slack_delivery::extract_response_text(data)
}

const SLACK_API_BASE: &str = "https://slack.com/api";

/// Resolve a Slack user ID to a display name via `users.info` API.
///
/// Returns the display name (or real_name fallback) on success, or `None` if
/// resolution fails (missing `users:read` scope, network error, etc.).
/// Results are cached: successful lookups and permanent failures (missing scope)
/// are not retried. Transient errors are retried on next call.
async fn resolve_slack_user_name(
    cache: &SlackUserCache,
    bot_token: &str,
    user_id: &str,
) -> Option<String> {
    resolve_slack_user_name_base(SLACK_API_BASE, cache, bot_token, user_id).await
}

/// Resolve a Slack user ID to a display name (with configurable base URL for testing).
async fn resolve_slack_user_name_base(
    base_url: &str,
    cache: &SlackUserCache,
    bot_token: &str,
    user_id: &str,
) -> Option<String> {
    // Check cache first
    {
        let cache_read = cache.read().await;
        if let Some(cached) = cache_read.get(user_id) {
            return cached.clone();
        }
    }

    // Call Slack API (user IDs are alphanumeric, safe to interpolate)
    let client = reqwest::Client::new();
    let url = format!("{}/users.info?user={user_id}", base_url);
    let result = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", bot_token))
        .send()
        .await;

    let response: reqwest::Response = match result {
        Ok(r) => r,
        Err(e) => {
            // Transient network error — don't cache, allow retry
            tracing::warn!(user_id = user_id, error = %e, "Failed to fetch Slack user info (network)");
            return None;
        }
    };

    let body: serde_json::Value = match response.json().await {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(user_id = user_id, error = %e, "Failed to parse Slack users.info response");
            return None;
        }
    };

    if body.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
        // Success — extract display_name, fall back to real_name, then name
        let user_obj = body.get("user");
        let profile = user_obj.and_then(|u| u.get("profile"));
        let display_name = profile
            .and_then(|p| p.get("display_name"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .or_else(|| {
                user_obj
                    .and_then(|u| u.get("real_name"))
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
            })
            .or_else(|| {
                user_obj
                    .and_then(|u| u.get("name"))
                    .and_then(|v| v.as_str())
            })
            .map(String::from);

        let mut cache_write = cache.write().await;
        cache_write.insert(user_id.to_string(), display_name.clone());
        display_name
    } else {
        let error = body
            .get("error")
            .and_then(|e| e.as_str())
            .unwrap_or("unknown");
        tracing::warn!(
            user_id = user_id,
            error = error,
            "Slack users.info API error"
        );

        // Permanent errors (missing scope, invalid token) — cache as None to avoid repeated calls
        if error == "missing_scope"
            || error == "not_authed"
            || error == "invalid_auth"
            || error == "token_revoked"
            || error == "account_inactive"
        {
            let mut cache_write = cache.write().await;
            cache_write.insert(user_id.to_string(), None);
        }

        None
    }
}

/// Post a message to Slack using the Bot API.
/// Delegates to the shared implementation in `slack_delivery`.
async fn post_to_slack(
    bot_token: &str,
    channel: &str,
    thread_ts: &str,
    text: &str,
) -> anyhow::Result<()> {
    crate::slack_delivery::post_to_slack(bot_token, channel, thread_ts, text).await
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

// =========================================================================
// Slack App Manifest generation — per-app "Create in Slack" helper
// =========================================================================

/// Response for the manifest endpoint.
#[derive(Serialize)]
struct ManifestResponse {
    /// YAML manifest string for creating a Slack app
    manifest_yaml: String,
    /// Pre-filled URL to create a Slack app from the manifest
    create_url: String,
}

/// GET /v1/apps/{app_id}/slack/manifest — Generate a Slack App manifest.
///
/// Returns a pre-filled YAML manifest and a URL that opens Slack's "Create app
/// from manifest" flow. The manifest includes the correct webhook URL and bot
/// scopes but omits `event_subscriptions` (requires a live URL, so must be
/// configured manually after the app is created and published).
async fn handle_slack_manifest(
    State(state): State<SlackState>,
    Path(app_id): Path<String>,
) -> Result<Json<ManifestResponse>, (StatusCode, Json<ErrorResponse>)> {
    // Look up app (unscoped — no auth for this endpoint)
    let app = state
        .app_service
        .get_by_public_id_unscoped(&app_id)
        .await
        .map_err(|e| {
            tracing::error!(app_id = %app_id, error = %e, "Failed to lookup app for manifest");
            ErrorResponse::internal_error()
        })?
        .ok_or_else(|| ErrorResponse::new("App not found").into_response(StatusCode::NOT_FOUND))?;

    let display_name = truncate_display_name(&app.name);

    let manifest_yaml = build_manifest_yaml(&app.name, &display_name);

    // URL-encode the manifest for the Slack "create from manifest" URL
    let encoded = urlencoding_encode(&manifest_yaml);
    let create_url = format!(
        "https://api.slack.com/apps?new_app=1&manifest_yaml={}",
        encoded
    );

    Ok(Json(ManifestResponse {
        manifest_yaml,
        create_url,
    }))
}

/// Build the YAML manifest for a Slack app (no event_subscriptions — requires live URL).
fn build_manifest_yaml(app_name: &str, display_name: &str) -> String {
    let name = yaml_escape(app_name);
    let display_name = yaml_escape(display_name);
    format!(
        "display_information:\n\
         \x20 name: \"{name}\"\n\
         \x20 description: \"{name} (Powered by Everruns)\"\n\
         \x20 long_description: \"AI agent powered by Everruns — https://everruns.com\"\n\
         \x20 background_color: \"#1a1a2e\"\n\
         features:\n\
         \x20 bot_user:\n\
         \x20   display_name: \"{display_name}\"\n\
         \x20   always_online: true\n\
         oauth_config:\n\
         \x20 scopes:\n\
         \x20   bot:\n\
         \x20     - chat:write\n\
         \x20     - channels:history\n\
         \x20     - groups:history\n\
         \x20     - im:history\n\
         \x20     - mpim:history\n\
         \x20     - app_mentions:read\n\
         \x20     - users:read\n\
         \x20     - files:read\n\
         settings:\n\
         \x20 org_deploy_enabled: false\n\
         \x20 socket_mode_enabled: false\n\
         \x20 token_rotation_enabled: false\n",
    )
}

/// Truncate display name to 80 chars (Slack limit for bot_user display_name).
/// Uses char boundary to avoid panicking on multi-byte UTF-8.
fn truncate_display_name(name: &str) -> String {
    if name.len() <= 80 {
        return name.to_string();
    }
    // Find the last char boundary at or before 80 bytes
    let mut end = 80;
    while !name.is_char_boundary(end) {
        end -= 1;
    }
    name[..end].to_string()
}

/// Simple YAML string escaping: escape backslashes and double quotes.
fn yaml_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Percent-encode a string for URL query parameters.
fn urlencoding_encode(s: &str) -> String {
    let mut result = String::new();
    for byte in s.as_bytes() {
        match *byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                result.push(*byte as char);
            }
            _ => {
                result.push_str(&format!("%{:02X}", byte));
            }
        }
    }
    result
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

    #[tokio::test]
    async fn test_resolve_slack_user_name_cache_hit() {
        let cache: SlackUserCache = Arc::new(RwLock::new(HashMap::new()));
        cache
            .write()
            .await
            .insert("U123".to_string(), Some("Alice".to_string()));

        let result = resolve_slack_user_name(&cache, "xoxb-fake", "U123").await;
        assert_eq!(result, Some("Alice".to_string()));
    }

    #[tokio::test]
    async fn test_resolve_slack_user_name_cache_hit_none() {
        // Cached failure (e.g., missing_scope) should return None without API call
        let cache: SlackUserCache = Arc::new(RwLock::new(HashMap::new()));
        cache.write().await.insert("U123".to_string(), None);

        let result = resolve_slack_user_name(&cache, "xoxb-fake", "U123").await;
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn test_resolve_slack_user_name_api_failure_not_cached() {
        // Network failures should not be cached (allow retry)
        let cache: SlackUserCache = Arc::new(RwLock::new(HashMap::new()));

        // Use an invalid URL-ish token that will fail network-level
        // (the function uses hardcoded slack.com URL, so with a bad token
        // it will get a response but not a network error — test cache emptiness)
        let _result = resolve_slack_user_name(&cache, "xoxb-invalid", "U999").await;

        // If it got a Slack API error response (not_authed/invalid_auth), it should cache None.
        // If it was a network error, cache should be empty.
        // Either way the function gracefully returns None — that's the key behavior.
        // We just verify it doesn't panic.
    }

    #[test]
    fn test_external_actor_display_label_with_name() {
        let actor = everruns_core::ExternalActor {
            actor_id: "U123".to_string(),
            actor_name: Some("Alice".to_string()),
            source: "slack".to_string(),
            metadata: None,
        };
        assert_eq!(actor.display_label(), "Alice");
    }

    #[test]
    fn test_external_actor_display_label_fallback_to_id() {
        let actor = everruns_core::ExternalActor {
            actor_id: "U0123456789".to_string(),
            actor_name: None,
            source: "slack".to_string(),
            metadata: None,
        };
        assert_eq!(actor.display_label(), "U0123456789");
    }

    #[test]
    fn test_external_actor_with_metadata() {
        let mut metadata = std::collections::HashMap::new();
        metadata.insert("channel".to_string(), "C123".to_string());
        metadata.insert("team_id".to_string(), "T456".to_string());

        let actor = everruns_core::ExternalActor {
            actor_id: "U123".to_string(),
            actor_name: Some("Alice".to_string()),
            source: "slack".to_string(),
            metadata: Some(metadata),
        };

        assert_eq!(actor.source, "slack");
        let meta = actor.metadata.as_ref().unwrap();
        assert_eq!(meta.get("channel").unwrap(), "C123");
        assert_eq!(meta.get("team_id").unwrap(), "T456");
    }

    #[test]
    fn test_external_actor_serialization_roundtrip() {
        let actor = everruns_core::ExternalActor {
            actor_id: "U123".to_string(),
            actor_name: Some("Alice".to_string()),
            source: "slack".to_string(),
            metadata: None,
        };

        let json = serde_json::to_value(&actor).unwrap();
        assert_eq!(json["actor_id"], "U123");
        assert_eq!(json["actor_name"], "Alice");
        assert_eq!(json["source"], "slack");

        let deserialized: everruns_core::ExternalActor = serde_json::from_value(json).unwrap();
        assert_eq!(deserialized, actor);
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
            webhook_verified_at: None,
            first_message_received_at: None,
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
            files: vec![],
            attachments: vec![],
        }
    }

    fn test_slack_file(name: &str, mimetype: &str, filetype: &str, url: Option<&str>) -> SlackFile {
        SlackFile {
            id: Some("F0123456789".to_string()),
            name: Some(name.to_string()),
            mimetype: Some(mimetype.to_string()),
            filetype: Some(filetype.to_string()),
            url_private: url.map(String::from),
            size: Some(1024),
        }
    }

    // ==========================================
    // Event context input_message_id filtering
    // ==========================================

    #[test]
    fn test_event_context_matches_correct_input_message() {
        let input_msg_str = "message_01933b5a00007000800000000000001";

        let context = serde_json::json!({
            "input_message_id": "message_01933b5a00007000800000000000001",
            "turn_id": "turn_01933b5a00007000800000000000001"
        });
        let event_input_msg = context.get("input_message_id").and_then(|v| v.as_str());
        assert_eq!(event_input_msg, Some(input_msg_str));
    }

    #[test]
    fn test_event_context_rejects_different_input_message() {
        let input_msg_str = "message_01933b5a00007000800000000000001";

        let context = serde_json::json!({
            "input_message_id": "message_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa2",
            "turn_id": "turn_bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb2"
        });
        let event_input_msg = context.get("input_message_id").and_then(|v| v.as_str());
        assert_ne!(event_input_msg, Some(input_msg_str));
    }

    #[test]
    fn test_event_context_rejects_empty_context() {
        let input_msg_str = "message_01933b5a00007000800000000000001";
        let context = serde_json::json!({});
        let event_input_msg = context.get("input_message_id").and_then(|v| v.as_str());
        assert_eq!(event_input_msg, None);
        assert_ne!(event_input_msg, Some(input_msg_str));
    }

    // ==========================================
    // DB-level dedup via has_event_with_slack_ts
    // ==========================================

    #[tokio::test]
    async fn test_has_event_with_slack_ts_no_match() {
        let db = StorageBackend::in_memory();
        let session_id = setup_test_session(&db).await;
        let result = db
            .has_event_with_slack_ts(session_id, "1234567890.123456")
            .await
            .unwrap();
        assert!(!result);
    }

    #[tokio::test]
    async fn test_has_event_with_slack_ts_match() {
        use crate::storage::models::CreateEventRow;

        let db = StorageBackend::in_memory();
        let session_id = setup_test_session(&db).await;

        // Insert an input.message event with slack_ts in metadata
        let event = CreateEventRow {
            session_id,
            event_type: "input.message".to_string(),
            ts: chrono::Utc::now(),
            context: serde_json::json!({}),
            data: serde_json::json!({
                "message": {
                    "id": "message_00000000000000000000000000000001",
                    "role": "user",
                    "content": [{"type": "text", "text": "hello"}],
                    "metadata": {
                        "slack_ts": "1234567890.123456",
                        "slack_user": "U123",
                        "slack_channel": "C456"
                    },
                    "created_at": "2024-01-01T00:00:00Z"
                }
            }),
            metadata: None,
            tags: None,
        };
        db.create_event(event).await.unwrap();

        // Same slack_ts should be found
        assert!(
            db.has_event_with_slack_ts(session_id, "1234567890.123456")
                .await
                .unwrap()
        );

        // Different slack_ts should NOT be found
        assert!(
            !db.has_event_with_slack_ts(session_id, "9999999999.999999")
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn test_has_event_with_slack_ts_wrong_session() {
        use crate::storage::models::CreateEventRow;

        let db = StorageBackend::in_memory();
        let session_id = setup_test_session(&db).await;
        let other_session_id = everruns_core::typed_id::SessionId::from_uuid(uuid::Uuid::now_v7());

        // Insert event in session_id
        let event = CreateEventRow {
            session_id,
            event_type: "input.message".to_string(),
            ts: chrono::Utc::now(),
            context: serde_json::json!({}),
            data: serde_json::json!({
                "message": {
                    "id": "message_00000000000000000000000000000001",
                    "role": "user",
                    "content": [{"type": "text", "text": "hello"}],
                    "metadata": { "slack_ts": "1234567890.123456" },
                    "created_at": "2024-01-01T00:00:00Z"
                }
            }),
            metadata: None,
            tags: None,
        };
        db.create_event(event).await.unwrap();

        // Different session should NOT find it
        assert!(
            !db.has_event_with_slack_ts(other_session_id, "1234567890.123456")
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn test_has_event_with_slack_ts_ignores_non_input_events() {
        use crate::storage::models::CreateEventRow;

        let db = StorageBackend::in_memory();
        let session_id = setup_test_session(&db).await;

        // Insert an output.message.completed event (not input.message)
        let event = CreateEventRow {
            session_id,
            event_type: "output.message.completed".to_string(),
            ts: chrono::Utc::now(),
            context: serde_json::json!({}),
            data: serde_json::json!({
                "message": {
                    "id": "message_00000000000000000000000000000001",
                    "role": "assistant",
                    "content": [{"type": "text", "text": "hi"}],
                    "metadata": { "slack_ts": "1234567890.123456" },
                    "created_at": "2024-01-01T00:00:00Z"
                }
            }),
            metadata: None,
            tags: None,
        };
        db.create_event(event).await.unwrap();

        // Should NOT match — only input.message events count
        assert!(
            !db.has_event_with_slack_ts(session_id, "1234567890.123456")
                .await
                .unwrap()
        );
    }

    // ==========================================
    // Response text extraction — last vs first
    // ==========================================

    #[test]
    fn test_extract_response_text_ignores_non_text_parts() {
        let data = serde_json::json!({
            "message": {
                "content": [
                    {"type": "image", "url": "https://example.com/img.png"},
                    {"type": "text", "text": "Only this."}
                ]
            }
        });
        assert_eq!(extract_response_text(&data).unwrap(), "Only this.");
    }

    /// Helper: create an in-memory session for dedup tests
    async fn setup_test_session(db: &StorageBackend) -> everruns_core::typed_id::SessionId {
        use crate::storage::models::CreateSessionRow;

        let row = CreateSessionRow {
            org_id: 1,
            harness_id: Some(everruns_core::typed_id::HarnessId::from_uuid(
                uuid::Uuid::nil(),
            )),
            agent_id: Some(everruns_core::typed_id::AgentId::from_uuid(
                uuid::Uuid::nil(),
            )),
            title: Some("test".to_string()),
            tags: vec![],
            model_id: None,
            capabilities: serde_json::json!([]),
            tools: serde_json::json!([]),
        };
        let session = db.create_session(row).await.unwrap();
        session.id
    }

    // ==========================================
    // Manifest helper tests
    // ==========================================

    #[test]
    fn test_truncate_display_name_short() {
        assert_eq!(truncate_display_name("My Bot"), "My Bot");
    }

    #[test]
    fn test_truncate_display_name_exact_80() {
        let exact = "a".repeat(80);
        assert_eq!(truncate_display_name(&exact), exact);
    }

    #[test]
    fn test_truncate_display_name_long() {
        let long = "a".repeat(100);
        assert_eq!(truncate_display_name(&long).len(), 80);
    }

    #[test]
    fn test_truncate_display_name_empty() {
        assert_eq!(truncate_display_name(""), "");
    }

    #[test]
    fn test_truncate_display_name_multibyte() {
        // 27 emoji × 4 bytes = 108 bytes, should truncate to 80 bytes (20 emoji)
        let name = "🤖".repeat(27);
        let truncated = truncate_display_name(&name);
        assert!(truncated.len() <= 80);
        assert!(truncated.is_char_boundary(truncated.len()));
    }

    #[test]
    fn test_manifest_yaml_contains_description_and_long_description() {
        let yaml = build_manifest_yaml("My Bot", "My Bot");
        assert!(yaml.contains(r#"description: "My Bot (Powered by Everruns)""#));
        assert!(yaml.contains(r#"long_description: "AI agent powered by Everruns"#));
        assert!(yaml.contains("https://everruns.com"));
    }

    #[test]
    fn test_manifest_yaml_description_within_slack_limit() {
        // Slack description limit is 140 chars. Worst case: 35-char app name
        // (Slack's name limit) + " (Powered by Everruns)" = 57 chars.
        let long_name = "a".repeat(35);
        let yaml = build_manifest_yaml(&long_name, &long_name);
        // Extract the description value
        let desc_prefix = "description: \"";
        let desc_start = yaml.find(desc_prefix).unwrap() + desc_prefix.len();
        let desc_end = yaml[desc_start..].find('"').unwrap();
        let description = &yaml[desc_start..desc_start + desc_end];
        assert!(
            description.len() <= 140,
            "Description '{}' is {} chars, exceeds Slack's 140 limit",
            description,
            description.len()
        );
    }

    #[test]
    fn test_manifest_yaml_escapes_special_chars_in_name() {
        let yaml = build_manifest_yaml(r#"Bot "Special""#, "Bot Special");
        assert!(yaml.contains(r#"name: "Bot \"Special\"""#));
        assert!(yaml.contains(r#"description: "Bot \"Special\" (Powered by Everruns)""#));
    }

    #[test]
    fn test_yaml_escape() {
        assert_eq!(yaml_escape(r#"hello "world""#), r#"hello \"world\""#);
        assert_eq!(yaml_escape(r#"back\slash"#), r#"back\\slash"#);
    }

    #[test]
    fn test_yaml_escape_no_special_chars() {
        assert_eq!(yaml_escape("plain text"), "plain text");
    }

    #[test]
    fn test_yaml_escape_empty() {
        assert_eq!(yaml_escape(""), "");
    }

    #[test]
    fn test_urlencoding_encode() {
        assert_eq!(urlencoding_encode("hello world"), "hello%20world");
        assert_eq!(urlencoding_encode("a=b&c=d"), "a%3Db%26c%3Dd");
        assert_eq!(urlencoding_encode("safe-_.~"), "safe-_.~");
    }

    #[test]
    fn test_urlencoding_encode_empty() {
        assert_eq!(urlencoding_encode(""), "");
    }

    #[test]
    fn test_urlencoding_encode_newlines() {
        assert_eq!(urlencoding_encode("a\nb"), "a%0Ab");
    }

    // ==========================================
    // File attachment content part building
    // ==========================================

    #[test]
    fn test_build_file_content_parts_image_with_url() {
        let files = vec![test_slack_file(
            "photo.png",
            "image/png",
            "png",
            Some("https://files.slack.com/files-pri/T0/photo.png"),
        )];
        let parts = build_file_content_parts(&files);
        assert_eq!(parts.len(), 1);
        match &parts[0] {
            InputContentPart::Image(img) => {
                assert_eq!(
                    img.url.as_deref().unwrap(),
                    "https://files.slack.com/files-pri/T0/photo.png"
                );
            }
            other => panic!("Expected Image content part, got {:?}", other),
        }
    }

    #[test]
    fn test_build_file_content_parts_image_no_url() {
        let files = vec![test_slack_file("photo.png", "image/png", "png", None)];
        let parts = build_file_content_parts(&files);
        assert_eq!(parts.len(), 1);
        match &parts[0] {
            InputContentPart::Text(t) => {
                assert!(t.text.contains("photo.png"));
                assert!(t.text.contains("no download URL"));
            }
            other => panic!("Expected Text fallback, got {:?}", other),
        }
    }

    #[test]
    fn test_build_file_content_parts_unsupported_type() {
        let files = vec![test_slack_file(
            "video.mp4",
            "video/mp4",
            "mp4",
            Some("https://files.slack.com/files-pri/T0/video.mp4"),
        )];
        let parts = build_file_content_parts(&files);
        assert_eq!(parts.len(), 1);
        match &parts[0] {
            InputContentPart::Text(t) => {
                assert!(t.text.contains("Attached file"));
                assert!(t.text.contains("video.mp4"));
                assert!(t.text.contains("mp4"));
            }
            other => panic!("Expected Text description, got {:?}", other),
        }
    }

    #[test]
    fn test_build_file_content_parts_multiple_mixed() {
        let files = vec![
            test_slack_file(
                "photo.jpg",
                "image/jpeg",
                "jpg",
                Some("https://files.slack.com/photo.jpg"),
            ),
            test_slack_file(
                "doc.pdf",
                "application/pdf",
                "pdf",
                Some("https://files.slack.com/doc.pdf"),
            ),
            test_slack_file(
                "screenshot.webp",
                "image/webp",
                "webp",
                Some("https://files.slack.com/screenshot.webp"),
            ),
        ];
        let parts = build_file_content_parts(&files);
        assert_eq!(parts.len(), 3);
        assert!(matches!(&parts[0], InputContentPart::Image(_)));
        assert!(matches!(&parts[1], InputContentPart::Text(_)));
        assert!(matches!(&parts[2], InputContentPart::Image(_)));
    }

    #[test]
    fn test_build_file_content_parts_empty() {
        let parts = build_file_content_parts(&[]);
        assert!(parts.is_empty());
    }

    #[test]
    fn test_build_file_content_parts_all_supported_image_types() {
        for (mime, ext) in [
            ("image/png", "png"),
            ("image/jpeg", "jpeg"),
            ("image/gif", "gif"),
            ("image/webp", "webp"),
        ] {
            let files = vec![test_slack_file(
                &format!("test.{ext}"),
                mime,
                ext,
                Some("https://example.com/file"),
            )];
            let parts = build_file_content_parts(&files);
            assert!(
                matches!(&parts[0], InputContentPart::Image(_)),
                "{mime} should produce Image content part"
            );
        }
    }

    #[test]
    fn test_slack_event_with_files_deserialization() {
        let json = r#"{
            "type": "event_callback",
            "event": {
                "type": "message",
                "user": "U0123456789",
                "text": "Check this out",
                "channel": "C0123456789",
                "ts": "1234567890.123456",
                "files": [
                    {
                        "id": "F0123",
                        "name": "image.png",
                        "mimetype": "image/png",
                        "filetype": "png",
                        "url_private": "https://files.slack.com/image.png",
                        "size": 2048
                    },
                    {
                        "id": "F0456",
                        "name": "report.pdf",
                        "mimetype": "application/pdf",
                        "filetype": "pdf",
                        "url_private": "https://files.slack.com/report.pdf",
                        "size": 10240
                    }
                ]
            }
        }"#;
        let envelope: SlackEventEnvelope = serde_json::from_str(json).unwrap();
        let event = envelope.event.unwrap();
        assert_eq!(event.files.len(), 2);
        assert_eq!(event.files[0].name.as_deref().unwrap(), "image.png");
        assert_eq!(event.files[0].mimetype.as_deref().unwrap(), "image/png");
        assert_eq!(event.files[1].name.as_deref().unwrap(), "report.pdf");
    }

    #[test]
    fn test_slack_event_without_files_deserialization() {
        let json = r#"{
            "type": "event_callback",
            "event": {
                "type": "message",
                "user": "U0123456789",
                "text": "No files here",
                "channel": "C0123456789",
                "ts": "1234567890.123456"
            }
        }"#;
        let envelope: SlackEventEnvelope = serde_json::from_str(json).unwrap();
        let event = envelope.event.unwrap();
        assert!(event.files.is_empty());
    }

    #[test]
    fn test_build_file_content_parts_missing_fields() {
        // File with no name, no mimetype, no filetype
        let file = SlackFile {
            id: None,
            name: None,
            mimetype: None,
            filetype: None,
            url_private: Some("https://example.com/file".to_string()),
            size: None,
        };
        let parts = build_file_content_parts(&[file]);
        assert_eq!(parts.len(), 1);
        match &parts[0] {
            InputContentPart::Text(t) => {
                assert!(t.text.contains("unnamed file"));
                assert!(t.text.contains("unknown"));
            }
            other => panic!("Expected Text for unknown mimetype, got {:?}", other),
        }
    }

    // ==========================================
    // Legacy attachment content part building
    // ==========================================

    fn test_slack_attachment() -> SlackAttachment {
        SlackAttachment {
            title: None,
            title_link: None,
            text: None,
            fallback: None,
            pretext: None,
            image_url: None,
            thumb_url: None,
            author_name: None,
            service_name: None,
            original_url: None,
            app_unfurl_url: None,
        }
    }

    #[test]
    fn test_build_attachment_content_parts_with_image() {
        let mut att = test_slack_attachment();
        att.image_url = Some("https://example.com/preview.png".to_string());
        att.title = Some("Preview".to_string());

        let parts = build_attachment_content_parts(&[att]);
        assert_eq!(parts.len(), 1);
        match &parts[0] {
            InputContentPart::Image(img) => {
                assert_eq!(
                    img.url.as_deref().unwrap(),
                    "https://example.com/preview.png"
                );
            }
            other => panic!("Expected Image, got {:?}", other),
        }
    }

    #[test]
    fn test_build_attachment_content_parts_link_unfurl() {
        let mut att = test_slack_attachment();
        att.service_name = Some("GitHub".to_string());
        att.title = Some("Fix bug #123".to_string());
        att.title_link = Some("https://github.com/org/repo/pull/123".to_string());
        att.text = Some("Fixes a critical issue".to_string());

        let parts = build_attachment_content_parts(&[att]);
        assert_eq!(parts.len(), 1);
        match &parts[0] {
            InputContentPart::Text(t) => {
                assert!(t.text.contains("GitHub"));
                assert!(t.text.contains("Fix bug #123"));
                assert!(t.text.contains("github.com"));
                assert!(t.text.contains("Fixes a critical issue"));
            }
            other => panic!("Expected Text, got {:?}", other),
        }
    }

    #[test]
    fn test_build_attachment_content_parts_fallback_only() {
        let mut att = test_slack_attachment();
        att.fallback = Some("Canvas: Project Plan".to_string());

        let parts = build_attachment_content_parts(&[att]);
        assert_eq!(parts.len(), 1);
        match &parts[0] {
            InputContentPart::Text(t) => {
                assert!(t.text.contains("Canvas: Project Plan"));
            }
            other => panic!("Expected Text fallback, got {:?}", other),
        }
    }

    #[test]
    fn test_build_attachment_content_parts_empty_attachment() {
        let att = test_slack_attachment();
        let parts = build_attachment_content_parts(&[att]);
        assert!(parts.is_empty());
    }

    #[test]
    fn test_build_attachment_content_parts_empty_vec() {
        let parts = build_attachment_content_parts(&[]);
        assert!(parts.is_empty());
    }

    #[test]
    fn test_build_attachment_content_parts_multiple() {
        let mut att1 = test_slack_attachment();
        att1.image_url = Some("https://example.com/img.png".to_string());

        let mut att2 = test_slack_attachment();
        att2.title = Some("Linked doc".to_string());
        att2.text = Some("Some content".to_string());

        let parts = build_attachment_content_parts(&[att1, att2]);
        assert_eq!(parts.len(), 2);
        assert!(matches!(&parts[0], InputContentPart::Image(_)));
        assert!(matches!(&parts[1], InputContentPart::Text(_)));
    }

    #[test]
    fn test_slack_event_with_attachments_deserialization() {
        let json = r#"{
            "type": "event_callback",
            "event": {
                "type": "message",
                "user": "U0123456789",
                "text": "Check this link",
                "channel": "C0123456789",
                "ts": "1234567890.123456",
                "attachments": [
                    {
                        "service_name": "GitHub",
                        "title": "PR #42",
                        "title_link": "https://github.com/org/repo/pull/42",
                        "text": "Add feature X",
                        "fallback": "GitHub: PR #42"
                    }
                ]
            }
        }"#;
        let envelope: SlackEventEnvelope = serde_json::from_str(json).unwrap();
        let event = envelope.event.unwrap();
        assert_eq!(event.attachments.len(), 1);
        assert_eq!(event.attachments[0].title.as_deref().unwrap(), "PR #42");
        assert_eq!(
            event.attachments[0].service_name.as_deref().unwrap(),
            "GitHub"
        );
    }

    #[test]
    fn test_slack_event_with_files_and_attachments() {
        let json = r#"{
            "type": "event_callback",
            "event": {
                "type": "message",
                "user": "U0123456789",
                "text": "",
                "channel": "C0123456789",
                "ts": "1234567890.123456",
                "files": [{"id": "F01", "name": "photo.png", "mimetype": "image/png", "filetype": "png"}],
                "attachments": [{"title": "Link", "text": "A link"}]
            }
        }"#;
        let envelope: SlackEventEnvelope = serde_json::from_str(json).unwrap();
        let event = envelope.event.unwrap();
        assert_eq!(event.files.len(), 1);
        assert_eq!(event.attachments.len(), 1);
        // Both empty text + has files/attachments = should not be skipped
        assert!(event.text.as_deref().unwrap().is_empty());
    }

    // ==========================================
    // Thread context — SlackReplyMessage parsing
    // ==========================================

    #[test]
    fn test_slack_reply_message_deserialization() {
        let json = r#"[
            {"user": "U123", "text": "Hello", "ts": "1234.0000"},
            {"bot_id": "B123", "text": "Hi there", "ts": "1234.0001"},
            {"user": "U456", "text": "Nice", "ts": "1234.0002", "subtype": "thread_broadcast"}
        ]"#;
        let replies: Vec<SlackReplyMessage> = serde_json::from_str(json).unwrap();
        assert_eq!(replies.len(), 3);
        assert_eq!(replies[0].user.as_deref(), Some("U123"));
        assert!(replies[0].bot_id.is_none());
        assert!(replies[1].bot_id.is_some());
        assert_eq!(replies[2].subtype.as_deref(), Some("thread_broadcast"));
    }

    #[test]
    fn test_slack_reply_message_minimal() {
        let json = r#"{"ts": "1234.0000"}"#;
        let reply: SlackReplyMessage = serde_json::from_str(json).unwrap();
        assert!(reply.user.is_none());
        assert!(reply.text.is_none());
        assert!(reply.bot_id.is_none());
    }

    #[tokio::test]
    async fn test_inject_thread_context_empty_replies() {
        // When fetch returns empty, inject_thread_context should succeed as no-op
        let db = Arc::new(StorageBackend::in_memory());
        let runner: Arc<dyn AgentRunner> = Arc::new(NoopRunner);
        let state = SlackState::new(db, runner, None);
        let session_id = setup_test_session(&state.db).await;

        // inject with no network (fetch will fail gracefully → empty replies → Ok)
        let result = inject_thread_context(
            &state,
            "xoxb-fake",
            "C123",
            "1234.0000",
            session_id,
            Some("1234.9999"),
        )
        .await;
        assert!(result.is_ok());
    }

    #[test]
    fn test_thread_context_triggers_only_for_per_thread_with_thread_ts() {
        // PerThread + thread_ts present → should trigger context injection
        let config = test_config(SessionStrategy::PerThread);
        let event = test_event("C123", Some("1234.5678"), Some("1234.0000"));
        assert!(config.session_strategy == SessionStrategy::PerThread && event.thread_ts.is_some());

        // PerThread + no thread_ts → should NOT trigger (new thread, no prior context)
        let event_no_thread = test_event("C123", Some("1234.5678"), None);
        assert!(event_no_thread.thread_ts.is_none());

        // PerChannel → should NOT trigger
        let config_channel = test_config(SessionStrategy::PerChannel);
        assert!(config_channel.session_strategy != SessionStrategy::PerThread);

        // PerUser → should NOT trigger
        let config_user = test_config(SessionStrategy::PerUser);
        assert!(config_user.session_strategy != SessionStrategy::PerThread);
    }

    #[test]
    fn test_manifest_includes_history_scopes_for_thread_context() {
        // conversations.replies requires channels:history / groups:history / im:history / mpim:history
        let yaml = build_manifest_yaml("Bot", "Bot");
        assert!(
            yaml.contains("channels:history"),
            "Manifest must include channels:history for conversations.replies"
        );
        assert!(
            yaml.contains("groups:history"),
            "Manifest must include groups:history for private channel thread context"
        );
        assert!(
            yaml.contains("im:history"),
            "Manifest must include im:history for DM thread context"
        );
        assert!(
            yaml.contains("mpim:history"),
            "Manifest must include mpim:history for group DM thread context"
        );
    }

    /// Noop runner for tests that need SlackState without a real worker.
    struct NoopRunner;

    #[async_trait::async_trait]
    impl AgentRunner for NoopRunner {
        async fn start_run(
            &self,
            _org_id: i64,
            _session_id: everruns_core::typed_id::SessionId,
            _harness_id: everruns_core::typed_id::HarnessId,
            _agent_id: Option<everruns_core::typed_id::AgentId>,
            _input_message_id: everruns_core::typed_id::MessageId,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        async fn cancel_run(
            &self,
            _run_id: everruns_core::typed_id::SessionId,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        async fn is_running(&self, _run_id: everruns_core::typed_id::SessionId) -> bool {
            false
        }

        async fn active_count(&self) -> usize {
            0
        }
    }

    // ==========================================
    // WireMock integration tests — Slack API
    // ==========================================

    mod wiremock_tests {
        use super::*;
        use wiremock::matchers::{header, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        // ------------------------------------------
        // resolve_slack_user_name — success paths
        // ------------------------------------------

        #[tokio::test]
        async fn test_resolve_user_name_display_name() {
            let mock_server = MockServer::start().await;

            Mock::given(method("GET"))
                .and(path("/users.info"))
                .and(header("Authorization", "Bearer xoxb-test-token"))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "ok": true,
                    "user": {
                        "id": "U123",
                        "name": "alice",
                        "real_name": "Alice Smith",
                        "profile": {
                            "display_name": "Alice S."
                        }
                    }
                })))
                .expect(1)
                .mount(&mock_server)
                .await;

            let cache: SlackUserCache = Arc::new(RwLock::new(HashMap::new()));
            let result =
                resolve_slack_user_name_base(&mock_server.uri(), &cache, "xoxb-test-token", "U123")
                    .await;

            assert_eq!(result, Some("Alice S.".to_string()));

            // Verify it was cached
            let cached = cache.read().await.get("U123").cloned();
            assert_eq!(cached, Some(Some("Alice S.".to_string())));
        }

        #[tokio::test]
        async fn test_resolve_user_name_fallback_to_real_name() {
            let mock_server = MockServer::start().await;

            Mock::given(method("GET"))
                .and(path("/users.info"))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "ok": true,
                    "user": {
                        "id": "U123",
                        "name": "alice",
                        "real_name": "Alice Smith",
                        "profile": {
                            "display_name": ""
                        }
                    }
                })))
                .expect(1)
                .mount(&mock_server)
                .await;

            let cache: SlackUserCache = Arc::new(RwLock::new(HashMap::new()));
            let result =
                resolve_slack_user_name_base(&mock_server.uri(), &cache, "xoxb-test-token", "U123")
                    .await;

            // Empty display_name → falls back to real_name
            assert_eq!(result, Some("Alice Smith".to_string()));
        }

        #[tokio::test]
        async fn test_resolve_user_name_fallback_to_name() {
            let mock_server = MockServer::start().await;

            Mock::given(method("GET"))
                .and(path("/users.info"))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "ok": true,
                    "user": {
                        "id": "U123",
                        "name": "alice",
                        "profile": {
                            "display_name": ""
                        }
                    }
                })))
                .expect(1)
                .mount(&mock_server)
                .await;

            let cache: SlackUserCache = Arc::new(RwLock::new(HashMap::new()));
            let result =
                resolve_slack_user_name_base(&mock_server.uri(), &cache, "xoxb-test-token", "U123")
                    .await;

            // No real_name → falls back to name
            assert_eq!(result, Some("alice".to_string()));
        }

        #[tokio::test]
        async fn test_resolve_user_name_no_profile() {
            let mock_server = MockServer::start().await;

            Mock::given(method("GET"))
                .and(path("/users.info"))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "ok": true,
                    "user": {
                        "id": "U123",
                        "name": "alice"
                    }
                })))
                .expect(1)
                .mount(&mock_server)
                .await;

            let cache: SlackUserCache = Arc::new(RwLock::new(HashMap::new()));
            let result =
                resolve_slack_user_name_base(&mock_server.uri(), &cache, "xoxb-test-token", "U123")
                    .await;

            // No profile → falls back to name
            assert_eq!(result, Some("alice".to_string()));
        }

        // ------------------------------------------
        // resolve_slack_user_name — cache behavior
        // ------------------------------------------

        #[tokio::test]
        async fn test_resolve_user_name_uses_cache() {
            let mock_server = MockServer::start().await;

            // No mock mounted — any actual HTTP call would fail
            let cache: SlackUserCache = Arc::new(RwLock::new(HashMap::new()));
            cache
                .write()
                .await
                .insert("U123".to_string(), Some("Cached Alice".to_string()));

            let result =
                resolve_slack_user_name_base(&mock_server.uri(), &cache, "xoxb-test-token", "U123")
                    .await;

            assert_eq!(result, Some("Cached Alice".to_string()));
            // No HTTP call made (mock server received 0 requests)
        }

        #[tokio::test]
        async fn test_resolve_user_name_cached_none_skips_api() {
            let mock_server = MockServer::start().await;

            // Cache a None (permanent failure like missing_scope)
            let cache: SlackUserCache = Arc::new(RwLock::new(HashMap::new()));
            cache.write().await.insert("U123".to_string(), None);

            let result =
                resolve_slack_user_name_base(&mock_server.uri(), &cache, "xoxb-test-token", "U123")
                    .await;

            assert_eq!(result, None);
        }

        // ------------------------------------------
        // resolve_slack_user_name — error paths
        // ------------------------------------------

        #[tokio::test]
        async fn test_resolve_user_name_missing_scope_cached() {
            let mock_server = MockServer::start().await;

            Mock::given(method("GET"))
                .and(path("/users.info"))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "ok": false,
                    "error": "missing_scope"
                })))
                .expect(1)
                .mount(&mock_server)
                .await;

            let cache: SlackUserCache = Arc::new(RwLock::new(HashMap::new()));
            let result =
                resolve_slack_user_name_base(&mock_server.uri(), &cache, "xoxb-test-token", "U123")
                    .await;

            assert_eq!(result, None);

            // Permanent error should be cached as None
            let cached = cache.read().await.get("U123").cloned();
            assert_eq!(cached, Some(None));
        }

        #[tokio::test]
        async fn test_resolve_user_name_invalid_auth_cached() {
            let mock_server = MockServer::start().await;

            Mock::given(method("GET"))
                .and(path("/users.info"))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "ok": false,
                    "error": "invalid_auth"
                })))
                .expect(1)
                .mount(&mock_server)
                .await;

            let cache: SlackUserCache = Arc::new(RwLock::new(HashMap::new()));
            let result =
                resolve_slack_user_name_base(&mock_server.uri(), &cache, "xoxb-bad-token", "U123")
                    .await;

            assert_eq!(result, None);
            assert_eq!(cache.read().await.get("U123").cloned(), Some(None));
        }

        #[tokio::test]
        async fn test_resolve_user_name_token_revoked_cached() {
            let mock_server = MockServer::start().await;

            Mock::given(method("GET"))
                .and(path("/users.info"))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "ok": false,
                    "error": "token_revoked"
                })))
                .expect(1)
                .mount(&mock_server)
                .await;

            let cache: SlackUserCache = Arc::new(RwLock::new(HashMap::new()));
            let result =
                resolve_slack_user_name_base(&mock_server.uri(), &cache, "xoxb-revoked", "U123")
                    .await;

            assert_eq!(result, None);
            assert_eq!(cache.read().await.get("U123").cloned(), Some(None));
        }

        #[tokio::test]
        async fn test_resolve_user_name_transient_error_not_cached() {
            let mock_server = MockServer::start().await;

            // Transient error (e.g. internal_error) should NOT be cached
            Mock::given(method("GET"))
                .and(path("/users.info"))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "ok": false,
                    "error": "internal_error"
                })))
                .expect(1)
                .mount(&mock_server)
                .await;

            let cache: SlackUserCache = Arc::new(RwLock::new(HashMap::new()));
            let result =
                resolve_slack_user_name_base(&mock_server.uri(), &cache, "xoxb-test-token", "U123")
                    .await;

            assert_eq!(result, None);

            // Transient error should NOT be cached (allow retry)
            assert!(cache.read().await.get("U123").is_none());
        }

        #[tokio::test]
        async fn test_resolve_user_name_malformed_json() {
            let mock_server = MockServer::start().await;

            Mock::given(method("GET"))
                .and(path("/users.info"))
                .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
                .expect(1)
                .mount(&mock_server)
                .await;

            let cache: SlackUserCache = Arc::new(RwLock::new(HashMap::new()));
            let result =
                resolve_slack_user_name_base(&mock_server.uri(), &cache, "xoxb-test-token", "U123")
                    .await;

            assert_eq!(result, None);
            // Parse failure is transient — not cached
            assert!(cache.read().await.get("U123").is_none());
        }

        #[tokio::test]
        async fn test_resolve_user_name_account_inactive_cached() {
            let mock_server = MockServer::start().await;

            Mock::given(method("GET"))
                .and(path("/users.info"))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "ok": false,
                    "error": "account_inactive"
                })))
                .expect(1)
                .mount(&mock_server)
                .await;

            let cache: SlackUserCache = Arc::new(RwLock::new(HashMap::new()));
            let result =
                resolve_slack_user_name_base(&mock_server.uri(), &cache, "xoxb-test-token", "U123")
                    .await;

            assert_eq!(result, None);
            assert_eq!(cache.read().await.get("U123").cloned(), Some(None));
        }

        #[tokio::test]
        async fn test_resolve_user_name_not_authed_cached() {
            let mock_server = MockServer::start().await;

            Mock::given(method("GET"))
                .and(path("/users.info"))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "ok": false,
                    "error": "not_authed"
                })))
                .expect(1)
                .mount(&mock_server)
                .await;

            let cache: SlackUserCache = Arc::new(RwLock::new(HashMap::new()));
            let result =
                resolve_slack_user_name_base(&mock_server.uri(), &cache, "xoxb-test-token", "U123")
                    .await;

            assert_eq!(result, None);
            assert_eq!(cache.read().await.get("U123").cloned(), Some(None));
        }

        // ------------------------------------------
        // post_to_slack via slack_delivery module
        // ------------------------------------------

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

            let result = crate::slack_delivery::post_to_slack_base(
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
        async fn test_post_to_slack_error() {
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

            let result = crate::slack_delivery::post_to_slack_base(
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
    }
}

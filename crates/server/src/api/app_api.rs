// App api_endpoint ingress — native session routes authenticated by an
// app-scoped, execution-only API key (`evr_app_...`).
//
// Design Decision: api_endpoint channels are app-scoped and channel-scoped
// (`/v1/apps/{app_id}/api/{channel_id}/...`) so a single app can expose
// multiple execution keys with independent session routing and rate limits.
// The key is structurally execution-only: it reaches only these app-mounted
// routes and has no path to any management API. Every session it touches is
// confined to the channel by routing tags (TM-APIKEY-002).
//
// See `knowledge/integrations/app-api-keys.md`.

use std::sync::Arc;

use axum::{
    Extension, Json, Router,
    extract::{ConnectInfo, Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use everruns_core::ContentPart;
use everruns_core::events::{
    OUTPUT_MESSAGE_COMPLETED, OutputMessageCompletedData, TURN_CANCELLED, TURN_COMPLETED,
    TURN_FAILED, TURN_STARTED,
};
use everruns_provider::execution_phase::ExecutionPhase;
use serde::{Deserialize, Serialize};

use crate::api::app_endpoint_auth::{
    AppEndpointAuthError, AppEndpointAuthVerifier, LegacyEndpointAuth, extract_bearer,
};
use crate::api::channel_rate_limit::ChannelRateLimiter;
use crate::api::common::ErrorResponse;
use crate::auth::rate_limit::extract_client_ip_from_parts;
use crate::domains::apps::{
    ApiInvocationRequest, hash_app_api_key, invoke_api_app_channel, post_api_app_channel_message,
    resolve_api_app_channel, session_has_app_channel_tags,
};
use crate::domains::messages::MessageService;
use crate::domains::sessions::SessionService;
use crate::event_delivery::EventDelivery;
use crate::middleware::RequestId;
use crate::security::constant_time_eq;
use crate::storage::models::EventRow;
use crate::storage::{EncryptionService, StorageBackend};

#[derive(Clone)]
pub struct AppApiState {
    pub db: Arc<StorageBackend>,
    pub encryption: Option<Arc<EncryptionService>>,
    pub session_service: Arc<SessionService>,
    pub message_service: Arc<MessageService>,
    pub rate_limiter: ChannelRateLimiter,
    pub auth_verifier: AppEndpointAuthVerifier,
}

impl AppApiState {
    pub fn new(
        db: Arc<StorageBackend>,
        encryption: Option<Arc<EncryptionService>>,
        runner: Arc<dyn everruns_scale::RunController>,
        notifications_enabled: bool,
        event_delivery: EventDelivery,
        rate_limiter: ChannelRateLimiter,
    ) -> Self {
        Self {
            session_service: Arc::new(SessionService::new(db.clone())),
            message_service: Arc::new(MessageService::new(
                db.clone(),
                runner,
                notifications_enabled,
                event_delivery,
            )),
            db,
            encryption,
            rate_limiter,
            auth_verifier: AppEndpointAuthVerifier::new(),
        }
    }
}

pub fn routes(state: AppApiState) -> Router {
    Router::new()
        .route(
            "/v1/apps/{app_id}/api/{channel_id}/sessions",
            post(create_session),
        )
        .route(
            "/v1/apps/{app_id}/api/{channel_id}/sessions/{session_id}",
            get(get_session),
        )
        .route(
            "/v1/apps/{app_id}/api/{channel_id}/sessions/{session_id}/messages",
            post(post_message),
        )
        .route(
            "/v1/apps/{app_id}/api/{channel_id}/sessions/{session_id}/cancel",
            post(cancel_session),
        )
        .with_state(state)
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct MessageBody {
    /// Message text dispatched to the agent.
    #[schema(example = "Summarize the latest support tickets.")]
    message: String,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct SessionRef {
    session_id: String,
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    created_session: Option<bool>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct AgentMessage {
    role: &'static str,
    text: String,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct SessionStatus {
    session_id: String,
    status: &'static str,
    messages: Vec<AgentMessage>,
}

/// Authenticated request context — published-app + enabled-channel resolution
/// plus the API key check. Carries the public ids downstream handlers use to
/// confine session access to the authenticated channel (TM-APIKEY-002).
struct AuthorizedApi {
    org_id: i64,
    app_public_id: String,
    channel_public_id: String,
}

/// POST /v1/apps/{app_id}/api/{channel_id}/sessions
#[utoipa::path(
    post,
    path = "/v1/apps/{app_id}/api/{channel_id}/sessions",
    params(
        ("app_id" = String, Path, description = "App ID"),
        ("channel_id" = String, Path, description = "api_endpoint channel ID")
    ),
    request_body = MessageBody,
    responses(
        (status = 201, description = "Session created and message dispatched", body = SessionRef),
        (status = 401, description = "Missing or invalid API key", body = ErrorResponse),
        (status = 403, description = "App not published or channel disabled", body = ErrorResponse),
        (status = 404, description = "App or channel not found", body = ErrorResponse),
        (status = 429, description = "Per-channel rate limit exceeded", body = ErrorResponse),
    ),
    tag = "apps"
)]
pub async fn create_session(
    State(state): State<AppApiState>,
    Path((app_id, channel_id)): Path<(String, String)>,
    req_id: Option<Extension<RequestId>>,
    connect_info: Option<Extension<ConnectInfo<std::net::SocketAddr>>>,
    headers: HeaderMap,
    Json(body): Json<MessageBody>,
) -> Response {
    let peer_addr = connect_info.map(|Extension(ConnectInfo(addr))| addr);
    let _auth = match authenticate_request(&state, &app_id, &channel_id, &headers, peer_addr).await
    {
        Ok(auth) => auth,
        Err(err) => return err.into_response(),
    };
    let request_id = req_id.map(|Extension(id)| id.0);

    match invoke_api_app_channel(
        &state.db,
        state.encryption.as_ref(),
        &state.session_service,
        &state.message_service,
        ApiInvocationRequest {
            app_id,
            channel_id,
            message: body.message,
        },
        request_id,
    )
    .await
    {
        Ok(result) => (
            StatusCode::CREATED,
            Json(SessionRef {
                session_id: result.session_id.to_string(),
                status: "working",
                created_session: Some(result.created_session),
            }),
        )
            .into_response(),
        Err(err) => command_error_response(err).into_response(),
    }
}

/// POST /v1/apps/{app_id}/api/{channel_id}/sessions/{session_id}/messages
#[utoipa::path(
    post,
    path = "/v1/apps/{app_id}/api/{channel_id}/sessions/{session_id}/messages",
    params(
        ("app_id" = String, Path, description = "App ID"),
        ("channel_id" = String, Path, description = "api_endpoint channel ID"),
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = MessageBody,
    responses(
        (status = 202, description = "Follow-up message dispatched", body = SessionRef),
        (status = 401, description = "Missing or invalid API key", body = ErrorResponse),
        (status = 403, description = "App not published or channel disabled", body = ErrorResponse),
        (status = 404, description = "Channel or session not found / not owned by this channel", body = ErrorResponse),
        (status = 429, description = "Per-channel rate limit exceeded", body = ErrorResponse),
    ),
    tag = "apps"
)]
pub async fn post_message(
    State(state): State<AppApiState>,
    Path((app_id, channel_id, session_id)): Path<(String, String, String)>,
    req_id: Option<Extension<RequestId>>,
    connect_info: Option<Extension<ConnectInfo<std::net::SocketAddr>>>,
    headers: HeaderMap,
    Json(body): Json<MessageBody>,
) -> Response {
    let peer_addr = connect_info.map(|Extension(ConnectInfo(addr))| addr);
    let _auth = match authenticate_request(&state, &app_id, &channel_id, &headers, peer_addr).await
    {
        Ok(auth) => auth,
        Err(err) => return err.into_response(),
    };
    let session_id = match session_id.parse::<everruns_provider::typed_id::SessionId>() {
        Ok(id) => id,
        Err(_) => return not_found().into_response(),
    };
    let request_id = req_id.map(|Extension(id)| id.0);

    match post_api_app_channel_message(
        &state.db,
        state.encryption.as_ref(),
        &state.message_service,
        &app_id,
        &channel_id,
        session_id,
        body.message,
        request_id,
    )
    .await
    {
        Ok(_) => (
            StatusCode::ACCEPTED,
            Json(SessionRef {
                session_id: session_id.to_string(),
                status: "working",
                created_session: None,
            }),
        )
            .into_response(),
        Err(err) => command_error_response(err).into_response(),
    }
}

/// GET /v1/apps/{app_id}/api/{channel_id}/sessions/{session_id}
#[utoipa::path(
    get,
    path = "/v1/apps/{app_id}/api/{channel_id}/sessions/{session_id}",
    params(
        ("app_id" = String, Path, description = "App ID"),
        ("channel_id" = String, Path, description = "api_endpoint channel ID"),
        ("session_id" = String, Path, description = "Session ID")
    ),
    responses(
        (status = 200, description = "Derived status and the agent's completed messages (no raw tool detail)", body = SessionStatus),
        (status = 401, description = "Missing or invalid API key", body = ErrorResponse),
        (status = 403, description = "App not published or channel disabled", body = ErrorResponse),
        (status = 404, description = "Channel or session not found / not owned by this channel", body = ErrorResponse),
        (status = 429, description = "Per-channel rate limit exceeded", body = ErrorResponse),
    ),
    tag = "apps"
)]
pub async fn get_session(
    State(state): State<AppApiState>,
    Path((app_id, channel_id, session_id)): Path<(String, String, String)>,
    headers: HeaderMap,
    connect_info: Option<Extension<ConnectInfo<std::net::SocketAddr>>>,
) -> Response {
    let peer_addr = connect_info.map(|Extension(ConnectInfo(addr))| addr);
    let auth = match authenticate_request(&state, &app_id, &channel_id, &headers, peer_addr).await {
        Ok(auth) => auth,
        Err(err) => return err.into_response(),
    };
    let session_id = match session_id.parse::<everruns_provider::typed_id::SessionId>() {
        Ok(id) => id,
        Err(_) => return not_found().into_response(),
    };

    let session = match state.db.get_session(auth.org_id, session_id).await {
        Ok(Some(s)) => s,
        Ok(None) => return not_found().into_response(),
        Err(err) => return internal_error(err).into_response(),
    };
    if !session_has_app_channel_tags(&session.tags, &auth.app_public_id, &auth.channel_public_id) {
        return not_found().into_response();
    }

    let (status, messages) = match read_session_output(&state.db, session_id).await {
        Ok(out) => out,
        Err(err) => return internal_error(err).into_response(),
    };

    (
        StatusCode::OK,
        Json(SessionStatus {
            session_id: session_id.to_string(),
            status,
            messages,
        }),
    )
        .into_response()
}

/// POST /v1/apps/{app_id}/api/{channel_id}/sessions/{session_id}/cancel
#[utoipa::path(
    post,
    path = "/v1/apps/{app_id}/api/{channel_id}/sessions/{session_id}/cancel",
    params(
        ("app_id" = String, Path, description = "App ID"),
        ("channel_id" = String, Path, description = "api_endpoint channel ID"),
        ("session_id" = String, Path, description = "Session ID")
    ),
    responses(
        (status = 200, description = "In-flight turn canceled", body = SessionRef),
        (status = 401, description = "Missing or invalid API key", body = ErrorResponse),
        (status = 403, description = "App not published or channel disabled", body = ErrorResponse),
        (status = 404, description = "Channel or session not found / not owned by this channel", body = ErrorResponse),
        (status = 429, description = "Per-channel rate limit exceeded", body = ErrorResponse),
    ),
    tag = "apps"
)]
pub async fn cancel_session(
    State(state): State<AppApiState>,
    Path((app_id, channel_id, session_id)): Path<(String, String, String)>,
    headers: HeaderMap,
    connect_info: Option<Extension<ConnectInfo<std::net::SocketAddr>>>,
) -> Response {
    let peer_addr = connect_info.map(|Extension(ConnectInfo(addr))| addr);
    let auth = match authenticate_request(&state, &app_id, &channel_id, &headers, peer_addr).await {
        Ok(auth) => auth,
        Err(err) => return err.into_response(),
    };
    let session_id = match session_id.parse::<everruns_provider::typed_id::SessionId>() {
        Ok(id) => id,
        Err(_) => return not_found().into_response(),
    };

    let session = match state.db.get_session(auth.org_id, session_id).await {
        Ok(Some(s)) => s,
        Ok(None) => return not_found().into_response(),
        Err(err) => return internal_error(err).into_response(),
    };
    if !session_has_app_channel_tags(&session.tags, &auth.app_public_id, &auth.channel_public_id) {
        return not_found().into_response();
    }

    if let Err(err) = cancel_session_turn(&state, session_id).await {
        return internal_error(err).into_response();
    }

    (
        StatusCode::OK,
        Json(SessionRef {
            session_id: session_id.to_string(),
            status: "canceled",
            created_session: None,
        }),
    )
        .into_response()
}

async fn authenticate_request(
    state: &AppApiState,
    app_id: &str,
    channel_id: &str,
    headers: &HeaderMap,
    peer_addr: Option<std::net::SocketAddr>,
) -> Result<AuthorizedApi, (StatusCode, Json<ErrorResponse>)> {
    // THREAT[TM-APIKEY-001/005]: published-app + enabled-channel + channel-type
    // gate. Shared with the command layer (`invoke_api_app_channel` /
    // `post_api_app_channel_message`) via `resolve_api_app_channel` so the gate
    // lives in one place and cannot drift between HTTP and command paths.
    let (app, channel) =
        resolve_api_app_channel(&state.db, state.encryption.as_ref(), app_id, channel_id)
            .await
            .map_err(command_error_response)?;

    let config = channel
        .api_endpoint_config()
        .ok_or_else(|| bad_request("Invalid api_endpoint channel configuration"))?;

    if let Some(auth) = config.auth.as_ref() {
        if auth.mode == everruns_platform::AppEndpointAuthMode::ApiKey {
            verify_api_key(headers, &config.api_key_hash)?;
        } else {
            state
                .auth_verifier
                .verify(
                    auth,
                    headers,
                    LegacyEndpointAuth {
                        shared_secret: None,
                        api_key: None,
                    },
                )
                .await
                .map_err(api_auth_error_response)?;
        }
    } else {
        verify_api_key(headers, &config.api_key_hash)?;
    }

    // THREAT[TM-APIKEY-003]: per-channel, per-IP rate limit on top of the
    // global API limit. The check runs after the key comparison so an
    // unauthenticated caller cannot grow the limiter cache or probe channel
    // existence from rate-limit signals. Scope includes the channel id so apps
    // exposing multiple keys keep independent buckets.
    if let Some(limit) = config.rate_limit_per_minute
        && limit > 0
    {
        let scope = format!("{}:{}", app.public_id, channel.public_id);
        let client_ip = extract_client_ip_from_parts(peer_addr, headers);
        if state
            .rate_limiter
            .check(&scope, client_ip, limit)
            .await
            .is_err()
        {
            return Err(too_many_requests(
                "api_endpoint rate limit exceeded for this app channel",
            ));
        }
    }

    Ok(AuthorizedApi {
        org_id: app.org_id,
        app_public_id: app.public_id.to_string(),
        channel_public_id: channel.public_id.to_string(),
    })
}

fn verify_api_key(
    headers: &HeaderMap,
    expected_hash: &str,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    // THREAT[TM-APIKEY-001]: keys are stored as SHA-256 hashes; plaintext is
    // never persisted. Hash the inbound key and constant-time compare. Parse
    // the bearer scheme case-insensitively with whitespace trimming (RFC 7235)
    // via the shared `extract_bearer` helper, matching the rest of the
    // app-endpoint auth stack.
    let provided_key = extract_bearer(headers).ok_or_else(unauthorized)?;
    let provided_hash = hash_app_api_key(provided_key);
    if constant_time_eq(provided_hash.as_bytes(), expected_hash.as_bytes()) {
        Ok(())
    } else {
        Err(unauthorized())
    }
}

/// Read the derived status and the agent's completed messages for a session.
/// Only final (non-Commentary) assistant messages are surfaced — raw tool
/// names, arguments, and results are never exposed to the execution key
/// (TM-APIKEY-004).
async fn read_session_output(
    db: &Arc<StorageBackend>,
    session_id: everruns_provider::typed_id::SessionId,
) -> anyhow::Result<(&'static str, Vec<AgentMessage>)> {
    let filter_types = vec![
        OUTPUT_MESSAGE_COMPLETED.to_string(),
        TURN_STARTED.to_string(),
        TURN_COMPLETED.to_string(),
        TURN_FAILED.to_string(),
        TURN_CANCELLED.to_string(),
    ];
    let events = db
        .list_events(session_id, None, None, &filter_types, &[], None, Some(500))
        .await?;
    Ok(project_session_output(&events))
}

/// Pure projection of a session's filtered event tail into a derived turn
/// status plus the agent's completed messages. This is the primary
/// TM-APIKEY-004 mitigation: it surfaces only **final** (non-Commentary),
/// **text-only** assistant content. Commentary-phase messages, non-text
/// content parts (tool calls, images), and any non-message event are dropped,
/// so raw tool names, arguments, and results never reach the execution key.
fn project_session_output(events: &[EventRow]) -> (&'static str, Vec<AgentMessage>) {
    let mut status = "submitted";
    let mut messages = Vec::new();
    for evt in events {
        match evt.event_type.as_str() {
            TURN_STARTED => status = "working",
            TURN_COMPLETED => status = "completed",
            TURN_FAILED => status = "failed",
            TURN_CANCELLED => status = "canceled",
            OUTPUT_MESSAGE_COMPLETED => {
                let Ok(data) =
                    serde_json::from_value::<OutputMessageCompletedData>(evt.data.clone())
                else {
                    continue;
                };
                if matches!(data.message.phase, Some(ExecutionPhase::Commentary)) {
                    continue;
                }
                let text = content_parts_to_text(&data.message.content);
                if text.trim().is_empty() {
                    continue;
                }
                messages.push(AgentMessage {
                    role: "agent",
                    text,
                });
            }
            _ => {}
        }
    }
    (status, messages)
}

fn content_parts_to_text(parts: &[ContentPart]) -> String {
    parts
        .iter()
        .filter_map(|part| match part {
            ContentPart::Text(text) => Some(text.text.clone()),
            _ => None,
        })
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Cancel the in-flight workflow turn for a session and emit a synthetic
/// `turn.cancelled` event so subsequent reads observe the canceled state.
/// Mirrors the A2A `tasks/cancel` behavior.
async fn cancel_session_turn(
    state: &AppApiState,
    session_id: everruns_provider::typed_id::SessionId,
) -> anyhow::Result<()> {
    use everruns_core::events::{EventContext, EventRequest, TurnCancelledData};
    use everruns_provider::typed_id::{MessageId, TurnId};

    if let Err(err) = state.message_service.runner().cancel_run(session_id).await {
        tracing::warn!(session_id = %session_id, error = %err, "api_endpoint cancel: cancel_run failed");
    }

    // Skip emission if the turn already reached a terminal state, so a real
    // completed/failed turn is not race-flipped to canceled.
    let (status, _) = read_session_output(&state.db, session_id).await?;
    if matches!(status, "completed" | "failed" | "canceled") {
        return Ok(());
    }

    let turn_id = TurnId::from_uuid(session_id.uuid());
    let input_message_id = MessageId::new();
    let event_service = state.message_service.event_service();
    let cancelled_event = EventRequest::new(
        session_id,
        EventContext::turn(turn_id, input_message_id),
        TurnCancelledData {
            turn_id,
            reason: Some("api_endpoint cancel".to_string()),
            usage: None,
        },
    );
    if let Err(err) = event_service.emit(cancelled_event).await {
        tracing::warn!(session_id = %session_id, error = %err, "api_endpoint cancel: emit turn.cancelled failed");
    }
    Ok(())
}

fn command_error_response(
    err: crate::domains::common::CommandError,
) -> (StatusCode, Json<ErrorResponse>) {
    use crate::domains::common::CommandErrorKind;
    match err.kind {
        CommandErrorKind::BadRequest(msg) => bad_request(msg),
        CommandErrorKind::Forbidden(msg) => forbidden(msg),
        CommandErrorKind::NotFound(_) => not_found(),
        CommandErrorKind::Conflict(msg) => {
            ErrorResponse::new(msg).into_response(StatusCode::CONFLICT)
        }
        CommandErrorKind::RateLimited(msg) => {
            ErrorResponse::new(msg).into_response(StatusCode::TOO_MANY_REQUESTS)
        }
        CommandErrorKind::Unprocessable(msg) => {
            ErrorResponse::new(msg).into_response(StatusCode::UNPROCESSABLE_ENTITY)
        }
        CommandErrorKind::Internal(error) => internal_error(error),
    }
}

fn api_auth_error_response(error: AppEndpointAuthError) -> (StatusCode, Json<ErrorResponse>) {
    match error {
        AppEndpointAuthError::Unauthorized => unauthorized(),
        AppEndpointAuthError::Misconfigured => forbidden("api_endpoint auth is misconfigured"),
        AppEndpointAuthError::ProviderUnavailable => {
            ErrorResponse::new("api_endpoint auth provider is unavailable".to_string())
                .into_response(StatusCode::SERVICE_UNAVAILABLE)
        }
    }
}

fn bad_request(message: impl Into<String>) -> (StatusCode, Json<ErrorResponse>) {
    ErrorResponse::new(message.into()).into_response(StatusCode::BAD_REQUEST)
}

fn forbidden(message: impl Into<String>) -> (StatusCode, Json<ErrorResponse>) {
    ErrorResponse::new(message.into()).into_response(StatusCode::FORBIDDEN)
}

fn unauthorized() -> (StatusCode, Json<ErrorResponse>) {
    ErrorResponse::new("Invalid or missing API key".to_string())
        .into_response(StatusCode::UNAUTHORIZED)
}

fn not_found() -> (StatusCode, Json<ErrorResponse>) {
    ErrorResponse::new("App channel or session not found".to_string())
        .into_response(StatusCode::NOT_FOUND)
}

fn too_many_requests(message: &str) -> (StatusCode, Json<ErrorResponse>) {
    ErrorResponse::new(message.to_string()).into_response(StatusCode::TOO_MANY_REQUESTS)
}

fn internal_error(error: anyhow::Error) -> (StatusCode, Json<ErrorResponse>) {
    tracing::error!(error = %error, "Failed to handle app api_endpoint request");
    ErrorResponse::new("Internal server error".to_string())
        .into_response(StatusCode::INTERNAL_SERVER_ERROR)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_api_key_accepts_case_insensitive_bearer() {
        let hash = hash_app_api_key("evr_app_secret");
        for header in ["Bearer evr_app_secret", "bearer evr_app_secret"] {
            let mut headers = HeaderMap::new();
            headers.insert(axum::http::header::AUTHORIZATION, header.parse().unwrap());
            assert!(verify_api_key(&headers, &hash).is_ok(), "header: {header}");
        }
        // Wrong key and missing header are rejected.
        let mut wrong = HeaderMap::new();
        wrong.insert(
            axum::http::header::AUTHORIZATION,
            "Bearer evr_app_other".parse().unwrap(),
        );
        assert!(verify_api_key(&wrong, &hash).is_err());
        assert!(verify_api_key(&HeaderMap::new(), &hash).is_err());
    }

    /// TM-APIKEY-004: reads must surface only final assistant text — never
    /// commentary, tool calls, or other internal detail.
    #[test]
    fn project_session_output_returns_only_final_assistant_text() {
        use chrono::Utc;
        use everruns_core::ContentPart;
        use everruns_core::message::Message;
        use everruns_provider::execution_phase::ExecutionPhase;
        use everruns_provider::typed_id::{EventId, SessionId};
        use serde_json::json;

        let sid = SessionId::new();
        let mut seq = 0;
        let mut output_event = |msg: &Message| {
            seq += 1;
            EventRow {
                id: EventId::new(),
                session_id: sid,
                sequence: seq,
                event_type: OUTPUT_MESSAGE_COMPLETED.to_string(),
                ts: Utc::now(),
                context: json!({}),
                data: json!({ "message": serde_json::to_value(msg).unwrap() }),
                metadata: None,
                tags: None,
                created_at: Utc::now(),
            }
        };

        // Intermediate commentary — must be excluded.
        let commentary =
            Message::assistant("internal commentary").with_phase(ExecutionPhase::Commentary);
        // Final answer that also carries a tool call part — only the text
        // should survive; the tool name must never leak.
        let mut final_msg = Message::assistant("the final answer");
        final_msg.content.push(ContentPart::tool_call(
            "call_1",
            "secret_internal_tool",
            json!({ "arg": "sensitive" }),
        ));
        final_msg.phase = Some(ExecutionPhase::FinalAnswer);

        let commentary_evt = output_event(&commentary);
        let final_evt = output_event(&final_msg);
        let turn_done = EventRow {
            id: EventId::new(),
            session_id: sid,
            sequence: 99,
            event_type: TURN_COMPLETED.to_string(),
            ts: Utc::now(),
            context: json!({}),
            data: json!({}),
            metadata: None,
            tags: None,
            created_at: Utc::now(),
        };

        let (status, messages) = project_session_output(&[commentary_evt, final_evt, turn_done]);

        assert_eq!(status, "completed");
        assert_eq!(messages.len(), 1, "only the final assistant message");
        assert_eq!(messages[0].role, "agent");
        assert_eq!(messages[0].text, "the final answer");
        // Neither commentary nor tool detail may appear anywhere.
        let all = messages.iter().map(|m| m.text.as_str()).collect::<String>();
        assert!(!all.contains("commentary"));
        assert!(!all.contains("secret_internal_tool"));
    }
}

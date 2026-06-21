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
// See `specs/app-api-keys.md`.

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
use everruns_core::message::ExecutionPhase;
use serde::{Deserialize, Serialize};

use crate::api::app_endpoint_auth::{
    AppEndpointAuthError, AppEndpointAuthVerifier, LegacyEndpointAuth,
};
use crate::api::channel_rate_limit::ChannelRateLimiter;
use crate::api::common::ErrorResponse;
use crate::auth::rate_limit::extract_client_ip_from_parts;
use crate::domains::apps::{
    ApiInvocationRequest, hash_app_api_key, invoke_api_app_channel, post_api_app_channel_message,
    queries as app_queries, session_has_app_channel_tags,
};
use crate::domains::messages::MessageService;
use crate::domains::sessions::SessionService;
use crate::event_delivery::EventDelivery;
use crate::middleware::RequestId;
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
        runner: Arc<dyn everruns_worker::AgentRunner>,
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

#[derive(Debug, Deserialize)]
struct MessageBody {
    /// Message text dispatched to the agent.
    message: String,
}

#[derive(Debug, Serialize)]
struct SessionRef {
    session_id: String,
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    created_session: Option<bool>,
}

#[derive(Debug, Serialize)]
struct AgentMessage {
    role: &'static str,
    text: String,
}

#[derive(Debug, Serialize)]
struct SessionStatus {
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
async fn create_session(
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
async fn post_message(
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
    let session_id = match session_id.parse::<everruns_core::typed_id::SessionId>() {
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
async fn get_session(
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
    let session_id = match session_id.parse::<everruns_core::typed_id::SessionId>() {
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
async fn cancel_session(
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
    let session_id = match session_id.parse::<everruns_core::typed_id::SessionId>() {
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
    let app = app_queries::get_by_public_id_unscoped(&state.db, state.encryption.as_ref(), app_id)
        .await
        .map_err(internal_error)?
        .ok_or_else(not_found)?;

    if app.status != everruns_core::AppStatus::Published {
        return Err(forbidden("App is not published"));
    }

    let channel_id_typed = channel_id
        .parse::<everruns_core::typed_id::AppChannelId>()
        .map_err(|e| bad_request(format!("Invalid channel ID: {e}")))?;
    let channel = app.channel_by_id(&channel_id_typed).ok_or_else(not_found)?;
    if channel.channel_type != everruns_core::ChannelType::ApiEndpoint {
        return Err(not_found());
    }
    // THREAT[TM-APIKEY-001]: anonymous ingress must never reach a draft or
    // disabled channel, and every request must present the per-channel key.
    if !channel.enabled {
        return Err(forbidden("api_endpoint channel is disabled"));
    }

    let config = channel
        .api_endpoint_config()
        .ok_or_else(|| bad_request("Invalid api_endpoint channel configuration"))?;

    if let Some(auth) = config.auth.as_ref() {
        if auth.mode == everruns_core::AppEndpointAuthMode::ApiKey {
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
        let scope = format!("{}:{}", app.public_id, channel_id_typed);
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
        channel_public_id: channel_id_typed.to_string(),
    })
}

fn verify_api_key(
    headers: &HeaderMap,
    expected_hash: &str,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    // THREAT[TM-APIKEY-001]: keys are stored as SHA-256 hashes; plaintext is
    // never persisted. Hash the inbound key and constant-time compare.
    let provided_key = extract_api_key(headers).ok_or_else(unauthorized)?;
    let provided_hash = hash_app_api_key(&provided_key);
    if constant_time_eq(provided_hash.as_bytes(), expected_hash.as_bytes()) {
        Ok(())
    } else {
        Err(unauthorized())
    }
}

fn extract_api_key(headers: &HeaderMap) -> Option<String> {
    let auth = headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?;
    auth.strip_prefix("Bearer ").map(ToOwned::to_owned)
}

// THREAT[TM-APIKEY-001]: constant-time comparison of the SHA-256 hex digests
// avoids leaking partial-match timing information.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter()
        .zip(b.iter())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}

/// Read the derived status and the agent's completed messages for a session.
/// Only final (non-Commentary) assistant messages are surfaced — raw tool
/// names, arguments, and results are never exposed to the execution key
/// (TM-APIKEY-004).
async fn read_session_output(
    db: &Arc<StorageBackend>,
    session_id: everruns_core::typed_id::SessionId,
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

    let mut status = "submitted";
    let mut messages = Vec::new();
    for evt in &events {
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
    Ok((status, messages))
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
    session_id: everruns_core::typed_id::SessionId,
) -> anyhow::Result<()> {
    use everruns_core::events::{EventContext, EventRequest, TurnCancelledData};
    use everruns_core::typed_id::{MessageId, TurnId};

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
    fn constant_time_eq_matches() {
        assert!(constant_time_eq(b"abcd", b"abcd"));
        assert!(!constant_time_eq(b"abcd", b"abce"));
        assert!(!constant_time_eq(b"abc", b"abcd"));
    }

    #[test]
    fn extract_api_key_reads_bearer() {
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            "Bearer evr_app_abc".parse().unwrap(),
        );
        assert_eq!(extract_api_key(&headers).as_deref(), Some("evr_app_abc"));
    }

    #[test]
    fn extract_api_key_rejects_missing() {
        let headers = HeaderMap::new();
        assert!(extract_api_key(&headers).is_none());
    }
}

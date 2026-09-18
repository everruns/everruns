// App webhook ingress — endpoint-scoped token-authenticated invocation.
//
// Design Decision: Webhooks use `POST /v1/e/{channel_id}/webhook` so one app
// can expose multiple entry points with different tokens and invocation
// behavior. App-and-channel routes remain permanent aliases.

use std::collections::HashMap;
use std::sync::Arc;

use axum::{
    Json, Router,
    body::Bytes,
    extract::{ConnectInfo, Extension, Path, State},
    http::{HeaderMap, StatusCode},
    routing::post,
};
use utoipa::ToSchema;

use crate::api::channel_rate_limit::ChannelRateLimiter;
use crate::api::common::ErrorResponse;
use crate::auth::rate_limit::extract_client_ip_from_parts;
use crate::domains::agent_triggers::{
    WebhookTriggerInvocationRequest, invoke_webhook_agent_trigger,
};
use crate::domains::apps::{WebhookInvocationRequest, invoke_webhook_app_channel};
use crate::domains::common::{CommandError, CommandErrorKind};
use crate::domains::messages::MessageService;
use crate::domains::sessions::SessionService;
use crate::middleware::RequestId;
use crate::security::constant_time_eq;
use crate::storage::{EncryptionService, StorageBackend};

const REDACTED_HEADER_VALUE: &str = "[REDACTED]";
const SENSITIVE_WEBHOOK_HEADERS: &[&str] = &[
    "authorization",
    "cookie",
    "proxy-authorization",
    "x-everruns-webhook-token",
];

#[derive(Clone)]
pub struct AppWebhookState {
    pub db: Arc<StorageBackend>,
    pub encryption: Option<Arc<EncryptionService>>,
    pub session_service: Arc<SessionService>,
    pub message_service: Arc<MessageService>,
    /// Per-channel, per-IP rate limiter (namespace `webhook`). Mirrors the
    /// sibling public channels (AG-UI/A2A/app_api/FCP) so a webhook token
    /// holder cannot drive unbounded session/LLM burn (EVE-627, TM-DOS-010).
    pub rate_limiter: ChannelRateLimiter,
}

impl AppWebhookState {
    pub fn new(
        db: Arc<StorageBackend>,
        encryption: Option<Arc<EncryptionService>>,
        runner: Arc<dyn everruns_worker::AgentRunner>,
        notifications_enabled: bool,
        event_delivery: crate::event_delivery::EventDelivery,
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
        }
    }
}

/// Response body for webhook invocation.
#[derive(Debug, serde::Serialize, ToSchema)]
pub struct WebhookInvocationResponse {
    pub accepted: bool,
    #[schema(value_type = String)]
    /// Session's prefixed public identifier.
    pub session_id: everruns_provider::typed_id::SessionId,
    pub created_session: bool,
}

pub fn routes(state: AppWebhookState) -> Router {
    Router::new()
        .route(
            "/v1/apps/{app_id}/webhooks/{channel_id}",
            post(invoke_webhook_legacy),
        )
        .route("/v1/e/{channel_id}/webhook", post(invoke_webhook_endpoint))
        .with_state(state)
}

#[utoipa::path(
    description = "Invoke a webhook channel for a published App. The body is forwarded to the agent as a message.",
    post,
    path = "/v1/apps/{app_id}/webhooks/{channel_id}",
    params(
        ("app_id" = String, Path, description = "App ID"),
        ("channel_id" = String, Path, description = "Webhook channel ID")
    ),
    request_body(content = String, content_type = "application/octet-stream"),
    responses(
        (status = 202, description = "Webhook accepted", body = WebhookInvocationResponse),
        (status = 401, description = "Invalid or missing webhook token", body = ErrorResponse),
        (status = 404, description = "App or channel not found, not published, or channel disabled (collapsed to a single generic 404 to prevent app-existence enumeration)", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "apps"
)]
pub async fn invoke_webhook_legacy(
    State(state): State<AppWebhookState>,
    Path((app_id, channel_id)): Path<(String, String)>,
    req_id: Option<axum::Extension<RequestId>>,
    connect_info: Option<Extension<ConnectInfo<std::net::SocketAddr>>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<(StatusCode, Json<WebhookInvocationResponse>), (StatusCode, Json<ErrorResponse>)> {
    invoke_webhook(
        state,
        Some(app_id),
        channel_id,
        req_id,
        connect_info,
        headers,
        body,
    )
    .await
}

#[utoipa::path(
    description = "Invoke a published webhook endpoint. Authenticate with its channel token in Authorization: Bearer or X-Everruns-Webhook-Token.",
    post,
    path = "/v1/e/{channel_id}/webhook",
    params(("channel_id" = String, Path, description = "Webhook endpoint channel ID")),
    request_body(content = String, content_type = "application/octet-stream"),
    responses(
        (status = 202, description = "Webhook accepted", body = WebhookInvocationResponse),
        (status = 401, description = "Invalid or missing webhook token", body = ErrorResponse),
        (status = 404, description = "Endpoint not found, app not published, or channel disabled", body = ErrorResponse),
        (status = 429, description = "Per-channel rate limit exceeded", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "apps"
)]
pub async fn invoke_webhook_endpoint(
    State(state): State<AppWebhookState>,
    Path(channel_id): Path<String>,
    req_id: Option<axum::Extension<RequestId>>,
    connect_info: Option<Extension<ConnectInfo<std::net::SocketAddr>>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<(StatusCode, Json<WebhookInvocationResponse>), (StatusCode, Json<ErrorResponse>)> {
    if state
        .db
        .get_agent_trigger_by_ingress_id_unscoped(&channel_id)
        .await
        .map_err(internal_error)?
        .is_some()
    {
        return invoke_webhook(state, None, channel_id, req_id, connect_info, headers, body).await;
    }
    invoke_webhook(state, None, channel_id, req_id, connect_info, headers, body).await
}

async fn invoke_webhook(
    state: AppWebhookState,
    app_id: Option<String>,
    channel_id: String,
    req_id: Option<axum::Extension<RequestId>>,
    connect_info: Option<Extension<ConnectInfo<std::net::SocketAddr>>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<(StatusCode, Json<WebhookInvocationResponse>), (StatusCode, Json<ErrorResponse>)> {
    if let Some(trigger) = state
        .db
        .get_agent_trigger_by_ingress_id_unscoped(&channel_id)
        .await
        .map_err(internal_error)?
    {
        if app_id.is_some() {
            return Err(not_found());
        }
        return invoke_trigger_webhook(
            state,
            channel_id,
            trigger,
            req_id,
            connect_info,
            headers,
            body,
        )
        .await;
    }
    let (app, channel) = crate::api::app_ingress::resolve_endpoint(
        &state.db,
        state.encryption.as_ref(),
        &channel_id,
    )
    .await
    .map_err(internal_error)?
    .ok_or_else(not_found)?;
    if app_id
        .as_deref()
        .is_some_and(|legacy_app_id| !app.matches_legacy_app_id(legacy_app_id))
    {
        return Err(not_found());
    }
    let app_id = app.public_id.to_string();

    // THREAT[TM-TENANT-002]: An unauthenticated caller must not be able to tell
    // "app does not exist" apart from "app exists but is not published / the
    // channel is disabled / misconfigured". Every such case collapses to a
    // single generic 404 (matching the FCP channel in `api/fcp.rs`); the real
    // reason is logged server-side only.
    if channel.channel_type != everruns_platform::ChannelType::Webhook {
        return Err(not_found());
    }
    // THREAT[TM-AUTHZ-006]: Anonymous webhook ingress must never reach a
    // non-live endpoint, and every request must present the per-channel shared
    // secret before session creation. Liveness is resolved before auth so a
    // caller cannot distinguish a misconfigured endpoint from a bad token.
    if let Err(reason) = crate::api::app_ingress::endpoint_liveness(&app, &channel) {
        tracing::debug!(
            app_id = %app.public_id,
            endpoint_id = %channel.public_id,
            reason = reason.as_str(),
            "Webhook request rejected: endpoint not live"
        );
        return Err(not_found());
    }
    let Some(config) = channel.webhook_config() else {
        tracing::error!(app_id = %app.public_id, "Webhook channel config did not deserialize");
        return Err(not_found());
    };
    let provided_token = extract_webhook_token(&headers).ok_or_else(unauthorized)?;
    // EVE-627 (TM-AUTHZ-006 / TM-APIKEY-003): constant-time comparison, matching
    // every sibling channel — a `!=` on the secret leaks a remote timing
    // side-channel that can recover the token byte by byte.
    if !constant_time_eq(provided_token.as_bytes(), config.token.as_bytes()) {
        return Err(unauthorized());
    }

    // EVE-627 (TM-DOS-010): per-channel, per-IP rate limit after the secret is
    // verified (so rejected traffic costs nothing) and before any session/LLM
    // work. Scope includes the channel id so multiple webhook channels on one
    // app keep independent buckets. `None`/`0` disables (global limit applies).
    if let Some(limit) = config.rate_limit_per_minute
        && limit > 0
    {
        let peer_addr = connect_info.map(|Extension(ConnectInfo(addr))| addr);
        let client_ip = extract_client_ip_from_parts(peer_addr, &headers);
        let scope = format!("{app_id}:{channel_id}");
        if state
            .rate_limiter
            .check(&scope, client_ip, limit)
            .await
            .is_err()
        {
            return Err(too_many_requests(
                "Webhook rate limit exceeded for this app channel",
            ));
        }
    }

    let json_payload = serde_json::from_slice(&body).ok();
    let body = String::from_utf8_lossy(&body).into_owned();
    let request_headers = flatten_headers(&headers);
    let request_id = req_id.map(|axum::Extension(id)| id.0);

    let result = invoke_webhook_app_channel(
        &state.db,
        state.encryption.as_ref(),
        &state.session_service,
        &state.message_service,
        WebhookInvocationRequest {
            app_id,
            channel_id,
            body,
            json_payload,
            headers: request_headers,
        },
        request_id,
    )
    .await
    .map_err(command_error_response)?;

    Ok((
        StatusCode::ACCEPTED,
        Json(WebhookInvocationResponse {
            accepted: true,
            session_id: result.session_id,
            created_session: result.created_session,
        }),
    ))
}

#[allow(clippy::too_many_arguments)]
async fn invoke_trigger_webhook(
    state: AppWebhookState,
    ingress_id: String,
    trigger: crate::storage::models::AgentTriggerRow,
    req_id: Option<axum::Extension<RequestId>>,
    connect_info: Option<Extension<ConnectInfo<std::net::SocketAddr>>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<(StatusCode, Json<WebhookInvocationResponse>), (StatusCode, Json<ErrorResponse>)> {
    if trigger.trigger_type != everruns_platform::AgentTriggerType::Webhook.to_string()
        || !trigger.enabled
    {
        return Err(not_found());
    }
    let agent_is_live = state
        .db
        .get_agent(trigger.org_id, trigger.agent_id)
        .await
        .map_err(internal_error)?
        .is_some_and(|agent| agent.status == "active" && !agent.exposures_suspended);
    if !agent_is_live {
        return Err(not_found());
    }

    let config_value = crate::domains::apps::queries::decrypt_channel_config(
        state.encryption.as_ref(),
        trigger.config_encrypted.as_deref(),
        &trigger.config,
    );
    let config: everruns_platform::WebhookTriggerConfig = serde_json::from_value(config_value)
        .map_err(|error| {
            tracing::error!(%error, %ingress_id, "Webhook trigger config did not deserialize");
            not_found()
        })?;
    let provided_token = extract_webhook_token(&headers).ok_or_else(unauthorized)?;
    if !constant_time_eq(provided_token.as_bytes(), config.token.as_bytes()) {
        return Err(unauthorized());
    }
    if let Some(limit) = config.rate_limit_per_minute
        && limit > 0
    {
        let peer_addr = connect_info.map(|Extension(ConnectInfo(addr))| addr);
        let client_ip = extract_client_ip_from_parts(peer_addr, &headers);
        if state
            .rate_limiter
            .check(&ingress_id, client_ip, limit)
            .await
            .is_err()
        {
            return Err(too_many_requests(
                "Webhook rate limit exceeded for this app channel",
            ));
        }
    }

    let json_payload = serde_json::from_slice(&body).ok();
    let body = String::from_utf8_lossy(&body).into_owned();
    let request_headers = flatten_headers(&headers);
    let request_id = req_id.map(|axum::Extension(id)| id.0);
    let result = invoke_webhook_agent_trigger(
        &state.db,
        state.encryption.as_ref(),
        &state.session_service,
        &state.message_service,
        WebhookTriggerInvocationRequest {
            ingress_id,
            body,
            json_payload,
            headers: request_headers,
        },
        request_id,
    )
    .await
    .map_err(command_error_response)?;
    Ok((
        StatusCode::ACCEPTED,
        Json(WebhookInvocationResponse {
            accepted: true,
            session_id: result.session_id,
            created_session: result.created_session,
        }),
    ))
}

fn extract_webhook_token(headers: &HeaderMap) -> Option<String> {
    if let Some(value) = headers.get("x-everruns-webhook-token") {
        return value.to_str().ok().map(ToOwned::to_owned);
    }

    let auth = headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?;
    auth.strip_prefix("Bearer ").map(ToOwned::to_owned)
}

fn flatten_headers(headers: &HeaderMap) -> HashMap<String, String> {
    headers
        .iter()
        .filter_map(|(name, value)| {
            let normalized_name = name.as_str().to_ascii_lowercase();
            if SENSITIVE_WEBHOOK_HEADERS.contains(&normalized_name.as_str()) {
                return Some((normalized_name, REDACTED_HEADER_VALUE.to_string()));
            }
            value
                .to_str()
                .ok()
                .map(|value| (normalized_name, value.to_string()))
        })
        .collect()
}

fn bad_request(message: impl Into<String>) -> (StatusCode, Json<ErrorResponse>) {
    ErrorResponse::new(message.into()).into_response(StatusCode::BAD_REQUEST)
}

fn forbidden(message: impl Into<String>) -> (StatusCode, Json<ErrorResponse>) {
    ErrorResponse::new(message.into()).into_response(StatusCode::FORBIDDEN)
}

fn too_many_requests(message: impl Into<String>) -> (StatusCode, Json<ErrorResponse>) {
    ErrorResponse::new(message.into()).into_response(StatusCode::TOO_MANY_REQUESTS)
}

fn unauthorized() -> (StatusCode, Json<ErrorResponse>) {
    ErrorResponse::new("Invalid or missing webhook token".to_string())
        .into_response(StatusCode::UNAUTHORIZED)
}

fn not_found() -> (StatusCode, Json<ErrorResponse>) {
    ErrorResponse::new("App channel not found".to_string()).into_response(StatusCode::NOT_FOUND)
}

fn internal_error(error: anyhow::Error) -> (StatusCode, Json<ErrorResponse>) {
    tracing::error!(error = %error, "Failed to invoke app webhook");
    ErrorResponse::new("Internal server error".to_string())
        .into_response(StatusCode::INTERNAL_SERVER_ERROR)
}

fn command_error_response(error: CommandError) -> (StatusCode, Json<ErrorResponse>) {
    match error {
        CommandError {
            kind: CommandErrorKind::BadRequest(message),
            ..
        } => bad_request(message),
        CommandError {
            kind: CommandErrorKind::Unprocessable(message),
            ..
        } => ErrorResponse::new(message).into_response(StatusCode::UNPROCESSABLE_ENTITY),
        CommandError {
            kind: CommandErrorKind::Forbidden(message),
            ..
        } => forbidden(message),
        CommandError {
            kind: CommandErrorKind::NotFound(_),
            ..
        } => not_found(),
        CommandError {
            kind: CommandErrorKind::Conflict(message),
            ..
        } => ErrorResponse::new(message).into_response(StatusCode::CONFLICT),
        CommandError {
            kind: CommandErrorKind::RateLimited(message),
            ..
        } => ErrorResponse::new(message).into_response(StatusCode::TOO_MANY_REQUESTS),
        CommandError {
            kind: CommandErrorKind::Internal(error),
            ..
        } => internal_error(error),
    }
}

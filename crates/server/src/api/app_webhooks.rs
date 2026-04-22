// App webhook ingress — app-scoped token-authenticated invocation endpoint.
//
// Design Decision: Webhooks stay app-scoped and channel-scoped
// (`POST /v1/apps/{app_id}/webhooks/{channel_id}`) so one app can expose
// multiple webhook entry points with different tokens and invocation behavior.

use std::collections::HashMap;
use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::post,
};
use utoipa::ToSchema;

use crate::api::common::ErrorResponse;
use crate::domains::apps::{WebhookInvocationRequest, invoke_webhook_app_channel};
use crate::domains::common::CommandError;
use crate::middleware::RequestId;
use crate::services::{MessageService, SessionService};
use crate::storage::{EncryptionService, StorageBackend};

#[derive(Clone)]
pub struct AppWebhookState {
    pub db: Arc<StorageBackend>,
    pub encryption: Option<Arc<EncryptionService>>,
    pub session_service: Arc<SessionService>,
    pub message_service: Arc<MessageService>,
}

impl AppWebhookState {
    pub fn new(
        db: Arc<StorageBackend>,
        encryption: Option<Arc<EncryptionService>>,
        runner: Arc<dyn everruns_worker::AgentRunner>,
        notifications_enabled: bool,
        event_delivery: crate::event_delivery::EventDelivery,
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
        }
    }
}

#[derive(Debug, serde::Serialize, ToSchema)]
pub struct WebhookInvocationResponse {
    pub accepted: bool,
    #[schema(value_type = String)]
    pub session_id: everruns_core::typed_id::SessionId,
    pub created_session: bool,
}

pub fn routes(state: AppWebhookState) -> Router {
    Router::new()
        .route(
            "/v1/apps/{app_id}/webhooks/{channel_id}",
            post(invoke_webhook),
        )
        .with_state(state)
}

#[utoipa::path(
    post,
    path = "/v1/apps/{app_id}/webhooks/{channel_id}",
    params(
        ("app_id" = String, Path, description = "App ID"),
        ("channel_id" = String, Path, description = "Webhook channel ID")
    ),
    responses(
        (status = 202, description = "Webhook accepted", body = WebhookInvocationResponse),
        (status = 401, description = "Invalid or missing webhook token", body = ErrorResponse),
        (status = 403, description = "App is not published or channel disabled", body = ErrorResponse),
        (status = 404, description = "App or channel not found", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "apps"
)]
pub async fn invoke_webhook(
    State(state): State<AppWebhookState>,
    Path((app_id, channel_id)): Path<(String, String)>,
    req_id: Option<axum::Extension<RequestId>>,
    headers: HeaderMap,
    body: String,
) -> Result<(StatusCode, Json<WebhookInvocationResponse>), (StatusCode, Json<ErrorResponse>)> {
    let app = crate::domains::apps::queries::get_by_public_id_unscoped(
        &state.db,
        state.encryption.as_ref(),
        &app_id,
    )
    .await
    .map_err(internal_error)?
    .ok_or_else(not_found)?;

    if app.status != everruns_core::AppStatus::Published {
        return Err(forbidden("App is not published"));
    }

    let channel_id_typed = channel_id
        .parse::<everruns_core::typed_id::AppChannelId>()
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: format!("Invalid channel ID: {e}"),
                }),
            )
        })?;
    let channel = app.channel_by_id(&channel_id_typed).ok_or_else(not_found)?;
    if channel.channel_type != everruns_core::ChannelType::Webhook {
        return Err(not_found());
    }
    // THREAT[TM-AUTHZ-006]: Anonymous webhook ingress must never reach draft
    // or disabled app channels, and every request must present the per-channel
    // shared secret before session creation.
    if !channel.enabled {
        return Err(forbidden("Webhook channel is disabled"));
    }
    let config = channel
        .webhook_config()
        .ok_or_else(|| bad_request("Invalid webhook channel configuration"))?;
    let provided_token = extract_webhook_token(&headers).ok_or_else(unauthorized)?;
    if provided_token != config.token {
        return Err(unauthorized());
    }

    let json_payload = serde_json::from_str(&body).ok();
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
            value
                .to_str()
                .ok()
                .map(|value| (name.as_str().to_ascii_lowercase(), value.to_string()))
        })
        .collect()
}

fn bad_request(message: impl Into<String>) -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::BAD_REQUEST,
        Json(ErrorResponse {
            error: message.into(),
        }),
    )
}

fn forbidden(message: impl Into<String>) -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::FORBIDDEN,
        Json(ErrorResponse {
            error: message.into(),
        }),
    )
}

fn unauthorized() -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::UNAUTHORIZED,
        Json(ErrorResponse {
            error: "Invalid or missing webhook token".to_string(),
        }),
    )
}

fn not_found() -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::NOT_FOUND,
        Json(ErrorResponse {
            error: "App channel not found".to_string(),
        }),
    )
}

fn internal_error(error: anyhow::Error) -> (StatusCode, Json<ErrorResponse>) {
    tracing::error!(error = %error, "Failed to invoke app webhook");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorResponse {
            error: "Internal server error".to_string(),
        }),
    )
}

fn command_error_response(error: CommandError) -> (StatusCode, Json<ErrorResponse>) {
    match error {
        CommandError::BadRequest(message) => bad_request(message),
        CommandError::Unprocessable(message) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(ErrorResponse { error: message }),
        ),
        CommandError::Forbidden(message) => forbidden(message),
        CommandError::NotFound(_) => not_found(),
        CommandError::Conflict(message) => {
            (StatusCode::CONFLICT, Json(ErrorResponse { error: message }))
        }
        CommandError::Internal(error) => internal_error(error),
    }
}

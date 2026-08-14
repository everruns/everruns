// Organization task webhook CRUD — outbound HTTP targets notified on terminal
// task transitions (Succeeded / Failed / Canceled). See EVE-579 and
// knowledge/runtime-resources/session-tasks.md.

use crate::auth::middleware::{AuthState, OrgAdmin};
use crate::storage::{
    StorageBackend,
    models::{CreateOrgTaskWebhook, UpdateOrgTaskWebhook},
};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::get,
};
use chrono::{DateTime, Utc};
use everruns_provider::url_validation::validate_safe_url;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

use super::common::{ApiOptionExt, ApiResultExt, ErrorResponse, ListResponse, impl_auth_state};

/// App state for task webhook routes.
#[derive(Clone)]
pub struct AppState {
    pub db: Arc<StorageBackend>,
    pub auth: AuthState,
}

impl AppState {
    pub fn new(db: Arc<StorageBackend>, auth: AuthState) -> Self {
        Self { db, auth }
    }
}

impl_auth_state!(AppState);

fn generate_webhook_public_id() -> String {
    let uuid = Uuid::new_v4();
    format!("wh_{}", uuid.simple())
}

/// A configured outbound webhook target.
#[derive(Debug, Serialize, ToSchema)]
pub struct TaskWebhookResponse {
    /// Public identifier (wh_<32-hex-chars>).
    pub id: String,
    /// Target URL that receives POST requests.
    pub url: String,
    /// Whether this webhook is enabled.
    pub enabled: bool,
    /// Whether a signing secret is configured (the secret itself is never returned).
    pub has_secret: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl TaskWebhookResponse {
    fn from_row(row: crate::storage::models::OrgTaskWebhookRow) -> Self {
        Self {
            id: row.public_id,
            url: row.url,
            enabled: row.enabled,
            has_secret: row.secret.is_some(),
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

/// Request body for creating a task webhook.
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateTaskWebhookRequest {
    /// The URL to POST task terminal-state events to.
    pub url: String,
    /// Optional HMAC-SHA256 signing secret. When set, every delivery includes
    /// an `X-Everruns-Signature: sha256=<hex>` header.
    #[serde(default)]
    pub secret: Option<String>,
    /// Whether the webhook starts enabled. Defaults to true.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

fn default_enabled() -> bool {
    true
}

/// Request body for updating a task webhook.
#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateTaskWebhookRequest {
    /// New target URL.
    #[serde(default)]
    pub url: Option<String>,
    /// Replace the signing secret. Pass null to remove the existing secret.
    #[serde(default, deserialize_with = "double_option")]
    pub secret: Option<Option<String>>,
    /// Enable or disable the webhook.
    #[serde(default)]
    pub enabled: Option<bool>,
}

fn double_option<'de, T, D>(de: D) -> Result<Option<Option<T>>, D::Error>
where
    T: serde::Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    // Deserialize Option<T> and wrap in Some so the outer Option distinguishes
    // between "field absent" (default None) and "field present with null value"
    // (Some(None) = clear the secret).
    Option::<T>::deserialize(de).map(Some)
}

pub fn routes(state: AppState) -> Router {
    Router::new()
        .route("/v1/task-webhooks", get(list_webhooks).post(create_webhook))
        .route(
            "/v1/task-webhooks/{webhook_id}",
            get(get_webhook)
                .patch(update_webhook)
                .delete(delete_webhook),
        )
        .with_state(state)
}

/// List all task webhooks configured for the organization.
#[utoipa::path(
    get,
    path = "/v1/task-webhooks",
    responses(
        (status = 200, description = "Webhook list", body = ListResponse<TaskWebhookResponse>),
        (status = 401, description = "Unauthorized", body = ErrorResponse),
        (status = 403, description = "Forbidden — requires admin", body = ErrorResponse),
    ),
    tag = "task-webhooks"
)]
pub async fn list_webhooks(
    State(state): State<AppState>,
    OrgAdmin(org): OrgAdmin,
) -> Result<Json<ListResponse<TaskWebhookResponse>>, (StatusCode, Json<ErrorResponse>)> {
    let rows = state
        .db
        .list_org_task_webhooks(org.org_id)
        .await
        .log_internal_error_json("list task webhooks")?;
    let data = rows
        .into_iter()
        .map(TaskWebhookResponse::from_row)
        .collect();
    Ok(Json(ListResponse { data }))
}

/// Create a new task webhook for the organization.
#[utoipa::path(
    post,
    path = "/v1/task-webhooks",
    request_body = CreateTaskWebhookRequest,
    responses(
        (status = 201, description = "Webhook created", body = TaskWebhookResponse),
        (status = 400, description = "Invalid request", body = ErrorResponse),
        (status = 401, description = "Unauthorized", body = ErrorResponse),
        (status = 403, description = "Forbidden — requires admin", body = ErrorResponse),
    ),
    tag = "task-webhooks"
)]
pub async fn create_webhook(
    State(state): State<AppState>,
    OrgAdmin(org): OrgAdmin,
    Json(req): Json<CreateTaskWebhookRequest>,
) -> Result<(StatusCode, Json<TaskWebhookResponse>), (StatusCode, Json<ErrorResponse>)> {
    // THREAT[TM-API-020]: Validate webhook URL against private/internal ranges
    // to prevent SSRF. Uses the same static-check logic as MCP server URLs.
    validate_safe_url(&req.url).map_err(|e| {
        ErrorResponse::new(format!("Invalid webhook URL: {e}"))
            .into_response(StatusCode::BAD_REQUEST)
    })?;
    let input = CreateOrgTaskWebhook {
        public_id: generate_webhook_public_id(),
        org_id: org.org_id,
        url: req.url,
        secret: req.secret,
        enabled: req.enabled,
    };
    let row = state
        .db
        .create_org_task_webhook(input)
        .await
        .log_internal_error_json("create task webhook")?;
    Ok((
        StatusCode::CREATED,
        Json(TaskWebhookResponse::from_row(row)),
    ))
}

/// Get a single task webhook.
#[utoipa::path(
    get,
    path = "/v1/task-webhooks/{webhook_id}",
    params(("webhook_id" = String, Path, description = "Webhook public ID")),
    responses(
        (status = 200, description = "Webhook", body = TaskWebhookResponse),
        (status = 401, description = "Unauthorized", body = ErrorResponse),
        (status = 403, description = "Forbidden — requires admin", body = ErrorResponse),
        (status = 404, description = "Webhook not found", body = ErrorResponse),
    ),
    tag = "task-webhooks"
)]
pub async fn get_webhook(
    State(state): State<AppState>,
    OrgAdmin(org): OrgAdmin,
    Path(webhook_id): Path<String>,
) -> Result<Json<TaskWebhookResponse>, (StatusCode, Json<ErrorResponse>)> {
    let row = state
        .db
        .get_org_task_webhook(org.org_id, &webhook_id)
        .await
        .log_internal_error_json("get task webhook")?
        .ok_or_not_found_json("Webhook")?;
    Ok(Json(TaskWebhookResponse::from_row(row)))
}

/// Update a task webhook.
#[utoipa::path(
    patch,
    path = "/v1/task-webhooks/{webhook_id}",
    params(("webhook_id" = String, Path, description = "Webhook public ID")),
    request_body = UpdateTaskWebhookRequest,
    responses(
        (status = 200, description = "Updated webhook", body = TaskWebhookResponse),
        (status = 400, description = "Invalid request", body = ErrorResponse),
        (status = 401, description = "Unauthorized", body = ErrorResponse),
        (status = 403, description = "Forbidden — requires admin", body = ErrorResponse),
        (status = 404, description = "Webhook not found", body = ErrorResponse),
    ),
    tag = "task-webhooks"
)]
pub async fn update_webhook(
    State(state): State<AppState>,
    OrgAdmin(org): OrgAdmin,
    Path(webhook_id): Path<String>,
    Json(req): Json<UpdateTaskWebhookRequest>,
) -> Result<Json<TaskWebhookResponse>, (StatusCode, Json<ErrorResponse>)> {
    // THREAT[TM-API-020]: validate updated URL against private/internal ranges.
    if let Some(ref url) = req.url {
        validate_safe_url(url).map_err(|e| {
            ErrorResponse::new(format!("Invalid webhook URL: {e}"))
                .into_response(StatusCode::BAD_REQUEST)
        })?;
    }
    let input = UpdateOrgTaskWebhook {
        url: req.url,
        secret: req.secret,
        enabled: req.enabled,
    };
    let row = state
        .db
        .update_org_task_webhook(org.org_id, &webhook_id, input)
        .await
        .log_internal_error_json("update task webhook")?
        .ok_or_not_found_json("Webhook")?;
    Ok(Json(TaskWebhookResponse::from_row(row)))
}

/// Delete a task webhook.
#[utoipa::path(
    delete,
    path = "/v1/task-webhooks/{webhook_id}",
    params(("webhook_id" = String, Path, description = "Webhook public ID")),
    responses(
        (status = 204, description = "Webhook deleted"),
        (status = 401, description = "Unauthorized", body = ErrorResponse),
        (status = 403, description = "Forbidden — requires admin", body = ErrorResponse),
        (status = 404, description = "Webhook not found", body = ErrorResponse),
    ),
    tag = "task-webhooks"
)]
pub async fn delete_webhook(
    State(state): State<AppState>,
    OrgAdmin(org): OrgAdmin,
    Path(webhook_id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let deleted = state
        .db
        .delete_org_task_webhook(org.org_id, &webhook_id)
        .await
        .log_internal_error_json("delete task webhook")?;
    if deleted {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ErrorResponse::new("Webhook not found").into_response(StatusCode::NOT_FOUND))
    }
}

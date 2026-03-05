// App CRUD HTTP routes
// Routes use ResolvedOrg: org derived from auth context (API key or cookie)

use crate::auth::{AuthState, ResolvedOrg};
use crate::storage::StorageBackend;
use axum::extract::FromRef;
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
};
use everruns_core::typed_id::{AgentId, AppId, HarnessId};
use everruns_core::{App, AppStatus, ChannelType};

use super::common::{ApiOptionExt, ApiResultExt, ErrorResponse, ListResponse};
use serde::Deserialize;
use std::sync::Arc;
use utoipa::ToSchema;

use crate::services::AppService;

/// Request to create a new app
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateAppRequest {
    /// Display name of the app.
    #[schema(example = "Support Bot")]
    pub name: String,
    /// Description of what the app does.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "Customer support bot connected to Slack")]
    pub description: Option<String>,
    /// ID of the harness to use.
    #[schema(value_type = String, example = "harness_01933b5a00007000800000000000001")]
    pub harness_id: HarnessId,
    /// ID of the agent to use.
    #[schema(value_type = String, example = "agent_01933b5a00007000800000000000001")]
    pub agent_id: AgentId,
    /// Distribution channel type.
    pub channel_type: ChannelType,
    /// Channel-specific configuration.
    #[serde(default)]
    pub channel_config: Option<serde_json::Value>,
}

/// Request to update an app. Only provided fields will be updated.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpdateAppRequest {
    /// Display name of the app.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Description of what the app does.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// ID of the harness to use.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>)]
    pub harness_id: Option<HarnessId>,
    /// ID of the agent to use.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>)]
    pub agent_id: Option<AgentId>,
    /// Distribution channel type.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_type: Option<ChannelType>,
    /// Channel-specific configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_config: Option<serde_json::Value>,
    /// Lifecycle status (draft or archived). Use publish/unpublish endpoints for publishing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<AppStatus>,
}

/// App state for routes
#[derive(Clone)]
pub struct AppState {
    pub service: Arc<AppService>,
    pub auth: AuthState,
}

impl AppState {
    pub fn new(db: Arc<StorageBackend>, auth: AuthState) -> Self {
        Self {
            service: Arc::new(AppService::new(db)),
            auth,
        }
    }
}

impl FromRef<AppState> for AuthState {
    fn from_ref(input: &AppState) -> Self {
        input.auth.clone()
    }
}

/// Create app routes
pub fn routes(state: AppState) -> Router {
    Router::new()
        .route("/v1/apps", post(create_app).get(list_apps))
        .route(
            "/v1/apps/{app_id}",
            get(get_app).patch(update_app).delete(delete_app),
        )
        .route("/v1/apps/{app_id}/publish", post(publish_app))
        .route("/v1/apps/{app_id}/unpublish", post(unpublish_app))
        .with_state(state)
}

/// POST /v1/apps - Create a new app
#[utoipa::path(
    post,
    path = "/v1/apps",
    request_body = CreateAppRequest,
    responses(
        (status = 201, description = "App created successfully", body = App),
        (status = 400, description = "Invalid input", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "apps"
)]
pub async fn create_app(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Json(req): Json<CreateAppRequest>,
) -> Result<(StatusCode, Json<App>), (StatusCode, Json<ErrorResponse>)> {
    let app = state.service.create(org.org_id, req).await.map_err(|e| {
        let msg = e.to_string();
        if msg.contains("not found") {
            return ErrorResponse::new(&msg).into_response(StatusCode::BAD_REQUEST);
        }
        tracing::error!("Failed to create app: {}", msg);
        ErrorResponse::internal_error()
    })?;

    Ok((StatusCode::CREATED, Json(app)))
}

/// GET /v1/apps - List all non-archived apps
#[utoipa::path(
    get,
    path = "/v1/apps",
    responses(
        (status = 200, description = "List of apps", body = ListResponse<App>),
        (status = 500, description = "Internal server error")
    ),
    tag = "apps"
)]
pub async fn list_apps(
    org: ResolvedOrg,
    State(state): State<AppState>,
) -> Result<Json<ListResponse<App>>, StatusCode> {
    let apps = state
        .service
        .list(org.org_id)
        .await
        .log_internal_error("list apps")?;

    Ok(Json(ListResponse::new(apps)))
}

/// GET /v1/apps/{app_id} - Get app by ID
#[utoipa::path(
    get,
    path = "/v1/apps/{app_id}",
    params(("app_id" = String, Path, description = "App ID")),
    responses(
        (status = 200, description = "App found", body = App),
        (status = 400, description = "Invalid app ID"),
        (status = 404, description = "App not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "apps"
)]
pub async fn get_app(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(app_id): Path<String>,
) -> Result<Json<App>, (StatusCode, Json<ErrorResponse>)> {
    let app_id: AppId = app_id.parse().map_err(|e| {
        ErrorResponse::new(format!("Invalid app ID: {}", e)).into_response(StatusCode::BAD_REQUEST)
    })?;

    let app = state
        .service
        .get_by_public_id(org.org_id, &app_id.to_string())
        .await
        .log_internal_error_json("get app")?
        .ok_or_not_found_json("App")?;

    Ok(Json(app))
}

/// PATCH /v1/apps/{app_id} - Update app
#[utoipa::path(
    patch,
    path = "/v1/apps/{app_id}",
    params(("app_id" = String, Path, description = "App ID")),
    request_body = UpdateAppRequest,
    responses(
        (status = 200, description = "App updated successfully", body = App),
        (status = 400, description = "Invalid app ID or input", body = ErrorResponse),
        (status = 404, description = "App not found", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "apps"
)]
pub async fn update_app(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(app_id): Path<String>,
    Json(req): Json<UpdateAppRequest>,
) -> Result<Json<App>, (StatusCode, Json<ErrorResponse>)> {
    let app_id: AppId = app_id.parse().map_err(|e| {
        ErrorResponse::new(format!("Invalid app ID: {}", e)).into_response(StatusCode::BAD_REQUEST)
    })?;

    let app = state
        .service
        .update(org.org_id, &app_id.to_string(), req)
        .await
        .map_err(|e| {
            let msg = e.to_string();
            if msg.contains("not found") {
                return ErrorResponse::new(&msg).into_response(StatusCode::BAD_REQUEST);
            }
            tracing::error!("Failed to update app: {}", msg);
            ErrorResponse::internal_error()
        })?
        .ok_or_not_found_json("App")?;

    Ok(Json(app))
}

/// DELETE /v1/apps/{app_id} - Archive app
#[utoipa::path(
    delete,
    path = "/v1/apps/{app_id}",
    params(("app_id" = String, Path, description = "App ID")),
    responses(
        (status = 204, description = "App archived successfully"),
        (status = 400, description = "Invalid app ID"),
        (status = 404, description = "App not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "apps"
)]
pub async fn delete_app(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(app_id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let app_id: AppId = app_id.parse().map_err(|e| {
        ErrorResponse::new(format!("Invalid app ID: {}", e)).into_response(StatusCode::BAD_REQUEST)
    })?;

    let deleted = state
        .service
        .delete(org.org_id, &app_id.to_string())
        .await
        .log_internal_error_json("delete app")?;

    if deleted {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ErrorResponse::new("App not found").into_response(StatusCode::NOT_FOUND))
    }
}

/// POST /v1/apps/{app_id}/publish - Publish app (start accepting requests)
#[utoipa::path(
    post,
    path = "/v1/apps/{app_id}/publish",
    params(("app_id" = String, Path, description = "App ID")),
    responses(
        (status = 200, description = "App published", body = App),
        (status = 400, description = "Invalid app ID"),
        (status = 404, description = "App not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "apps"
)]
pub async fn publish_app(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(app_id): Path<String>,
) -> Result<Json<App>, (StatusCode, Json<ErrorResponse>)> {
    let app_id: AppId = app_id.parse().map_err(|e| {
        ErrorResponse::new(format!("Invalid app ID: {}", e)).into_response(StatusCode::BAD_REQUEST)
    })?;

    let app = state
        .service
        .publish(org.org_id, &app_id.to_string())
        .await
        .log_internal_error_json("publish app")?
        .ok_or_not_found_json("App")?;

    Ok(Json(app))
}

/// POST /v1/apps/{app_id}/unpublish - Unpublish app (stop accepting requests)
#[utoipa::path(
    post,
    path = "/v1/apps/{app_id}/unpublish",
    params(("app_id" = String, Path, description = "App ID")),
    responses(
        (status = 200, description = "App unpublished", body = App),
        (status = 400, description = "Invalid app ID"),
        (status = 404, description = "App not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "apps"
)]
pub async fn unpublish_app(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(app_id): Path<String>,
) -> Result<Json<App>, (StatusCode, Json<ErrorResponse>)> {
    let app_id: AppId = app_id.parse().map_err(|e| {
        ErrorResponse::new(format!("Invalid app ID: {}", e)).into_response(StatusCode::BAD_REQUEST)
    })?;

    let app = state
        .service
        .unpublish(org.org_id, &app_id.to_string())
        .await
        .log_internal_error_json("unpublish app")?
        .ok_or_not_found_json("App")?;

    Ok(Json(app))
}

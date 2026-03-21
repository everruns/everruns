// App CRUD HTTP routes
// Routes use ResolvedOrg: org derived from auth context (API key or cookie)
// Policy enforcement happens at the service layer via #[policy] macro.

use crate::auth::{AuthState, ResolvedOrg};
use crate::services::app::{APP_DANGEROUS, APP_MANAGE, APP_VIEW};
use crate::storage::StorageBackend;
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
};
use everruns_core::typed_id::{AgentId, AgentIdentityId, AppId, HarnessId};
use everruns_core::{
    App, AppStatus, Caller, ChannelType, ResourceConfigResponse, evaluate_policies_with,
};

use super::common::{
    ApiOptionExt, ApiPolicyResultExt, ApiResult, ErrorResponse, ListResponse,
    deserialize_nullable_update_field, impl_auth_state,
};
use everruns_durable::UpdateField;
use serde::Deserialize;
use std::sync::Arc;
use utoipa::{IntoParams, ToSchema};

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
    /// Optional resident agent identity for unattended/channel execution.
    #[serde(default)]
    #[schema(value_type = Option<String>, example = "identity_01933b5a00007000800000000000001")]
    pub agent_identity_id: Option<AgentIdentityId>,
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
    /// Optional resident agent identity for unattended/channel execution.
    #[serde(default, deserialize_with = "deserialize_nullable_update_field")]
    #[schema(value_type = Option<String>, nullable = true)]
    pub agent_identity_id: UpdateField<AgentIdentityId>,
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

/// Query parameters for listing apps.
#[derive(Debug, Clone, Deserialize, IntoParams)]
pub struct ListAppsQuery {
    /// Search by name or description (case-insensitive substring match).
    pub search: Option<String>,
    /// Include archived apps. Deleted apps never appear in lists.
    pub include_archived: Option<bool>,
}

/// App state for routes
#[derive(Clone)]
pub struct AppState {
    pub service: Arc<AppService>,
    pub auth: AuthState,
}

impl AppState {
    pub fn new(
        db: Arc<StorageBackend>,
        encryption: Option<Arc<crate::storage::EncryptionService>>,
        auth: AuthState,
    ) -> Self {
        Self {
            service: Arc::new(AppService::new(db, encryption)),
            auth,
        }
    }
}

impl_auth_state!(AppState);

/// Create app routes
pub fn routes(state: AppState) -> Router {
    Router::new()
        .route("/v1/apps/config", get(app_config))
        .route("/v1/apps", post(create_app).get(list_apps))
        .route(
            "/v1/apps/{app_id}",
            get(get_app).patch(update_app).delete(delete_app),
        )
        .route("/v1/apps/{app_id}/delete", post(destroy_app))
        .route("/v1/apps/{app_id}/publish", post(publish_app))
        .route("/v1/apps/{app_id}/unpublish", post(unpublish_app))
        .with_state(state)
}

/// GET /v1/apps/config
#[utoipa::path(
    get,
    path = "/v1/apps/config",
    responses(
        (status = 200, description = "Resource config for apps", body = ResourceConfigResponse),
    ),
    tag = "apps"
)]
pub async fn app_config(
    State(auth): State<AuthState>,
    org: ResolvedOrg,
) -> Json<ResourceConfigResponse> {
    let caller = Caller::from(&org);
    let policies = evaluate_policies_with(
        auth.permission_resolver.as_ref(),
        &caller,
        &[&APP_VIEW, &APP_MANAGE, &APP_DANGEROUS],
    );
    Json(ResourceConfigResponse { policies })
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
    let caller = Caller::from(&org);
    let app = state
        .service
        .create(&caller, req)
        .await
        .map_policy_or_internal("create app")?;

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
    params(ListAppsQuery),
    tag = "apps"
)]
pub async fn list_apps(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Query(query): Query<ListAppsQuery>,
) -> ApiResult<ListResponse<App>> {
    let caller = Caller::from(&org);
    let apps = state
        .service
        .list(
            &caller,
            query.search.as_deref(),
            query.include_archived.unwrap_or(false),
        )
        .await
        .map_policy_or_internal("list apps")?;

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
) -> ApiResult<App> {
    let app_id: AppId = app_id.parse().map_err(|e| {
        ErrorResponse::new(format!("Invalid app ID: {}", e)).into_response(StatusCode::BAD_REQUEST)
    })?;

    let caller = Caller::from(&org);
    let app = state
        .service
        .get_by_public_id(&caller, &app_id.to_string())
        .await
        .map_policy_or_internal("get app")?
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
) -> ApiResult<App> {
    let app_id: AppId = app_id.parse().map_err(|e| {
        ErrorResponse::new(format!("Invalid app ID: {}", e)).into_response(StatusCode::BAD_REQUEST)
    })?;

    let caller = Caller::from(&org);
    let app = state
        .service
        .update(&caller, &app_id.to_string(), req)
        .await
        .map_policy_or_internal("update app")?
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

    let caller = Caller::from(&org);
    let deleted = state
        .service
        .delete(&caller, &app_id.to_string())
        .await
        .map_policy_or_internal("delete app")?;

    if deleted {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ErrorResponse::new("App not found").into_response(StatusCode::NOT_FOUND))
    }
}

pub async fn destroy_app(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(app_id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let app_id: AppId = app_id.parse().map_err(|e| {
        ErrorResponse::new(format!("Invalid app ID: {}", e)).into_response(StatusCode::BAD_REQUEST)
    })?;

    let caller = Caller::from(&org);
    let deleted = state
        .service
        .destroy(&caller, &app_id.to_string())
        .await
        .map_policy_or_internal("destroy app")?;

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
) -> ApiResult<App> {
    let app_id: AppId = app_id.parse().map_err(|e| {
        ErrorResponse::new(format!("Invalid app ID: {}", e)).into_response(StatusCode::BAD_REQUEST)
    })?;

    let caller = Caller::from(&org);
    let app = state
        .service
        .publish(&caller, &app_id.to_string())
        .await
        .map_policy_or_internal("publish app")?
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
) -> ApiResult<App> {
    let app_id: AppId = app_id.parse().map_err(|e| {
        ErrorResponse::new(format!("Invalid app ID: {}", e)).into_response(StatusCode::BAD_REQUEST)
    })?;

    let caller = Caller::from(&org);
    let app = state
        .service
        .unpublish(&caller, &app_id.to_string())
        .await
        .map_policy_or_internal("unpublish app")?
        .ok_or_not_found_json("App")?;

    Ok(Json(app))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_AGENT_IDENTITY_ID: &str = "identity_550e8400e29b41d4a716446655440000";

    #[test]
    fn update_app_request_leaves_agent_identity_unchanged_when_omitted() {
        let req: UpdateAppRequest = serde_json::from_str("{}").unwrap();
        assert_eq!(req.agent_identity_id, UpdateField::Unchanged);
    }

    #[test]
    fn update_app_request_clears_agent_identity_when_null() {
        let req: UpdateAppRequest = serde_json::from_str(r#"{"agent_identity_id":null}"#).unwrap();
        assert_eq!(req.agent_identity_id, UpdateField::Clear);
    }

    #[test]
    fn update_app_request_sets_agent_identity_when_present() {
        let req: UpdateAppRequest = serde_json::from_str(&format!(
            r#"{{"agent_identity_id":"{}"}}"#,
            TEST_AGENT_IDENTITY_ID
        ))
        .unwrap();
        let expected: AgentIdentityId = TEST_AGENT_IDENTITY_ID.parse().unwrap();
        assert_eq!(req.agent_identity_id, UpdateField::Set(expected));
    }
}

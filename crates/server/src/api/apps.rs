// App CRUD HTTP routes
// Routes use ResolvedOrg: org derived from auth context (API key or cookie)
// Handlers call domain commands; policy enforcement happens inside commands.

use crate::auth::{AuthState, ResolvedOrg};
use crate::domains::apps::{APP_DANGEROUS, APP_MANAGE, APP_VIEW};
use crate::services::CapabilityService;
use crate::storage::{EncryptionService, StorageBackend};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
};
use everruns_core::typed_id::{AgentId, AgentIdentityId, HarnessId};
use everruns_core::{
    App, AppChannel, AppStatus, Caller, ChannelType, ResourceConfigResponse, evaluate_policies_with,
};

use super::common::{
    ApiResult, ErrorResponse, ListResponse, UrlBuilder, WithUrls,
    deserialize_nullable_update_field, impl_auth_state,
};
use crate::domains::common::Command;
use everruns_durable::UpdateField;
use serde::Deserialize;
use std::sync::Arc;
use utoipa::{IntoParams, ToSchema};

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
    /// Initial channel type (creates the first channel on the app).
    pub channel_type: ChannelType,
    /// Initial channel configuration.
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
    /// Lifecycle status (draft or archived). Use publish/unpublish endpoints for publishing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<AppStatus>,
}

/// Request to add a channel to an app.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct AddChannelRequest {
    /// Channel type.
    pub channel_type: ChannelType,
    /// Channel-specific configuration.
    #[serde(default)]
    pub channel_config: Option<serde_json::Value>,
    /// Whether the channel is enabled (default: true).
    #[serde(default)]
    pub enabled: Option<bool>,
}

/// Request to update a channel.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpdateChannelRequest {
    /// Channel type.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_type: Option<ChannelType>,
    /// Channel-specific configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_config: Option<serde_json::Value>,
    /// Whether the channel is enabled.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
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
    pub db: Arc<StorageBackend>,
    pub encryption: Option<Arc<EncryptionService>>,
    pub capability_service: Arc<CapabilityService>,
    pub auth: AuthState,
}

impl AppState {
    pub fn new(
        db: Arc<StorageBackend>,
        encryption: Option<Arc<EncryptionService>>,
        capability_service: Arc<CapabilityService>,
        auth: AuthState,
    ) -> Self {
        Self {
            db,
            encryption,
            capability_service,
            auth,
        }
    }

    /// Build a domain Ctx from this AppState for the given org.
    pub fn ctx(&self, org: &ResolvedOrg) -> crate::domains::common::Ctx {
        crate::domains::common::Ctx::new(
            Caller::from(org),
            self.db.clone(),
            self.capability_service.clone(),
            self.encryption.clone(),
        )
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
        // Channel sub-routes
        .route("/v1/apps/{app_id}/channels", post(add_channel))
        .route(
            "/v1/apps/{app_id}/channels/{channel_id}",
            axum::routing::patch(update_channel).delete(delete_channel),
        )
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
        (status = 201, description = "App created successfully", body = WithUrls<App>),
        (status = 400, description = "Invalid input", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "apps"
)]
pub async fn create_app(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Json(req): Json<CreateAppRequest>,
) -> Result<(StatusCode, Json<WithUrls<App>>), (StatusCode, Json<ErrorResponse>)> {
    let app = crate::domains::apps::CreateApp(req)
        .execute(&state.ctx(&org))
        .await?;

    let builder = UrlBuilder::from_auth_config(&state.auth.config);
    Ok((StatusCode::CREATED, Json(builder.wrap(app))))
}

/// GET /v1/apps - List all non-archived apps
#[utoipa::path(
    get,
    path = "/v1/apps",
    responses(
        (status = 200, description = "List of apps", body = ListResponse<WithUrls<App>>),
        (status = 500, description = "Internal server error")
    ),
    params(ListAppsQuery),
    tag = "apps"
)]
pub async fn list_apps(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Query(query): Query<ListAppsQuery>,
) -> ApiResult<ListResponse<WithUrls<App>>> {
    let apps = crate::domains::apps::ListApps {
        search: query.search,
        include_archived: query.include_archived.unwrap_or(false),
    }
    .execute(&state.ctx(&org))
    .await?;

    let builder = UrlBuilder::from_auth_config(&state.auth.config);
    Ok(Json(ListResponse::new(apps).with_urls(&builder)))
}

/// GET /v1/apps/{app_id} - Get app by ID
#[utoipa::path(
    get,
    path = "/v1/apps/{app_id}",
    params(("app_id" = String, Path, description = "App ID")),
    responses(
        (status = 200, description = "App found", body = WithUrls<App>),
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
) -> ApiResult<WithUrls<App>> {
    let app = crate::domains::apps::GetApp { id: app_id }
        .execute(&state.ctx(&org))
        .await?;

    let builder = UrlBuilder::from_auth_config(&state.auth.config);
    Ok(Json(builder.wrap(app)))
}

/// PATCH /v1/apps/{app_id} - Update app
#[utoipa::path(
    patch,
    path = "/v1/apps/{app_id}",
    params(("app_id" = String, Path, description = "App ID")),
    request_body = UpdateAppRequest,
    responses(
        (status = 200, description = "App updated successfully", body = WithUrls<App>),
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
) -> ApiResult<WithUrls<App>> {
    let app = crate::domains::apps::UpdateAppCmd { id: app_id, req }
        .execute(&state.ctx(&org))
        .await?;

    let builder = UrlBuilder::from_auth_config(&state.auth.config);
    Ok(Json(builder.wrap(app)))
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
    crate::domains::apps::DeleteApp { id: app_id }
        .execute(&state.ctx(&org))
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn destroy_app(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(app_id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    crate::domains::apps::DestroyApp { id: app_id }
        .execute(&state.ctx(&org))
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// POST /v1/apps/{app_id}/publish - Publish app (start accepting requests)
#[utoipa::path(
    post,
    path = "/v1/apps/{app_id}/publish",
    params(("app_id" = String, Path, description = "App ID")),
    responses(
        (status = 200, description = "App published", body = WithUrls<App>),
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
) -> ApiResult<WithUrls<App>> {
    let app = crate::domains::apps::PublishApp { id: app_id }
        .execute(&state.ctx(&org))
        .await?;

    let builder = UrlBuilder::from_auth_config(&state.auth.config);
    Ok(Json(builder.wrap(app)))
}

/// POST /v1/apps/{app_id}/unpublish - Unpublish app (stop accepting requests)
#[utoipa::path(
    post,
    path = "/v1/apps/{app_id}/unpublish",
    params(("app_id" = String, Path, description = "App ID")),
    responses(
        (status = 200, description = "App unpublished", body = WithUrls<App>),
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
) -> ApiResult<WithUrls<App>> {
    let app = crate::domains::apps::UnpublishApp { id: app_id }
        .execute(&state.ctx(&org))
        .await?;

    let builder = UrlBuilder::from_auth_config(&state.auth.config);
    Ok(Json(builder.wrap(app)))
}

// ============================================
// Channel sub-routes
// ============================================

/// POST /v1/apps/{app_id}/channels - Add a channel to an app
pub async fn add_channel(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(app_id): Path<String>,
    Json(req): Json<AddChannelRequest>,
) -> Result<(StatusCode, Json<AppChannel>), (StatusCode, Json<ErrorResponse>)> {
    let channel = crate::domains::apps::AddChannel { app_id, req }
        .execute(&state.ctx(&org))
        .await?;
    Ok((StatusCode::CREATED, Json(channel)))
}

/// PATCH /v1/apps/{app_id}/channels/{channel_id} - Update a channel
pub async fn update_channel(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((app_id, channel_id)): Path<(String, String)>,
    Json(req): Json<UpdateChannelRequest>,
) -> ApiResult<AppChannel> {
    let channel = crate::domains::apps::UpdateChannelCmd {
        app_id,
        channel_id,
        req,
    }
    .execute(&state.ctx(&org))
    .await?;
    Ok(Json(channel))
}

/// DELETE /v1/apps/{app_id}/channels/{channel_id} - Remove a channel
pub async fn delete_channel(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((app_id, channel_id)): Path<(String, String)>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    crate::domains::apps::DeleteChannel { app_id, channel_id }
        .execute(&state.ctx(&org))
        .await?;
    Ok(StatusCode::NO_CONTENT)
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

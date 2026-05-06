// App CRUD HTTP routes
// Routes use ResolvedOrg: org derived from auth context (API key or cookie)
// Handlers call domain commands; policy enforcement happens inside commands.

use crate::auth::{AuthState, ResolvedOrg};
use crate::domains::apps::types::{
    AddChannelRequest, CreateAppRequest, ListAppsQuery, UpdateAppRequest, UpdateChannelRequest,
};
use crate::domains::apps::{
    APP_DANGEROUS, APP_MANAGE, APP_VIEW, AddA2aChannelCmd, AddA2aChannelOutput,
    RegenerateA2aApiKeyCmd, RegenerateA2aApiKeyOutput,
};
use crate::services::CapabilityService;
use crate::storage::{EncryptionService, StorageBackend};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
};
use everruns_core::{App, AppChannel, Caller, ResourceConfigResponse, evaluate_policies_with};
use everruns_durable::WorkflowEventStore;

use super::common::{
    ApiResult, ErrorResponse, ListResponse, UrlBuilder, WithUrls, impl_auth_state,
};
use super::dispatch::{Dispatchable, impl_dispatchable};
use crate::domains::common::Command;
use std::sync::Arc;

/// App state for routes
#[derive(Clone)]
pub struct AppState {
    pub db: Arc<StorageBackend>,
    pub encryption: Option<Arc<EncryptionService>>,
    pub workflow_store: Option<Arc<dyn WorkflowEventStore + Send + Sync>>,
    pub capability_service: Arc<CapabilityService>,
    pub auth: AuthState,
}

impl AppState {
    pub fn new(
        db: Arc<StorageBackend>,
        encryption: Option<Arc<EncryptionService>>,
        workflow_store: Option<Arc<dyn WorkflowEventStore + Send + Sync>>,
        capability_service: Arc<CapabilityService>,
        auth: AuthState,
    ) -> Self {
        Self {
            db,
            encryption,
            workflow_store,
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
            self.auth.permission_resolver.clone(),
        )
        .with_workflow_store(self.workflow_store.clone())
    }
}

impl_auth_state!(AppState);
impl_dispatchable!(AppState);

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
        // A2A channel sub-routes (channel-specific because the API key is
        // generated server-side and returned exactly once). Uses a dedicated
        // path prefix to avoid colliding with `/channels/{channel_id}`.
        .route("/v1/apps/{app_id}/a2a-channels", post(add_a2a_channel))
        .route(
            "/v1/apps/{app_id}/a2a-channels/{channel_id}/regenerate-key",
            post(regenerate_a2a_key),
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
    state
        .dispatcher(&org)
        .run_created_with_urls(crate::domains::apps::CreateApp(req))
        .await
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
    .run(&state.ctx(&org))
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
    state
        .dispatcher(&org)
        .run_with_urls(crate::domains::apps::GetApp { id: app_id })
        .await
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
    state
        .dispatcher(&org)
        .run_with_urls(crate::domains::apps::UpdateAppCmd { id: app_id, req })
        .await
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
    state
        .dispatcher(&org)
        .run_no_content(crate::domains::apps::DeleteApp { id: app_id })
        .await
}

pub async fn destroy_app(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(app_id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    state
        .dispatcher(&org)
        .run_no_content(crate::domains::apps::DestroyApp { id: app_id })
        .await
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
    state
        .dispatcher(&org)
        .run_with_urls(crate::domains::apps::PublishApp { id: app_id })
        .await
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
    state
        .dispatcher(&org)
        .run_with_urls(crate::domains::apps::UnpublishApp { id: app_id })
        .await
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
        .run(&state.ctx(&org))
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
    .run(&state.ctx(&org))
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
        .run(&state.ctx(&org))
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct AddA2aChannelHttpRequest {
    #[serde(default)]
    pub session_mode: everruns_core::app::InvocationSessionMode,
    pub message: String,
    pub agent_card_name: Option<String>,
    pub agent_card_description: Option<String>,
    pub enabled: Option<bool>,
}

/// POST /v1/apps/{app_id}/channels/a2a - Add an A2A channel (returns plaintext key once).
pub async fn add_a2a_channel(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(app_id): Path<String>,
    Json(req): Json<AddA2aChannelHttpRequest>,
) -> Result<(StatusCode, Json<AddA2aChannelOutput>), (StatusCode, Json<ErrorResponse>)> {
    let output = AddA2aChannelCmd {
        app_id,
        session_mode: req.session_mode,
        message: req.message,
        agent_card_name: req.agent_card_name,
        agent_card_description: req.agent_card_description,
        enabled: req.enabled,
    }
    .run(&state.ctx(&org))
    .await?;
    Ok((StatusCode::CREATED, Json(output)))
}

/// POST /v1/apps/{app_id}/channels/{channel_id}/regenerate-key
/// Regenerate an A2A channel API key. Returns the new plaintext key once.
pub async fn regenerate_a2a_key(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((app_id, channel_id)): Path<(String, String)>,
) -> Result<Json<RegenerateA2aApiKeyOutput>, (StatusCode, Json<ErrorResponse>)> {
    let output = RegenerateA2aApiKeyCmd { app_id, channel_id }
        .run(&state.ctx(&org))
        .await?;
    Ok(Json(output))
}

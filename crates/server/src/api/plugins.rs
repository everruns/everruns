// Plugin marketplace and install HTTP routes.
//
// Routes:
//   GET/POST   /v1/plugin_marketplaces
//   GET/PATCH/DELETE /v1/plugin_marketplaces/{id}
//   POST       /v1/plugin_marketplaces/{id}/sync
//   GET        /v1/plugin_marketplaces/{id}/plugins
//   GET/POST   /v1/plugins
//   GET/PATCH/DELETE /v1/plugins/{id}
//   POST       /v1/plugins/{id}/update
//
// Spec: knowledge/integrations/plugins.md

use crate::auth::{AuthState, ResolvedOrg};
use crate::domains::plugins::types::{
    CreatePluginMarketplaceRequest, InstallPluginRequest, UpdateInstalledPluginRequest,
    UpdatePluginMarketplaceRequest,
};
use crate::services::CapabilityService;
use crate::storage::StorageBackend;
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
};
use everruns_core::{Caller, EgressService};
use everruns_host::DirectEgressService;
use serde::Deserialize;
use std::sync::Arc;
use utoipa::IntoParams;

use super::common::impl_auth_state;
use super::common::{ApiResult, ErrorResponse, ListResponse};
use super::dispatch::{Dispatchable, impl_dispatchable};

/// Query parameters for listing plugin marketplaces.
#[derive(Debug, Clone, Deserialize, IntoParams)]
pub struct ListPluginMarketplacesQuery {
    /// Search by name (case-insensitive substring match).
    pub search: Option<String>,
}

/// Query parameters for listing installed plugins.
#[derive(Debug, Clone, Deserialize, IntoParams)]
pub struct ListPluginsQuery {
    /// Search by name (case-insensitive substring match).
    pub search: Option<String>,
}

/// App state for plugin routes.
#[derive(Clone)]
pub struct AppState {
    pub db: Arc<StorageBackend>,
    pub capability_service: Arc<CapabilityService>,
    pub auth: AuthState,
    pub egress_service: Arc<dyn EgressService>,
}

impl AppState {
    pub fn new(
        db: Arc<StorageBackend>,
        capability_service: Arc<CapabilityService>,
        auth: AuthState,
    ) -> Self {
        Self {
            db,
            capability_service,
            auth,
            egress_service: Arc::new(DirectEgressService::for_runtime_traffic_from_env()),
        }
    }

    /// Override the egress service (e.g., in tests with a wiremock-backed service).
    pub fn with_egress_service(mut self, service: Arc<dyn EgressService>) -> Self {
        self.egress_service = service;
        self
    }

    pub fn ctx(&self, org: &ResolvedOrg) -> crate::domains::common::Ctx {
        crate::domains::common::Ctx::new(
            Caller::from(org),
            self.db.clone(),
            self.capability_service.clone(),
            None,
            self.auth.permission_resolver.clone(),
        )
        .with_feature_flags(org.feature_flags.clone())
        .with_egress_service(self.egress_service.clone())
    }
}

impl_auth_state!(AppState);
impl_dispatchable!(AppState);

/// Build plugin routes.
pub fn routes(state: AppState) -> Router {
    Router::new()
        .route(
            "/v1/plugin_marketplaces",
            get(list_plugin_marketplaces).post(create_plugin_marketplace),
        )
        .route(
            "/v1/plugin_marketplaces/{id}",
            get(get_plugin_marketplace)
                .patch(update_plugin_marketplace)
                .delete(delete_plugin_marketplace),
        )
        .route(
            "/v1/plugin_marketplaces/{id}/sync",
            post(sync_plugin_marketplace),
        )
        .route(
            "/v1/plugin_marketplaces/{id}/plugins",
            get(get_marketplace_catalog),
        )
        .route("/v1/plugins", get(list_plugins).post(install_plugin))
        .route(
            "/v1/plugins/{id}",
            get(get_plugin)
                .patch(patch_installed_plugin)
                .delete(uninstall_plugin),
        )
        .route("/v1/plugins/{id}/update", post(update_plugin))
        .with_state(state)
}

// ============================================================================
// Marketplace handlers
// ============================================================================

/// GET /v1/plugin_marketplaces
#[utoipa::path(
    get,
    path = "/v1/plugin_marketplaces",
    params(ListPluginMarketplacesQuery),
    responses(
        (status = 200, description = "List of plugin marketplaces"),
        (status = 500, description = "Internal server error", body = ErrorResponse),
    ),
    tag = "plugins"
)]
pub async fn list_plugin_marketplaces(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Query(query): Query<ListPluginMarketplacesQuery>,
) -> ApiResult<ListResponse<crate::domains::plugins::types::PluginMarketplace>> {
    state
        .dispatcher(&org)
        .run_list(crate::domains::plugins::ListPluginMarketplaces {
            search: query.search,
        })
        .await
}

/// POST /v1/plugin_marketplaces
#[utoipa::path(
    post,
    path = "/v1/plugin_marketplaces",
    request_body = CreatePluginMarketplaceRequest,
    responses(
        (status = 201, description = "Marketplace created"),
        (status = 400, description = "Invalid input", body = ErrorResponse),
        (status = 409, description = "Name already exists", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse),
    ),
    tag = "plugins"
)]
pub async fn create_plugin_marketplace(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Json(req): Json<CreatePluginMarketplaceRequest>,
) -> Result<
    (
        StatusCode,
        Json<crate::domains::plugins::types::PluginMarketplace>,
    ),
    (StatusCode, Json<ErrorResponse>),
> {
    state
        .dispatcher(&org)
        .run_created(crate::domains::plugins::CreatePluginMarketplaceCmd(req))
        .await
}

/// GET /v1/plugin_marketplaces/{id}
#[utoipa::path(
    get,
    path = "/v1/plugin_marketplaces/{id}",
    params(("id" = String, Path, description = "Marketplace ID (plgmkt_...)")),
    responses(
        (status = 200, description = "Marketplace found"),
        (status = 404, description = "Not found", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse),
    ),
    tag = "plugins"
)]
pub async fn get_plugin_marketplace(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<crate::domains::plugins::types::PluginMarketplace> {
    state
        .dispatcher(&org)
        .run(crate::domains::plugins::GetPluginMarketplace { id })
        .await
}

/// PATCH /v1/plugin_marketplaces/{id}
#[utoipa::path(
    patch,
    path = "/v1/plugin_marketplaces/{id}",
    params(("id" = String, Path, description = "Marketplace ID (plgmkt_...)")),
    request_body = UpdatePluginMarketplaceRequest,
    responses(
        (status = 200, description = "Marketplace updated"),
        (status = 400, description = "Invalid input", body = ErrorResponse),
        (status = 404, description = "Not found", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse),
    ),
    tag = "plugins"
)]
pub async fn update_plugin_marketplace(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdatePluginMarketplaceRequest>,
) -> ApiResult<crate::domains::plugins::types::PluginMarketplace> {
    state
        .dispatcher(&org)
        .run(crate::domains::plugins::UpdatePluginMarketplaceCmd { id, req })
        .await
}

/// DELETE /v1/plugin_marketplaces/{id}
#[utoipa::path(
    delete,
    path = "/v1/plugin_marketplaces/{id}",
    params(("id" = String, Path, description = "Marketplace ID (plgmkt_...)")),
    responses(
        (status = 204, description = "Marketplace deleted"),
        (status = 404, description = "Not found", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse),
    ),
    tag = "plugins"
)]
pub async fn delete_plugin_marketplace(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    state
        .dispatcher(&org)
        .run_no_content(crate::domains::plugins::DeletePluginMarketplace { id })
        .await
}

/// POST /v1/plugin_marketplaces/{id}/sync
#[utoipa::path(
    post,
    path = "/v1/plugin_marketplaces/{id}/sync",
    params(("id" = String, Path, description = "Marketplace ID (plgmkt_...)")),
    responses(
        (status = 200, description = "Marketplace synced"),
        (status = 400, description = "Sync not supported or source error", body = ErrorResponse),
        (status = 404, description = "Not found", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse),
    ),
    tag = "plugins"
)]
pub async fn sync_plugin_marketplace(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<crate::domains::plugins::types::PluginMarketplace> {
    state
        .dispatcher(&org)
        .run(crate::domains::plugins::SyncPluginMarketplace { id })
        .await
}

/// GET /v1/plugin_marketplaces/{id}/plugins
#[utoipa::path(
    get,
    path = "/v1/plugin_marketplaces/{id}/plugins",
    params(("id" = String, Path, description = "Marketplace ID (plgmkt_...)")),
    responses(
        (status = 200, description = "Catalog entries"),
        (status = 400, description = "Marketplace not yet synced", body = ErrorResponse),
        (status = 404, description = "Not found", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse),
    ),
    tag = "plugins"
)]
pub async fn get_marketplace_catalog(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<ListResponse<crate::domains::plugins::types::MarketplaceCatalogEntry>> {
    state
        .dispatcher(&org)
        .run_list(crate::domains::plugins::GetMarketplaceCatalog { id })
        .await
}

// ============================================================================
// Installed plugin handlers
// ============================================================================

/// GET /v1/plugins
#[utoipa::path(
    get,
    path = "/v1/plugins",
    params(ListPluginsQuery),
    responses(
        (status = 200, description = "List of installed plugins"),
        (status = 500, description = "Internal server error", body = ErrorResponse),
    ),
    tag = "plugins"
)]
pub async fn list_plugins(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Query(query): Query<ListPluginsQuery>,
) -> ApiResult<ListResponse<crate::domains::plugins::types::InstalledPlugin>> {
    state
        .dispatcher(&org)
        .run_list(crate::domains::plugins::ListPlugins {
            search: query.search,
        })
        .await
}

/// POST /v1/plugins
#[utoipa::path(
    post,
    path = "/v1/plugins",
    request_body = InstallPluginRequest,
    responses(
        (status = 201, description = "Plugin installed"),
        (status = 400, description = "Compilation or source error", body = ErrorResponse),
        (status = 409, description = "Plugin already installed", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse),
    ),
    tag = "plugins"
)]
pub async fn install_plugin(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Json(req): Json<InstallPluginRequest>,
) -> Result<
    (
        StatusCode,
        Json<crate::domains::plugins::types::InstalledPlugin>,
    ),
    (StatusCode, Json<ErrorResponse>),
> {
    state
        .dispatcher(&org)
        .run_created(crate::domains::plugins::InstallPluginCmd(req))
        .await
}

/// GET /v1/plugins/{id}
#[utoipa::path(
    get,
    path = "/v1/plugins/{id}",
    params(("id" = String, Path, description = "Plugin install ID (plugin_...)")),
    responses(
        (status = 200, description = "Installed plugin"),
        (status = 404, description = "Not found", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse),
    ),
    tag = "plugins"
)]
pub async fn get_plugin(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<crate::domains::plugins::types::InstalledPlugin> {
    state
        .dispatcher(&org)
        .run(crate::domains::plugins::GetPlugin { id })
        .await
}

/// PATCH /v1/plugins/{id}
#[utoipa::path(
    patch,
    path = "/v1/plugins/{id}",
    params(("id" = String, Path, description = "Plugin install ID (plugin_...)")),
    request_body = UpdateInstalledPluginRequest,
    responses(
        (status = 200, description = "Plugin updated"),
        (status = 400, description = "Invalid input", body = ErrorResponse),
        (status = 404, description = "Not found", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse),
    ),
    tag = "plugins"
)]
pub async fn patch_installed_plugin(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateInstalledPluginRequest>,
) -> ApiResult<crate::domains::plugins::types::InstalledPlugin> {
    state
        .dispatcher(&org)
        .run(crate::domains::plugins::PatchInstalledPlugin { id, req })
        .await
}

/// DELETE /v1/plugins/{id}
#[utoipa::path(
    delete,
    path = "/v1/plugins/{id}",
    params(("id" = String, Path, description = "Plugin install ID (plugin_...)")),
    responses(
        (status = 204, description = "Plugin uninstalled"),
        (status = 404, description = "Not found", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse),
    ),
    tag = "plugins"
)]
pub async fn uninstall_plugin(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    state
        .dispatcher(&org)
        .run_no_content(crate::domains::plugins::UninstallPlugin { id })
        .await
}

/// POST /v1/plugins/{id}/update
#[utoipa::path(
    post,
    path = "/v1/plugins/{id}/update",
    params(("id" = String, Path, description = "Plugin install ID (plugin_...)")),
    responses(
        (status = 200, description = "Plugin updated"),
        (status = 400, description = "No marketplace or source error", body = ErrorResponse),
        (status = 404, description = "Not found", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse),
    ),
    tag = "plugins"
)]
pub async fn update_plugin(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<crate::domains::plugins::types::InstalledPlugin> {
    state
        .dispatcher(&org)
        .run(crate::domains::plugins::UpdatePlugin { id })
        .await
}

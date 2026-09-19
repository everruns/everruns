// MCP Server CRUD HTTP routes
// Routes: /v1/mcp-servers/...
//
// Spec: knowledge/integrations/mcp.md (umbrella), knowledge/integrations/mcp-servers.md (API endpoints)

use crate::auth::{AuthState, ResolvedOrg};
use crate::domains::mcp_servers::types::{CreateMcpServerRequest, UpdateMcpServerRequest};
use crate::domains::mcp_servers::{MCP_SERVER_DANGEROUS, MCP_SERVER_MANAGE, MCP_SERVER_VIEW};
use crate::services::CapabilityService;
use crate::storage::{EncryptionService, StorageBackend};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
};
use everruns_core::{Caller, McpServer, ResourceConfigResponse, evaluate_policies_with};
use everruns_provider::typed_id::McpServerId;

use super::common::{
    ApiResult, ErrorResponse, ListResponse, UrlBuilder, WithUrls, impl_auth_state,
};
use super::dispatch::{Dispatchable, impl_dispatchable};
use super::pagination::bounded_page_limit;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::{IntoParams, ToSchema};

const USAGE_AGENT_NAME_LIMIT: i64 = 25;
const CATALOG_DEFAULT_LIMIT: u32 = 50;
const CATALOG_MAX_LIMIT: u32 = 100;

/// Query parameters for listing MCP servers.
#[derive(Debug, Clone, Deserialize, IntoParams)]
pub struct ListMcpServersQuery {
    /// Search by name or description (case-insensitive substring match).
    pub search: Option<String>,
    /// Include archived MCP servers. Deleted MCP servers never appear in lists.
    pub include_archived: Option<bool>,
}
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct McpServerCatalogResponse {
    pub data: Vec<McpServerCatalogEntry>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, IntoParams)]
pub struct ListMcpServerCatalogQuery {
    /// Continue after this MCP server ID.
    pub cursor: Option<String>,
    /// Page size (default: 50, max: 100).
    pub limit: Option<u32>,
}
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct McpServerCatalogEntry {
    #[serde(flatten)]
    pub server: WithUrls<McpServer>,
    pub used_by_agents: i64,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct McpServerUsageResponse {
    pub agent_names: Vec<String>,
    pub total_count: i64,
    pub truncated: bool,
}

/// Response for a simple MCP server config (matches Claude Desktop format)
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct McpServerConfigResponse {
    /// The URL of the MCP server
    pub url: String,
    /// The transport type ("http")
    #[serde(rename = "type")]
    pub transport_type: String,
}

/// App state for MCP servers routes
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
            self.auth.permission_resolver.clone(),
        )
    }
}

impl_auth_state!(AppState);
impl_dispatchable!(AppState);

/// Create MCP server routes
pub fn routes(state: AppState) -> Router {
    Router::new()
        .route(
            "/v1/mcp-servers",
            post(create_mcp_server).get(list_mcp_servers),
        )
        .route("/v1/mcp-servers/config", get(mcp_server_config))
        .route("/v1/mcp-servers/catalog", get(list_mcp_server_catalog))
        .route(
            "/v1/mcp-servers/{server_id}/usage",
            get(get_mcp_server_usage),
        )
        .route(
            "/v1/mcp-servers/{server_id}",
            get(get_mcp_server)
                .patch(update_mcp_server)
                .delete(delete_mcp_server),
        )
        .route(
            "/v1/mcp-servers/{server_id}/delete",
            post(destroy_mcp_server),
        )
        .with_state(state)
}

/// GET /v1/mcp-servers/config
///
/// Returns which MCP server policies the caller satisfies.
#[utoipa::path(
    get,
    path = "/v1/mcp-servers/config",
    responses(
        (status = 200, description = "Resource config for MCP servers", body = ResourceConfigResponse),
    ),
    tag = "mcp-servers"
)]
pub async fn mcp_server_config(
    State(auth): State<AuthState>,
    org: ResolvedOrg,
) -> Json<ResourceConfigResponse> {
    let caller = Caller::from(&org);
    let policies = evaluate_policies_with(
        auth.permission_resolver.as_ref(),
        &caller,
        &[&MCP_SERVER_VIEW, &MCP_SERVER_MANAGE, &MCP_SERVER_DANGEROUS],
    );
    Json(ResourceConfigResponse { policies })
}

/// POST /v1/mcp-servers - Create a new MCP server
#[utoipa::path(
    post,
    path = "/v1/mcp-servers",
    request_body = CreateMcpServerRequest,
    responses(
        (status = 201, description = "MCP server created successfully", body = WithUrls<McpServer>),
        (status = 400, description = "Invalid input or duplicate name", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "mcp-servers"
)]
pub async fn create_mcp_server(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Json(req): Json<CreateMcpServerRequest>,
) -> Result<(StatusCode, Json<WithUrls<McpServer>>), (StatusCode, Json<ErrorResponse>)> {
    state
        .dispatcher(&org)
        .run_created_with_urls(crate::domains::mcp_servers::CreateMcpServer(req))
        .await
}

/// GET /v1/mcp-servers - List all MCP servers
#[utoipa::path(
    get,
    path = "/v1/mcp-servers",
    responses(
        (status = 200, description = "List of MCP servers", body = ListResponse<WithUrls<McpServer>>),
        (status = 500, description = "Internal server error")
    ),
    params(ListMcpServersQuery),
    tag = "mcp-servers"
)]
pub async fn list_mcp_servers(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Query(query): Query<ListMcpServersQuery>,
) -> ApiResult<ListResponse<WithUrls<McpServer>>> {
    state
        .dispatcher(&org)
        .run_list_with_urls(crate::domains::mcp_servers::ListMcpServers {
            search: query.search,
            include_archived: query.include_archived.unwrap_or(false),
        })
        .await
}

#[utoipa::path(
    get,
    path = "/v1/mcp-servers/catalog",
    params(ListMcpServerCatalogQuery),
    responses(
        (status = 200, description = "Cursor-paginated MCP server catalog with active-agent usage counts", body = McpServerCatalogResponse),
        (status = 400, description = "Invalid cursor or limit", body = ErrorResponse),
        (status = 403, description = "Permission denied", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse),
    ),
    tag = "mcp-servers"
)]
pub async fn list_mcp_server_catalog(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Query(query): Query<ListMcpServerCatalogQuery>,
) -> Result<Json<McpServerCatalogResponse>, (StatusCode, Json<ErrorResponse>)> {
    let caller = Caller::from(&org);
    MCP_SERVER_VIEW
        .evaluate_with(state.auth.permission_resolver.as_ref(), &caller)
        .map_err(|error| ErrorResponse::new(error.message).into_response(StatusCode::FORBIDDEN))?;
    let limit = bounded_page_limit(query.limit, CATALOG_DEFAULT_LIMIT, CATALOG_MAX_LIMIT)
        .map_err(|message| ErrorResponse::new(message).into_response(StatusCode::BAD_REQUEST))?;
    let cursor = query
        .cursor
        .map(|cursor| {
            cursor.parse::<McpServerId>().map_err(|_| {
                ErrorResponse::new("Invalid catalog cursor").into_response(StatusCode::BAD_REQUEST)
            })
        })
        .transpose()?;
    let mut servers = state
        .db
        .list_mcp_server_catalog_page(org.org_id, cursor, i64::from(limit) + 1)
        .await
        .map_err(|error| {
            tracing::error!(%error, "failed to list MCP catalog");
            ErrorResponse::internal_error()
        })?;
    let has_more = servers.len() > limit as usize;
    if has_more {
        servers.truncate(limit as usize);
    }
    let next_cursor = has_more
        .then(|| servers.last().map(|server| server.id.to_string()))
        .flatten();
    let server_ids = servers
        .iter()
        .map(|server| server.id.uuid())
        .collect::<Vec<_>>();
    let usage = state
        .db
        .list_mcp_server_agent_usage_for_ids(org.org_id, &server_ids)
        .await
        .map_err(|error| {
            tracing::error!(%error, "failed to load MCP catalog usage");
            ErrorResponse::internal_error()
        })?
        .into_iter()
        .map(|row| (row.mcp_server_id, row.used_by_agents))
        .collect::<std::collections::HashMap<_, _>>();
    let urls = UrlBuilder::from_auth_config(&state.auth.config);
    let data = servers
        .into_iter()
        .map(|row| {
            let used_by_agents = usage.get(&row.id).copied().unwrap_or_default();
            McpServerCatalogEntry {
                server: urls.wrap(crate::domains::mcp_servers::queries::row_to_mcp_server(
                    &row,
                )),
                used_by_agents,
            }
        })
        .collect();
    Ok(Json(McpServerCatalogResponse { data, next_cursor }))
}

#[utoipa::path(
    get,
    path = "/v1/mcp-servers/{server_id}/usage",
    params(("server_id" = String, Path, description = "MCP server ID")),
    responses(
        (status = 200, description = "Bounded active-agent archive impact", body = McpServerUsageResponse),
        (status = 400, description = "Invalid MCP server ID", body = ErrorResponse),
        (status = 403, description = "Permission denied", body = ErrorResponse),
        (status = 404, description = "MCP server not found", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse),
    ),
    tag = "mcp-servers"
)]
pub async fn get_mcp_server_usage(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(server_id): Path<String>,
) -> Result<Json<McpServerUsageResponse>, (StatusCode, Json<ErrorResponse>)> {
    let caller = Caller::from(&org);
    MCP_SERVER_VIEW
        .evaluate_with(state.auth.permission_resolver.as_ref(), &caller)
        .map_err(|error| ErrorResponse::new(error.message).into_response(StatusCode::FORBIDDEN))?;
    let server_id = server_id.parse::<McpServerId>().map_err(|error| {
        ErrorResponse::new(format!("Invalid MCP server ID: {error}"))
            .into_response(StatusCode::BAD_REQUEST)
    })?;
    state
        .db
        .get_mcp_server(org.org_id, server_id.uuid())
        .await
        .map_err(|error| {
            tracing::error!(%error, "failed to load MCP server for usage");
            ErrorResponse::internal_error()
        })?
        .filter(|server| server.status != "deleted")
        .ok_or_else(|| ErrorResponse::not_found("MCP server"))?;
    let usage = state
        .db
        .get_mcp_server_agent_names(org.org_id, server_id, USAGE_AGENT_NAME_LIMIT)
        .await
        .map_err(|error| {
            tracing::error!(%error, "failed to load MCP server usage");
            ErrorResponse::internal_error()
        })?;
    Ok(Json(McpServerUsageResponse {
        truncated: usage.total_count > usage.agent_names.len() as i64,
        agent_names: usage.agent_names,
        total_count: usage.total_count,
    }))
}

/// GET /v1/mcp-servers/{server_id} - Get MCP server by ID
#[utoipa::path(
    get,
    path = "/v1/mcp-servers/{server_id}",
    params(
        ("server_id" = String, Path, description = "MCP server ID (prefixed, e.g., mcp_...)")
    ),
    responses(
        (status = 200, description = "MCP server found", body = WithUrls<McpServer>),
        (status = 400, description = "Invalid server ID"),
        (status = 404, description = "MCP server not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "mcp-servers"
)]
pub async fn get_mcp_server(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(server_id): Path<String>,
) -> ApiResult<WithUrls<McpServer>> {
    state
        .dispatcher(&org)
        .run_with_urls(crate::domains::mcp_servers::GetMcpServer { id: server_id })
        .await
}

/// PATCH /v1/mcp-servers/{server_id} - Update MCP server
#[utoipa::path(
    patch,
    path = "/v1/mcp-servers/{server_id}",
    params(
        ("server_id" = String, Path, description = "MCP server ID (prefixed, e.g., mcp_...)")
    ),
    request_body = UpdateMcpServerRequest,
    responses(
        (status = 200, description = "MCP server updated successfully", body = WithUrls<McpServer>),
        (status = 400, description = "Invalid server ID or input", body = ErrorResponse),
        (status = 404, description = "MCP server not found", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "mcp-servers"
)]
pub async fn update_mcp_server(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(server_id): Path<String>,
    Json(req): Json<UpdateMcpServerRequest>,
) -> ApiResult<WithUrls<McpServer>> {
    state
        .dispatcher(&org)
        .run_with_urls(crate::domains::mcp_servers::UpdateMcpServerCmd { id: server_id, req })
        .await
}

/// DELETE /v1/mcp-servers/{server_id} - Delete MCP server
#[utoipa::path(
    delete,
    path = "/v1/mcp-servers/{server_id}",
    params(
        ("server_id" = String, Path, description = "MCP server ID (prefixed, e.g., mcp_...)")
    ),
    responses(
        (status = 204, description = "MCP server deleted successfully"),
        (status = 400, description = "Invalid server ID"),
        (status = 404, description = "MCP server not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "mcp-servers"
)]
pub async fn delete_mcp_server(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(server_id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    state
        .dispatcher(&org)
        .run_no_content(crate::domains::mcp_servers::DeleteMcpServer { id: server_id })
        .await
}

pub async fn destroy_mcp_server(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(server_id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    state
        .dispatcher(&org)
        .run_no_content(crate::domains::mcp_servers::DestroyMcpServer { id: server_id })
        .await
}

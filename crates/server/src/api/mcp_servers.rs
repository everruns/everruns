// MCP Server CRUD HTTP routes
// Routes: /v1/mcp-servers/...
//
// Spec: specs/mcp.md (umbrella), specs/mcp-servers.md (API endpoints)

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

use super::common::{ApiResult, ErrorResponse, ListResponse, WithUrls, impl_auth_state};
use super::dispatch::{Dispatchable, impl_dispatchable};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::{IntoParams, ToSchema};

/// Query parameters for listing MCP servers.
#[derive(Debug, Clone, Deserialize, IntoParams)]
pub struct ListMcpServersQuery {
    /// Search by name or description (case-insensitive substring match).
    pub search: Option<String>,
    /// Include archived MCP servers. Deleted MCP servers never appear in lists.
    pub include_archived: Option<bool>,
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

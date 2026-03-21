// MCP Server CRUD HTTP routes
// Routes: /v1/mcp-servers/...

use crate::auth::{AuthState, ResolvedOrg};
use crate::services::McpServerService;
use crate::services::mcp_server::{MCP_SERVER_DANGEROUS, MCP_SERVER_MANAGE, MCP_SERVER_VIEW};
use crate::storage::{EncryptionService, StorageBackend};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
};
use everruns_core::typed_id::McpServerId;
use everruns_core::{
    Caller, McpServer, McpServerAuthMode, McpServerStatus, McpServerTransportType,
    ResourceConfigResponse, evaluate_policies_with, validate_safe_url,
};

use super::common::{
    ApiOptionExt, ApiPolicyResultExt, ApiResult, ErrorResponse, ListResponse, impl_auth_state,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
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

/// Request to create a new MCP server
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateMcpServerRequest {
    /// The name of the MCP server. Must be unique.
    #[schema(example = "atlassian-mcp-server")]
    pub name: String,
    /// A human-readable description of what the MCP server provides.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "Atlassian MCP Server for Jira and Confluence")]
    pub description: Option<String>,
    /// The URL of the MCP server endpoint.
    #[schema(example = "https://mcp.atlassian.com/v1/mcp")]
    pub url: String,
    /// Transport type. Currently only "http" is supported.
    #[serde(default = "default_transport_type")]
    pub transport_type: McpServerTransportType,
    /// Authentication mode. Defaults to `api_key` when `api_key` is provided, otherwise `none`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_mode: Option<McpServerAuthMode>,
    /// API key for authentication (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    /// Additional HTTP headers for authentication.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub headers: Option<HashMap<String, String>>,
}

fn default_transport_type() -> McpServerTransportType {
    McpServerTransportType::Http
}

/// Request to update an MCP server. Only provided fields will be updated.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpdateMcpServerRequest {
    /// The name of the MCP server.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "updated-mcp-server")]
    pub name: Option<String>,
    /// A human-readable description of what the MCP server provides.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "Updated description")]
    pub description: Option<String>,
    /// The URL of the MCP server endpoint.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "https://mcp.example.com/v1/mcp")]
    pub url: Option<String>,
    /// Transport type.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transport_type: Option<McpServerTransportType>,
    /// Authentication mode.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_mode: Option<McpServerAuthMode>,
    /// The status of the MCP server. Set to "disabled" to disable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<McpServerStatus>,
    /// API key for authentication. Set to update.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    /// Additional HTTP headers for authentication.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub headers: Option<HashMap<String, String>>,
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
    pub service: Arc<McpServerService>,
    pub auth: AuthState,
}

impl AppState {
    pub fn new(
        db: Arc<StorageBackend>,
        encryption: Option<Arc<EncryptionService>>,
        auth: AuthState,
    ) -> Self {
        Self {
            service: Arc::new(McpServerService::new(db, encryption)),
            auth,
        }
    }
}

impl_auth_state!(AppState);

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
        (status = 201, description = "MCP server created successfully", body = McpServer),
        (status = 400, description = "Invalid input or duplicate name", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "mcp-servers"
)]
pub async fn create_mcp_server(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Json(req): Json<CreateMcpServerRequest>,
) -> Result<(StatusCode, Json<McpServer>), (StatusCode, Json<ErrorResponse>)> {
    // Validate name is not empty
    if req.name.trim().is_empty() {
        return Err(
            ErrorResponse::new("Name cannot be empty").into_response(StatusCode::BAD_REQUEST)
        );
    }

    // Validate URL: non-empty, safe scheme, no private/internal targets (SSRF)
    if req.url.trim().is_empty() {
        return Err(
            ErrorResponse::new("URL cannot be empty").into_response(StatusCode::BAD_REQUEST)
        );
    }
    validate_safe_url(&req.url).map_err(|e| {
        ErrorResponse::new(format!("Invalid MCP server URL: {e}"))
            .into_response(StatusCode::BAD_REQUEST)
    })?;

    let caller = Caller::from(&org);
    let server = state
        .service
        .create(&caller, req)
        .await
        .map_policy_or_internal("create MCP server")?;

    Ok((StatusCode::CREATED, Json(server)))
}

/// GET /v1/mcp-servers - List all MCP servers
#[utoipa::path(
    get,
    path = "/v1/mcp-servers",
    responses(
        (status = 200, description = "List of MCP servers", body = ListResponse<McpServer>),
        (status = 500, description = "Internal server error")
    ),
    params(ListMcpServersQuery),
    tag = "mcp-servers"
)]
pub async fn list_mcp_servers(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Query(query): Query<ListMcpServersQuery>,
) -> ApiResult<ListResponse<McpServer>> {
    let caller = Caller::from(&org);
    let servers = state
        .service
        .list(
            &caller,
            query.search.as_deref(),
            query.include_archived.unwrap_or(false),
        )
        .await
        .map_policy_or_internal("list MCP servers")?;

    Ok(Json(ListResponse::new(servers)))
}

/// GET /v1/mcp-servers/{server_id} - Get MCP server by ID
#[utoipa::path(
    get,
    path = "/v1/mcp-servers/{server_id}",
    params(
        ("server_id" = String, Path, description = "MCP server ID (prefixed, e.g., mcp_...)")
    ),
    responses(
        (status = 200, description = "MCP server found", body = McpServer),
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
) -> ApiResult<McpServer> {
    let server_id: McpServerId = server_id.parse().map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Invalid server ID: {}", e),
            }),
        )
    })?;

    let caller = Caller::from(&org);
    let server = state
        .service
        .get(&caller, server_id.uuid())
        .await
        .map_policy_or_internal("get MCP server")?
        .ok_or_not_found_json("MCP server")?;

    Ok(Json(server))
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
        (status = 200, description = "MCP server updated successfully", body = McpServer),
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
) -> ApiResult<McpServer> {
    let server_id: McpServerId = server_id.parse().map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Invalid server ID: {}", e),
            }),
        )
    })?;

    // Validate name if provided
    if let Some(ref name) = req.name
        && name.trim().is_empty()
    {
        return Err(
            ErrorResponse::new("Name cannot be empty").into_response(StatusCode::BAD_REQUEST)
        );
    }

    // Validate URL if provided: non-empty, safe scheme, no private/internal targets (SSRF)
    if let Some(ref url) = req.url {
        if url.trim().is_empty() {
            return Err(
                ErrorResponse::new("URL cannot be empty").into_response(StatusCode::BAD_REQUEST)
            );
        }
        validate_safe_url(url).map_err(|e| {
            ErrorResponse::new(format!("Invalid MCP server URL: {e}"))
                .into_response(StatusCode::BAD_REQUEST)
        })?;
    }

    let caller = Caller::from(&org);
    let server = state
        .service
        .update(&caller, server_id.uuid(), req)
        .await
        .map_policy_or_internal("update MCP server")?
        .ok_or_not_found_json("MCP server")?;

    Ok(Json(server))
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
    let server_id: McpServerId = server_id.parse().map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Invalid server ID: {}", e),
            }),
        )
    })?;

    let caller = Caller::from(&org);
    let deleted = state
        .service
        .delete(&caller, server_id.uuid())
        .await
        .map_policy_or_internal("delete MCP server")?;

    if deleted {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "MCP server not found".to_string(),
            }),
        ))
    }
}

pub async fn destroy_mcp_server(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(server_id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let server_id: McpServerId = server_id.parse().map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Invalid server ID: {}", e),
            }),
        )
    })?;

    let caller = Caller::from(&org);
    let deleted = state
        .service
        .destroy(&caller, server_id.uuid())
        .await
        .map_policy_or_internal("destroy MCP server")?;

    if deleted {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ErrorResponse::new("MCP server not found").into_response(StatusCode::NOT_FOUND))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json;

    #[test]
    fn test_create_request_serialization() {
        let json = r#"{
            "name": "test-server",
            "url": "https://mcp.example.com/v1/mcp"
        }"#;

        let req: CreateMcpServerRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.name, "test-server");
        assert_eq!(req.url, "https://mcp.example.com/v1/mcp");
        assert_eq!(req.transport_type, McpServerTransportType::Http);
        assert!(req.description.is_none());
        assert!(req.api_key.is_none());
        assert!(req.headers.is_none());
    }

    #[test]
    fn test_create_request_with_all_fields() {
        let json = r#"{
            "name": "full-server",
            "description": "A test MCP server",
            "url": "https://mcp.example.com/v1/mcp",
            "transport_type": "http",
            "api_key": "secret-key",
            "headers": {"X-Custom": "value"}
        }"#;

        let req: CreateMcpServerRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.name, "full-server");
        assert_eq!(req.description, Some("A test MCP server".to_string()));
        assert_eq!(req.api_key, Some("secret-key".to_string()));
        assert!(req.headers.is_some());
        assert_eq!(
            req.headers.unwrap().get("X-Custom"),
            Some(&"value".to_string())
        );
    }

    #[test]
    fn test_update_request_partial() {
        let json = r#"{
            "description": "Updated description"
        }"#;

        let req: UpdateMcpServerRequest = serde_json::from_str(json).unwrap();
        assert!(req.name.is_none());
        assert_eq!(req.description, Some("Updated description".to_string()));
        assert!(req.url.is_none());
        assert!(req.status.is_none());
    }

    #[test]
    fn test_update_request_status() {
        let json = r#"{
            "status": "disabled"
        }"#;

        let req: UpdateMcpServerRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.status, Some(McpServerStatus::Disabled));
    }

    #[test]
    fn test_transport_type_deserialization() {
        let json = r#"{"name": "test", "url": "http://test", "transport_type": "http"}"#;
        let req: CreateMcpServerRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.transport_type, McpServerTransportType::Http);
    }

    #[test]
    fn test_default_transport_type() {
        assert_eq!(default_transport_type(), McpServerTransportType::Http);
    }

    // --- SSRF validation tests (URL safety) ---

    use everruns_core::validate_safe_url;

    #[test]
    fn ssrf_rejects_localhost_url() {
        assert!(validate_safe_url("http://localhost/mcp").is_err());
        assert!(validate_safe_url("http://localhost:8080/mcp").is_err());
    }

    #[test]
    fn ssrf_rejects_loopback_ip() {
        assert!(validate_safe_url("http://127.0.0.1/mcp").is_err());
        assert!(validate_safe_url("http://127.0.0.2:9999/mcp").is_err());
    }

    #[test]
    fn ssrf_rejects_private_ips() {
        assert!(validate_safe_url("http://10.0.0.1/mcp").is_err());
        assert!(validate_safe_url("http://172.16.0.1/mcp").is_err());
        assert!(validate_safe_url("http://192.168.1.1/mcp").is_err());
    }

    #[test]
    fn ssrf_rejects_cloud_metadata() {
        assert!(validate_safe_url("http://169.254.169.254/latest/meta-data/").is_err());
        assert!(validate_safe_url("http://metadata.google.internal/computeMetadata/v1/").is_err());
    }

    #[test]
    fn ssrf_rejects_ipv6_loopback() {
        assert!(validate_safe_url("http://[::1]/mcp").is_err());
    }

    #[test]
    fn ssrf_rejects_ipv4_mapped_ipv6() {
        assert!(validate_safe_url("http://[::ffff:127.0.0.1]/mcp").is_err());
        assert!(validate_safe_url("http://[::ffff:169.254.169.254]/mcp").is_err());
    }

    #[test]
    fn ssrf_rejects_disallowed_schemes() {
        assert!(validate_safe_url("ftp://example.com/mcp").is_err());
        assert!(validate_safe_url("file:///etc/passwd").is_err());
    }

    #[test]
    fn ssrf_allows_valid_public_https() {
        assert!(validate_safe_url("https://mcp.atlassian.com/v1/mcp").is_ok());
        assert!(validate_safe_url("https://mcp.example.com:8443/v1").is_ok());
    }
}

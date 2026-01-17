// MCP Server CRUD HTTP routes
// Routes are org-scoped: /v1/orgs/:org/mcp-servers/...

use crate::auth::{AuthState, OrgContext, middleware::FromRef};
use crate::services::McpServerService;
use crate::storage::{EncryptionService, StorageBackend};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
};
use everruns_core::{McpServer, McpServerStatus, McpServerTransportType};

use super::common::{ApiOptionExt, ApiResultExt, ErrorResponse, ListResponse};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

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

impl FromRef<AppState> for AuthState {
    fn from_ref(input: &AppState) -> Self {
        input.auth.clone()
    }
}

/// Create MCP server routes (org-scoped)
pub fn routes(state: AppState) -> Router {
    Router::new()
        .route(
            "/v1/orgs/:org/mcp-servers",
            post(create_mcp_server).get(list_mcp_servers),
        )
        .route(
            "/v1/orgs/:org/mcp-servers/:server_id",
            get(get_mcp_server)
                .patch(update_mcp_server)
                .delete(delete_mcp_server),
        )
        .with_state(state)
}

/// POST /v1/orgs/{org}/mcp-servers - Create a new MCP server
#[utoipa::path(
    post,
    path = "/v1/orgs/{org}/mcp-servers",
    params(
        ("org" = String, Path, description = "Organization public ID")
    ),
    request_body = CreateMcpServerRequest,
    responses(
        (status = 201, description = "MCP server created successfully", body = McpServer),
        (status = 400, description = "Invalid input or duplicate name", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "mcp-servers"
)]
pub async fn create_mcp_server(
    _org: OrgContext,
    State(state): State<AppState>,
    Json(req): Json<CreateMcpServerRequest>,
) -> Result<(StatusCode, Json<McpServer>), (StatusCode, Json<ErrorResponse>)> {
    // Validate name is not empty
    if req.name.trim().is_empty() {
        return Err(
            ErrorResponse::new("Name cannot be empty").into_response(StatusCode::BAD_REQUEST)
        );
    }

    // Validate URL is not empty
    if req.url.trim().is_empty() {
        return Err(
            ErrorResponse::new("URL cannot be empty").into_response(StatusCode::BAD_REQUEST)
        );
    }

    let server = state.service.create(req).await.map_err(|e| {
        // Check if it's a duplicate name error
        let msg = e.to_string();
        if msg.contains("already exists") {
            ErrorResponse::new(msg).into_response(StatusCode::BAD_REQUEST)
        } else {
            tracing::error!("Failed to create MCP server: {}", e);
            ErrorResponse::internal_error()
        }
    })?;

    Ok((StatusCode::CREATED, Json(server)))
}

/// GET /v1/orgs/{org}/mcp-servers - List all MCP servers
#[utoipa::path(
    get,
    path = "/v1/orgs/{org}/mcp-servers",
    params(
        ("org" = String, Path, description = "Organization public ID")
    ),
    responses(
        (status = 200, description = "List of MCP servers", body = ListResponse<McpServer>),
        (status = 500, description = "Internal server error")
    ),
    tag = "mcp-servers"
)]
pub async fn list_mcp_servers(
    _org: OrgContext,
    State(state): State<AppState>,
) -> Result<Json<ListResponse<McpServer>>, StatusCode> {
    let servers = state
        .service
        .list()
        .await
        .log_internal_error("list MCP servers")?;

    Ok(Json(ListResponse::new(servers)))
}

/// GET /v1/orgs/{org}/mcp-servers/{server_id} - Get MCP server by ID
#[utoipa::path(
    get,
    path = "/v1/orgs/{org}/mcp-servers/{server_id}",
    params(
        ("org" = String, Path, description = "Organization public ID"),
        ("server_id" = Uuid, Path, description = "MCP server ID")
    ),
    responses(
        (status = 200, description = "MCP server found", body = McpServer),
        (status = 404, description = "MCP server not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "mcp-servers"
)]
pub async fn get_mcp_server(
    _org: OrgContext,
    State(state): State<AppState>,
    Path((_org_path, server_id)): Path<(String, Uuid)>,
) -> Result<Json<McpServer>, StatusCode> {
    let server = state
        .service
        .get(server_id)
        .await
        .log_internal_error("get MCP server")?
        .ok_or_not_found()?;

    Ok(Json(server))
}

/// PATCH /v1/orgs/{org}/mcp-servers/{server_id} - Update MCP server
#[utoipa::path(
    patch,
    path = "/v1/orgs/{org}/mcp-servers/{server_id}",
    params(
        ("org" = String, Path, description = "Organization public ID"),
        ("server_id" = Uuid, Path, description = "MCP server ID")
    ),
    request_body = UpdateMcpServerRequest,
    responses(
        (status = 200, description = "MCP server updated successfully", body = McpServer),
        (status = 400, description = "Invalid input", body = ErrorResponse),
        (status = 404, description = "MCP server not found", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "mcp-servers"
)]
pub async fn update_mcp_server(
    _org: OrgContext,
    State(state): State<AppState>,
    Path((_org_path, server_id)): Path<(String, Uuid)>,
    Json(req): Json<UpdateMcpServerRequest>,
) -> Result<Json<McpServer>, (StatusCode, Json<ErrorResponse>)> {
    // Validate name if provided
    if let Some(ref name) = req.name
        && name.trim().is_empty()
    {
        return Err(
            ErrorResponse::new("Name cannot be empty").into_response(StatusCode::BAD_REQUEST)
        );
    }

    // Validate URL if provided
    if let Some(ref url) = req.url
        && url.trim().is_empty()
    {
        return Err(
            ErrorResponse::new("URL cannot be empty").into_response(StatusCode::BAD_REQUEST)
        );
    }

    let server = state
        .service
        .update(server_id, req)
        .await
        .log_internal_error_json("update MCP server")?
        .ok_or_not_found_json("MCP server")?;

    Ok(Json(server))
}

/// DELETE /v1/orgs/{org}/mcp-servers/{server_id} - Delete MCP server
#[utoipa::path(
    delete,
    path = "/v1/orgs/{org}/mcp-servers/{server_id}",
    params(
        ("org" = String, Path, description = "Organization public ID"),
        ("server_id" = Uuid, Path, description = "MCP server ID")
    ),
    responses(
        (status = 204, description = "MCP server deleted successfully"),
        (status = 404, description = "MCP server not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "mcp-servers"
)]
pub async fn delete_mcp_server(
    _org: OrgContext,
    State(state): State<AppState>,
    Path((_org_path, server_id)): Path<(String, Uuid)>,
) -> Result<StatusCode, StatusCode> {
    let deleted = state
        .service
        .delete(server_id)
        .await
        .log_internal_error("delete MCP server")?;

    if deleted {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(StatusCode::NOT_FOUND)
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
}

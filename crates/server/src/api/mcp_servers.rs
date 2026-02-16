// MCP Server CRUD HTTP routes
// Routes: /v1/mcp-servers/...

use crate::auth::middleware::AuthUser;
use crate::auth::{AuthState, ResolvedOrg};
use crate::services::{McpOAuthService, McpServerService};
use crate::storage::{EncryptionService, StorageBackend};
use axum::extract::FromRef;
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::Redirect,
    routing::{delete, get, post},
};
use everruns_core::typed_id::McpServerId;
use everruns_core::{
    McpOAuthStatus, McpServer, McpServerAuthType, McpServerStatus, McpServerTransportType,
};

use super::common::{ApiOptionExt, ApiResultExt, ErrorResponse, ListResponse};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

/// OAuth configuration input for MCP server
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct OAuthConfigInput {
    /// OAuth authorization endpoint URL.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authorization_url: Option<String>,
    /// OAuth token endpoint URL.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_url: Option<String>,
    /// OAuth client ID (public).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    /// OAuth client secret (write-only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_secret: Option<String>,
    /// Required OAuth scopes.
    #[serde(default)]
    pub scopes: Vec<String>,
    /// RFC 9728 Protected Resource Metadata URL.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_metadata_url: Option<String>,
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
    /// Authentication type: none, api_key, or oauth.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_type: Option<McpServerAuthType>,
    /// API key for authentication (when auth_type is api_key).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    /// Additional HTTP headers for authentication.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub headers: Option<HashMap<String, String>>,
    /// OAuth configuration (when auth_type is oauth).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub oauth_config: Option<OAuthConfigInput>,
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
    /// Authentication type: none, api_key, or oauth.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_type: Option<McpServerAuthType>,
    /// API key for authentication. Set to update.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    /// Additional HTTP headers for authentication.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub headers: Option<HashMap<String, String>>,
    /// OAuth configuration. Set to update OAuth settings.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub oauth_config: Option<OAuthConfigInput>,
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
    pub oauth_service: Option<Arc<McpOAuthService>>,
    pub auth: AuthState,
    pub base_url: String,
}

impl AppState {
    pub fn new(
        db: Arc<StorageBackend>,
        encryption: Option<Arc<EncryptionService>>,
        auth: AuthState,
        base_url: String,
    ) -> Self {
        let oauth_service = encryption.as_ref().map(|enc| {
            Arc::new(McpOAuthService::new(
                db.clone(),
                enc.clone(),
                base_url.clone(),
            ))
        });

        Self {
            service: Arc::new(McpServerService::new(db, encryption)),
            oauth_service,
            auth,
            base_url,
        }
    }
}

impl FromRef<AppState> for AuthState {
    fn from_ref(input: &AppState) -> Self {
        input.auth.clone()
    }
}

/// Create MCP server routes
pub fn routes(state: AppState) -> Router {
    Router::new()
        .route(
            "/v1/mcp-servers",
            post(create_mcp_server).get(list_mcp_servers),
        )
        .route(
            "/v1/mcp-servers/{server_id}",
            get(get_mcp_server)
                .patch(update_mcp_server)
                .delete(delete_mcp_server),
        )
        // OAuth routes
        .route(
            "/v1/mcp-servers/{server_id}/oauth/status",
            get(get_oauth_status),
        )
        .route(
            "/v1/mcp-servers/{server_id}/oauth/authorize",
            get(start_oauth_authorization).post(start_oauth_authorization_json),
        )
        .route("/v1/oauth/callback", get(handle_oauth_callback))
        .route(
            "/v1/mcp-servers/{server_id}/oauth/token",
            delete(revoke_oauth_token),
        )
        .with_state(state)
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

    // Validate URL is not empty
    if req.url.trim().is_empty() {
        return Err(
            ErrorResponse::new("URL cannot be empty").into_response(StatusCode::BAD_REQUEST)
        );
    }

    let server = state.service.create(org.org_id, req).await.map_err(|e| {
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

/// GET /v1/mcp-servers - List all MCP servers
#[utoipa::path(
    get,
    path = "/v1/mcp-servers",
    responses(
        (status = 200, description = "List of MCP servers", body = ListResponse<McpServer>),
        (status = 500, description = "Internal server error")
    ),
    tag = "mcp-servers"
)]
pub async fn list_mcp_servers(
    org: ResolvedOrg,
    State(state): State<AppState>,
) -> Result<Json<ListResponse<McpServer>>, StatusCode> {
    let servers = state
        .service
        .list(org.org_id)
        .await
        .log_internal_error("list MCP servers")?;

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
) -> Result<Json<McpServer>, (StatusCode, Json<ErrorResponse>)> {
    let server_id: McpServerId = server_id.parse().map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Invalid server ID: {}", e),
            }),
        )
    })?;

    let server = state
        .service
        .get(org.org_id, server_id.uuid())
        .await
        .log_internal_error_json("get MCP server")?
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
) -> Result<Json<McpServer>, (StatusCode, Json<ErrorResponse>)> {
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
        .update(org.org_id, server_id.uuid(), req)
        .await
        .log_internal_error_json("update MCP server")?
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

    let deleted = state
        .service
        .delete(org.org_id, server_id.uuid())
        .await
        .log_internal_error_json("delete MCP server")?;

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

// ============================================
// MCP OAuth Endpoints
// ============================================

/// Query parameters / body for OAuth authorization
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct OAuthAuthorizeQuery {
    /// URL to return to after authorization
    pub return_url: Option<String>,
}

/// Response from POST /oauth/authorize with the authorization URL
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct McpOAuthAuthorizationResponse {
    /// URL to redirect the user to for OAuth authorization
    pub authorization_url: String,
}

/// Query parameters for OAuth callback
#[derive(Debug, Clone, Deserialize)]
pub struct OAuthCallbackQuery {
    /// Authorization code from OAuth provider
    pub code: String,
    /// State parameter for CSRF protection
    pub state: String,
}

/// GET /v1/mcp-servers/{server_id}/oauth/status - Get OAuth authorization status
#[utoipa::path(
    get,
    path = "/v1/mcp-servers/{server_id}/oauth/status",
    params(
        ("server_id" = Uuid, Path, description = "MCP server ID")
    ),
    responses(
        (status = 200, description = "OAuth status", body = McpOAuthStatus),
        (status = 401, description = "Not authenticated"),
        (status = 404, description = "MCP server not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "mcp-oauth"
)]
pub async fn get_oauth_status(
    user: AuthUser,
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(server_id): Path<Uuid>,
) -> Result<Json<McpOAuthStatus>, (StatusCode, Json<ErrorResponse>)> {
    let oauth_service = state.oauth_service.as_ref().ok_or_else(|| {
        ErrorResponse::new("OAuth not configured (encryption not enabled)")
            .into_response(StatusCode::INTERNAL_SERVER_ERROR)
    })?;

    let status = oauth_service
        .get_oauth_status(server_id, org.org_id, user.id)
        .await
        .map_err(|e| {
            if e.to_string().contains("not found") {
                ErrorResponse::new("MCP server not found").into_response(StatusCode::NOT_FOUND)
            } else {
                tracing::error!("Failed to get OAuth status: {}", e);
                ErrorResponse::internal_error()
            }
        })?;

    Ok(Json(status))
}

/// GET /v1/mcp-servers/{server_id}/oauth/authorize - Start OAuth authorization
#[utoipa::path(
    get,
    path = "/v1/mcp-servers/{server_id}/oauth/authorize",
    params(
        ("server_id" = Uuid, Path, description = "MCP server ID"),
        ("return_url" = Option<String>, Query, description = "URL to return to after authorization")
    ),
    responses(
        (status = 302, description = "Redirect to OAuth provider"),
        (status = 401, description = "Not authenticated"),
        (status = 400, description = "MCP server doesn't use OAuth"),
        (status = 404, description = "MCP server not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "mcp-oauth"
)]
pub async fn start_oauth_authorization(
    user: AuthUser,
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(server_id): Path<Uuid>,
    Query(query): Query<OAuthAuthorizeQuery>,
) -> Result<Redirect, (StatusCode, Json<ErrorResponse>)> {
    let oauth_service = state.oauth_service.as_ref().ok_or_else(|| {
        ErrorResponse::new("OAuth not configured (encryption not enabled)")
            .into_response(StatusCode::INTERNAL_SERVER_ERROR)
    })?;

    let auth_url = oauth_service
        .start_authorization(server_id, org.org_id, user.id, query.return_url)
        .await
        .map_err(|e| {
            let msg = e.to_string();
            if msg.contains("not found") {
                ErrorResponse::new("MCP server not found").into_response(StatusCode::NOT_FOUND)
            } else if msg.contains("does not use OAuth") {
                ErrorResponse::new("MCP server does not use OAuth authentication")
                    .into_response(StatusCode::BAD_REQUEST)
            } else if msg.contains("not configured") {
                ErrorResponse::new(msg).into_response(StatusCode::BAD_REQUEST)
            } else {
                tracing::error!("Failed to start OAuth authorization: {}", e);
                ErrorResponse::internal_error()
            }
        })?;

    Ok(Redirect::to(&auth_url))
}

/// POST /v1/mcp-servers/{server_id}/oauth/authorize - Start OAuth authorization (JSON response)
///
/// Returns a JSON body with the authorization URL instead of a redirect.
/// Used by the UI to programmatically get the URL before navigating.
#[utoipa::path(
    post,
    path = "/v1/mcp-servers/{server_id}/oauth/authorize",
    params(
        ("server_id" = Uuid, Path, description = "MCP server ID"),
    ),
    request_body(content = OAuthAuthorizeQuery, description = "Optional return URL"),
    responses(
        (status = 200, description = "Authorization URL", body = McpOAuthAuthorizationResponse),
        (status = 401, description = "Not authenticated"),
        (status = 400, description = "MCP server doesn't use OAuth"),
        (status = 404, description = "MCP server not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "mcp-oauth"
)]
pub async fn start_oauth_authorization_json(
    user: AuthUser,
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(server_id): Path<Uuid>,
    Json(body): Json<OAuthAuthorizeQuery>,
) -> Result<Json<McpOAuthAuthorizationResponse>, (StatusCode, Json<ErrorResponse>)> {
    let oauth_service = state.oauth_service.as_ref().ok_or_else(|| {
        ErrorResponse::new("OAuth not configured (encryption not enabled)")
            .into_response(StatusCode::INTERNAL_SERVER_ERROR)
    })?;

    let auth_url = oauth_service
        .start_authorization(server_id, org.org_id, user.id, body.return_url)
        .await
        .map_err(|e| {
            let msg = e.to_string();
            if msg.contains("not found") {
                ErrorResponse::new("MCP server not found").into_response(StatusCode::NOT_FOUND)
            } else if msg.contains("does not use OAuth") {
                ErrorResponse::new("MCP server does not use OAuth authentication")
                    .into_response(StatusCode::BAD_REQUEST)
            } else if msg.contains("not configured") {
                ErrorResponse::new(msg).into_response(StatusCode::BAD_REQUEST)
            } else {
                tracing::error!("Failed to start OAuth authorization: {}", e);
                ErrorResponse::internal_error()
            }
        })?;

    Ok(Json(McpOAuthAuthorizationResponse {
        authorization_url: auth_url,
    }))
}

/// GET /v1/oauth/callback - OAuth callback handler
///
/// Single stable callback URL for all MCP servers. The MCP server ID is recovered
/// from the `state` parameter (stored in mcp_oauth_states table). This avoids
/// needing to register a separate redirect URI per MCP server with OAuth providers.
#[utoipa::path(
    get,
    path = "/v1/oauth/callback",
    params(
        ("code" = String, Query, description = "Authorization code from OAuth provider"),
        ("state" = String, Query, description = "State parameter for CSRF protection")
    ),
    responses(
        (status = 302, description = "Redirect to return URL or default page"),
        (status = 400, description = "Invalid or expired state"),
        (status = 500, description = "Internal server error")
    ),
    tag = "mcp-oauth"
)]
pub async fn handle_oauth_callback(
    State(state): State<AppState>,
    Query(query): Query<OAuthCallbackQuery>,
) -> Result<Redirect, (StatusCode, String)> {
    let oauth_service = state.oauth_service.as_ref().ok_or_else(|| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "OAuth not configured".to_string(),
        )
    })?;

    // Parse state UUID
    let state_id = Uuid::parse_str(&query.state).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            "Invalid state parameter".to_string(),
        )
    })?;

    // Handle callback
    let (mcp_server_id, return_url) = oauth_service
        .handle_callback(state_id, &query.code)
        .await
        .map_err(|e| {
            let msg = e.to_string();
            if msg.contains("Invalid or expired") {
                (StatusCode::BAD_REQUEST, msg)
            } else {
                tracing::error!("OAuth callback failed: {}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "OAuth callback failed".to_string(),
                )
            }
        })?;

    // Redirect to return URL or default
    let redirect_url =
        return_url.unwrap_or_else(|| format!("/mcp-servers/{}/oauth/success", mcp_server_id));

    Ok(Redirect::to(&redirect_url))
}

/// DELETE /v1/mcp-servers/{server_id}/oauth/token - Revoke OAuth authorization
#[utoipa::path(
    delete,
    path = "/v1/mcp-servers/{server_id}/oauth/token",
    params(
        ("server_id" = Uuid, Path, description = "MCP server ID")
    ),
    responses(
        (status = 204, description = "Token revoked successfully"),
        (status = 401, description = "Not authenticated"),
        (status = 404, description = "No token found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "mcp-oauth"
)]
pub async fn revoke_oauth_token(
    user: AuthUser,
    State(state): State<AppState>,
    Path(server_id): Path<Uuid>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let oauth_service = state.oauth_service.as_ref().ok_or_else(|| {
        ErrorResponse::new("OAuth not configured (encryption not enabled)")
            .into_response(StatusCode::INTERNAL_SERVER_ERROR)
    })?;

    let deleted = oauth_service
        .revoke_authorization(server_id, user.id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to revoke OAuth token: {}", e);
            ErrorResponse::internal_error()
        })?;

    if deleted {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ErrorResponse::new("No token found").into_response(StatusCode::NOT_FOUND))
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

    #[test]
    fn test_create_request_with_oauth() {
        let json = r#"{
            "name": "oauth-server",
            "url": "https://mcp.example.com/v1/mcp",
            "auth_type": "oauth",
            "oauth_config": {
                "authorization_url": "https://auth.example.com/authorize",
                "token_url": "https://auth.example.com/token",
                "client_id": "client123",
                "client_secret": "secret456",
                "scopes": ["read", "write"]
            }
        }"#;

        let req: CreateMcpServerRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.name, "oauth-server");
        assert_eq!(req.auth_type, Some(McpServerAuthType::OAuth));

        let oauth = req.oauth_config.unwrap();
        assert_eq!(
            oauth.authorization_url,
            Some("https://auth.example.com/authorize".to_string())
        );
        assert_eq!(
            oauth.token_url,
            Some("https://auth.example.com/token".to_string())
        );
        assert_eq!(oauth.client_id, Some("client123".to_string()));
        assert_eq!(oauth.client_secret, Some("secret456".to_string()));
        assert_eq!(oauth.scopes, vec!["read", "write"]);
    }

    #[test]
    fn test_create_request_with_api_key_auth() {
        let json = r#"{
            "name": "apikey-server",
            "url": "https://mcp.example.com/v1/mcp",
            "auth_type": "api_key",
            "api_key": "my-secret-key"
        }"#;

        let req: CreateMcpServerRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.auth_type, Some(McpServerAuthType::ApiKey));
        assert_eq!(req.api_key, Some("my-secret-key".to_string()));
    }

    #[test]
    fn test_oauth_config_optional_fields() {
        let json = r#"{
            "name": "discovery-oauth-server",
            "url": "https://mcp.example.com/v1/mcp",
            "auth_type": "oauth",
            "oauth_config": {
                "client_id": "client123",
                "resource_metadata_url": "https://mcp.example.com/.well-known/oauth-protected-resource"
            }
        }"#;

        let req: CreateMcpServerRequest = serde_json::from_str(json).unwrap();
        let oauth = req.oauth_config.unwrap();
        assert_eq!(oauth.client_id, Some("client123".to_string()));
        assert_eq!(
            oauth.resource_metadata_url,
            Some("https://mcp.example.com/.well-known/oauth-protected-resource".to_string())
        );
        assert!(oauth.authorization_url.is_none());
        assert!(oauth.token_url.is_none());
        assert!(oauth.client_secret.is_none());
        assert!(oauth.scopes.is_empty());
    }

    #[test]
    fn test_update_request_with_oauth_config() {
        let json = r#"{
            "auth_type": "oauth",
            "oauth_config": {
                "scopes": ["read", "write", "admin"]
            }
        }"#;

        let req: UpdateMcpServerRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.auth_type, Some(McpServerAuthType::OAuth));
        let oauth = req.oauth_config.unwrap();
        assert_eq!(oauth.scopes, vec!["read", "write", "admin"]);
    }

    #[test]
    fn test_oauth_config_serialization() {
        let config = OAuthConfigInput {
            authorization_url: Some("https://auth.example.com/authorize".to_string()),
            token_url: Some("https://auth.example.com/token".to_string()),
            client_id: Some("client123".to_string()),
            client_secret: None, // Should be skipped in serialization
            scopes: vec!["read".to_string()],
            resource_metadata_url: None,
        };

        let json = serde_json::to_string(&config).unwrap();
        // Verify client_secret is not in the output (skip_serializing_if = "Option::is_none")
        assert!(json.contains("authorization_url"));
        assert!(json.contains("client_id"));
        assert!(!json.contains("client_secret"));
        assert!(!json.contains("resource_metadata_url"));
    }
}

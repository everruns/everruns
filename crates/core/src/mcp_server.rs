// MCP Server domain types
//
// These types represent the MCP (Model Context Protocol) server configuration.
// Used by both API and worker crates.
//
// Currently supports only HTTP (Streamable HTTP) transport.
// MCP tool types follow the MCP specification for tool discovery and execution.
//
// OAuth 2.1 support follows the MCP Authorization specification (2025-06-18):
// - RFC 9728: Protected Resource Metadata for authorization server discovery
// - RFC 8414: OAuth Authorization Server Metadata
// - RFC 8707: Resource Indicators for audience binding
// - PKCE (S256): Required for all authorization flows

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use uuid::Uuid;

use crate::typed_id::McpServerId;

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

/// MCP Server transport type.
/// Currently only HTTP is supported.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum McpServerTransportType {
    /// HTTP (Streamable HTTP) transport
    Http,
}

impl std::fmt::Display for McpServerTransportType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            McpServerTransportType::Http => write!(f, "http"),
        }
    }
}

impl From<&str> for McpServerTransportType {
    fn from(s: &str) -> Self {
        match s {
            "http" => McpServerTransportType::Http,
            _ => McpServerTransportType::Http, // Default to HTTP
        }
    }
}

/// MCP Server lifecycle status.
/// - `active`: Server is available for use
/// - `disabled`: Server is disabled and not used
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum McpServerStatus {
    /// Server is available for use.
    Active,
    /// Server is disabled and not used.
    Disabled,
}

impl std::fmt::Display for McpServerStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            McpServerStatus::Active => write!(f, "active"),
            McpServerStatus::Disabled => write!(f, "disabled"),
        }
    }
}

impl From<&str> for McpServerStatus {
    fn from(s: &str) -> Self {
        match s {
            "disabled" => McpServerStatus::Disabled,
            _ => McpServerStatus::Active,
        }
    }
}

/// MCP Server authentication type.
/// Determines how authentication is handled for requests to the MCP server.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum McpServerAuthType {
    /// No authentication required.
    #[default]
    None,
    /// Static API key authentication (uses api_key_encrypted field).
    ApiKey,
    /// OAuth 2.1 authentication (per-user tokens).
    #[serde(rename = "oauth")]
    OAuth,
}

impl std::fmt::Display for McpServerAuthType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            McpServerAuthType::None => write!(f, "none"),
            McpServerAuthType::ApiKey => write!(f, "api_key"),
            McpServerAuthType::OAuth => write!(f, "oauth"),
        }
    }
}

impl From<&str> for McpServerAuthType {
    fn from(s: &str) -> Self {
        match s {
            "api_key" => McpServerAuthType::ApiKey,
            "oauth" => McpServerAuthType::OAuth,
            _ => McpServerAuthType::None,
        }
    }
}

/// MCP Server configuration.
/// Represents a remote MCP server that can provide tools and resources.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct McpServer {
    /// Unique identifier for the MCP server.
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "mcp_01933b5a00007000800000000000001"))]
    pub id: McpServerId,
    /// Display name of the MCP server.
    #[cfg_attr(feature = "openapi", schema(example = "atlassian-mcp-server"))]
    pub name: String,
    /// Human-readable description of the MCP server.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(
        feature = "openapi",
        schema(example = "Atlassian MCP Server for Jira and Confluence")
    )]
    pub description: Option<String>,
    /// URL of the MCP server endpoint.
    #[cfg_attr(
        feature = "openapi",
        schema(example = "https://mcp.atlassian.com/v1/mcp")
    )]
    pub url: String,
    /// Transport type (currently only HTTP supported).
    pub transport_type: McpServerTransportType,
    /// Current lifecycle status of the MCP server.
    pub status: McpServerStatus,
    /// Authentication type for the MCP server.
    #[serde(default)]
    pub auth_type: McpServerAuthType,
    /// Whether an API key has been configured (for api_key auth_type).
    pub api_key_set: bool,
    /// Additional HTTP headers for authentication.
    /// Keys are header names, values are header values.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub headers: HashMap<String, String>,
    /// OAuth configuration (only set when auth_type is OAuth).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oauth_config: Option<McpServerOAuthConfig>,
    /// Timestamp when the MCP server was created.
    pub created_at: DateTime<Utc>,
    /// Timestamp when the MCP server was last updated.
    pub updated_at: DateTime<Utc>,
}

/// OAuth configuration for an MCP server.
/// Contains the OAuth endpoints and client credentials.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct McpServerOAuthConfig {
    /// OAuth authorization endpoint URL.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authorization_url: Option<String>,
    /// OAuth token endpoint URL.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_url: Option<String>,
    /// OAuth client ID (public).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    /// Whether a client secret has been configured.
    #[serde(default)]
    pub client_secret_set: bool,
    /// Required OAuth scopes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scopes: Vec<String>,
    /// RFC 9728 Protected Resource Metadata URL (auto-discovered).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_metadata_url: Option<String>,
}

// ============================================================================
// MCP Tool Types (following MCP specification)
// ============================================================================

/// MCP Tool definition as returned by tools/list.
/// Follows the MCP specification for tool discovery.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct McpToolDefinition {
    /// Unique name of the tool within the MCP server.
    pub name: String,
    /// Human-readable description of what the tool does.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// JSON Schema describing the tool's input parameters.
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
}

/// Request for MCP tools/list endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolsListRequest {
    pub jsonrpc: String,
    pub id: i64,
    pub method: String,
}

impl Default for McpToolsListRequest {
    fn default() -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id: 1,
            method: "tools/list".to_string(),
        }
    }
}

/// Response from MCP tools/list endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolsListResponse {
    pub jsonrpc: String,
    pub id: i64,
    #[serde(default)]
    pub result: Option<McpToolsListResult>,
    #[serde(default)]
    pub error: Option<McpError>,
}

/// Result of tools/list containing the list of tools.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolsListResult {
    pub tools: Vec<McpToolDefinition>,
    #[serde(rename = "nextCursor", skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// MCP error response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpError {
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

/// Request for MCP tools/call endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolCallRequest {
    pub jsonrpc: String,
    pub id: i64,
    pub method: String,
    pub params: McpToolCallParams,
}

/// Parameters for tools/call request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolCallParams {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arguments: Option<Value>,
}

impl McpToolCallRequest {
    pub fn new(id: i64, name: String, arguments: Option<Value>) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            method: "tools/call".to_string(),
            params: McpToolCallParams { name, arguments },
        }
    }
}

/// Response from MCP tools/call endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolCallResponse {
    pub jsonrpc: String,
    pub id: i64,
    #[serde(default)]
    pub result: Option<McpToolCallResult>,
    #[serde(default)]
    pub error: Option<McpError>,
}

/// Result of tools/call containing content.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolCallResult {
    pub content: Vec<McpContent>,
    #[serde(rename = "isError", default)]
    pub is_error: bool,
}

/// MCP content type (text, image, etc.).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum McpContent {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image")]
    Image { data: String, mime_type: String },
    #[serde(rename = "resource")]
    Resource {
        uri: String,
        mime_type: Option<String>,
        text: Option<String>,
    },
}

/// Helper to generate prefixed tool name for MCP tools.
/// Format: mcp_{server_name}__{tool_name} (double underscore separator)
/// The double underscore allows unambiguous parsing when server names contain underscores.
pub fn mcp_tool_name(server_name: &str, tool_name: &str) -> String {
    // Sanitize server name: lowercase, replace non-alphanumeric with underscore
    let sanitized_server = server_name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect::<String>();
    format!("mcp_{}__{}", sanitized_server, tool_name)
}

/// Check if a tool name is an MCP tool (starts with "mcp_").
pub fn is_mcp_tool(tool_name: &str) -> bool {
    tool_name.starts_with("mcp_")
}

/// Parse MCP tool name to extract server name prefix and original tool name.
/// Returns (server_name_prefix, original_tool_name) if valid MCP tool.
/// Expected format: mcp_{server_name}__{tool_name} (double underscore separator)
pub fn parse_mcp_tool_name(tool_name: &str) -> Option<(String, String)> {
    if !tool_name.starts_with("mcp_") {
        return None;
    }
    let rest = &tool_name[4..]; // Skip "mcp_"
    // Find the double underscore separator between server name and tool name
    if let Some(pos) = rest.find("__") {
        let server_prefix = rest[..pos].to_string();
        let original_name = rest[pos + 2..].to_string(); // Skip "__"
        if !server_prefix.is_empty() && !original_name.is_empty() {
            return Some((server_prefix, original_name));
        }
    }
    None
}

// ============================================================================
// MCP OAuth Types (following MCP Authorization specification 2025-06-18)
// ============================================================================

/// Per-user OAuth token for an MCP server.
/// Each user has their own token for each OAuth-protected MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct McpUserToken {
    /// Unique identifier for the token record.
    pub id: Uuid,
    /// MCP server this token is for.
    pub mcp_server_id: Uuid,
    /// User who owns this token.
    pub user_id: Uuid,
    /// Token type (typically "Bearer").
    pub token_type: String,
    /// Granted OAuth scopes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    /// When the access token expires.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<DateTime<Utc>>,
    /// When the token was created.
    pub created_at: DateTime<Utc>,
    /// When the token was last updated.
    pub updated_at: DateTime<Utc>,
}

/// OAuth authorization status for a user and MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct McpOAuthStatus {
    /// Authentication type of the MCP server.
    pub auth_type: McpServerAuthType,
    /// Whether the user is authorized (has valid tokens).
    pub authorized: bool,
    /// When the current access token expires.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<DateTime<Utc>>,
    /// Granted OAuth scopes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scopes: Vec<String>,
    /// URL to initiate authorization (if not authorized).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authorization_url: Option<String>,
}

/// RFC 9728: OAuth Protected Resource Metadata.
/// Advertises authorization servers for a protected resource.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProtectedResourceMetadata {
    /// The resource server's identifier (typically the MCP server URL).
    pub resource: String,
    /// List of authorization server URLs that can issue tokens for this resource.
    pub authorization_servers: Vec<String>,
    /// Bearer token authentication methods supported.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bearer_methods_supported: Vec<String>,
    /// Resource documentation URL.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_documentation: Option<String>,
    /// Resource policy URI.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_policy_uri: Option<String>,
    /// Resource terms of service URI.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_tos_uri: Option<String>,
    /// Scopes supported by this resource.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scopes_supported: Vec<String>,
}

/// RFC 8414: OAuth Authorization Server Metadata.
/// Contains OAuth endpoints and capabilities.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthorizationServerMetadata {
    /// Authorization server's issuer identifier.
    pub issuer: String,
    /// Authorization endpoint URL.
    pub authorization_endpoint: String,
    /// Token endpoint URL.
    pub token_endpoint: String,
    /// Client registration endpoint URL (RFC 7591).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub registration_endpoint: Option<String>,
    /// Token revocation endpoint URL.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revocation_endpoint: Option<String>,
    /// Supported response types.
    #[serde(default)]
    pub response_types_supported: Vec<String>,
    /// Supported grant types.
    #[serde(default)]
    pub grant_types_supported: Vec<String>,
    /// Supported scopes.
    #[serde(default)]
    pub scopes_supported: Vec<String>,
    /// Supported code challenge methods (PKCE).
    #[serde(default)]
    pub code_challenge_methods_supported: Vec<String>,
    /// Token endpoint authentication methods.
    #[serde(default)]
    pub token_endpoint_auth_methods_supported: Vec<String>,
}

/// OAuth token response from token endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthTokenResponse {
    /// Access token.
    pub access_token: String,
    /// Token type (typically "Bearer").
    pub token_type: String,
    /// Token lifetime in seconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_in: Option<i64>,
    /// Refresh token for obtaining new access tokens.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    /// Granted scopes (may differ from requested).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
}

/// OAuth error response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthErrorResponse {
    /// Error code.
    pub error: String,
    /// Human-readable error description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_description: Option<String>,
    /// URI for more information.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_uri: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mcp_tool_name_simple() {
        // Simple server name without special characters
        assert_eq!(mcp_tool_name("github", "search"), "mcp_github__search");
    }

    #[test]
    fn test_mcp_tool_name_with_underscores() {
        // Server name with underscores (e.g., microsoft_learn)
        assert_eq!(
            mcp_tool_name("microsoft_learn", "docs_search"),
            "mcp_microsoft_learn__docs_search"
        );
    }

    #[test]
    fn test_mcp_tool_name_with_dashes() {
        // Server name with dashes gets converted to underscores
        assert_eq!(
            mcp_tool_name("microsoft-learn", "search"),
            "mcp_microsoft_learn__search"
        );
    }

    #[test]
    fn test_mcp_tool_name_uppercase() {
        // Server name is lowercased
        assert_eq!(mcp_tool_name("GitHub", "search"), "mcp_github__search");
    }

    #[test]
    fn test_mcp_tool_name_special_chars() {
        // Special characters are replaced with underscores
        assert_eq!(
            mcp_tool_name("my.server.name", "tool"),
            "mcp_my_server_name__tool"
        );
    }

    #[test]
    fn test_is_mcp_tool() {
        assert!(is_mcp_tool("mcp_github__search"));
        assert!(is_mcp_tool("mcp_microsoft_learn__docs_search"));
        assert!(!is_mcp_tool("get_weather"));
        assert!(!is_mcp_tool("mcpsearch")); // Must have underscore after mcp
    }

    #[test]
    fn test_parse_mcp_tool_name_simple() {
        let result = parse_mcp_tool_name("mcp_github__search");
        assert_eq!(result, Some(("github".to_string(), "search".to_string())));
    }

    #[test]
    fn test_parse_mcp_tool_name_with_underscores() {
        // Server name with underscores should be parsed correctly
        let result = parse_mcp_tool_name("mcp_microsoft_learn__docs_search");
        assert_eq!(
            result,
            Some(("microsoft_learn".to_string(), "docs_search".to_string()))
        );
    }

    #[test]
    fn test_parse_mcp_tool_name_complex() {
        // Multiple underscores in both server name and tool name
        let result = parse_mcp_tool_name("mcp_my_long_server_name__my_complex_tool");
        assert_eq!(
            result,
            Some((
                "my_long_server_name".to_string(),
                "my_complex_tool".to_string()
            ))
        );
    }

    #[test]
    fn test_parse_mcp_tool_name_invalid_prefix() {
        // Not an MCP tool
        assert_eq!(parse_mcp_tool_name("get_weather"), None);
    }

    #[test]
    fn test_parse_mcp_tool_name_no_separator() {
        // Missing double underscore separator
        assert_eq!(parse_mcp_tool_name("mcp_github_search"), None);
    }

    #[test]
    fn test_parse_mcp_tool_name_empty_parts() {
        // Empty server name or tool name
        assert_eq!(parse_mcp_tool_name("mcp___search"), None);
        assert_eq!(parse_mcp_tool_name("mcp_github__"), None);
    }

    #[test]
    fn test_roundtrip() {
        // Generate and parse should roundtrip
        let server = "microsoft_learn";
        let tool = "docs_search";
        let full_name = mcp_tool_name(server, tool);
        let parsed = parse_mcp_tool_name(&full_name);
        assert_eq!(
            parsed,
            Some(("microsoft_learn".to_string(), "docs_search".to_string()))
        );
    }

    // ============================================================================
    // OAuth Types Tests
    // ============================================================================

    #[test]
    fn test_auth_type_from_str() {
        assert_eq!(McpServerAuthType::from("none"), McpServerAuthType::None);
        assert_eq!(
            McpServerAuthType::from("api_key"),
            McpServerAuthType::ApiKey
        );
        assert_eq!(McpServerAuthType::from("oauth"), McpServerAuthType::OAuth);
        assert_eq!(McpServerAuthType::from("unknown"), McpServerAuthType::None);
    }

    #[test]
    fn test_auth_type_display() {
        assert_eq!(McpServerAuthType::None.to_string(), "none");
        assert_eq!(McpServerAuthType::ApiKey.to_string(), "api_key");
        assert_eq!(McpServerAuthType::OAuth.to_string(), "oauth");
    }

    #[test]
    fn test_auth_type_serialization() {
        assert_eq!(
            serde_json::to_string(&McpServerAuthType::None).unwrap(),
            "\"none\""
        );
        assert_eq!(
            serde_json::to_string(&McpServerAuthType::ApiKey).unwrap(),
            "\"api_key\""
        );
        // OAuth has explicit serde(rename = "oauth") to avoid "o_auth"
        assert_eq!(
            serde_json::to_string(&McpServerAuthType::OAuth).unwrap(),
            "\"oauth\""
        );
    }

    #[test]
    fn test_auth_type_deserialization() {
        let none: McpServerAuthType = serde_json::from_str("\"none\"").unwrap();
        assert_eq!(none, McpServerAuthType::None);

        let api_key: McpServerAuthType = serde_json::from_str("\"api_key\"").unwrap();
        assert_eq!(api_key, McpServerAuthType::ApiKey);

        let oauth: McpServerAuthType = serde_json::from_str("\"oauth\"").unwrap();
        assert_eq!(oauth, McpServerAuthType::OAuth);
    }

    #[test]
    fn test_oauth_status_serialization() {
        let status = McpOAuthStatus {
            auth_type: McpServerAuthType::OAuth,
            authorized: true,
            expires_at: None,
            scopes: vec!["read".to_string(), "write".to_string()],
            authorization_url: None,
        };

        let json = serde_json::to_string(&status).unwrap();
        assert!(json.contains("\"auth_type\":\"oauth\""));
        assert!(json.contains("\"authorized\":true"));
        assert!(json.contains("\"scopes\":[\"read\",\"write\"]"));
        // authorization_url should be omitted when None
        assert!(!json.contains("authorization_url"));
    }

    #[test]
    fn test_oauth_status_deserialization() {
        let json = r#"{
            "auth_type": "oauth",
            "authorized": false,
            "scopes": [],
            "authorization_url": "https://auth.example.com/authorize"
        }"#;

        let status: McpOAuthStatus = serde_json::from_str(json).unwrap();
        assert_eq!(status.auth_type, McpServerAuthType::OAuth);
        assert!(!status.authorized);
        assert!(status.scopes.is_empty());
        assert_eq!(
            status.authorization_url,
            Some("https://auth.example.com/authorize".to_string())
        );
    }

    #[test]
    fn test_oauth_token_response_serialization() {
        let response = OAuthTokenResponse {
            access_token: "access123".to_string(),
            token_type: "Bearer".to_string(),
            expires_in: Some(3600),
            refresh_token: Some("refresh456".to_string()),
            scope: Some("read write".to_string()),
        };

        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"access_token\":\"access123\""));
        assert!(json.contains("\"token_type\":\"Bearer\""));
        assert!(json.contains("\"expires_in\":3600"));
        assert!(json.contains("\"refresh_token\":\"refresh456\""));
        assert!(json.contains("\"scope\":\"read write\""));
    }

    #[test]
    fn test_oauth_token_response_deserialization_minimal() {
        let json = r#"{
            "access_token": "token123",
            "token_type": "Bearer"
        }"#;

        let response: OAuthTokenResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.access_token, "token123");
        assert_eq!(response.token_type, "Bearer");
        assert!(response.expires_in.is_none());
        assert!(response.refresh_token.is_none());
        assert!(response.scope.is_none());
    }

    #[test]
    fn test_protected_resource_metadata_serialization() {
        let metadata = ProtectedResourceMetadata {
            resource: "https://mcp.example.com".to_string(),
            authorization_servers: vec!["https://auth.example.com".to_string()],
            bearer_methods_supported: vec!["header".to_string()],
            resource_documentation: None,
            resource_policy_uri: None,
            resource_tos_uri: None,
            scopes_supported: vec!["read".to_string()],
        };

        let json = serde_json::to_string(&metadata).unwrap();
        assert!(json.contains("\"resource\":\"https://mcp.example.com\""));
        assert!(json.contains("\"authorization_servers\":[\"https://auth.example.com\"]"));
    }

    #[test]
    fn test_authorization_server_metadata_deserialization() {
        let json = r#"{
            "issuer": "https://auth.example.com",
            "authorization_endpoint": "https://auth.example.com/authorize",
            "token_endpoint": "https://auth.example.com/token",
            "code_challenge_methods_supported": ["S256"]
        }"#;

        let metadata: AuthorizationServerMetadata = serde_json::from_str(json).unwrap();
        assert_eq!(metadata.issuer, "https://auth.example.com");
        assert_eq!(
            metadata.authorization_endpoint,
            "https://auth.example.com/authorize"
        );
        assert_eq!(metadata.token_endpoint, "https://auth.example.com/token");
        assert_eq!(metadata.code_challenge_methods_supported, vec!["S256"]);
    }

    #[test]
    fn test_mcp_server_oauth_config_default() {
        let config = McpServerOAuthConfig::default();
        assert!(config.authorization_url.is_none());
        assert!(config.token_url.is_none());
        assert!(config.client_id.is_none());
        assert!(!config.client_secret_set);
        assert!(config.scopes.is_empty());
        assert!(config.resource_metadata_url.is_none());
    }

    #[test]
    fn test_oauth_error_response() {
        let json = r#"{
            "error": "invalid_grant",
            "error_description": "The authorization code has expired"
        }"#;

        let error: OAuthErrorResponse = serde_json::from_str(json).unwrap();
        assert_eq!(error.error, "invalid_grant");
        assert_eq!(
            error.error_description,
            Some("The authorization code has expired".to_string())
        );
    }
}

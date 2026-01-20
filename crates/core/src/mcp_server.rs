// MCP Server domain types
//
// These types represent the MCP (Model Context Protocol) server configuration.
// Used by both API and worker crates.
//
// Currently supports only HTTP (Streamable HTTP) transport.
// MCP tool types follow the MCP specification for tool discovery and execution.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

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
    /// Whether an API key has been configured.
    pub api_key_set: bool,
    /// Additional HTTP headers for authentication.
    /// Keys are header names, values are header values.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub headers: HashMap<String, String>,
    /// Timestamp when the MCP server was created.
    pub created_at: DateTime<Utc>,
    /// Timestamp when the MCP server was last updated.
    pub updated_at: DateTime<Utc>,
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
}

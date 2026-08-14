// MCP Server domain types — canonical definitions for request shapes.
//
// Storage row types are re-exported from `storage::models` so domain code
// has a single import path.

use everruns_core::{McpProtocolMode, McpServerAuthMode, McpServerStatus, McpServerTransportType};
use serde::Deserialize;
use std::collections::HashMap;
use utoipa::ToSchema;

pub use crate::storage::models::{CreateMcpServerRow, McpServerRow, UpdateMcpServer};

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
    /// Example shape is defined on `McpServerTransportType`.
    #[serde(default = "default_transport_type")]
    pub transport_type: McpServerTransportType,
    /// Authentication mode. Defaults to `api_key` when `api_key` is provided, otherwise `none`.
    /// Example shape is defined on `McpServerAuthMode`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_mode: Option<McpServerAuthMode>,
    /// Protocol-era policy. Defaults to `auto` (negotiates every protocol era).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocol_mode: Option<McpProtocolMode>,
    /// API key for authentication (optional). Sent with each request; never echoed in responses.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "sk-mcp-redacted-1234567890abcdef")]
    pub api_key: Option<String>,
    /// Additional HTTP headers for authentication.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = json!({"X-Atlassian-Cloud-Id": "00000000-0000-0000-0000-000000000000"}))]
    pub headers: Option<HashMap<String, String>>,
}

pub(crate) fn default_transport_type() -> McpServerTransportType {
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
    /// Protocol-era policy (`auto`, `legacy`, `stable`, `rc`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocol_mode: Option<McpProtocolMode>,
    /// The status of the MCP server. Set to "disabled" to disable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<McpServerStatus>,
    /// API key for authentication. Set to update.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "sk-mcp-redacted-1234567890abcdef")]
    pub api_key: Option<String>,
    /// Additional HTTP headers for authentication.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = json!({"X-Atlassian-Cloud-Id": "00000000-0000-0000-0000-000000000000"}))]
    pub headers: Option<HashMap<String, String>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_request_deserialization() {
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

    use everruns_provider::url_validation::validate_safe_url;

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

// MCP Server domain types
//
// These types represent the MCP (Model Context Protocol) server configuration.
// Used by both API and worker crates.
//
// Currently supports only HTTP (Streamable HTTP) transport.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

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
    pub id: Uuid,
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

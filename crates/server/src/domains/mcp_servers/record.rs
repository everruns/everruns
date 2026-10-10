//! Organization-owned MCP server connections.
use chrono::{DateTime, Utc};
use everruns_contracts::typed_id::McpServerId;
use everruns_core::mcp_server::{McpServerAuthMode, McpServerTransportType};
use everruns_core::{McpElicitationPolicy, McpProtocolMode};

use super::presentation::McpServerPresentation;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::ToSchema;
/// MCP Server lifecycle status.
/// - `active`: Server is available for use
/// - `disabled`: Server is disabled and not used
/// - `archived`: Server is hidden from listings and cannot be modified or assigned
/// - `deleted`: Server is a tombstone kept only for historical references
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[schema(example = "active")]
#[serde(rename_all = "lowercase")]
pub enum McpServerStatus {
    /// Server is available for use.
    Active,
    /// Server is disabled and not used.
    Disabled,
    /// Server is hidden from listings and cannot be modified or assigned.
    Archived,
    /// Server is deleted and should only survive as a tombstone for references.
    Deleted,
}

impl std::fmt::Display for McpServerStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            McpServerStatus::Active => write!(f, "active"),
            McpServerStatus::Disabled => write!(f, "disabled"),
            McpServerStatus::Archived => write!(f, "archived"),
            McpServerStatus::Deleted => write!(f, "deleted"),
        }
    }
}

impl From<&str> for McpServerStatus {
    fn from(s: &str) -> Self {
        match s {
            "disabled" => McpServerStatus::Disabled,
            "archived" => McpServerStatus::Archived,
            "deleted" => McpServerStatus::Deleted,
            _ => McpServerStatus::Active,
        }
    }
}

/// MCP Server configuration.
/// Represents a remote MCP server that can provide tools and resources.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct McpServer {
    /// Unique identifier for the MCP server.
    #[schema(value_type = String, example = "mcp_01933b5a00007000800000000000001")]
    pub id: McpServerId,
    /// Display name of the MCP server.
    #[schema(example = "atlassian-mcp-server")]
    pub name: String,
    /// Human-readable description of the MCP server.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "Atlassian MCP Server for Jira and Confluence")]
    pub description: Option<String>,
    /// URL of the MCP server endpoint.
    #[schema(example = "https://mcp.atlassian.com/v1/mcp")]
    pub url: String,
    /// Transport type (currently only HTTP supported).
    pub transport_type: McpServerTransportType,
    /// Current lifecycle status of the MCP server.
    pub status: McpServerStatus,
    /// Authentication mode for this MCP server.
    #[serde(default)]
    pub auth_mode: McpServerAuthMode,
    /// Protocol-era adoption policy for the MCP client (`auto` negotiates).
    #[serde(default, skip_serializing_if = "McpProtocolMode::is_auto")]
    pub protocol_mode: McpProtocolMode,
    /// Which elicitation modes this server may use (`url` by default).
    #[serde(default, skip_serializing_if = "McpElicitationPolicy::is_default")]
    pub elicitation_policy: McpElicitationPolicy,
    /// Stable provider id used for user-scoped OAuth connections.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub oauth_provider_id: Option<String>,
    /// Connection provider whose connection on an agent's service virtual user
    /// supplies the service credential instead of an MCP OAuth grant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(example = "github")]
    pub service_connection_provider: Option<String>,
    /// Whether an API key has been configured.
    pub api_key_set: bool,
    /// Additional HTTP headers for authentication.
    /// Keys are header names, values are header values.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub headers: HashMap<String, String>,
    /// Title, icon, and links published by the remote server.
    /// The operator `name` and `description` are unchanged.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub presentation: Option<McpServerPresentation>,
    /// Timestamp when the MCP server was created.
    pub created_at: DateTime<Utc>,
    /// Timestamp when the MCP server was last updated.
    pub updated_at: DateTime<Utc>,
    /// Timestamp when the MCP server was archived.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archived_at: Option<DateTime<Utc>>,
    /// Timestamp when the MCP server was deleted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deleted_at: Option<DateTime<Utc>>,
}

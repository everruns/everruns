//! Transport abstraction over MCP connections (specs/runtime-mcp.md D2).
//!
//! A [`McpTransport`] speaks JSON-RPC against one logical server and returns
//! parsed results, so result/content mapping is shared across transports. The
//! HTTP transport is always compiled; stdio is behind the `stdio` feature and
//! is the "hard-off in hosted" boundary.

use crate::auth::McpCredential;
use async_trait::async_trait;
use everruns_core::{McpServerAuthMode, McpToolCallResult, McpToolDefinition};
use serde_json::Value;
use std::collections::HashMap;

/// Transport-specific connection target.
#[derive(Debug, Clone)]
pub enum McpEndpoint {
    /// Remote Streamable-HTTP MCP server.
    Http {
        url: String,
        headers: HashMap<String, String>,
    },
    /// Local process speaking MCP over stdio. Only constructible when the
    /// `stdio` feature is enabled (hard-off in hosted builds).
    #[cfg(feature = "stdio")]
    Stdio {
        command: String,
        args: Vec<String>,
        env: HashMap<String, String>,
    },
}

/// A resolved, transport-agnostic MCP server connection.
#[derive(Debug, Clone)]
pub struct McpConnection {
    /// Logical server name (used for tool-name prefixing and auth lookup).
    pub name: String,
    pub endpoint: McpEndpoint,
    pub auth_mode: McpServerAuthMode,
    pub oauth_provider_id: Option<String>,
}

impl McpConnection {
    /// Convenience constructor for a no-auth HTTP server.
    pub fn http(name: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            endpoint: McpEndpoint::Http {
                url: url.into(),
                headers: HashMap::new(),
            },
            auth_mode: McpServerAuthMode::None,
            oauth_provider_id: None,
        }
    }
}

/// Speaks MCP JSON-RPC against a single connection.
#[async_trait]
pub trait McpTransport: Send + Sync {
    async fn list_tools(
        &self,
        connection: &McpConnection,
        credential: Option<&McpCredential>,
    ) -> anyhow::Result<Vec<McpToolDefinition>>;

    async fn call_tool(
        &self,
        connection: &McpConnection,
        tool_name: &str,
        arguments: Value,
        credential: Option<&McpCredential>,
    ) -> anyhow::Result<McpToolCallResult>;
}

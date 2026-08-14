//! High-level MCP client: resolves credentials via a [`McpAuthProvider`],
//! selects the transport for a connection's endpoint, and maps results.

use crate::auth::{McpAuthProvider, McpAuthRequest, McpCredential, NoAuthProvider};
use crate::http::HttpTransport;
use crate::result::map_tool_call_result;
use crate::transport::{McpConnection, McpEndpoint, McpTransport};
use anyhow::Result;
use everruns_core::{EgressService, McpToolCallResult, McpToolDefinition};
use everruns_provider::ToolResultImage;
use everruns_provider::tool_types::ToolResult;
use serde_json::Value;
use std::sync::Arc;

/// Transport-selecting MCP client shared by all hosts.
pub struct McpClient {
    http: Arc<HttpTransport>,
    #[cfg(feature = "stdio")]
    stdio: Arc<crate::stdio::StdioTransport>,
    auth: Arc<dyn McpAuthProvider>,
}

impl McpClient {
    /// Build a client over a custom egress boundary with the given auth
    /// provider.
    pub fn new(egress: Arc<dyn EgressService>, auth: Arc<dyn McpAuthProvider>) -> Self {
        Self::with_http(Arc::new(HttpTransport::new(egress)), auth)
    }

    /// Build a client backed by direct outbound HTTP and no auth provider.
    pub fn direct() -> Self {
        Self::with_http(Arc::new(HttpTransport::direct()), Arc::new(NoAuthProvider))
    }

    pub fn with_http(http: Arc<HttpTransport>, auth: Arc<dyn McpAuthProvider>) -> Self {
        Self {
            http,
            #[cfg(feature = "stdio")]
            stdio: Arc::new(crate::stdio::StdioTransport),
            auth,
        }
    }

    fn transport_for(&self, connection: &McpConnection) -> &dyn McpTransport {
        match &connection.endpoint {
            McpEndpoint::Http { .. } => self.http.as_ref(),
            #[cfg(feature = "stdio")]
            McpEndpoint::Stdio { .. } => self.stdio.as_ref(),
        }
    }

    async fn credential(&self, connection: &McpConnection) -> Result<Option<McpCredential>> {
        self.auth
            .authorization(&McpAuthRequest {
                server_name: &connection.name,
                auth_mode: connection.auth_mode.clone(),
                oauth_provider_id: connection.oauth_provider_id.as_deref(),
            })
            .await
    }

    /// Discover the tools a server exposes via `tools/list`.
    pub async fn discover(&self, connection: &McpConnection) -> Result<Vec<McpToolDefinition>> {
        let credential = self.credential(connection).await?;
        self.transport_for(connection)
            .list_tools(connection, credential.as_ref())
            .await
    }

    /// Execute a tool via `tools/call`, returning the raw MCP result.
    pub async fn call(
        &self,
        connection: &McpConnection,
        tool_name: &str,
        arguments: Value,
    ) -> Result<McpToolCallResult> {
        let credential = self.credential(connection).await?;
        self.transport_for(connection)
            .call_tool(connection, tool_name, arguments, credential.as_ref())
            .await
    }

    /// Execute a tool and map the result into the internal `ToolResult` shape.
    pub async fn call_as_tool_result(
        &self,
        connection: &McpConnection,
        tool_call_id: impl Into<String>,
        tool_name: &str,
        arguments: Value,
    ) -> Result<ToolResult> {
        let result = self.call(connection, tool_name, arguments).await?;
        let (json_result, images) = map_tool_call_result(&result);
        Ok(tool_result(tool_call_id.into(), json_result, images))
    }
}

pub(crate) fn tool_result(
    tool_call_id: String,
    result: Value,
    images: Vec<ToolResultImage>,
) -> ToolResult {
    ToolResult {
        tool_call_id,
        result: Some(result),
        images: if images.is_empty() {
            None
        } else {
            Some(images)
        },
        error: None,
        connection_required: None,
        raw_output: None,
    }
}

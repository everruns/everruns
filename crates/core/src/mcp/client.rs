//! High-level MCP client: resolves credentials via a [`McpAuthProvider`],
//! selects the transport for a connection's endpoint, and maps results.

use crate::mcp::auth::{McpAuthProvider, McpAuthRequest, McpCredential};
use crate::mcp::elicitation::UrlElicitationHandler;
use crate::mcp::form_elicitation::FormElicitationHandler;
use crate::mcp::http::HttpTransport;
use crate::mcp::result::map_tool_call_result;
use crate::mcp::transport::{McpConnection, McpEndpoint, McpTransport};
use crate::{
    EgressService, McpReadResourceResult, McpResourceTemplatesPage, McpResourcesPage,
    McpToolCallResult, McpToolDefinition,
};
use anyhow::Result;
use everruns_contracts::ToolResultImage;
use everruns_contracts::tool_types::ToolResult;
use serde_json::{Value, json};
use std::sync::Arc;

/// Transport-selecting MCP client shared by all hosts.
pub struct McpClient {
    http: Arc<HttpTransport>,
    #[cfg(feature = "mcp-stdio")]
    stdio: Arc<crate::mcp::stdio::StdioTransport>,
    auth: Arc<dyn McpAuthProvider>,
}

impl McpClient {
    /// Build a client over a custom egress boundary with the given auth
    /// provider.
    pub fn new(egress: Arc<dyn EgressService>, auth: Arc<dyn McpAuthProvider>) -> Self {
        Self::with_http(Arc::new(HttpTransport::new(egress)), auth)
    }

    /// Build a client whose HTTP transport can put URL mode elicitations in
    /// front of a human via `elicitation`. Hosts with no reachable user (an
    /// unattended worker run) use [`McpClient::new`] instead, so the client
    /// declares no elicitation capability and servers must not ask.
    pub fn with_url_elicitation(
        egress: Arc<dyn EgressService>,
        auth: Arc<dyn McpAuthProvider>,
        elicitation: Arc<dyn UrlElicitationHandler>,
    ) -> Self {
        Self::with_http(
            Arc::new(HttpTransport::new(egress).with_elicitation_handler(elicitation)),
            auth,
        )
    }

    /// Build a client that also answers form mode elicitations through
    /// `form`, for servers whose `elicitation_policy` opts in. Form mode is
    /// never declared to any other server.
    pub fn with_url_and_form_elicitation(
        egress: Arc<dyn EgressService>,
        auth: Arc<dyn McpAuthProvider>,
        url: Arc<dyn UrlElicitationHandler>,
        form: Arc<dyn FormElicitationHandler>,
    ) -> Self {
        Self::with_http(
            Arc::new(
                HttpTransport::new(egress)
                    .with_elicitation_handler(url)
                    .with_form_elicitation_handler(form),
            ),
            auth,
        )
    }

    pub fn with_http(http: Arc<HttpTransport>, auth: Arc<dyn McpAuthProvider>) -> Self {
        Self {
            http,
            #[cfg(feature = "mcp-stdio")]
            stdio: Arc::new(crate::mcp::stdio::StdioTransport),
            auth,
        }
    }

    fn transport_for(&self, connection: &McpConnection) -> &dyn McpTransport {
        match &connection.endpoint {
            McpEndpoint::Http { .. } => self.http.as_ref(),
            #[cfg(feature = "mcp-stdio")]
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

    /// List one page of the server's resources (`resources/list`). Pass the
    /// previous page's `next_cursor` to continue.
    pub async fn list_resources(
        &self,
        connection: &McpConnection,
        cursor: Option<&str>,
    ) -> Result<McpResourcesPage> {
        let result = self
            .request(connection, "resources/list", cursor_params(cursor))
            .await?;
        Ok(serde_json::from_value(result)?)
    }

    /// List one page of the server's resource templates
    /// (`resources/templates/list`).
    pub async fn list_resource_templates(
        &self,
        connection: &McpConnection,
        cursor: Option<&str>,
    ) -> Result<McpResourceTemplatesPage> {
        let result = self
            .request(
                connection,
                "resources/templates/list",
                cursor_params(cursor),
            )
            .await?;
        Ok(serde_json::from_value(result)?)
    }

    /// Read one resource's contents (`resources/read`). An unknown URI comes
    /// back as an [`McpRpcError`](crate::mcp::McpRpcError), code `-32002` on
    /// servers that follow the spec.
    pub async fn read_resource(
        &self,
        connection: &McpConnection,
        uri: &str,
    ) -> Result<McpReadResourceResult> {
        let result = self
            .request(connection, "resources/read", json!({ "uri": uri }))
            .await?;
        Ok(serde_json::from_value(result)?)
    }

    async fn request(
        &self,
        connection: &McpConnection,
        method: &str,
        params: Value,
    ) -> Result<Value> {
        let credential = self.credential(connection).await?;
        self.transport_for(connection)
            .request(connection, method, params, credential.as_ref())
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

fn cursor_params(cursor: Option<&str>) -> Value {
    match cursor {
        Some(cursor) => json!({ "cursor": cursor }),
        None => json!({}),
    }
}

pub(crate) fn tool_result(
    tool_call_id: String,
    result: Value,
    images: Vec<ToolResultImage>,
) -> ToolResult {
    ToolResult {
        details: None,
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

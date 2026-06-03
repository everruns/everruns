//! HTTP (Streamable-HTTP) MCP transport.
//!
//! Lifted from `worker/src/mcp_executor.rs::call_mcp_tool` and
//! `server/.../service.rs::fetch_mcp_tools` so the DNS-pinned SSRF contract
//! (TM-TOOL-018) and SSE/JSON handling are shared, not duplicated
//! (specs/runtime-mcp.md D1/D5).

use crate::auth::McpCredential;
use crate::result::extract_json_from_response;
use crate::transport::{McpConnection, McpEndpoint, McpTransport};
use anyhow::{Result, anyhow};
use async_trait::async_trait;
use everruns_core::{
    DirectEgressService, EgressRequest, EgressRequestKind, EgressService, McpToolCallRequest,
    McpToolCallResponse, McpToolCallResult, McpToolDefinition, McpToolsListRequest,
    McpToolsListResponse, validate_url_dns_pinned,
};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

const MCP_TIMEOUT: Duration = Duration::from_secs(60);

/// MCP transport over the platform [`EgressService`] boundary.
pub struct HttpTransport {
    egress: Arc<dyn EgressService>,
}

impl HttpTransport {
    pub fn new(egress: Arc<dyn EgressService>) -> Self {
        Self { egress }
    }

    /// Build a transport backed by a [`DirectEgressService`] (real outbound
    /// HTTP). Used by hosts without a custom egress boundary, e.g. the CLI.
    pub fn direct() -> Self {
        Self {
            egress: Arc::new(DirectEgressService::default()),
        }
    }

    fn http_parts(connection: &McpConnection) -> Result<(&str, &HashMap<String, String>)> {
        match &connection.endpoint {
            McpEndpoint::Http { url, headers } => Ok((url.as_str(), headers)),
            #[cfg(feature = "stdio")]
            _ => Err(anyhow!(
                "HttpTransport received a non-HTTP endpoint for server '{}'",
                connection.name
            )),
        }
    }

    async fn send_rpc(
        &self,
        connection: &McpConnection,
        body: Vec<u8>,
        credential: Option<&McpCredential>,
    ) -> Result<String> {
        let (url, headers) = Self::http_parts(connection)?;

        // Re-validate with DNS resolution and pin the connection to the
        // resolved IPs (TM-TOOL-018).
        let (validated_url, resolved_addrs) = validate_url_dns_pinned(url).await.map_err(|e| {
            tracing::warn!(mcp_server = %connection.name, url = %url, error = %e,
                "Blocked MCP request: URL failed SSRF validation");
            anyhow!("MCP server URL blocked: {}", e)
        })?;
        let pin_host = validated_url.host_str().unwrap_or("").to_string();

        let mut request = EgressRequest::new("POST", url, EgressRequestKind::Mcp)
            .pinned_addrs(pin_host, resolved_addrs)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
            .timeout_ms(MCP_TIMEOUT.as_millis() as u64)
            .body(body);

        for (name, value) in headers {
            request = request.header(name, value);
        }
        if let Some(credential) = credential {
            if let Some(auth) = &credential.authorization {
                request = request.header("Authorization", auth.clone());
            }
            for (name, value) in &credential.headers {
                request = request.header(name, value.clone());
            }
        }

        let response = self
            .egress
            .send(request)
            .await
            .map_err(|e| anyhow!("Failed to call MCP server: {}", e))?;

        if !(200..300).contains(&response.status) {
            let body = String::from_utf8_lossy(&response.body).into_owned();
            return Err(anyhow!(
                "MCP server returned error: {} - {}",
                response.status,
                body
            ));
        }

        String::from_utf8(response.body)
            .map_err(|e| anyhow!("Failed to read MCP response body: {}", e))
    }
}

#[async_trait]
impl McpTransport for HttpTransport {
    async fn list_tools(
        &self,
        connection: &McpConnection,
        credential: Option<&McpCredential>,
    ) -> Result<Vec<McpToolDefinition>> {
        let body = serde_json::to_vec(&McpToolsListRequest::default())?;
        let text = self.send_rpc(connection, body, credential).await?;
        let json_str = extract_json_from_response(&text)
            .ok_or_else(|| anyhow!("SSE response missing data line"))?;
        let response: McpToolsListResponse = serde_json::from_str(json_str)?;
        if let Some(error) = response.error {
            return Err(anyhow!(
                "MCP server error: {} ({})",
                error.message,
                error.code
            ));
        }
        Ok(response
            .result
            .ok_or_else(|| anyhow!("MCP server returned empty result"))?
            .tools)
    }

    async fn call_tool(
        &self,
        connection: &McpConnection,
        tool_name: &str,
        arguments: Value,
        credential: Option<&McpCredential>,
    ) -> Result<McpToolCallResult> {
        let request = McpToolCallRequest::new(1, tool_name.to_string(), Some(arguments));
        let body = serde_json::to_vec(&request)?;
        let text = self.send_rpc(connection, body, credential).await?;
        let json_str = extract_json_from_response(&text)
            .ok_or_else(|| anyhow!("SSE response missing data line"))?;
        let response: McpToolCallResponse = serde_json::from_str(json_str)?;
        if let Some(error) = response.error {
            return Err(anyhow!(
                "MCP tool error: {} (code: {})",
                error.message,
                error.code
            ));
        }
        response
            .result
            .ok_or_else(|| anyhow!("MCP server returned empty result"))
    }
}

// MCP Tool Executor
//
// Handles execution of MCP tools by calling remote MCP servers.
// MCP tools are identified by their prefixed name: "mcp_{server_name}_{tool_name}"

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use everruns_core::traits::{ToolContext, ToolExecutor};
use everruns_core::{
    McpContent, McpToolCallRequest, McpToolCallResponse, ToolCall, ToolDefinition, ToolResult,
    is_mcp_tool, parse_mcp_tool_name,
};
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use crate::grpc_adapters::GrpcClient;

/// HTTP client timeout for MCP server calls
const MCP_TOOL_TIMEOUT: Duration = Duration::from_secs(60);

/// MCP server info needed for tool execution
#[derive(Debug, Clone)]
pub struct McpServerInfo {
    pub id: uuid::Uuid,
    pub name: String,
    pub url: String,
    pub api_key: Option<String>,
    pub headers: HashMap<String, String>,
}

/// MCP Tool Executor - executes tools by calling remote MCP servers
pub struct McpToolExecutor {
    grpc_client: GrpcClient,
    org_id: i64,
    /// Cache of MCP server info by server name prefix
    server_cache: tokio::sync::RwLock<HashMap<String, McpServerInfo>>,
}

impl McpToolExecutor {
    pub fn new(grpc_client: GrpcClient, org_id: i64) -> Self {
        Self {
            grpc_client,
            org_id,
            server_cache: tokio::sync::RwLock::new(HashMap::new()),
        }
    }

    /// Execute an MCP tool by calling the remote server
    pub async fn execute_mcp_tool(&self, tool_call: &ToolCall) -> Result<ToolResult> {
        // Parse tool name to get server prefix and original tool name
        let (server_prefix, original_tool_name) = parse_mcp_tool_name(&tool_call.name)
            .ok_or_else(|| anyhow!("Invalid MCP tool name: {}", tool_call.name))?;

        // Get MCP server info (from cache or gRPC)
        let server_info = self.get_server_info(&server_prefix).await?;

        // Call the MCP server
        let result = call_mcp_tool(
            &server_info,
            &original_tool_name,
            tool_call.arguments.clone(),
        )
        .await?;

        Ok(ToolResult {
            tool_call_id: tool_call.id.clone(),
            result: Some(result),
            error: None,
        })
    }

    /// Get MCP server info by name prefix, caching for efficiency
    async fn get_server_info(&self, server_prefix: &str) -> Result<McpServerInfo> {
        // Check cache first
        {
            let cache = self.server_cache.read().await;
            if let Some(info) = cache.get(server_prefix) {
                return Ok(info.clone());
            }
        }

        // Fetch from gRPC
        let info = self
            .grpc_client
            .get_mcp_server_by_prefix(self.org_id, server_prefix)
            .await?;

        // Cache for future use
        {
            let mut cache = self.server_cache.write().await;
            cache.insert(server_prefix.to_string(), info.clone());
        }

        Ok(info)
    }

    /// Check if a tool is an MCP tool
    pub fn is_mcp_tool(tool_name: &str) -> bool {
        is_mcp_tool(tool_name)
    }
}

/// Call an MCP server's tools/call endpoint
async fn call_mcp_tool(
    server_info: &McpServerInfo,
    tool_name: &str,
    arguments: serde_json::Value,
) -> Result<serde_json::Value> {
    let client = reqwest::Client::builder()
        .timeout(MCP_TOOL_TIMEOUT)
        .build()?;

    // Create JSON-RPC request
    let request = McpToolCallRequest::new(
        1, // Request ID
        tool_name.to_string(),
        Some(arguments),
    );

    let mut req_builder = client.post(&server_info.url).json(&request);

    // Add API key if provided
    if let Some(ref api_key) = server_info.api_key {
        req_builder = req_builder.header("Authorization", format!("Bearer {}", api_key));
    }

    // Add custom headers
    for (name, value) in &server_info.headers {
        req_builder = req_builder.header(name, value);
    }

    tracing::debug!(
        mcp_server = %server_info.name,
        tool_name = %tool_name,
        "Calling MCP server"
    );

    let response = req_builder.send().await.map_err(|e| {
        tracing::error!(
            mcp_server = %server_info.name,
            tool_name = %tool_name,
            error = %e,
            "Failed to call MCP server"
        );
        anyhow!("Failed to call MCP server: {}", e)
    })?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        tracing::error!(
            mcp_server = %server_info.name,
            tool_name = %tool_name,
            status = %status,
            body = %body,
            "MCP server returned error"
        );
        return Err(anyhow!("MCP server returned error: {} - {}", status, body));
    }

    // Parse response - handle both plain JSON and SSE (Server-Sent Events) formats
    let response_text = response.text().await.map_err(|e| {
        tracing::error!(
            mcp_server = %server_info.name,
            tool_name = %tool_name,
            error = %e,
            "Failed to read MCP response body"
        );
        anyhow!("Failed to read MCP response body: {}", e)
    })?;

    let json_str = extract_json_from_response(&response_text)
        .ok_or_else(|| anyhow!("SSE response missing data line"))?;

    let mcp_response: McpToolCallResponse = serde_json::from_str(json_str).map_err(|e| {
        tracing::error!(
            mcp_server = %server_info.name,
            tool_name = %tool_name,
            error = %e,
            response_preview = %&json_str[..json_str.len().min(200)],
            "Failed to parse MCP response"
        );
        anyhow!("Failed to parse MCP response: {}", e)
    })?;

    // Check for MCP-level errors
    if let Some(error) = mcp_response.error {
        return Err(anyhow!(
            "MCP tool error: {} (code: {})",
            error.message,
            error.code
        ));
    }

    // Extract result
    let result = mcp_response
        .result
        .ok_or_else(|| anyhow!("MCP server returned empty result"))?;

    // Check if tool returned an error
    if result.is_error {
        let error_text = result
            .content
            .iter()
            .filter_map(|c| match c {
                McpContent::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        return Ok(json!({ "error": error_text }));
    }

    // Convert MCP content to JSON
    let content_json: Vec<serde_json::Value> = result
        .content
        .iter()
        .map(|c| match c {
            McpContent::Text { text } => json!({ "type": "text", "text": text }),
            McpContent::Image { data, mime_type } => json!({
                "type": "image",
                "data": data,
                "mime_type": mime_type
            }),
            McpContent::Resource {
                uri,
                mime_type,
                text,
            } => json!({
                "type": "resource",
                "uri": uri,
                "mime_type": mime_type,
                "text": text
            }),
        })
        .collect();

    // Return simplified result for single text content
    if content_json.len() == 1 && content_json[0].get("text").is_some() {
        let text = &content_json[0]["text"];
        return Ok(json!({ "result": text }));
    }

    Ok(json!({ "content": content_json }))
}

/// Composite Tool Executor that handles both built-in tools and MCP tools
pub struct CompositeToolExecutor {
    builtin: everruns_core::ToolRegistry,
    mcp: Arc<McpToolExecutor>,
}

impl CompositeToolExecutor {
    pub fn new(builtin: everruns_core::ToolRegistry, mcp: Arc<McpToolExecutor>) -> Self {
        Self { builtin, mcp }
    }
}

#[async_trait]
impl ToolExecutor for CompositeToolExecutor {
    async fn execute(
        &self,
        tool_call: &ToolCall,
        tool_def: &ToolDefinition,
    ) -> everruns_core::Result<ToolResult> {
        if McpToolExecutor::is_mcp_tool(&tool_call.name) {
            // Execute MCP tool
            self.mcp.execute_mcp_tool(tool_call).await.map_err(|e| {
                tracing::error!(error = %e, "MCP tool execution failed");
                everruns_core::AgentLoopError::tool(e.to_string())
            })
        } else {
            // Execute built-in tool
            self.builtin.execute(tool_call, tool_def).await
        }
    }

    async fn execute_with_context(
        &self,
        tool_call: &ToolCall,
        tool_def: &ToolDefinition,
        context: &ToolContext,
    ) -> everruns_core::Result<ToolResult> {
        if McpToolExecutor::is_mcp_tool(&tool_call.name) {
            // MCP tools don't use context - execute directly
            self.mcp.execute_mcp_tool(tool_call).await.map_err(|e| {
                tracing::error!(error = %e, "MCP tool execution failed");
                everruns_core::AgentLoopError::tool(e.to_string())
            })
        } else {
            // Execute built-in tool with context
            self.builtin
                .execute_with_context(tool_call, tool_def, context)
                .await
        }
    }
}

/// Extract JSON from MCP response, handling both plain JSON and SSE formats.
/// SSE format: "event: message\ndata: {...json...}\n"
fn extract_json_from_response(response_text: &str) -> Option<&str> {
    if response_text.starts_with("event:") || response_text.contains("\ndata:") {
        // Parse SSE format - extract JSON from "data: {...}" line
        response_text
            .lines()
            .find(|line| line.starts_with("data:"))
            .map(|line| line.trim_start_matches("data:").trim())
    } else {
        // Plain JSON response
        Some(response_text.trim())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_json_from_plain_json() {
        let response = r#"{"result": {"content": [{"type": "text", "text": "Hello"}]}, "id": 1, "jsonrpc": "2.0"}"#;
        let result = extract_json_from_response(response);
        assert_eq!(result, Some(response.trim()));
    }

    #[test]
    fn test_extract_json_from_sse_format() {
        let response = "event: message\ndata: {\"result\": {\"content\": []}, \"id\": 1, \"jsonrpc\": \"2.0\"}\n";
        let result = extract_json_from_response(response);
        assert_eq!(
            result,
            Some("{\"result\": {\"content\": []}, \"id\": 1, \"jsonrpc\": \"2.0\"}")
        );
    }

    #[test]
    fn test_extract_json_from_sse_with_multiple_events() {
        let response = "event: open\ndata: \n\nevent: message\ndata: {\"result\": {}, \"id\": 1}\n";
        let result = extract_json_from_response(response);
        // Should find first data line (empty one)
        assert!(result.is_some());
    }

    #[test]
    fn test_extract_json_from_sse_real_microsoft_learn() {
        // Simulated Microsoft Learn MCP response format
        let response = r#"event: message
data: {"result":{"tools":[{"name":"search","description":"Search docs"}]},"id":1,"jsonrpc":"2.0"}"#;
        let result = extract_json_from_response(response);
        assert_eq!(
            result,
            Some(
                r#"{"result":{"tools":[{"name":"search","description":"Search docs"}]},"id":1,"jsonrpc":"2.0"}"#
            )
        );
    }

    #[test]
    fn test_extract_json_from_plain_json_with_whitespace() {
        let response = "  \n{\"result\": {}}\n  ";
        let result = extract_json_from_response(response);
        assert_eq!(result, Some("{\"result\": {}}"));
    }

    #[test]
    fn test_extract_json_detects_sse_with_newline_data() {
        // Edge case: newline before data: should still detect SSE format
        let response = "some text\ndata: {\"key\": \"value\"}";
        let result = extract_json_from_response(response);
        assert_eq!(result, Some("{\"key\": \"value\"}"));
    }
}

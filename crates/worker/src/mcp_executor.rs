// MCP Tool Executor
//
// Handles execution of MCP tools by calling remote MCP servers.
// MCP tools are identified by their prefixed name: "mcp_{server_name}_{tool_name}"

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use everruns_core::traits::{ToolContext, ToolExecutor};
use everruns_core::{
    McpContent, McpToolCallRequest, McpToolCallResponse, ToolCall, ToolDefinition, ToolResult,
    ToolResultImage, is_mcp_tool, parse_mcp_tool_name,
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
    /// Authentication type: "none", "api_key", "oauth"
    pub auth_type: String,
    /// OAuth access token (only set if auth_type="oauth" and user has authorized)
    pub oauth_access_token: Option<String>,
    /// True if OAuth auth is required but user hasn't authorized yet
    pub oauth_required: bool,
}

/// MCP Tool Executor - executes tools by calling remote MCP servers
pub struct McpToolExecutor {
    grpc_client: GrpcClient,
    org_id: i64,
    /// User ID for per-user OAuth token lookup
    user_id: Option<uuid::Uuid>,
    /// Cache of MCP server info by server name prefix
    /// Note: Not used for OAuth servers since tokens may change
    server_cache: tokio::sync::RwLock<HashMap<String, McpServerInfo>>,
}

impl McpToolExecutor {
    pub fn new(grpc_client: GrpcClient, org_id: i64, user_id: Option<uuid::Uuid>) -> Self {
        Self {
            grpc_client,
            org_id,
            user_id,
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

        // Check if OAuth authorization is required but missing
        if server_info.oauth_required {
            return Err(anyhow!(
                "MCP server '{}' requires OAuth authorization. Please authorize access in the UI.",
                server_info.name
            ));
        }

        // Call the MCP server
        let (result, images) = call_mcp_tool(
            &server_info,
            &original_tool_name,
            tool_call.arguments.clone(),
        )
        .await?;

        Ok(ToolResult {
            tool_call_id: tool_call.id.clone(),
            result: Some(result),
            images: if images.is_empty() {
                None
            } else {
                Some(images)
            },
            error: None,
        })
    }

    /// Get MCP server info by name prefix, caching for efficiency
    /// Note: OAuth servers are not cached since tokens may change
    async fn get_server_info(&self, server_prefix: &str) -> Result<McpServerInfo> {
        // Check cache first (only for non-OAuth servers)
        {
            let cache = self.server_cache.read().await;
            if let Some(info) = cache.get(server_prefix) {
                // Don't use cache for OAuth servers since tokens may change
                if info.auth_type != "oauth" {
                    return Ok(info.clone());
                }
            }
        }

        // Fetch from gRPC (includes user-specific OAuth token)
        let info = self
            .grpc_client
            .get_mcp_server_by_prefix(self.org_id, server_prefix, self.user_id)
            .await?;

        // Cache for future use (only non-OAuth servers)
        if info.auth_type != "oauth" {
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

/// Call an MCP server's tools/call endpoint.
/// Returns (json_result, images) where images are extracted from MCP image content.
async fn call_mcp_tool(
    server_info: &McpServerInfo,
    tool_name: &str,
    arguments: serde_json::Value,
) -> Result<(serde_json::Value, Vec<ToolResultImage>)> {
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

    // Add authentication based on auth_type
    match server_info.auth_type.as_str() {
        "oauth" => {
            // Use OAuth access token
            if let Some(ref token) = server_info.oauth_access_token {
                req_builder = req_builder.header("Authorization", format!("Bearer {}", token));
            }
        }
        "api_key" => {
            // Use API key
            if let Some(ref api_key) = server_info.api_key {
                req_builder = req_builder.header("Authorization", format!("Bearer {}", api_key));
            }
        }
        _ => {
            // "none" - no authentication needed
        }
    }

    // Add custom headers
    for (name, value) in &server_info.headers {
        req_builder = req_builder.header(name, value);
    }

    tracing::debug!(
        mcp_server = %server_info.name,
        tool_name = %tool_name,
        auth_type = %server_info.auth_type,
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
        return Ok((json!({ "error": error_text }), vec![]));
    }

    // Separate images from other content types.
    // Images are returned as ToolResultImage for native LLM image support.
    let mut images = Vec::new();
    let mut text_parts = Vec::new();
    let mut resource_parts = Vec::new();

    for c in &result.content {
        match c {
            McpContent::Text { text } => {
                text_parts.push(text.clone());
            }
            McpContent::Image { data, mime_type } => {
                images.push(ToolResultImage {
                    base64: data.clone(),
                    media_type: mime_type.clone(),
                });
            }
            McpContent::Resource {
                uri,
                mime_type,
                text,
            } => {
                resource_parts.push(json!({
                    "type": "resource",
                    "uri": uri,
                    "mime_type": mime_type,
                    "text": text
                }));
            }
        }
    }

    // Build JSON result from text and resource content (images are separate)
    let json_result = if resource_parts.is_empty() {
        if text_parts.len() == 1 {
            json!({ "result": text_parts[0] })
        } else if text_parts.is_empty() && !images.is_empty() {
            // Image-only response
            json!({ "result": format!("[{} image(s)]", images.len()) })
        } else {
            json!({ "result": text_parts.join("\n") })
        }
    } else {
        // Mix of text and resources
        let mut content = Vec::new();
        for t in &text_parts {
            content.push(json!({ "type": "text", "text": t }));
        }
        content.extend(resource_parts);
        json!({ "content": content })
    };

    Ok((json_result, images))
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

    // MCP image content extraction tests

    #[test]
    fn test_mcp_text_only_result() {
        let content = vec![McpContent::Text {
            text: "Hello world".to_string(),
        }];
        let result = everruns_core::McpToolCallResult {
            content,
            is_error: false,
        };

        // Simulate what call_mcp_tool does after getting the result
        let mut images = Vec::new();
        let mut text_parts = Vec::new();
        for c in &result.content {
            match c {
                McpContent::Text { text } => text_parts.push(text.clone()),
                McpContent::Image { data, mime_type } => images.push(ToolResultImage {
                    base64: data.clone(),
                    media_type: mime_type.clone(),
                }),
                _ => {}
            }
        }
        assert!(images.is_empty());
        assert_eq!(text_parts, vec!["Hello world"]);
    }

    #[test]
    fn test_mcp_image_extraction() {
        let content = vec![
            McpContent::Text {
                text: "Here's the chart".to_string(),
            },
            McpContent::Image {
                data: "iVBORw0KGgo=".to_string(),
                mime_type: "image/png".to_string(),
            },
        ];

        let mut images = Vec::new();
        let mut text_parts = Vec::new();
        for c in &content {
            match c {
                McpContent::Text { text } => text_parts.push(text.clone()),
                McpContent::Image { data, mime_type } => images.push(ToolResultImage {
                    base64: data.clone(),
                    media_type: mime_type.clone(),
                }),
                _ => {}
            }
        }

        assert_eq!(images.len(), 1);
        assert_eq!(images[0].media_type, "image/png");
        assert_eq!(images[0].base64, "iVBORw0KGgo=");
        assert_eq!(text_parts, vec!["Here's the chart"]);
    }

    #[test]
    fn test_mcp_multiple_images() {
        let content = vec![
            McpContent::Image {
                data: "AAA=".to_string(),
                mime_type: "image/png".to_string(),
            },
            McpContent::Image {
                data: "BBB=".to_string(),
                mime_type: "image/jpeg".to_string(),
            },
        ];

        let mut images = Vec::new();
        for c in &content {
            if let McpContent::Image { data, mime_type } = c {
                images.push(ToolResultImage {
                    base64: data.clone(),
                    media_type: mime_type.clone(),
                });
            }
        }

        assert_eq!(images.len(), 2);
        assert_eq!(images[0].base64, "AAA=");
        assert_eq!(images[1].media_type, "image/jpeg");
    }
}

//! MCP tool execution.
//!
//! [`McpExecutor`] resolves a tool call's server prefix to a [`McpConnection`]
//! and executes it via [`McpClient`]. It implements [`McpToolInvoker`] so hosts
//! register MCP tools as first-class [`everruns_core::tools::Tool`] entries in
//! the regular `ToolRegistry` (via `everruns_core::build_mcp_proxy_tools`),
//! instead of routing `mcp_*` calls through a separate executor
//! (knowledge/integrations/runtime-mcp.md D5).

use crate::client::McpClient;
use crate::transport::McpConnection;
use anyhow::{Result, anyhow};
use async_trait::async_trait;
use everruns_core::mcp_server::sanitize_mcp_server_name;
use everruns_core::{AgentLoopError, McpToolInvoker, ToolCall, ToolResult, parse_mcp_tool_name};
use std::collections::HashMap;
use std::sync::Arc;

const REDACTED_CREDENTIAL: &str = "[REDACTED]";

fn redact_text(text: &mut String, secrets: &[String]) {
    for secret in secrets {
        if !secret.is_empty() {
            *text = text.replace(secret, REDACTED_CREDENTIAL);
        }
    }
}

fn redact_json(value: &mut serde_json::Value, secrets: &[String]) {
    match value {
        serde_json::Value::String(text) => redact_text(text, secrets),
        serde_json::Value::Array(values) => {
            for value in values {
                redact_json(value, secrets);
            }
        }
        serde_json::Value::Object(object) => {
            for value in object.values_mut() {
                redact_json(value, secrets);
            }
        }
        _ => {}
    }
}

fn redact_tool_result(result: &mut ToolResult, secrets: &[String]) {
    if let Some(value) = &mut result.result {
        redact_json(value, secrets);
    }
    if let Some(images) = &mut result.images {
        for image in images {
            redact_text(&mut image.base64, secrets);
            redact_text(&mut image.media_type, secrets);
        }
    }
    for text in [
        &mut result.error,
        &mut result.connection_required,
        &mut result.raw_output,
    ]
    .into_iter()
    .flatten()
    {
        redact_text(text, secrets);
    }
}

/// Resolves a sanitized server prefix to a connection. Implementations differ
/// per host (runtime: effective scoped servers; worker: gRPC lookup).
#[async_trait]
pub trait McpConnectionResolver: Send + Sync {
    async fn resolve(&self, server_prefix: &str) -> Result<Option<McpConnection>>;
}

/// In-memory resolver over a fixed set of connections, keyed by sanitized
/// server name. Used by runtime/CLI hosts that hold the effective scoped
/// servers up front.
#[derive(Default)]
pub struct StaticConnectionResolver {
    connections: HashMap<String, McpConnection>,
}

impl StaticConnectionResolver {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, connection: McpConnection) {
        let key = sanitize_mcp_server_name(&connection.name);
        self.connections.insert(key, connection);
    }

    pub fn with(mut self, connection: McpConnection) -> Self {
        self.insert(connection);
        self
    }

    pub fn from_connections(connections: impl IntoIterator<Item = McpConnection>) -> Self {
        let mut resolver = Self::new();
        for connection in connections {
            resolver.insert(connection);
        }
        resolver
    }

    pub fn is_empty(&self) -> bool {
        self.connections.is_empty()
    }
}

#[async_trait]
impl McpConnectionResolver for StaticConnectionResolver {
    async fn resolve(&self, server_prefix: &str) -> Result<Option<McpConnection>> {
        Ok(self.connections.get(server_prefix).cloned())
    }
}

/// Executes `mcp_*` tool calls against remote/local MCP servers.
pub struct McpExecutor {
    client: Arc<McpClient>,
    resolver: Arc<dyn McpConnectionResolver>,
}

impl McpExecutor {
    pub fn new(client: Arc<McpClient>, resolver: Arc<dyn McpConnectionResolver>) -> Self {
        Self { client, resolver }
    }

    pub async fn execute_mcp_tool(&self, tool_call: &ToolCall) -> Result<ToolResult> {
        let (server_prefix, original_tool_name) = parse_mcp_tool_name(&tool_call.name)
            .ok_or_else(|| anyhow!("Invalid MCP tool name: {}", tool_call.name))?;

        let connection = self
            .resolver
            .resolve(&server_prefix)
            .await?
            .ok_or_else(|| anyhow!("MCP server not found for prefix: {server_prefix}"))?;

        // The server requires an OAuth connection the user has not made yet.
        // Return a connection_required result (the host renders an inline
        // connect prompt) instead of letting the call fail with a 401.
        if let Some(provider) = &connection.pending_oauth_provider {
            return Ok(ToolResult {
                tool_call_id: tool_call.id.clone(),
                result: None,
                images: None,
                error: Some(format!(
                    "MCP server '{}' requires an OAuth connection. \
                     Ask the user to connect provider '{provider}'.",
                    connection.name
                )),
                connection_required: Some(provider.clone()),
                raw_output: None,
            });
        }

        let mut arguments = tool_call.arguments.clone();
        let mut injected_secrets = Vec::new();
        if let Some(bindings) = connection.secret_bindings.get(&original_tool_name) {
            let object = arguments
                .as_object_mut()
                .ok_or_else(|| anyhow!("MCP tool arguments must be an object"))?;
            for binding in bindings {
                if object.contains_key(&binding.parameter_name) {
                    return Ok(ToolResult {
                        tool_call_id: tool_call.id.clone(),
                        result: Some(serde_json::json!({
                            "code": "credential_override_rejected",
                            "error": format!(
                                "Credential parameter '{}' is securely bound and cannot be supplied by the model",
                                binding.parameter_name
                            ),
                        })),
                        images: None,
                        error: Some("Secure credential override rejected".to_string()),
                        connection_required: None,
                        raw_output: None,
                    });
                }
                let Some(value) = binding.value.as_ref() else {
                    return Ok(ToolResult {
                        tool_call_id: tool_call.id.clone(),
                        result: Some(serde_json::json!({
                            "code": "credential_required",
                            "error": format!("{} is not configured", binding.label),
                            "setup_url": binding.setup_url,
                            "credential_label": binding.label,
                        })),
                        images: None,
                        // Keep the structured setup payload intact through the
                        // MCP proxy so clients can render a direct affordance.
                        // This is an expected, user-actionable state rather
                        // than a transport failure.
                        error: None,
                        connection_required: None,
                        raw_output: None,
                    });
                };
                object.insert(
                    binding.parameter_name.clone(),
                    serde_json::Value::String(value.clone()),
                );
                injected_secrets.push(value.clone());
            }
        }

        // THREAT[TM-TOOL-029]: The remote endpoint is untrusted and may reflect
        // injected credentials in either successful content or transport errors.
        let result = self
            .client
            .call_as_tool_result(
                &connection,
                tool_call.id.clone(),
                &original_tool_name,
                arguments,
            )
            .await;
        match result {
            Ok(mut result) => {
                redact_tool_result(&mut result, &injected_secrets);
                Ok(result)
            }
            Err(error) => {
                let mut message = error.to_string();
                redact_text(&mut message, &injected_secrets);
                Err(anyhow!(message))
            }
        }
    }
}

#[async_trait]
impl McpToolInvoker for McpExecutor {
    async fn invoke(&self, tool_call: &ToolCall) -> everruns_core::Result<ToolResult> {
        self.execute_mcp_tool(tool_call).await.map_err(|e| {
            tracing::error!(error = %e, "MCP tool execution failed");
            AgentLoopError::tool(e.to_string())
        })
    }
}

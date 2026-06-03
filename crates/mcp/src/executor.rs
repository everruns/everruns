//! MCP tool execution and routing.
//!
//! [`McpExecutor`] resolves a tool call's server prefix to a [`McpConnection`]
//! and executes it via [`McpClient`]. [`CompositeToolExecutor`] routes
//! `mcp_*` tool calls to it and everything else to a builtin executor, so a
//! host's [`ToolExecutor`] transparently gains MCP support. This is the shared
//! replacement for `worker/src/mcp_executor.rs` (specs/runtime-mcp.md D5).

use crate::client::McpClient;
use crate::transport::McpConnection;
use anyhow::{Result, anyhow};
use async_trait::async_trait;
use everruns_core::mcp_server::sanitize_mcp_server_name;
use everruns_core::traits::{ToolContext, ToolExecutor};
use everruns_core::{
    AgentLoopError, ToolCall, ToolDefinition, ToolResult, is_mcp_tool, parse_mcp_tool_name,
};
use std::collections::HashMap;
use std::sync::Arc;

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

    pub fn is_mcp_tool(tool_name: &str) -> bool {
        is_mcp_tool(tool_name)
    }

    pub async fn execute_mcp_tool(&self, tool_call: &ToolCall) -> Result<ToolResult> {
        let (server_prefix, original_tool_name) = parse_mcp_tool_name(&tool_call.name)
            .ok_or_else(|| anyhow!("Invalid MCP tool name: {}", tool_call.name))?;

        let connection = self
            .resolver
            .resolve(&server_prefix)
            .await?
            .ok_or_else(|| anyhow!("MCP server not found for prefix: {server_prefix}"))?;

        self.client
            .call_as_tool_result(
                &connection,
                tool_call.id.clone(),
                &original_tool_name,
                tool_call.arguments.clone(),
            )
            .await
    }
}

/// Routes tool execution between a builtin executor and the MCP executor.
pub struct CompositeToolExecutor<B: ToolExecutor> {
    builtin: B,
    mcp: Arc<McpExecutor>,
}

impl<B: ToolExecutor> CompositeToolExecutor<B> {
    pub fn new(builtin: B, mcp: Arc<McpExecutor>) -> Self {
        Self { builtin, mcp }
    }
}

#[async_trait]
impl<B: ToolExecutor> ToolExecutor for CompositeToolExecutor<B> {
    async fn execute(
        &self,
        tool_call: &ToolCall,
        tool_def: &ToolDefinition,
    ) -> everruns_core::Result<ToolResult> {
        if McpExecutor::is_mcp_tool(&tool_call.name) {
            self.mcp.execute_mcp_tool(tool_call).await.map_err(|e| {
                tracing::error!(error = %e, "MCP tool execution failed");
                AgentLoopError::tool(e.to_string())
            })
        } else {
            self.builtin.execute(tool_call, tool_def).await
        }
    }

    async fn execute_with_context(
        &self,
        tool_call: &ToolCall,
        tool_def: &ToolDefinition,
        context: &ToolContext,
    ) -> everruns_core::Result<ToolResult> {
        if McpExecutor::is_mcp_tool(&tool_call.name) {
            self.mcp.execute_mcp_tool(tool_call).await.map_err(|e| {
                tracing::error!(error = %e, "MCP tool execution failed");
                AgentLoopError::tool(e.to_string())
            })
        } else {
            self.builtin
                .execute_with_context(tool_call, tool_def, context)
                .await
        }
    }
}

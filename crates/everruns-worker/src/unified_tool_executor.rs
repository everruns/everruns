// Unified Tool Executor
//
// This module provides a unified tool execution implementation that combines:
// - ToolRegistry from agent-loop for built-in tools
// - Webhook execution for external HTTP tools
//
// Both in-process and Temporal modes use this executor to ensure consistent
// tool execution behavior.

use async_trait::async_trait;
use everruns_agent_loop::{traits::ToolExecutor, Result, Tool, ToolExecutionResult, ToolRegistry};
use everruns_contracts::tools::{ToolCall, ToolDefinition, ToolResult};
use reqwest::Client;
use std::sync::Arc;
use tracing::{error, info};

use crate::tools::{execute_tool as execute_webhook_tool, requires_approval};

/// A unified tool executor that handles both built-in and webhook tools.
///
/// This executor:
/// - Uses a `ToolRegistry` for built-in tools registered at creation time
/// - Falls back to webhook execution for `ToolDefinition::Webhook` tools
/// - For `ToolDefinition::Builtin` tools, looks up in the registry by name
///
/// # Example
///
/// ```ignore
/// use everruns_agent_loop::{ToolRegistry, GetCurrentTime, EchoTool};
/// use everruns_worker::unified_tool_executor::UnifiedToolExecutor;
///
/// // Create registry with built-in tools
/// let registry = ToolRegistry::builder()
///     .tool(GetCurrentTime)
///     .tool(EchoTool)
///     .build();
///
/// // Create unified executor
/// let executor = UnifiedToolExecutor::new(registry);
///
/// // Use with AgentLoop
/// let agent_loop = AgentLoop::new(config, emitter, store, llm, executor);
/// ```
pub struct UnifiedToolExecutor {
    /// Registry of built-in tools
    registry: Arc<ToolRegistry>,
    /// HTTP client for webhook calls
    client: Client,
}

impl UnifiedToolExecutor {
    /// Create a new unified tool executor with the given tool registry.
    pub fn new(registry: ToolRegistry) -> Self {
        Self {
            registry: Arc::new(registry),
            client: Client::new(),
        }
    }

    /// Create a new unified tool executor with default built-in tools.
    ///
    /// This includes:
    /// - `get_current_time`: Returns the current date and time
    /// - `echo`: Echoes back the provided message
    pub fn with_default_tools() -> Self {
        let registry = ToolRegistry::builder()
            .tool(everruns_agent_loop::GetCurrentTime)
            .tool(everruns_agent_loop::EchoTool)
            .build();

        Self::new(registry)
    }

    /// Create a new unified tool executor with an empty registry.
    ///
    /// Use this when you only need webhook tool execution.
    pub fn webhook_only() -> Self {
        Self::new(ToolRegistry::new())
    }

    /// Get reference to the tool registry.
    pub fn registry(&self) -> &ToolRegistry {
        &self.registry
    }

    /// Add a tool to the registry.
    ///
    /// Note: This creates a new Arc, so it's more efficient to build
    /// the registry upfront and pass it to `new()`.
    pub fn with_tool(mut self, tool: impl Tool + 'static) -> Self {
        let registry = Arc::make_mut(&mut self.registry);
        registry.register(tool);
        self
    }

    /// Check if a tool requires approval before execution.
    pub fn tool_requires_approval(&self, tool_def: &ToolDefinition) -> bool {
        requires_approval(tool_def)
    }

    /// Execute a built-in tool from the registry.
    async fn execute_builtin(
        &self,
        tool_call: &ToolCall,
        _builtin: &everruns_contracts::tools::BuiltinTool,
    ) -> Result<ToolResult> {
        // Look up the tool in the registry
        if let Some(tool) = self.registry.get(&tool_call.name) {
            info!(
                tool_name = %tool_call.name,
                tool_call_id = %tool_call.id,
                "Executing built-in tool from registry"
            );

            let result = tool.execute(tool_call.arguments.clone()).await;
            Ok(result.into_tool_result(&tool_call.id, &tool_call.name))
        } else {
            // Tool not found in registry
            error!(
                tool_name = %tool_call.name,
                tool_call_id = %tool_call.id,
                "Built-in tool not found in registry"
            );

            Ok(ToolResult {
                tool_call_id: tool_call.id.clone(),
                result: None,
                error: Some(format!(
                    "Built-in tool '{}' not found in registry",
                    tool_call.name
                )),
            })
        }
    }

    /// Execute a webhook tool.
    async fn execute_webhook(
        &self,
        tool_call: &ToolCall,
        tool_def: &ToolDefinition,
    ) -> Result<ToolResult> {
        info!(
            tool_name = %tool_call.name,
            tool_call_id = %tool_call.id,
            "Executing webhook tool"
        );

        let result = execute_webhook_tool(tool_call, tool_def, &self.client).await;

        Ok(ToolResult {
            tool_call_id: result.tool_call_id,
            result: result.result,
            error: result.error,
        })
    }
}

impl Default for UnifiedToolExecutor {
    fn default() -> Self {
        Self::with_default_tools()
    }
}

#[async_trait]
impl ToolExecutor for UnifiedToolExecutor {
    async fn execute(&self, tool_call: &ToolCall, tool_def: &ToolDefinition) -> Result<ToolResult> {
        match tool_def {
            ToolDefinition::Builtin(builtin) => self.execute_builtin(tool_call, builtin).await,
            ToolDefinition::Webhook(_) => self.execute_webhook(tool_call, tool_def).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_agent_loop::{EchoTool, FailingTool, GetCurrentTime};
    use everruns_contracts::tools::{BuiltinTool, BuiltinToolKind, ToolPolicy};
    use serde_json::json;

    #[tokio::test]
    async fn test_execute_builtin_tool_from_registry() {
        let registry = ToolRegistry::builder().tool(GetCurrentTime).build();
        let executor = UnifiedToolExecutor::new(registry);

        let tool_call = ToolCall {
            id: "call_1".to_string(),
            name: "get_current_time".to_string(),
            arguments: json!({"format": "iso8601"}),
        };

        let tool_def = ToolDefinition::Builtin(BuiltinTool {
            name: "get_current_time".to_string(),
            description: "Get current time".to_string(),
            parameters: json!({}),
            kind: BuiltinToolKind::HttpGet,
            policy: ToolPolicy::Auto,
        });

        let result = executor.execute(&tool_call, &tool_def).await.unwrap();

        assert!(result.error.is_none());
        assert!(result.result.is_some());
        let value = result.result.unwrap();
        assert!(value.get("datetime").is_some());
    }

    #[tokio::test]
    async fn test_execute_echo_tool() {
        let registry = ToolRegistry::builder().tool(EchoTool).build();
        let executor = UnifiedToolExecutor::new(registry);

        let tool_call = ToolCall {
            id: "call_2".to_string(),
            name: "echo".to_string(),
            arguments: json!({"message": "Hello, World!"}),
        };

        let tool_def = ToolDefinition::Builtin(BuiltinTool {
            name: "echo".to_string(),
            description: "Echo message".to_string(),
            parameters: json!({}),
            kind: BuiltinToolKind::HttpGet,
            policy: ToolPolicy::Auto,
        });

        let result = executor.execute(&tool_call, &tool_def).await.unwrap();

        assert!(result.error.is_none());
        let value = result.result.unwrap();
        assert_eq!(value["echoed"], "Hello, World!");
        assert_eq!(value["length"], 13);
    }

    #[tokio::test]
    async fn test_builtin_tool_not_in_registry() {
        let registry = ToolRegistry::new(); // Empty registry
        let executor = UnifiedToolExecutor::new(registry);

        let tool_call = ToolCall {
            id: "call_3".to_string(),
            name: "nonexistent_tool".to_string(),
            arguments: json!({}),
        };

        let tool_def = ToolDefinition::Builtin(BuiltinTool {
            name: "nonexistent_tool".to_string(),
            description: "Does not exist".to_string(),
            parameters: json!({}),
            kind: BuiltinToolKind::HttpGet,
            policy: ToolPolicy::Auto,
        });

        let result = executor.execute(&tool_call, &tool_def).await.unwrap();

        assert!(result.error.is_some());
        assert!(result.error.unwrap().contains("not found in registry"));
    }

    #[tokio::test]
    async fn test_failing_tool_returns_error() {
        let registry = ToolRegistry::builder()
            .tool(FailingTool::with_tool_error("Expected failure"))
            .build();
        let executor = UnifiedToolExecutor::new(registry);

        let tool_call = ToolCall {
            id: "call_4".to_string(),
            name: "failing_tool".to_string(),
            arguments: json!({}),
        };

        let tool_def = ToolDefinition::Builtin(BuiltinTool {
            name: "failing_tool".to_string(),
            description: "Always fails".to_string(),
            parameters: json!({}),
            kind: BuiltinToolKind::HttpGet,
            policy: ToolPolicy::Auto,
        });

        let result = executor.execute(&tool_call, &tool_def).await.unwrap();

        assert!(result.error.is_some());
        assert_eq!(result.error.unwrap(), "Expected failure");
    }

    #[tokio::test]
    async fn test_with_default_tools() {
        let executor = UnifiedToolExecutor::with_default_tools();

        // Should have get_current_time
        assert!(executor.registry().has("get_current_time"));
        // Should have echo
        assert!(executor.registry().has("echo"));
    }

    #[tokio::test]
    async fn test_with_tool_builder() {
        let executor = UnifiedToolExecutor::webhook_only()
            .with_tool(GetCurrentTime)
            .with_tool(EchoTool);

        assert!(executor.registry().has("get_current_time"));
        assert!(executor.registry().has("echo"));
    }

    #[test]
    fn test_tool_requires_approval() {
        let executor = UnifiedToolExecutor::webhook_only();

        let auto_tool = ToolDefinition::Builtin(BuiltinTool {
            name: "auto".to_string(),
            description: "Auto tool".to_string(),
            parameters: json!({}),
            kind: BuiltinToolKind::HttpGet,
            policy: ToolPolicy::Auto,
        });

        let approval_tool = ToolDefinition::Builtin(BuiltinTool {
            name: "approval".to_string(),
            description: "Approval tool".to_string(),
            parameters: json!({}),
            kind: BuiltinToolKind::HttpGet,
            policy: ToolPolicy::RequiresApproval,
        });

        assert!(!executor.tool_requires_approval(&auto_tool));
        assert!(executor.tool_requires_approval(&approval_tool));
    }
}

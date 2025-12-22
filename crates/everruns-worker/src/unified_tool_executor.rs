// Unified Tool Executor
//
// This module provides tool execution using ToolRegistry from everruns-core.
// All tools are registered at creation time and executed via the registry.

use async_trait::async_trait;
use everruns_contracts::tools::{ToolCall, ToolDefinition, ToolResult};
use everruns_core::{traits::ToolExecutor, Result, ToolRegistry};
use std::sync::Arc;
use tracing::{error, info};

/// A tool executor that uses ToolRegistry for built-in tools.
///
/// # Example
///
/// ```ignore
/// use everruns_core::{ToolRegistry, GetCurrentTime, EchoTool};
/// use everruns_worker::unified_tool_executor::UnifiedToolExecutor;
///
/// // Create registry with built-in tools
/// let registry = ToolRegistry::builder()
///     .tool(GetCurrentTime)
///     .tool(EchoTool)
///     .build();
///
/// // Create executor
/// let executor = UnifiedToolExecutor::new(registry);
///
/// // Use with AgentLoop
/// let agent_loop = AgentLoop::new(config, emitter, store, llm, executor);
/// ```
pub struct UnifiedToolExecutor {
    /// Registry of built-in tools
    registry: Arc<ToolRegistry>,
}

impl UnifiedToolExecutor {
    /// Create a new tool executor with the given tool registry.
    pub fn new(registry: ToolRegistry) -> Self {
        Self {
            registry: Arc::new(registry),
        }
    }

    /// Create a new tool executor with default built-in tools.
    ///
    /// This includes:
    /// - `get_current_time`: Returns the current date and time
    /// - `echo`: Echoes back the provided message
    /// - TestMath tools: add, subtract, multiply, divide
    /// - TestWeather tools: get_weather, get_forecast
    pub fn with_default_tools() -> Self {
        let registry = ToolRegistry::builder()
            .tool(everruns_core::GetCurrentTime)
            .tool(everruns_core::EchoTool)
            // TestMath capability tools
            .tool(everruns_core::AddTool)
            .tool(everruns_core::SubtractTool)
            .tool(everruns_core::MultiplyTool)
            .tool(everruns_core::DivideTool)
            // TestWeather capability tools
            .tool(everruns_core::GetWeatherTool)
            .tool(everruns_core::GetForecastTool)
            .build();

        Self::new(registry)
    }

    /// Get reference to the tool registry.
    pub fn registry(&self) -> &ToolRegistry {
        &self.registry
    }
}

impl Default for UnifiedToolExecutor {
    fn default() -> Self {
        Self::with_default_tools()
    }
}

#[async_trait]
impl ToolExecutor for UnifiedToolExecutor {
    async fn execute(
        &self,
        tool_call: &ToolCall,
        _tool_def: &ToolDefinition,
    ) -> Result<ToolResult> {
        // Look up the tool in the registry by name
        if let Some(tool) = self.registry.get(&tool_call.name) {
            info!(
                tool_name = %tool_call.name,
                tool_call_id = %tool_call.id,
                "Executing tool from registry"
            );

            let result = tool.execute(tool_call.arguments.clone()).await;
            Ok(result.into_tool_result(&tool_call.id, &tool_call.name))
        } else {
            // Tool not found in registry
            error!(
                tool_name = %tool_call.name,
                tool_call_id = %tool_call.id,
                "Tool not found in registry"
            );

            Ok(ToolResult {
                tool_call_id: tool_call.id.clone(),
                result: None,
                error: Some(format!("Tool '{}' not found in registry", tool_call.name)),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_contracts::tools::{BuiltinTool, BuiltinToolKind, ToolPolicy};
    use everruns_core::{EchoTool, FailingTool, GetCurrentTime};
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
    async fn test_tool_not_in_registry() {
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
}

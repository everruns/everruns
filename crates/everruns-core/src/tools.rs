// Tool Abstraction for Agent Loop
//
// This module provides a high-level abstraction for tools that can be executed
// by the agent loop. Tools are defined using the `Tool` trait and can be
// registered with a `ToolRegistry` for use in the loop.
//
// Design decisions:
// - Tools are defined via a trait for flexibility (function-style tools)
// - ToolRegistry implements ToolExecutor for integration with the agent loop
// - Error handling distinguishes between user-visible errors and internal errors
// - Internal errors are logged but not exposed to the LLM (security)

use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::error;

use crate::tool_types::{BuiltinTool, ToolCall, ToolDefinition, ToolPolicy, ToolResult};
use crate::traits::ToolContext;

use crate::error::Result;
use crate::traits::ToolExecutor;

// ============================================================================
// Tool Execution Result - Error Handling Contract
// ============================================================================

/// Result of a tool execution.
///
/// This enum distinguishes between different outcomes:
/// - `Success`: Tool executed successfully, result is returned to LLM
/// - `ToolError`: Tool-level error that should be shown to the LLM
///   (e.g., "City not found", "Invalid date format")
/// - `InternalError`: System-level error that should NOT be exposed to the LLM
///   (e.g., database connection failure, API key issues)
///
/// # Security
///
/// Internal errors are logged but replaced with a generic message when
/// returned to the LLM. This prevents leaking sensitive information like
/// database errors, API keys, or internal system details.
#[derive(Debug)]
pub enum ToolExecutionResult {
    /// Successful execution with a JSON result
    Success(Value),

    /// Tool-level error that is safe to show to the LLM
    ///
    /// Use this for expected error conditions that the LLM should know about,
    /// such as validation errors, resource not found, etc.
    ToolError(String),

    /// Internal/system error that should NOT be exposed to the LLM
    ///
    /// Use this for unexpected errors like network failures, database errors,
    /// or other internal issues. The error details will be logged but replaced
    /// with a generic message when returned to the LLM.
    InternalError(ToolInternalError),
}

impl ToolExecutionResult {
    /// Create a successful result
    pub fn success(value: impl Into<Value>) -> Self {
        ToolExecutionResult::Success(value.into())
    }

    /// Create a tool-level error (safe to show to LLM)
    pub fn tool_error(message: impl Into<String>) -> Self {
        ToolExecutionResult::ToolError(message.into())
    }

    /// Create an internal error (will be hidden from LLM)
    pub fn internal_error(error: impl std::error::Error + Send + Sync + 'static) -> Self {
        ToolExecutionResult::InternalError(ToolInternalError::new(error))
    }

    /// Create an internal error from a string message
    pub fn internal_error_msg(message: impl Into<String>) -> Self {
        ToolExecutionResult::InternalError(ToolInternalError::from_message(message))
    }

    /// Check if this is a successful result
    pub fn is_success(&self) -> bool {
        matches!(self, ToolExecutionResult::Success(_))
    }

    /// Check if this is an error (either tool error or internal error)
    pub fn is_error(&self) -> bool {
        !self.is_success()
    }

    /// Convert to a ToolResult for the agent loop
    ///
    /// Both tool errors and internal errors are packaged as `{"error": "..."}` in the
    /// result field. This provides a consistent contract where the result field always
    /// contains the payload, and the agent loop continues the same way for all outcomes.
    ///
    /// Internal errors are logged but replaced with a generic message when returned.
    pub fn into_tool_result(self, tool_call_id: &str, tool_name: &str) -> ToolResult {
        match self {
            ToolExecutionResult::Success(value) => ToolResult {
                tool_call_id: tool_call_id.to_string(),
                result: Some(value),
                error: None,
            },
            ToolExecutionResult::ToolError(message) => ToolResult {
                tool_call_id: tool_call_id.to_string(),
                result: Some(serde_json::json!({ "error": message })),
                error: None,
            },
            ToolExecutionResult::InternalError(err) => {
                // Log the full error details for debugging
                error!(
                    tool_name = %tool_name,
                    tool_call_id = %tool_call_id,
                    error = %err.message,
                    "Tool internal error (details hidden from LLM)"
                );

                // Return generic error message to LLM, packaged as {"error": "..."}
                ToolResult {
                    tool_call_id: tool_call_id.to_string(),
                    result: Some(serde_json::json!({
                        "error": "An internal error occurred while executing the tool"
                    })),
                    error: None,
                }
            }
        }
    }
}

/// Internal error details (logged but not exposed to LLM)
#[derive(Debug)]
pub struct ToolInternalError {
    /// Error message for logging
    pub message: String,
    /// Optional source error
    pub source: Option<Box<dyn std::error::Error + Send + Sync>>,
}

impl ToolInternalError {
    /// Create from an error
    pub fn new(error: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self {
            message: error.to_string(),
            source: Some(Box::new(error)),
        }
    }

    /// Create from a string message
    pub fn from_message(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            source: None,
        }
    }
}

impl std::fmt::Display for ToolInternalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for ToolInternalError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source
            .as_ref()
            .map(|e| e.as_ref() as &(dyn std::error::Error + 'static))
    }
}

// ============================================================================
// Tool Trait - Core Tool Abstraction
// ============================================================================

/// Trait for implementing tools that can be executed by the agent loop.
///
/// # Example
///
/// ```ignore
/// use async_trait::async_trait;
/// use serde_json::{json, Value};
///
/// struct GetCurrentTime;
///
/// #[async_trait]
/// impl Tool for GetCurrentTime {
///     fn name(&self) -> &str {
///         "get_current_time"
///     }
///
///     fn description(&self) -> &str {
///         "Get the current date and time"
///     }
///
///     fn parameters_schema(&self) -> Value {
///         json!({
///             "type": "object",
///             "properties": {
///                 "timezone": {
///                     "type": "string",
///                     "description": "Timezone (e.g., 'UTC', 'America/New_York')"
///                 }
///             }
///         })
///     }
///
///     async fn execute(&self, arguments: Value) -> ToolExecutionResult {
///         let timezone = arguments.get("timezone")
///             .and_then(|v| v.as_str())
///             .unwrap_or("UTC");
///
///         ToolExecutionResult::success(json!({
///             "current_time": chrono::Utc::now().to_rfc3339(),
///             "timezone": timezone
///         }))
///     }
/// }
/// ```
#[async_trait]
pub trait Tool: Send + Sync {
    /// Returns the tool's unique name.
    ///
    /// This name is used by the LLM to invoke the tool and must be unique
    /// within a ToolRegistry.
    fn name(&self) -> &str;

    /// Returns a description of what the tool does.
    ///
    /// This description is provided to the LLM to help it understand
    /// when and how to use the tool.
    fn description(&self) -> &str;

    /// Returns the JSON schema for the tool's parameters.
    ///
    /// This schema follows the JSON Schema specification and describes
    /// the expected arguments for the tool. The LLM uses this to
    /// generate valid tool calls.
    fn parameters_schema(&self) -> Value;

    /// Execute the tool with the given arguments.
    ///
    /// # Arguments
    ///
    /// * `arguments` - The arguments passed to the tool as a JSON value.
    ///   These should conform to the schema returned by `parameters_schema()`.
    ///
    /// # Returns
    ///
    /// A `ToolExecutionResult` indicating success, tool error, or internal error.
    async fn execute(&self, arguments: Value) -> ToolExecutionResult;

    /// Execute the tool with context.
    ///
    /// This method provides access to runtime context like session ID and
    /// optional stores (file store, etc.). Override this method for tools
    /// that need access to session context or external resources.
    ///
    /// The default implementation simply calls `execute()`, ignoring the context.
    ///
    /// # Arguments
    ///
    /// * `arguments` - The arguments passed to the tool as a JSON value.
    /// * `context` - Runtime context containing session ID and optional stores.
    ///
    /// # Returns
    ///
    /// A `ToolExecutionResult` indicating success, tool error, or internal error.
    async fn execute_with_context(
        &self,
        arguments: Value,
        _context: &ToolContext,
    ) -> ToolExecutionResult {
        // Default: delegate to execute(), ignoring context
        self.execute(arguments).await
    }

    /// Returns true if this tool requires context for execution.
    ///
    /// Tools that need session context (like filesystem tools) should
    /// override this to return true.
    fn requires_context(&self) -> bool {
        false
    }

    /// Returns the tool policy (auto or requires_approval).
    ///
    /// Default is `Auto` which means the tool executes immediately.
    /// Override to return `RequiresApproval` for sensitive operations.
    fn policy(&self) -> ToolPolicy {
        ToolPolicy::Auto
    }

    /// Convert this tool to a ToolDefinition for the agent config.
    ///
    /// This is used by ToolRegistry to generate tool definitions
    /// for the LLM provider.
    fn to_definition(&self) -> ToolDefinition {
        ToolDefinition::Builtin(BuiltinTool {
            name: self.name().to_string(),
            description: self.description().to_string(),
            parameters: self.parameters_schema(),
            policy: self.policy(),
        })
    }
}

// ============================================================================
// ToolRegistry - Collection of Tools
// ============================================================================

/// A registry that holds multiple tools and implements ToolExecutor.
///
/// ToolRegistry provides a convenient way to manage multiple tools and
/// integrate them with the agent loop. It implements `ToolExecutor` so
/// it can be used directly with `AgentLoop`.
///
/// # Example
///
/// ```ignore
/// use everruns_core::tools::{Tool, ToolRegistry};
///
/// // Create registry and add tools
/// let mut registry = ToolRegistry::new();
/// registry.register(Box::new(GetCurrentTime));
/// registry.register(Box::new(GetWeather));
///
/// // Get tool definitions for agent config
/// let definitions = registry.tool_definitions();
///
/// // Use with agent loop
/// let agent_loop = AgentLoop::new(config, emitter, store, llm, registry);
/// ```
#[derive(Default, Clone)]
pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
}

impl ToolRegistry {
    /// Create a new empty tool registry
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    /// Create a tool registry with default built-in tools.
    ///
    /// This includes:
    /// - `get_current_time`: Returns the current date and time
    /// - `echo`: Echoes back the provided message
    /// - TestMath tools: add, subtract, multiply, divide
    /// - TestWeather tools: get_weather, get_forecast
    /// - TaskList tools: write_todos
    /// - FileSystem tools: read_file, write_file, list_directory, grep_files, delete_file, stat_file
    /// - WebFetch tools: web_fetch
    pub fn with_defaults() -> Self {
        use crate::capabilities::{
            AddTool, DeleteFileTool, DivideTool, GetCurrentTimeTool, GetForecastTool,
            GetWeatherTool, GrepFilesTool, ListDirectoryTool, MultiplyTool, ReadFileTool,
            StatFileTool, SubtractTool, WebFetchTool, WriteFileTool, WriteTodosTool,
        };

        ToolRegistry::builder()
            .tool(GetCurrentTimeTool)
            .tool(EchoTool)
            // TestMath capability tools
            .tool(AddTool)
            .tool(SubtractTool)
            .tool(MultiplyTool)
            .tool(DivideTool)
            // TestWeather capability tools
            .tool(GetWeatherTool)
            .tool(GetForecastTool)
            // TaskList capability tools
            .tool(WriteTodosTool)
            // FileSystem capability tools
            .tool(ReadFileTool)
            .tool(WriteFileTool)
            .tool(ListDirectoryTool)
            .tool(GrepFilesTool)
            .tool(DeleteFileTool)
            .tool(StatFileTool)
            // WebFetch capability tools
            .tool(WebFetchTool)
            .build()
    }

    /// Register a tool with the registry.
    ///
    /// If a tool with the same name already exists, it will be replaced.
    pub fn register(&mut self, tool: impl Tool + 'static) {
        self.tools.insert(tool.name().to_string(), Arc::new(tool));
    }

    /// Register a boxed tool
    pub fn register_boxed(&mut self, tool: Box<dyn Tool>) {
        self.tools.insert(tool.name().to_string(), Arc::from(tool));
    }

    /// Register an Arc-wrapped tool
    pub fn register_arc(&mut self, tool: Arc<dyn Tool>) {
        self.tools.insert(tool.name().to_string(), tool);
    }

    /// Get a tool by name
    pub fn get(&self, name: &str) -> Option<&Arc<dyn Tool>> {
        self.tools.get(name)
    }

    /// Check if a tool is registered
    pub fn has(&self, name: &str) -> bool {
        self.tools.contains_key(name)
    }

    /// Get the number of registered tools
    pub fn len(&self) -> usize {
        self.tools.len()
    }

    /// Check if the registry is empty
    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    /// Get all tool names
    pub fn tool_names(&self) -> Vec<&str> {
        self.tools.keys().map(|s| s.as_str()).collect()
    }

    /// Get tool definitions for use in AgentConfig.
    ///
    /// Returns a Vec of ToolDefinition that can be passed to
    /// `AgentConfig::with_tools()`.
    pub fn tool_definitions(&self) -> Vec<ToolDefinition> {
        self.tools.values().map(|t| t.to_definition()).collect()
    }

    /// Remove a tool from the registry
    pub fn unregister(&mut self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.remove(name)
    }

    /// Clear all tools from the registry
    pub fn clear(&mut self) {
        self.tools.clear();
    }

    /// Create a builder for fluent tool registration
    pub fn builder() -> ToolRegistryBuilder {
        ToolRegistryBuilder::new()
    }
}

impl std::fmt::Debug for ToolRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolRegistry")
            .field("tools", &self.tool_names())
            .finish()
    }
}

#[async_trait]
impl ToolExecutor for ToolRegistry {
    async fn execute(
        &self,
        tool_call: &ToolCall,
        _tool_def: &ToolDefinition,
    ) -> Result<ToolResult> {
        let tool = self.tools.get(&tool_call.name).ok_or_else(|| {
            crate::error::AgentLoopError::tool(format!("Tool not found: {}", tool_call.name))
        })?;

        let result = tool.execute(tool_call.arguments.clone()).await;
        Ok(result.into_tool_result(&tool_call.id, &tool_call.name))
    }

    async fn execute_with_context(
        &self,
        tool_call: &ToolCall,
        _tool_def: &ToolDefinition,
        context: &ToolContext,
    ) -> Result<ToolResult> {
        let tool = self.tools.get(&tool_call.name).ok_or_else(|| {
            crate::error::AgentLoopError::tool(format!("Tool not found: {}", tool_call.name))
        })?;

        // Use execute_with_context for all tools - context-aware tools will use it,
        // regular tools will delegate to execute() via the default implementation
        let result = tool
            .execute_with_context(tool_call.arguments.clone(), context)
            .await;
        Ok(result.into_tool_result(&tool_call.id, &tool_call.name))
    }
}

// ============================================================================
// ToolRegistryBuilder - Fluent API for Building Registry
// ============================================================================

/// Builder for creating a ToolRegistry with a fluent API.
///
/// # Example
///
/// ```ignore
/// let registry = ToolRegistry::builder()
///     .tool(GetCurrentTime)
///     .tool(GetWeather)
///     .build();
/// ```
pub struct ToolRegistryBuilder {
    registry: ToolRegistry,
}

impl ToolRegistryBuilder {
    /// Create a new builder
    pub fn new() -> Self {
        Self {
            registry: ToolRegistry::new(),
        }
    }

    /// Add a tool to the registry
    pub fn tool(mut self, tool: impl Tool + 'static) -> Self {
        self.registry.register(tool);
        self
    }

    /// Add a boxed tool to the registry
    pub fn tool_boxed(mut self, tool: Box<dyn Tool>) -> Self {
        self.registry.register_boxed(tool);
        self
    }

    /// Add an Arc-wrapped tool to the registry
    pub fn tool_arc(mut self, tool: Arc<dyn Tool>) -> Self {
        self.registry.register_arc(tool);
        self
    }

    /// Build the registry
    pub fn build(self) -> ToolRegistry {
        self.registry
    }
}

impl Default for ToolRegistryBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Built-in Tools
// ============================================================================

/// A tool that echoes back its arguments (useful for testing)
pub struct EchoTool;

#[async_trait]
impl Tool for EchoTool {
    fn name(&self) -> &str {
        "echo"
    }

    fn description(&self) -> &str {
        "Echo back the provided message. Useful for testing tool execution."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "message": {
                    "type": "string",
                    "description": "The message to echo back"
                }
            },
            "required": ["message"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, arguments: Value) -> ToolExecutionResult {
        let message = arguments
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        ToolExecutionResult::success(serde_json::json!({
            "echoed": message,
            "length": message.len()
        }))
    }
}

/// A tool that always fails (useful for testing error handling)
pub struct FailingTool {
    error_message: String,
    use_internal_error: bool,
}

impl FailingTool {
    /// Create a failing tool with a tool-level error
    pub fn with_tool_error(message: impl Into<String>) -> Self {
        Self {
            error_message: message.into(),
            use_internal_error: false,
        }
    }

    /// Create a failing tool with an internal error
    pub fn with_internal_error(message: impl Into<String>) -> Self {
        Self {
            error_message: message.into(),
            use_internal_error: true,
        }
    }
}

impl Default for FailingTool {
    fn default() -> Self {
        Self::with_tool_error("Tool execution failed")
    }
}

#[async_trait]
impl Tool for FailingTool {
    fn name(&self) -> &str {
        "failing_tool"
    }

    fn description(&self) -> &str {
        "A tool that always fails (for testing error handling)"
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        if self.use_internal_error {
            ToolExecutionResult::internal_error_msg(&self.error_message)
        } else {
            ToolExecutionResult::tool_error(&self.error_message)
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capabilities::GetCurrentTimeTool;

    #[tokio::test]
    async fn test_echo_tool() {
        let tool = EchoTool;

        let result = tool
            .execute(serde_json::json!({"message": "Hello, world!"}))
            .await;

        if let ToolExecutionResult::Success(value) = result {
            assert_eq!(
                value.get("echoed").unwrap().as_str().unwrap(),
                "Hello, world!"
            );
            assert_eq!(value.get("length").unwrap().as_u64().unwrap(), 13);
        } else {
            panic!("Expected success");
        }
    }

    #[tokio::test]
    async fn test_failing_tool_with_tool_error() {
        let tool = FailingTool::with_tool_error("Something went wrong");

        let result = tool.execute(serde_json::json!({})).await;

        if let ToolExecutionResult::ToolError(msg) = result {
            assert_eq!(msg, "Something went wrong");
        } else {
            panic!("Expected tool error");
        }
    }

    #[tokio::test]
    async fn test_failing_tool_with_internal_error() {
        let tool = FailingTool::with_internal_error("Database connection failed");

        let result = tool.execute(serde_json::json!({})).await;

        if let ToolExecutionResult::InternalError(err) = result {
            assert_eq!(err.message, "Database connection failed");
        } else {
            panic!("Expected internal error");
        }
    }

    #[tokio::test]
    async fn test_tool_result_conversion() {
        // Success
        let result = ToolExecutionResult::success(serde_json::json!({"value": 42}));
        let tool_result = result.into_tool_result("call_1", "test_tool");
        assert!(tool_result.error.is_none());
        assert_eq!(tool_result.result.unwrap()["value"], 42);

        // Tool error (packaged as {"error": "..."} in result field)
        let result = ToolExecutionResult::tool_error("Invalid input");
        let tool_result = result.into_tool_result("call_2", "test_tool");
        assert!(tool_result.error.is_none());
        assert_eq!(
            tool_result.result.unwrap(),
            serde_json::json!({"error": "Invalid input"})
        );

        // Internal error (packaged as {"error": "..."} with generic message)
        let result = ToolExecutionResult::internal_error_msg("Secret database error");
        let tool_result = result.into_tool_result("call_3", "test_tool");
        assert!(tool_result.error.is_none());
        assert_eq!(
            tool_result.result.unwrap(),
            serde_json::json!({"error": "An internal error occurred while executing the tool"})
        );
    }

    #[tokio::test]
    async fn test_tool_registry() {
        let mut registry = ToolRegistry::new();
        registry.register(GetCurrentTimeTool);
        registry.register(EchoTool);

        assert_eq!(registry.len(), 2);
        assert!(registry.has("get_current_time"));
        assert!(registry.has("echo"));
        assert!(!registry.has("nonexistent"));

        let definitions = registry.tool_definitions();
        assert_eq!(definitions.len(), 2);
    }

    #[tokio::test]
    async fn test_tool_registry_builder() {
        let registry = ToolRegistry::builder()
            .tool(GetCurrentTimeTool)
            .tool(EchoTool)
            .build();

        assert_eq!(registry.len(), 2);
    }

    #[tokio::test]
    async fn test_tool_registry_as_executor() {
        let mut registry = ToolRegistry::new();
        registry.register(EchoTool);

        let tool_call = ToolCall {
            id: "call_1".to_string(),
            name: "echo".to_string(),
            arguments: serde_json::json!({"message": "test"}),
        };

        let tool_def = registry.get("echo").unwrap().to_definition();
        let result = registry.execute(&tool_call, &tool_def).await.unwrap();

        assert!(result.error.is_none());
        assert_eq!(result.result.unwrap()["echoed"], "test");
    }

    #[test]
    fn test_tool_to_definition() {
        let tool = GetCurrentTimeTool;
        let def = tool.to_definition();

        let ToolDefinition::Builtin(builtin) = def;
        assert_eq!(builtin.name, "get_current_time");
        assert_eq!(builtin.policy, ToolPolicy::Auto);
    }

    #[test]
    fn test_with_defaults_has_expected_tools() {
        let registry = ToolRegistry::with_defaults();

        // Core tools
        assert!(
            registry.has("get_current_time"),
            "should have get_current_time"
        );
        assert!(registry.has("echo"), "should have echo");

        // TestMath capability tools
        assert!(registry.has("add"), "should have add");
        assert!(registry.has("subtract"), "should have subtract");
        assert!(registry.has("multiply"), "should have multiply");
        assert!(registry.has("divide"), "should have divide");

        // TestWeather capability tools
        assert!(registry.has("get_weather"), "should have get_weather");
        assert!(registry.has("get_forecast"), "should have get_forecast");

        // TaskList capability tools
        assert!(registry.has("write_todos"), "should have write_todos");

        // FileSystem capability tools
        assert!(registry.has("read_file"), "should have read_file");
        assert!(registry.has("write_file"), "should have write_file");
        assert!(registry.has("list_directory"), "should have list_directory");
        assert!(registry.has("grep_files"), "should have grep_files");
        assert!(registry.has("delete_file"), "should have delete_file");
        assert!(registry.has("stat_file"), "should have stat_file");

        // WebFetch capability tools
        assert!(registry.has("web_fetch"), "should have web_fetch");

        // Total count
        assert_eq!(registry.len(), 16, "should have 16 default tools");
    }

    #[tokio::test]
    async fn test_with_defaults_tools_are_executable() {
        let registry = ToolRegistry::with_defaults();

        // Test echo tool execution
        let tool_call = ToolCall {
            id: "call_1".to_string(),
            name: "echo".to_string(),
            arguments: serde_json::json!({"message": "hello from defaults"}),
        };

        let tool_def = registry.get("echo").unwrap().to_definition();
        let result = registry.execute(&tool_call, &tool_def).await.unwrap();

        assert!(result.error.is_none());
        assert_eq!(result.result.unwrap()["echoed"], "hello from defaults");
    }

    #[tokio::test]
    async fn test_with_defaults_math_tools() {
        let registry = ToolRegistry::with_defaults();

        // Test add tool
        let tool_call = ToolCall {
            id: "call_add".to_string(),
            name: "add".to_string(),
            arguments: serde_json::json!({"a": 5, "b": 3}),
        };

        let tool_def = registry.get("add").unwrap().to_definition();
        let result = registry.execute(&tool_call, &tool_def).await.unwrap();

        assert!(result.error.is_none());
        // AddTool returns floats, so compare as f64
        assert_eq!(result.result.unwrap()["result"].as_f64().unwrap(), 8.0);
    }
}

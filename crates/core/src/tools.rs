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
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use tracing::error;

use crate::background::{
    BackgroundEventSink, BackgroundExecutableTool, BackgroundOutcome, BackgroundProgress,
};
use crate::session_resource::{RegisterSessionResource, SessionResourceStatus};
use crate::session_schedule::MAX_ACTIVE_SCHEDULES_PER_SESSION;
use crate::tool_types::{
    BuiltinTool, DeferrablePolicy, ToolCall, ToolDefinition, ToolHints, ToolPolicy, ToolResult,
};
use crate::traits::ToolContext;
use crate::typed_id::SessionId;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::error::Result;
use crate::traits::ToolExecutor;

/// Maximum active immediate background runs allowed for a single session.
///
/// This mirrors the scheduled monitor cap so model-visible background execution
/// cannot be used to queue unbounded active worker jobs for one session.
pub const MAX_ACTIVE_BACKGROUND_RUNS_PER_SESSION: usize = 5;

/// Maximum active immediate background runs allowed in this worker process.
///
/// The per-session semaphore limits tenant/session abuse; this process-wide
/// semaphore keeps concurrent sessions from exhausting worker-local resources.
const MAX_ACTIVE_BACKGROUND_RUNS_PER_WORKER: usize = 64;
static ACTIVE_BACKGROUND_RUNS_PER_WORKER: Semaphore =
    Semaphore::const_new(MAX_ACTIVE_BACKGROUND_RUNS_PER_WORKER);

/// Per-session semaphores that enforce `MAX_ACTIVE_BACKGROUND_RUNS_PER_SESSION`.
///
/// Using a semaphore rather than a DB count makes the check atomic: concurrent
/// `spawn_background` calls for the same session cannot both slip through the
/// guard (check-then-act race) because `try_acquire` is inherently atomic.
static SESSION_BACKGROUND_PERMITS: OnceLock<std::sync::Mutex<HashMap<SessionId, Arc<Semaphore>>>> =
    OnceLock::new();

struct SessionBackgroundPermit {
    session_id: SessionId,
    semaphore: Arc<Semaphore>,
    permit: Option<OwnedSemaphorePermit>,
}

impl Drop for SessionBackgroundPermit {
    fn drop(&mut self) {
        drop(self.permit.take());

        let Some(permits) = SESSION_BACKGROUND_PERMITS.get() else {
            return;
        };
        let mut permits = permits.lock().unwrap();
        let should_remove = permits.get(&self.session_id).is_some_and(|current| {
            Arc::ptr_eq(current, &self.semaphore)
                && self.semaphore.available_permits() == MAX_ACTIVE_BACKGROUND_RUNS_PER_SESSION
                && Arc::strong_count(&self.semaphore) == 2
        });
        if should_remove {
            permits.remove(&self.session_id);
        }
    }
}

fn try_acquire_session_background_permit(
    session_id: SessionId,
) -> std::result::Result<SessionBackgroundPermit, tokio::sync::TryAcquireError> {
    let permits = SESSION_BACKGROUND_PERMITS.get_or_init(Default::default);
    let mut permits = permits.lock().unwrap();
    let semaphore = permits
        .entry(session_id)
        .or_insert_with(|| Arc::new(Semaphore::new(MAX_ACTIVE_BACKGROUND_RUNS_PER_SESSION)))
        .clone();
    let permit = semaphore.clone().try_acquire_owned()?;

    Ok(SessionBackgroundPermit {
        session_id,
        semaphore,
        permit: Some(permit),
    })
}

#[cfg(test)]
fn has_session_background_permits(session_id: SessionId) -> bool {
    SESSION_BACKGROUND_PERMITS
        .get()
        .and_then(|permits| permits.lock().unwrap().get(&session_id).cloned())
        .is_some()
}

// ============================================================================
// Tool Execution Result - Error Handling Contract
// ============================================================================

/// Image data returned by a tool alongside text results.
///
/// This allows tools (built-in or MCP) to return images that are sent
/// to the LLM as native image content blocks, not stringified JSON.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResultImage {
    /// Base64-encoded image data
    pub base64: String,
    /// MIME type (e.g., "image/png", "image/jpeg")
    pub media_type: String,
}

/// Result of a tool execution.
///
/// This enum distinguishes between different outcomes:
/// - `Success`: Tool executed successfully, result is returned to LLM
/// - `SuccessWithImages`: Successful execution with JSON result plus images
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

    /// Successful execution with a JSON result and images.
    /// Images are sent to the LLM as native image content blocks
    /// (not stringified JSON), enabling visual understanding.
    SuccessWithImages {
        result: Value,
        images: Vec<ToolResultImage>,
    },

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

    /// A user connection is required to execute this tool.
    ///
    /// Instead of returning an error, this signals that the workflow should
    /// pause and ask the client to set up a connection for the given provider.
    /// The UI renders an inline connection dialog; once the user saves (or
    /// cancels), a tool result is submitted and execution resumes.
    ConnectionRequired {
        /// Connection provider id (e.g. "daytona", "brave_search")
        provider: String,
    },
}

impl ToolExecutionResult {
    /// Create a successful result
    pub fn success(value: impl Into<Value>) -> Self {
        ToolExecutionResult::Success(value.into())
    }

    /// Create a successful result with pre-truncation raw output for VFS persistence.
    /// The raw output is transferred to `ToolResult.raw_output` during `into_tool_result()`.
    pub fn success_with_raw_output(value: impl Into<Value>, raw_output: String) -> Self {
        let mut value = value.into();
        // Embed raw output in a sidecar key — extracted in into_tool_result().
        // Non-object values are wrapped in a scalar carrier so raw_output still
        // flows through; the carrier is unwrapped on extraction.
        match value.as_object_mut() {
            Some(obj) => {
                obj.insert("_raw_output".to_string(), Value::String(raw_output));
            }
            None => {
                value = serde_json::json!({
                    "_raw_output_scalar": value,
                    "_raw_output": raw_output,
                });
            }
        }
        ToolExecutionResult::Success(value)
    }

    /// Create a successful result with images
    pub fn success_with_images(value: impl Into<Value>, images: Vec<ToolResultImage>) -> Self {
        ToolExecutionResult::SuccessWithImages {
            result: value.into(),
            images,
        }
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

    /// Signal that a user connection is required before this tool can execute.
    pub fn connection_required(provider: impl Into<String>) -> Self {
        ToolExecutionResult::ConnectionRequired {
            provider: provider.into(),
        }
    }

    /// Check if this is a successful result
    pub fn is_success(&self) -> bool {
        matches!(
            self,
            ToolExecutionResult::Success(_) | ToolExecutionResult::SuccessWithImages { .. }
        )
    }

    /// Check if this is an error (either tool error or internal error)
    pub fn is_error(&self) -> bool {
        matches!(
            self,
            ToolExecutionResult::ToolError(_) | ToolExecutionResult::InternalError(_)
        )
    }

    /// Check if this requires a user connection setup
    pub fn is_connection_required(&self) -> bool {
        matches!(self, ToolExecutionResult::ConnectionRequired { .. })
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
            ToolExecutionResult::Success(mut value) => {
                // Extract sidecar raw output if present (from success_with_raw_output)
                let raw_output = value
                    .as_object_mut()
                    .and_then(|obj| obj.remove("_raw_output"))
                    .and_then(|v| v.as_str().map(|s| s.to_string()));
                // Unwrap scalar carrier only when it matches the exact wrapper shape
                // set by success_with_raw_output for non-object inputs.
                let result_value = if let Some(obj) = value.as_object_mut() {
                    let is_scalar_carrier = raw_output.is_some()
                        && obj.len() == 1
                        && obj.contains_key("_raw_output_scalar");
                    if is_scalar_carrier {
                        obj.remove("_raw_output_scalar").unwrap_or(Value::Null)
                    } else {
                        value
                    }
                } else {
                    value
                };
                ToolResult {
                    tool_call_id: tool_call_id.to_string(),
                    result: Some(result_value),
                    images: None,
                    error: None,
                    connection_required: None,
                    raw_output,
                }
            }
            ToolExecutionResult::SuccessWithImages { result, images } => ToolResult {
                tool_call_id: tool_call_id.to_string(),
                result: Some(result),
                images: if images.is_empty() {
                    None
                } else {
                    Some(images)
                },
                error: None,
                connection_required: None,
                raw_output: None,
            },
            ToolExecutionResult::ToolError(message) => ToolResult {
                tool_call_id: tool_call_id.to_string(),
                result: Some(serde_json::json!({ "error": &message })),
                images: None,
                error: Some(message),
                connection_required: None,
                raw_output: None,
            },
            ToolExecutionResult::InternalError(err) => {
                // Log the full error details for debugging
                error!(
                    tool_name = %tool_name,
                    tool_call_id = %tool_call_id,
                    error = %err.message,
                    error_chain = %err.chain_string(),
                    "Tool internal error (details hidden from LLM)"
                );

                // Return generic error message to LLM, packaged as {"error": "..."}
                let generic_msg = "An internal error occurred while executing the tool";
                ToolResult {
                    tool_call_id: tool_call_id.to_string(),
                    result: Some(serde_json::json!({
                        "error": generic_msg
                    })),
                    images: None,
                    error: Some(generic_msg.to_string()),
                    connection_required: None,
                    raw_output: None,
                }
            }
            ToolExecutionResult::ConnectionRequired { ref provider } => ToolResult {
                tool_call_id: tool_call_id.to_string(),
                result: Some(serde_json::json!({
                    "connection_required": provider,
                })),
                images: None,
                error: None,
                connection_required: Some(provider.clone()),
                raw_output: None,
            },
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

    pub fn chain_string(&self) -> String {
        let mut parts = vec![self.message.clone()];
        let mut current = <Self as std::error::Error>::source(self);
        while let Some(source) = current {
            let message = source.to_string();
            if parts.last() != Some(&message) {
                parts.push(message);
            }
            current = source.source();
        }
        parts.join(": ")
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

    /// Returns a human-readable display name for UI rendering.
    ///
    /// This name is shown to users in the UI instead of the technical tool name.
    /// For example, "Get Current Time" instead of "get_current_time".
    /// Returns None if no display name is set, in which case the UI may
    /// fall back to the technical name.
    fn display_name(&self) -> Option<&str> {
        None
    }

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

    /// Returns semantic hints describing the tool's behavioral properties.
    ///
    /// Override to provide hints like readonly, destructive, idempotent, etc.
    /// Default is empty (all hints unspecified).
    fn hints(&self) -> ToolHints {
        ToolHints::default()
    }

    /// Returns native background execution support when this tool opts into
    /// detached execution via `hints().supports_background`.
    fn as_background_executable(&self) -> Option<&dyn BackgroundExecutableTool> {
        None
    }

    /// Convert this tool to a ToolDefinition for the agent config.
    ///
    /// This is used by ToolRegistry to generate tool definitions
    /// for the LLM provider.
    fn to_definition(&self) -> ToolDefinition {
        ToolDefinition::Builtin(BuiltinTool {
            name: self.name().to_string(),
            display_name: self.display_name().map(|s| s.to_string()),
            description: self.description().to_string(),
            parameters: self.parameters_schema(),
            policy: self.policy(),
            category: None,
            deferrable: DeferrablePolicy::default(),
            hints: self.hints(),
            full_parameters: None,
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
    /// - `report_progress`: Emits deterministic external progress updates
    /// - TestMath tools: add, subtract, multiply, divide
    /// - TestWeather tools: get_weather, get_forecast
    /// - TaskList tools: write_todos
    /// - FileSystem tools: read_file, write_file, edit_file, list_directory, grep_files, delete_file, stat_file
    /// - WebFetch tools: web_fetch
    pub fn with_defaults() -> Self {
        use crate::capabilities::{
            AddTool, DeleteFileTool, DivideTool, EditFileTool, GetCurrentTimeTool, GetForecastTool,
            GetWeatherTool, GrepFilesTool, ListDirectoryTool, MultiplyTool, ReadFileTool,
            StatFileTool, SubtractTool, WebFetchTool, WriteFileTool, WriteTodosTool,
        };
        use crate::progress_reporting::ReportProgressTool;

        ToolRegistry::builder()
            .tool(GetCurrentTimeTool)
            .tool(EchoTool)
            // NOTE: `spawn_background` is intentionally NOT a default tool —
            // it is contributed by the `background_execution` capability,
            // which is auto-activated by
            // `collect_capabilities_with_configs` whenever a collected tool
            // declares `ToolHints::supports_background = Some(true)`. Keeping
            // it out of defaults preserves the lockstep contract between
            // model-visible tools and the worker execution registry: the
            // executor only knows about `spawn_background` when the model
            // can also see it.
            .tool(ReportProgressTool)
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
            .tool(EditFileTool)
            .tool(ListDirectoryTool)
            .tool(GrepFilesTool)
            .tool(DeleteFileTool)
            .tool(StatFileTool)
            // WebFetch capability tools
            .tool(WebFetchTool::default())
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

    /// Get tool definitions for use in RuntimeAgent.
    ///
    /// Returns a Vec of ToolDefinition that can be passed to
    /// `RuntimeAgent::with_tools()`.
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

    fn display_name(&self) -> Option<&str> {
        Some("Echo")
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

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_readonly(true)
            .with_idempotent(true)
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

/// Spawn a background-capable tool and return immediately with a run handle.
pub struct SpawnBackgroundTool;

#[derive(Debug, Clone)]
struct BackgroundScheduleRequest {
    cron_expression: Option<String>,
    scheduled_at: Option<chrono::DateTime<chrono::Utc>>,
    timezone: String,
}

fn parse_background_schedule(
    arguments: &Value,
) -> std::result::Result<Option<BackgroundScheduleRequest>, String> {
    let Some(schedule) = arguments.get("schedule") else {
        return Ok(None);
    };
    let Some(schedule) = schedule.as_object() else {
        return Err("schedule must be an object".to_string());
    };

    let cron_expression = schedule
        .get("cron_expression")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);
    let scheduled_at = match schedule.get("scheduled_at").and_then(Value::as_str) {
        Some(value) => {
            let value = value.trim();
            if value.is_empty() {
                None
            } else {
                Some(
                    chrono::DateTime::parse_from_rfc3339(value)
                        .map_err(|_| "scheduled_at must be RFC3339".to_string())?
                        .with_timezone(&chrono::Utc),
                )
            }
        }
        None => None,
    };

    match (cron_expression.is_some(), scheduled_at.is_some()) {
        (false, false) => {
            return Err(
                "schedule must include exactly one of cron_expression (recurring) or scheduled_at (one-shot)"
                    .to_string(),
            );
        }
        (true, true) => {
            return Err(
                "schedule must not include both cron_expression and scheduled_at; provide exactly one"
                    .to_string(),
            );
        }
        _ => {}
    }

    let timezone = schedule
        .get("timezone")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("UTC")
        .to_string();

    Ok(Some(BackgroundScheduleRequest {
        cron_expression,
        scheduled_at,
        timezone,
    }))
}

fn build_background_schedule_description(
    tool_name: &str,
    tool_args: &Value,
    title: &str,
    signal_on_completion: bool,
) -> String {
    let payload = json!({
        "tool": tool_name,
        "title": title,
        "signal_on_completion": signal_on_completion,
        "args": tool_args,
    });
    let payload_json =
        serde_json::to_string_pretty(&payload).unwrap_or_else(|_| payload.to_string());

    format!(
        "Monitor: {title}\n\n\
This scheduled monitor fired. Start the background run now.\n\n\
spawn_background payload:\n{payload_json}"
    )
}

#[async_trait]
impl Tool for SpawnBackgroundTool {
    fn name(&self) -> &str {
        "spawn_background"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Spawn Background")
    }

    fn description(&self) -> &str {
        "Run a background-capable built-in tool asynchronously. Returns immediately and signals the session when the background run completes."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "tool": {
                    "type": "string",
                    "description": "Name of the built-in tool to execute in the background"
                },
                "args": {
                    "type": "object",
                    "description": "Arguments to pass to the target tool"
                },
                "title": {
                    "type": "string",
                    "description": "Optional human-readable label for the background run"
                },
                "schedule": {
                    "type": "object",
                    "description": "Optional session schedule. When provided, this creates a scheduled monitor instead of starting the run immediately.",
                    "properties": {
                        "cron_expression": {
                            "type": "string",
                            "description": "Standard 5-field cron expression for recurring runs (e.g. '*/10 * * * *' for every 10 minutes)"
                        },
                        "scheduled_at": {
                            "type": "string",
                            "description": "ISO 8601 datetime for a one-shot run (e.g. '2026-04-16T15:30:00Z')"
                        },
                        "timezone": {
                            "type": "string",
                            "description": "IANA timezone for the schedule. Default: UTC"
                        }
                    },
                    "additionalProperties": false
                },
                "signal_on_completion": {
                    "type": "boolean",
                    "description": "Send a synthetic user message back to the session when the run completes",
                    "default": true
                }
            },
            "required": ["tool", "args"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "spawn_background requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let tool_name = match arguments.get("tool").and_then(|v| v.as_str()) {
            Some(name) if !name.trim().is_empty() => name.trim(),
            _ => return ToolExecutionResult::tool_error("Missing required parameter: tool"),
        };
        let tool_args = match arguments.get("args") {
            Some(args) if args.is_object() => args.clone(),
            _ => {
                return ToolExecutionResult::tool_error(
                    "Missing required parameter: args (object expected)",
                );
            }
        };
        let signal_on_completion = arguments
            .get("signal_on_completion")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let schedule_request = match parse_background_schedule(&arguments) {
            Ok(schedule) => schedule,
            Err(message) => return ToolExecutionResult::tool_error(message),
        };

        let Some(tool_registry) = &context.tool_registry else {
            return ToolExecutionResult::tool_error(
                "Tool registry not available in this context. spawn_background requires worker-side tool execution.",
            );
        };

        let Some(tool) = tool_registry.get(tool_name).cloned() else {
            return ToolExecutionResult::tool_error(format!("Unknown tool: {tool_name}"));
        };
        if tool_name == self.name() {
            return ToolExecutionResult::tool_error(
                "spawn_background cannot target itself recursively",
            );
        }
        if tool.hints().supports_background != Some(true) {
            return ToolExecutionResult::tool_error(format!(
                "Tool does not support background execution: {tool_name}"
            ));
        }
        if tool.as_background_executable().is_none() {
            return ToolExecutionResult::tool_error(format!(
                "Tool declared background support but has no background executor: {tool_name}"
            ));
        }
        let title = arguments
            .get("title")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .unwrap_or_else(|| {
                tool.display_name()
                    .map(ToString::to_string)
                    .unwrap_or_else(|| format!("Background {tool_name}"))
            });

        if let Some(schedule_request) = schedule_request {
            let Some(schedule_store) = &context.schedule_store else {
                return ToolExecutionResult::tool_error(
                    "Schedule store not available in this context. Scheduled monitors require session schedules.",
                );
            };

            match schedule_store
                .count_active_schedules(context.session_id)
                .await
            {
                Ok(count) if count >= MAX_ACTIVE_SCHEDULES_PER_SESSION => {
                    return ToolExecutionResult::tool_error(format!(
                        "Maximum {MAX_ACTIVE_SCHEDULES_PER_SESSION} active schedules per session. Cancel an existing schedule first."
                    ));
                }
                Err(err) => return ToolExecutionResult::internal_error(err),
                _ => {}
            }

            let description = build_background_schedule_description(
                tool_name,
                &tool_args,
                &title,
                signal_on_completion,
            );

            return match schedule_store
                .create_schedule(
                    context.session_id,
                    description,
                    schedule_request.cron_expression.clone(),
                    schedule_request.scheduled_at,
                    schedule_request.timezone.clone(),
                )
                .await
            {
                Ok(schedule) => ToolExecutionResult::success(json!({
                    "created": true,
                    "status": "scheduled",
                    "title": title,
                    "tool": tool_name,
                    "signal_on_completion": signal_on_completion,
                    "schedule_id": schedule.id.to_string(),
                    "schedule_type": schedule.schedule_type,
                    "cron_expression": schedule.cron_expression,
                    "scheduled_at": schedule.scheduled_at,
                    "timezone": schedule.timezone,
                    "next_trigger_at": schedule.next_trigger_at,
                    "enabled": schedule.enabled
                })),
                Err(err) => ToolExecutionResult::internal_error(err),
            };
        }

        let Some(resource_registry) = &context.session_resource_registry else {
            return ToolExecutionResult::tool_error(
                "Session resource registry not available in this context",
            );
        };
        if context.file_store.is_none() {
            return ToolExecutionResult::tool_error(
                "Session file store not available in this context. spawn_background requires artifact persistence.",
            );
        }

        let background_run_permit = match ACTIVE_BACKGROUND_RUNS_PER_WORKER.try_acquire() {
            Ok(permit) => permit,
            Err(_) => {
                return ToolExecutionResult::tool_error(format!(
                    "Worker is already running the maximum {MAX_ACTIVE_BACKGROUND_RUNS_PER_WORKER} active background runs. Try again after an existing run finishes."
                ));
            }
        };

        let session_run_permit = match try_acquire_session_background_permit(context.session_id) {
            Ok(permit) => permit,
            Err(_) => {
                return ToolExecutionResult::tool_error(format!(
                    "Maximum {MAX_ACTIVE_BACKGROUND_RUNS_PER_SESSION} active background runs per session. Wait for an existing run to finish before starting another."
                ));
            }
        };

        let run_id = format!("bg_{}", uuid::Uuid::now_v7().simple());
        let artifact_dir = format!("/.background/{run_id}");
        let log_path = format!("{artifact_dir}/output.log");
        let result_path = format!("{artifact_dir}/result.json");
        let metadata = json!({
            "tool": tool_name,
            "status_text": "Queued",
            "signal_on_completion": signal_on_completion,
            "artifact_dir": artifact_dir,
            "log_path": log_path,
            "result_path": result_path,
        });

        if let Err(e) = resource_registry
            .register(RegisterSessionResource {
                session_id: context.session_id,
                resource_id: run_id.clone(),
                kind: "background_run".to_string(),
                display_name: title.clone(),
                status: SessionResourceStatus::Active,
                metadata,
            })
            .await
        {
            return ToolExecutionResult::internal_error_msg(format!(
                "Failed to register background run: {e}"
            ));
        }

        let background_context = context.clone().with_tool_registry(tool_registry.clone());
        let sink = Arc::new(SessionBackgroundSink::new(
            background_context.clone(),
            run_id.clone(),
            title.clone(),
            tool_name.to_string(),
            log_path.clone(),
            result_path.clone(),
            signal_on_completion,
        ));
        let run_id_for_task = run_id.clone();
        let tool_for_task = tool.clone();
        let tool_name_for_task = tool_name.to_string();

        tokio::spawn(async move {
            let _background_run_permit = background_run_permit;
            let _session_run_permit = session_run_permit;
            let _ = sink.status("Starting").await;
            let outcome = match tool_for_task.as_background_executable() {
                Some(background_tool) => {
                    background_tool
                        .execute_background(tool_args, background_context, sink.clone())
                        .await
                }
                None => Err(ToolExecutionResult::tool_error(format!(
                    "Tool declared background support but has no background executor: {}",
                    tool_name_for_task
                ))),
            };

            if let Err(err) = sink.finalize(outcome).await {
                tracing::warn!(
                    run_id = run_id_for_task,
                    error = %err,
                    "Background run finalization failed"
                );
            }
        });

        ToolExecutionResult::success(json!({
            "run_id": run_id,
            "resource_id": run_id,
            "title": title,
            "tool": tool_name,
            "status": "running",
            "signal_on_completion": signal_on_completion,
            "artifact_dir": artifact_dir,
            "log_path": log_path,
            "result_path": result_path
        }))
    }

    fn requires_context(&self) -> bool {
        true
    }
}

#[derive(Debug, Default)]
struct SessionBackgroundState {
    status_text: String,
    progress: Option<BackgroundProgress>,
    output_tail: String,
    output_log: String,
    output_log_chars: usize,
    output_log_truncated: bool,
}

const MAX_BACKGROUND_OUTPUT_LOG_CHARS: usize = 256 * 1024;

struct SessionBackgroundSink {
    context: ToolContext,
    run_id: String,
    display_name: String,
    tool_name: String,
    log_path: String,
    result_path: String,
    signal_on_completion: bool,
    state: tokio::sync::Mutex<SessionBackgroundState>,
}

impl SessionBackgroundSink {
    fn new(
        context: ToolContext,
        run_id: String,
        display_name: String,
        tool_name: String,
        log_path: String,
        result_path: String,
        signal_on_completion: bool,
    ) -> Self {
        Self {
            context,
            run_id,
            display_name,
            tool_name,
            log_path,
            result_path,
            signal_on_completion,
            state: tokio::sync::Mutex::new(SessionBackgroundState {
                status_text: "Queued".to_string(),
                ..Default::default()
            }),
        }
    }

    async fn finalize(
        &self,
        outcome: std::result::Result<BackgroundOutcome, ToolExecutionResult>,
    ) -> Result<()> {
        match outcome {
            Ok(outcome) => {
                let output_log = if let Some(raw_output) = &outcome.raw_output {
                    raw_output.clone()
                } else {
                    let state = self.state.lock().await;
                    Self::final_output_log(&state)
                };
                self.write_text_file(&self.log_path, &output_log).await?;
                let result_json = serde_json::to_string_pretty(&outcome.result)
                    .unwrap_or_else(|_| outcome.result.to_string());
                self.write_text_file(&self.result_path, &result_json)
                    .await?;

                let mut state = self.state.lock().await;
                state.status_text = "Completed".to_string();
                drop(state);
                self.update_resource(SessionResourceStatus::Completed, Some(&outcome.summary))
                    .await?;
                if self.signal_on_completion {
                    self.signal_session("completed", &outcome.summary).await?;
                }
            }
            Err(err) => {
                let message = match err {
                    ToolExecutionResult::ToolError(msg) => msg,
                    ToolExecutionResult::InternalError(inner) => inner.message,
                    ToolExecutionResult::ConnectionRequired { provider } => {
                        format!("Background tool requires connection setup: {provider}")
                    }
                    ToolExecutionResult::Success(_)
                    | ToolExecutionResult::SuccessWithImages { .. } => {
                        "Background run ended unexpectedly".to_string()
                    }
                };
                let output_log = {
                    let state = self.state.lock().await;
                    Self::final_output_log(&state)
                };
                self.write_text_file(&self.log_path, &output_log).await?;
                let error_json = serde_json::to_string_pretty(&json!({
                    "status": "failed",
                    "error": &message,
                }))
                .unwrap_or_else(|_| {
                    json!({
                        "status": "failed",
                        "error": &message,
                    })
                    .to_string()
                });
                self.write_text_file(&self.result_path, &error_json).await?;
                let mut state = self.state.lock().await;
                state.status_text = "Failed".to_string();
                drop(state);
                self.update_resource(SessionResourceStatus::Failed, Some(&message))
                    .await?;
                if self.signal_on_completion {
                    self.signal_session("failed", &message).await?;
                }
            }
        }

        Ok(())
    }

    async fn signal_session(&self, status: &str, summary: &str) -> Result<()> {
        let Some(platform_store) = &self.context.platform_store else {
            return Ok(());
        };
        let message = format!(
            "Background run {status}.\n- run_id: {}\n- title: {}\n- tool: {}\n- summary: {}\n- result_path: {}\n- log_path: {}",
            self.run_id,
            self.display_name,
            self.tool_name,
            summary,
            self.result_path,
            self.log_path
        );
        platform_store
            .send_message(self.context.session_id, &message)
            .await
    }

    async fn update_resource(
        &self,
        status: SessionResourceStatus,
        summary: Option<&str>,
    ) -> Result<()> {
        let Some(registry) = &self.context.session_resource_registry else {
            return Ok(());
        };
        let state = self.state.lock().await;
        let status_text = state.status_text.clone();
        let progress = state.progress.clone();
        let output_tail = state.output_tail.clone();
        drop(state);
        registry
            .register(RegisterSessionResource {
                session_id: self.context.session_id,
                resource_id: self.run_id.clone(),
                kind: "background_run".to_string(),
                display_name: self.display_name.clone(),
                status,
                metadata: json!({
                    "tool": self.tool_name,
                    "status_text": status_text,
                    "progress": progress,
                    "output_tail": output_tail,
                    "log_path": self.log_path,
                    "result_path": self.result_path,
                    "summary": summary,
                    "signal_on_completion": self.signal_on_completion,
                }),
            })
            .await?;
        Ok(())
    }

    async fn write_text_file(&self, path: &str, content: &str) -> Result<()> {
        let file_store = self.context.file_store.as_ref().ok_or_else(|| {
            anyhow::anyhow!(
                "background run {} cannot persist artifact {} because no session file store is configured",
                self.run_id,
                path
            )
        })?;

        ensure_directory(file_store.as_ref(), self.context.session_id, "/.background").await?;
        let run_dir = format!("/.background/{}", self.run_id);
        ensure_directory(file_store.as_ref(), self.context.session_id, &run_dir).await?;
        file_store
            .write_file(self.context.session_id, path, content, "text")
            .await?;
        Ok(())
    }
}

#[async_trait]
impl BackgroundEventSink for SessionBackgroundSink {
    async fn status(&self, message: &str) -> Result<()> {
        let mut state = self.state.lock().await;
        state.status_text = message.to_string();
        drop(state);
        self.update_resource(SessionResourceStatus::Active, None)
            .await
    }

    async fn output(&self, stream: &str, delta: &str) -> Result<()> {
        let mut state = self.state.lock().await;
        if !delta.is_empty() {
            let prefix = format!("[{stream}] ");
            state.output_tail.push_str(&prefix);
            state.output_tail.push_str(delta);
            Self::append_to_output_log(&mut state, &prefix, delta);
            if state.output_tail.chars().count() > 2048 {
                state.output_tail = state
                    .output_tail
                    .chars()
                    .rev()
                    .take(2048)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect();
            }
        }
        drop(state);
        self.update_resource(SessionResourceStatus::Active, None)
            .await
    }

    async fn progress(&self, progress: BackgroundProgress) -> Result<()> {
        let mut state = self.state.lock().await;
        state.progress = Some(progress);
        drop(state);
        self.update_resource(SessionResourceStatus::Active, None)
            .await
    }
}

impl SessionBackgroundSink {
    fn append_to_output_log(state: &mut SessionBackgroundState, prefix: &str, delta: &str) {
        if state.output_log_chars >= MAX_BACKGROUND_OUTPUT_LOG_CHARS {
            state.output_log_truncated = true;
            return;
        }

        let chunk = format!("{prefix}{delta}");
        let remaining = MAX_BACKGROUND_OUTPUT_LOG_CHARS - state.output_log_chars;
        let chunk_chars = chunk.chars().count();

        if chunk_chars <= remaining {
            state.output_log.push_str(&chunk);
            state.output_log_chars += chunk_chars;
            return;
        }

        let truncated_chunk: String = chunk.chars().take(remaining).collect();
        state.output_log.push_str(&truncated_chunk);
        state.output_log_chars += truncated_chunk.chars().count();
        state.output_log_truncated = true;
    }

    fn final_output_log(state: &SessionBackgroundState) -> String {
        if !state.output_log_truncated {
            return state.output_log.clone();
        }

        format!(
            "{}\n[system] background output truncated at {} characters\n",
            state.output_log, MAX_BACKGROUND_OUTPUT_LOG_CHARS
        )
    }
}

async fn ensure_directory(
    file_store: &dyn crate::traits::SessionFileSystem,
    session_id: crate::SessionId,
    path: &str,
) -> Result<()> {
    if let Some(entry) = file_store.stat_file(session_id, path).await? {
        if entry.is_directory {
            return Ok(());
        }
        return Err(anyhow::anyhow!("path exists but is not a directory: {path}").into());
    }
    let _ = file_store.create_directory(session_id, path).await?;
    Ok(())
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

    fn display_name(&self) -> Option<&str> {
        Some("Failing Tool")
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

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_readonly(true)
            .with_idempotent(true)
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
    use crate::platform_store::PlatformStore;
    use crate::session_file::{FileInfo, FileStat, SessionFile};
    use crate::session_resource::{SessionResourceEntry, SessionResourceFilter};
    use crate::traits::{SessionFileSystem, SessionResourceRegistry, SessionScheduleStore};
    use crate::typed_id::{HarnessId, SessionId};
    use crate::{AgentId, KeyInfo, PlatformMessage, SecretInfo};
    use async_trait::async_trait;
    use std::sync::{
        Arc as StdArc, Mutex,
        atomic::{AtomicBool, Ordering},
    };

    #[derive(Default)]
    struct TestBackgroundTool;

    #[async_trait]
    impl BackgroundExecutableTool for TestBackgroundTool {
        async fn execute_background(
            &self,
            arguments: Value,
            _context: ToolContext,
            sink: Arc<dyn BackgroundEventSink>,
        ) -> std::result::Result<BackgroundOutcome, ToolExecutionResult> {
            sink.status("Waiting for test result")
                .await
                .map_err(ToolExecutionResult::internal_error)?;
            sink.output("stdout", "hello from background")
                .await
                .map_err(ToolExecutionResult::internal_error)?;
            sink.progress(BackgroundProgress {
                current: Some(1),
                total: Some(1),
                unit: Some("step".to_string()),
                label: Some("done".to_string()),
            })
            .await
            .map_err(ToolExecutionResult::internal_error)?;

            Ok(BackgroundOutcome {
                summary: arguments["summary"].as_str().unwrap_or("done").to_string(),
                result: json!({"ok": true}),
                raw_output: None,
            })
        }
    }

    #[async_trait]
    impl Tool for TestBackgroundTool {
        fn name(&self) -> &str {
            "test_background"
        }

        fn display_name(&self) -> Option<&str> {
            Some("Test Background")
        }

        fn description(&self) -> &str {
            "test tool"
        }

        fn parameters_schema(&self) -> Value {
            json!({
                "type": "object",
                "properties": {
                    "summary": { "type": "string" }
                }
            })
        }

        async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
            ToolExecutionResult::tool_error("foreground unsupported")
        }

        fn hints(&self) -> ToolHints {
            ToolHints::default().with_supports_background(true)
        }

        fn as_background_executable(&self) -> Option<&dyn BackgroundExecutableTool> {
            Some(self)
        }
    }

    #[derive(Default)]
    struct TestFailingBackgroundTool;

    #[async_trait]
    impl BackgroundExecutableTool for TestFailingBackgroundTool {
        async fn execute_background(
            &self,
            _arguments: Value,
            _context: ToolContext,
            sink: Arc<dyn BackgroundEventSink>,
        ) -> std::result::Result<BackgroundOutcome, ToolExecutionResult> {
            sink.status("Running failing test")
                .await
                .map_err(ToolExecutionResult::internal_error)?;
            sink.output("stderr", "background failed")
                .await
                .map_err(ToolExecutionResult::internal_error)?;
            Err(ToolExecutionResult::tool_error("boom"))
        }
    }

    #[async_trait]
    impl Tool for TestFailingBackgroundTool {
        fn name(&self) -> &str {
            "test_background_fail"
        }

        fn display_name(&self) -> Option<&str> {
            Some("Test Background Fail")
        }

        fn description(&self) -> &str {
            "failing background test tool"
        }

        fn parameters_schema(&self) -> Value {
            json!({
                "type": "object",
                "properties": {}
            })
        }

        async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
            ToolExecutionResult::tool_error("foreground unsupported")
        }

        fn hints(&self) -> ToolHints {
            ToolHints::default().with_supports_background(true)
        }

        fn as_background_executable(&self) -> Option<&dyn BackgroundExecutableTool> {
            Some(self)
        }
    }

    #[derive(Default)]
    struct TestLargeOutputBackgroundTool;

    #[async_trait]
    impl BackgroundExecutableTool for TestLargeOutputBackgroundTool {
        async fn execute_background(
            &self,
            _arguments: Value,
            _context: ToolContext,
            sink: Arc<dyn BackgroundEventSink>,
        ) -> std::result::Result<BackgroundOutcome, ToolExecutionResult> {
            let large_chunk = "x".repeat(MAX_BACKGROUND_OUTPUT_LOG_CHARS + 4096);
            sink.output("stdout", &large_chunk)
                .await
                .map_err(ToolExecutionResult::internal_error)?;
            Ok(BackgroundOutcome {
                summary: "large output complete".to_string(),
                result: json!({"ok": true}),
                raw_output: None,
            })
        }
    }

    #[async_trait]
    impl Tool for TestLargeOutputBackgroundTool {
        fn name(&self) -> &str {
            "test_background_large_output"
        }

        fn display_name(&self) -> Option<&str> {
            Some("Test Background Large Output")
        }

        fn description(&self) -> &str {
            "background test tool with huge output"
        }

        fn parameters_schema(&self) -> Value {
            json!({
                "type": "object",
                "properties": {}
            })
        }

        async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
            ToolExecutionResult::tool_error("foreground unsupported")
        }

        fn hints(&self) -> ToolHints {
            ToolHints::default().with_supports_background(true)
        }

        fn as_background_executable(&self) -> Option<&dyn BackgroundExecutableTool> {
            Some(self)
        }
    }

    struct BlockingBackgroundTool {
        release: StdArc<AtomicBool>,
    }

    #[async_trait]
    impl BackgroundExecutableTool for BlockingBackgroundTool {
        async fn execute_background(
            &self,
            _arguments: Value,
            _context: ToolContext,
            sink: Arc<dyn BackgroundEventSink>,
        ) -> std::result::Result<BackgroundOutcome, ToolExecutionResult> {
            sink.status("Blocking until released")
                .await
                .map_err(ToolExecutionResult::internal_error)?;
            while !self.release.load(Ordering::SeqCst) {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
            Ok(BackgroundOutcome {
                summary: "released".to_string(),
                result: json!({"ok": true}),
                raw_output: None,
            })
        }
    }

    #[async_trait]
    impl Tool for BlockingBackgroundTool {
        fn name(&self) -> &str {
            "test_background_blocking"
        }

        fn display_name(&self) -> Option<&str> {
            Some("Test Background Blocking")
        }

        fn description(&self) -> &str {
            "background test tool that waits for test release"
        }

        fn parameters_schema(&self) -> Value {
            json!({
                "type": "object",
                "properties": {}
            })
        }

        async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
            ToolExecutionResult::tool_error("foreground unsupported")
        }

        fn hints(&self) -> ToolHints {
            ToolHints::default().with_supports_background(true)
        }

        fn as_background_executable(&self) -> Option<&dyn BackgroundExecutableTool> {
            Some(self)
        }
    }

    #[derive(Default)]
    struct TestSessionResourceRegistry {
        entries: Mutex<HashMap<String, SessionResourceEntry>>,
    }

    #[async_trait]
    impl crate::traits::SessionResourceRegistry for TestSessionResourceRegistry {
        async fn register(
            &self,
            entry: RegisterSessionResource,
        ) -> crate::Result<SessionResourceEntry> {
            let stored = SessionResourceEntry {
                resource_id: entry.resource_id.clone(),
                session_id: entry.session_id,
                kind: entry.kind,
                display_name: entry.display_name,
                status: entry.status,
                metadata: entry.metadata,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            };
            self.entries
                .lock()
                .unwrap()
                .insert(entry.resource_id, stored.clone());
            Ok(stored)
        }

        async fn update_status(
            &self,
            _session_id: SessionId,
            resource_id: &str,
            status: SessionResourceStatus,
        ) -> crate::Result<Option<SessionResourceEntry>> {
            let mut entries = self.entries.lock().unwrap();
            if let Some(entry) = entries.get_mut(resource_id) {
                entry.status = status;
                entry.updated_at = chrono::Utc::now();
                return Ok(Some(entry.clone()));
            }
            Ok(None)
        }

        async fn get(
            &self,
            _session_id: SessionId,
            resource_id: &str,
        ) -> crate::Result<Option<SessionResourceEntry>> {
            Ok(self.entries.lock().unwrap().get(resource_id).cloned())
        }

        async fn list(
            &self,
            session_id: SessionId,
            filter: Option<&SessionResourceFilter>,
        ) -> crate::Result<Vec<SessionResourceEntry>> {
            Ok(self
                .entries
                .lock()
                .unwrap()
                .values()
                .filter(|entry| entry.session_id == session_id)
                .filter(|entry| {
                    filter.is_none_or(|filter| {
                        filter.kind.as_ref().is_none_or(|kind| &entry.kind == kind)
                            && filter.status.is_none_or(|status| entry.status == status)
                    })
                })
                .cloned()
                .collect())
        }

        async fn deregister(
            &self,
            _session_id: SessionId,
            resource_id: &str,
        ) -> crate::Result<bool> {
            Ok(self.entries.lock().unwrap().remove(resource_id).is_some())
        }
    }

    #[derive(Default)]
    struct TestFileStore {
        files: Mutex<HashMap<String, SessionFile>>,
    }

    #[async_trait]
    impl crate::traits::SessionFileSystem for TestFileStore {
        async fn read_file(
            &self,
            _session_id: SessionId,
            path: &str,
        ) -> crate::Result<Option<SessionFile>> {
            Ok(self.files.lock().unwrap().get(path).cloned())
        }

        async fn write_file(
            &self,
            session_id: SessionId,
            path: &str,
            content: &str,
            encoding: &str,
        ) -> crate::Result<SessionFile> {
            let now = chrono::Utc::now();
            let file = SessionFile {
                id: uuid::Uuid::now_v7(),
                session_id: session_id.uuid(),
                path: path.to_string(),
                name: FileInfo::name_from_path(path),
                content: Some(content.to_string()),
                encoding: encoding.to_string(),
                is_directory: false,
                is_readonly: false,
                size_bytes: content.len() as i64,
                created_at: now,
                updated_at: now,
            };
            self.files
                .lock()
                .unwrap()
                .insert(path.to_string(), file.clone());
            Ok(file)
        }

        async fn delete_file(
            &self,
            _session_id: SessionId,
            _path: &str,
            _recursive: bool,
        ) -> crate::Result<bool> {
            Ok(false)
        }

        async fn list_directory(
            &self,
            _session_id: SessionId,
            _path: &str,
        ) -> crate::Result<Vec<FileInfo>> {
            Ok(Vec::new())
        }

        async fn stat_file(
            &self,
            _session_id: SessionId,
            path: &str,
        ) -> crate::Result<Option<FileStat>> {
            let file = self.files.lock().unwrap().get(path).cloned();
            Ok(file.map(|entry| FileStat {
                path: entry.path,
                name: entry.name,
                is_directory: entry.is_directory,
                is_readonly: entry.is_readonly,
                size_bytes: entry.size_bytes,
                created_at: entry.created_at,
                updated_at: entry.updated_at,
            }))
        }

        async fn grep_files(
            &self,
            _session_id: SessionId,
            _pattern: &str,
            _path_pattern: Option<&str>,
        ) -> crate::Result<Vec<crate::session_file::GrepMatch>> {
            Ok(Vec::new())
        }

        async fn create_directory(
            &self,
            session_id: SessionId,
            path: &str,
        ) -> crate::Result<FileInfo> {
            let now = chrono::Utc::now();
            let id = uuid::Uuid::now_v7();
            let dir = SessionFile {
                id,
                session_id: session_id.uuid(),
                path: path.to_string(),
                name: FileInfo::name_from_path(path),
                content: None,
                encoding: "text".to_string(),
                is_directory: true,
                is_readonly: false,
                size_bytes: 0,
                created_at: now,
                updated_at: now,
            };
            self.files.lock().unwrap().insert(path.to_string(), dir);
            Ok(FileInfo {
                id,
                session_id: session_id.uuid(),
                path: path.to_string(),
                name: FileInfo::name_from_path(path),
                is_directory: true,
                is_readonly: false,
                size_bytes: 0,
                created_at: now,
                updated_at: now,
            })
        }
    }

    #[derive(Default)]
    struct TestPlatformStore {
        sent_messages: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl PlatformStore for TestPlatformStore {
        async fn list_harnesses(&self) -> crate::Result<Vec<crate::Harness>> {
            Ok(Vec::new())
        }
        async fn get_harness(&self, _id: HarnessId) -> crate::Result<Option<crate::Harness>> {
            Ok(None)
        }
        async fn create_harness(
            &self,
            _name: &str,
            _display_name: Option<&str>,
            _description: Option<&str>,
            _system_prompt: &str,
            _parent_harness_id: Option<HarnessId>,
            _capabilities: &[String],
        ) -> crate::Result<crate::Harness> {
            unreachable!()
        }
        async fn update_harness(
            &self,
            _id: HarnessId,
            _name: Option<&str>,
            _display_name: Option<&str>,
            _description: Option<&str>,
            _system_prompt: Option<&str>,
            _parent_harness_id: Option<Option<HarnessId>>,
        ) -> crate::Result<crate::Harness> {
            unreachable!()
        }
        async fn delete_harness(&self, _id: HarnessId) -> crate::Result<()> {
            Ok(())
        }
        async fn copy_harness(
            &self,
            _id: HarnessId,
            _new_name: Option<&str>,
        ) -> crate::Result<crate::Harness> {
            unreachable!()
        }
        async fn list_agents(&self) -> crate::Result<Vec<crate::Agent>> {
            Ok(Vec::new())
        }
        async fn get_agent_by_id(&self, _id: AgentId) -> crate::Result<Option<crate::Agent>> {
            Ok(None)
        }
        async fn create_agent(
            &self,
            _name: &str,
            _display_name: Option<&str>,
            _description: Option<&str>,
            _system_prompt: &str,
            _capabilities: &[String],
        ) -> crate::Result<crate::Agent> {
            unreachable!()
        }
        async fn update_agent(
            &self,
            _id: AgentId,
            _name: Option<&str>,
            _display_name: Option<&str>,
            _description: Option<&str>,
            _system_prompt: Option<&str>,
        ) -> crate::Result<crate::Agent> {
            unreachable!()
        }
        async fn delete_agent(&self, _id: AgentId) -> crate::Result<()> {
            Ok(())
        }
        async fn list_apps(
            &self,
            _search: Option<&str>,
            _include_archived: bool,
        ) -> crate::Result<Vec<crate::App>> {
            Ok(Vec::new())
        }
        async fn get_app(&self, _id: crate::AppId) -> crate::Result<Option<crate::App>> {
            Ok(None)
        }
        async fn create_app(
            &self,
            _name: &str,
            _description: Option<&str>,
            _harness_id: HarnessId,
            _agent_id: Option<AgentId>,
            _agent_identity_id: Option<crate::AgentIdentityId>,
            _channel_type: Option<crate::ChannelType>,
            _channel_config: Option<&serde_json::Value>,
        ) -> crate::Result<crate::App> {
            unreachable!()
        }
        async fn update_app(
            &self,
            _id: crate::AppId,
            _name: Option<&str>,
            _description: Option<&str>,
            _harness_id: Option<HarnessId>,
            _agent_id: Option<AgentId>,
            _agent_identity_id: Option<Option<crate::AgentIdentityId>>,
        ) -> crate::Result<crate::App> {
            unreachable!()
        }
        async fn delete_app(&self, _id: crate::AppId) -> crate::Result<()> {
            Ok(())
        }
        async fn destroy_app(&self, _id: crate::AppId) -> crate::Result<()> {
            Ok(())
        }
        async fn publish_app(&self, _id: crate::AppId) -> crate::Result<crate::App> {
            unreachable!()
        }
        async fn unpublish_app(&self, _id: crate::AppId) -> crate::Result<crate::App> {
            unreachable!()
        }
        async fn add_app_channel(
            &self,
            _app_id: crate::AppId,
            _channel_type: crate::ChannelType,
            _channel_config: Option<&serde_json::Value>,
            _enabled: Option<bool>,
        ) -> crate::Result<crate::AppChannel> {
            unreachable!()
        }
        async fn update_app_channel(
            &self,
            _app_id: crate::AppId,
            _channel_id: crate::AppChannelId,
            _channel_type: Option<crate::ChannelType>,
            _channel_config: Option<&serde_json::Value>,
            _enabled: Option<bool>,
        ) -> crate::Result<crate::AppChannel> {
            unreachable!()
        }
        async fn delete_app_channel(
            &self,
            _app_id: crate::AppId,
            _channel_id: crate::AppChannelId,
        ) -> crate::Result<()> {
            Ok(())
        }
        async fn list_sessions(
            &self,
            _limit: Option<usize>,
            _agent_id: Option<AgentId>,
        ) -> crate::Result<Vec<crate::Session>> {
            Ok(Vec::new())
        }
        async fn create_session(
            &self,
            _harness_id: HarnessId,
            _agent_id: Option<AgentId>,
            _title: Option<&str>,
            _locale: Option<&str>,
            _blueprint_id: Option<&str>,
            _blueprint_config: Option<&serde_json::Value>,
        ) -> crate::Result<crate::Session> {
            unreachable!()
        }
        async fn get_session_by_id(&self, _id: SessionId) -> crate::Result<Option<crate::Session>> {
            Ok(None)
        }
        async fn get_session_context_report(
            &self,
            id: SessionId,
        ) -> crate::Result<crate::SessionContextReport> {
            Ok(crate::SessionContextReport {
                session_id: id.to_string(),
                model: "llmsim".to_string(),
                context_window_tokens: None,
                estimated_input_tokens: 0,
                sections: vec![],
                contributions: vec![],
                cumulative_usage: None,
            })
        }
        async fn set_subagent_metadata(
            &self,
            _session_id: SessionId,
            _parent_session_id: SessionId,
            _subagent_name: &str,
            _subagent_task: &str,
            _subagent_status: crate::session::SubagentStatus,
        ) -> crate::Result<crate::Session> {
            unreachable!()
        }
        async fn delete_session(&self, _id: SessionId) -> crate::Result<()> {
            Ok(())
        }
        async fn send_message(&self, _session_id: SessionId, content: &str) -> crate::Result<()> {
            self.sent_messages.lock().unwrap().push(content.to_string());
            Ok(())
        }
        async fn get_messages(
            &self,
            _session_id: SessionId,
            _limit: Option<usize>,
        ) -> crate::Result<Vec<PlatformMessage>> {
            Ok(Vec::new())
        }
        async fn wait_for_idle(
            &self,
            _session_id: SessionId,
            _timeout_secs: Option<u64>,
        ) -> crate::Result<String> {
            Ok("idle".to_string())
        }
        async fn list_capabilities(
            &self,
            _search: Option<&str>,
        ) -> crate::Result<Vec<crate::CapabilityInfo>> {
            Ok(Vec::new())
        }
        fn base_url(&self) -> &str {
            "http://localhost:9300"
        }
    }

    #[derive(Default)]
    struct NoopStorageStore;

    #[derive(Default)]
    struct TestScheduleStore {
        schedules: Mutex<Vec<crate::session_schedule::SessionSchedule>>,
    }

    #[async_trait]
    impl crate::traits::SessionScheduleStore for TestScheduleStore {
        async fn create_schedule(
            &self,
            session_id: SessionId,
            description: String,
            cron_expression: Option<String>,
            scheduled_at: Option<chrono::DateTime<chrono::Utc>>,
            timezone: String,
        ) -> crate::Result<crate::session_schedule::SessionSchedule> {
            let schedule = crate::session_schedule::SessionSchedule {
                id: crate::typed_id::ScheduleId::new(),
                session_id,
                owner_principal_id: crate::PrincipalId::from_seed(1),
                resolved_owner_user_id: None,
                owner: None,
                effective_owner: None,
                description,
                cron_expression: cron_expression.clone(),
                scheduled_at,
                timezone,
                enabled: true,
                schedule_type: crate::session_schedule::SessionSchedule::derive_type(
                    &cron_expression,
                ),
                next_trigger_at: Some(chrono::Utc::now() + chrono::Duration::minutes(10)),
                last_triggered_at: None,
                trigger_count: 0,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            };
            self.schedules.lock().unwrap().push(schedule.clone());
            Ok(schedule)
        }

        async fn cancel_schedule(
            &self,
            _session_id: SessionId,
            schedule_id: crate::ScheduleId,
        ) -> crate::Result<crate::session_schedule::SessionSchedule> {
            let mut schedules = self.schedules.lock().unwrap();
            let schedule = schedules
                .iter_mut()
                .find(|schedule| schedule.id == schedule_id)
                .ok_or_else(|| crate::AgentLoopError::tool("Schedule not found".to_string()))?;
            schedule.enabled = false;
            Ok(schedule.clone())
        }

        async fn list_schedules(
            &self,
            session_id: SessionId,
        ) -> crate::Result<Vec<crate::session_schedule::SessionSchedule>> {
            Ok(self
                .schedules
                .lock()
                .unwrap()
                .iter()
                .filter(|schedule| schedule.session_id == session_id)
                .cloned()
                .collect())
        }

        async fn count_active_schedules(&self, session_id: SessionId) -> crate::Result<u32> {
            Ok(self
                .schedules
                .lock()
                .unwrap()
                .iter()
                .filter(|schedule| schedule.session_id == session_id && schedule.enabled)
                .count() as u32)
        }
    }

    #[async_trait]
    impl crate::traits::SessionStorageStore for NoopStorageStore {
        async fn set_value(
            &self,
            _session_id: SessionId,
            _key: &str,
            _value: &str,
        ) -> crate::Result<()> {
            Ok(())
        }
        async fn get_value(
            &self,
            _session_id: SessionId,
            _key: &str,
        ) -> crate::Result<Option<String>> {
            Ok(None)
        }
        async fn delete_value(&self, _session_id: SessionId, _key: &str) -> crate::Result<bool> {
            Ok(false)
        }
        async fn list_keys(&self, _session_id: SessionId) -> crate::Result<Vec<KeyInfo>> {
            Ok(Vec::new())
        }
        async fn set_secret(
            &self,
            _session_id: SessionId,
            _name: &str,
            _value: &str,
        ) -> crate::Result<()> {
            Ok(())
        }
        async fn get_secret(
            &self,
            _session_id: SessionId,
            _name: &str,
        ) -> crate::Result<Option<String>> {
            Ok(None)
        }
        async fn delete_secret(&self, _session_id: SessionId, _name: &str) -> crate::Result<bool> {
            Ok(false)
        }
        async fn list_secrets(&self, _session_id: SessionId) -> crate::Result<Vec<SecretInfo>> {
            Ok(Vec::new())
        }
    }

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

        // Tool error (packaged as {"error": "..."} in result field, also sets error)
        let result = ToolExecutionResult::tool_error("Invalid input");
        let tool_result = result.into_tool_result("call_2", "test_tool");
        assert_eq!(tool_result.error.as_deref(), Some("Invalid input"));
        assert_eq!(
            tool_result.result.unwrap(),
            serde_json::json!({"error": "Invalid input"})
        );

        // Internal error (packaged as {"error": "..."} with generic message)
        let result = ToolExecutionResult::internal_error_msg("Secret database error");
        let tool_result = result.into_tool_result("call_3", "test_tool");
        assert_eq!(
            tool_result.error.as_deref(),
            Some("An internal error occurred while executing the tool")
        );
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

    #[test]
    fn test_tool_display_name_in_definition() {
        // GetCurrentTimeTool has display_name "Get Current Time"
        let tool = GetCurrentTimeTool;
        assert_eq!(tool.display_name(), Some("Get Current Time"));

        let def = tool.to_definition();
        assert_eq!(def.display_name(), Some("Get Current Time"));
    }

    #[test]
    fn test_success_with_raw_output_object_preserves_shape() {
        let res = ToolExecutionResult::success_with_raw_output(
            serde_json::json!({"stdout": "hello"}),
            "raw stdout bytes".to_string(),
        );
        let tr = res.into_tool_result("call_1", "demo");
        assert_eq!(tr.result.as_ref().unwrap()["stdout"], "hello");
        assert!(
            tr.result
                .as_ref()
                .unwrap()
                .as_object()
                .unwrap()
                .get("_raw_output")
                .is_none(),
            "sidecar key must not leak to the LLM-visible result"
        );
        assert_eq!(tr.raw_output.as_deref(), Some("raw stdout bytes"));
    }

    #[test]
    fn test_success_with_raw_output_scalar_unwraps_to_string() {
        let res = ToolExecutionResult::success_with_raw_output(
            "compact summary".to_string(),
            "full output bytes".to_string(),
        );
        let tr = res.into_tool_result("call_1", "demo");
        assert_eq!(
            tr.result,
            Some(serde_json::Value::String("compact summary".into()))
        );
        assert_eq!(tr.raw_output.as_deref(), Some("full output bytes"));
    }

    #[test]
    fn test_success_result_with_raw_output_scalar_key_is_not_unwrapped() {
        let res = ToolExecutionResult::success(
            serde_json::json!({"_raw_output_scalar": "user_value", "kept": true}),
        );
        let tr = res.into_tool_result("call_1", "demo");
        assert_eq!(
            tr.result,
            Some(serde_json::json!({"_raw_output_scalar": "user_value", "kept": true}))
        );
        assert_eq!(tr.raw_output, None);
    }

    #[test]
    fn test_success_result_with_only_raw_output_scalar_key_is_not_unwrapped() {
        // Single-key object with _raw_output_scalar must not be mistaken for a
        // success_with_raw_output carrier when raw_output is absent.
        let res = ToolExecutionResult::success(serde_json::json!({"_raw_output_scalar": "v"}));
        let tr = res.into_tool_result("call_1", "demo");
        assert_eq!(
            tr.result,
            Some(serde_json::json!({"_raw_output_scalar": "v"}))
        );
        assert_eq!(tr.raw_output, None);
    }

    #[test]
    fn test_echo_tool_display_name() {
        let tool = EchoTool;
        assert_eq!(tool.display_name(), Some("Echo"));

        let def = tool.to_definition();
        assert_eq!(def.display_name(), Some("Echo"));
    }

    #[test]
    fn test_all_default_tools_have_display_names() {
        let registry = ToolRegistry::with_defaults();
        let definitions = registry.tool_definitions();

        for def in &definitions {
            assert!(
                def.display_name().is_some(),
                "Tool '{}' should have a display_name",
                def.name()
            );
        }
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

        let ToolDefinition::Builtin(builtin) = def else {
            panic!("expected Builtin variant");
        };
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
        // spawn_background is contributed by the background_execution
        // capability (auto-activated) — it must NOT be in defaults.
        assert!(
            !registry.has("spawn_background"),
            "spawn_background must NOT be in defaults — it comes from the \
             background_execution capability"
        );
        assert!(
            registry.has("report_progress"),
            "should have report_progress"
        );

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
        assert!(registry.has("edit_file"), "should have edit_file");
        assert!(registry.has("list_directory"), "should have list_directory");
        assert!(registry.has("grep_files"), "should have grep_files");
        assert!(registry.has("delete_file"), "should have delete_file");
        assert!(registry.has("stat_file"), "should have stat_file");

        // WebFetch capability tools
        assert!(registry.has("web_fetch"), "should have web_fetch");

        // Total count: 19 - 1 (spawn_background, moved to capability) = 18
        assert_eq!(registry.len(), 18, "should have 18 default tools");
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

    /// Regression: with_defaults() must NOT include capability-provided tools like
    /// 'bash'. These tools come from capabilities and must be registered separately.
    /// If bash were in defaults, the harness capability fallback would be masked.
    #[test]
    fn test_with_defaults_excludes_capability_only_tools() {
        let registry = ToolRegistry::with_defaults();

        // bash comes from virtual_bash capability, not defaults
        assert!(
            !registry.has("bash"),
            "bash must not be in defaults — it comes from virtual_bash capability"
        );
        // kv_store/secret_store come from session_storage capability
        assert!(
            !registry.has("kv_store"),
            "kv_store must not be in defaults — it comes from session_storage capability"
        );
        // spawn_background comes from background_execution capability and is
        // auto-activated by `collect_capabilities_with_configs` when a
        // background-capable tool is present (see EVE-501).
        assert!(
            !registry.has("spawn_background"),
            "spawn_background must not be in defaults — it comes from the \
             background_execution capability (auto-activated by tool hints)"
        );
    }

    #[tokio::test]
    async fn test_spawn_background_executes_and_signals_session() {
        let session_id = SessionId::new();
        let resource_registry = Arc::new(TestSessionResourceRegistry::default());
        let file_store = Arc::new(TestFileStore::default());
        let platform_store = Arc::new(TestPlatformStore::default());
        let storage_store = Arc::new(NoopStorageStore);
        let tool_registry = ToolRegistry::builder()
            .tool(SpawnBackgroundTool)
            .tool(TestBackgroundTool)
            .build();

        let context = ToolContext::with_stores(session_id, file_store.clone(), storage_store)
            .with_tool_registry(Arc::new(tool_registry))
            .with_platform_store(platform_store.clone())
            .with_session_resource_registry(resource_registry.clone());

        let tool = SpawnBackgroundTool;
        let result = tool
            .execute_with_context(
                json!({
                    "tool": "test_background",
                    "args": { "summary": "Background complete" }
                }),
                &context,
            )
            .await;

        let ToolExecutionResult::Success(value) = result else {
            panic!("spawn_background should succeed");
        };
        let run_id = value["run_id"].as_str().unwrap().to_string();

        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                let entry = resource_registry
                    .get(session_id, &run_id)
                    .await
                    .unwrap()
                    .expect("resource exists");
                if entry.status == SessionResourceStatus::Completed {
                    break entry;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("background run should complete");

        let messages = platform_store.sent_messages.lock().unwrap().clone();
        assert_eq!(messages.len(), 1);
        assert!(messages[0].contains("Background run completed"));
        assert!(messages[0].contains(&run_id));

        let log_file = file_store
            .read_file(session_id, &format!("/.background/{run_id}/output.log"))
            .await
            .unwrap()
            .expect("log file");
        assert!(
            log_file
                .content
                .as_deref()
                .unwrap_or_default()
                .contains("hello from background")
        );
    }

    #[tokio::test]
    async fn test_spawn_background_persists_failure_artifacts() {
        let session_id = SessionId::new();
        let resource_registry = Arc::new(TestSessionResourceRegistry::default());
        let file_store = Arc::new(TestFileStore::default());
        let storage_store = Arc::new(NoopStorageStore);
        let tool_registry = ToolRegistry::builder()
            .tool(SpawnBackgroundTool)
            .tool(TestFailingBackgroundTool)
            .build();

        let context = ToolContext::with_stores(session_id, file_store.clone(), storage_store)
            .with_tool_registry(Arc::new(tool_registry))
            .with_session_resource_registry(resource_registry.clone());

        let result = SpawnBackgroundTool
            .execute_with_context(
                json!({
                    "tool": "test_background_fail",
                    "args": {}
                }),
                &context,
            )
            .await;

        let ToolExecutionResult::Success(value) = result else {
            panic!("spawn_background should succeed");
        };
        let run_id = value["run_id"].as_str().unwrap().to_string();

        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                let entry = resource_registry
                    .get(session_id, &run_id)
                    .await
                    .unwrap()
                    .expect("resource exists");
                if entry.status == SessionResourceStatus::Failed {
                    break entry;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("background run should fail");

        let log_file = file_store
            .read_file(session_id, &format!("/.background/{run_id}/output.log"))
            .await
            .unwrap()
            .expect("log file");
        assert!(
            log_file
                .content
                .as_deref()
                .unwrap_or_default()
                .contains("background failed")
        );

        let result_file = file_store
            .read_file(session_id, &format!("/.background/{run_id}/result.json"))
            .await
            .unwrap()
            .expect("result file");
        let result_json: Value =
            serde_json::from_str(result_file.content.as_deref().unwrap_or_default())
                .expect("valid json");
        assert_eq!(result_json["status"], "failed");
        assert_eq!(result_json["error"], "boom");
    }

    #[tokio::test]
    async fn test_spawn_background_rejects_when_session_active_run_limit_reached() {
        let session_id = SessionId::new();
        let resource_registry = Arc::new(TestSessionResourceRegistry::default());
        let file_store = Arc::new(TestFileStore::default());
        let storage_store = Arc::new(NoopStorageStore);
        let release = StdArc::new(AtomicBool::new(false));
        let tool_registry = ToolRegistry::builder()
            .tool(SpawnBackgroundTool)
            .tool(BlockingBackgroundTool {
                release: release.clone(),
            })
            .build();

        let context = ToolContext::with_stores(session_id, file_store, storage_store)
            .with_tool_registry(Arc::new(tool_registry))
            .with_session_resource_registry(resource_registry.clone());

        let mut run_ids = Vec::new();
        for _ in 0..MAX_ACTIVE_BACKGROUND_RUNS_PER_SESSION {
            let result = SpawnBackgroundTool
                .execute_with_context(
                    json!({
                        "tool": "test_background_blocking",
                        "args": {}
                    }),
                    &context,
                )
                .await;

            let ToolExecutionResult::Success(value) = result else {
                panic!("background run below the session limit should start");
            };
            run_ids.push(value["run_id"].as_str().unwrap().to_string());
        }

        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                let active_runs = resource_registry
                    .list(
                        session_id,
                        Some(&SessionResourceFilter {
                            kind: Some("background_run".to_string()),
                            status: Some(SessionResourceStatus::Active),
                        }),
                    )
                    .await
                    .unwrap();
                if active_runs.len() == MAX_ACTIVE_BACKGROUND_RUNS_PER_SESSION {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("background runs should become active");

        let result = SpawnBackgroundTool
            .execute_with_context(
                json!({
                    "tool": "test_background_blocking",
                    "args": {}
                }),
                &context,
            )
            .await;

        let ToolExecutionResult::ToolError(message) = result else {
            release.store(true, Ordering::SeqCst);
            panic!("spawn_background should reject once the session limit is reached");
        };
        assert!(message.contains("active background runs per session"));

        release.store(true, Ordering::SeqCst);
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            for run_id in run_ids {
                loop {
                    let entry = resource_registry
                        .get(session_id, &run_id)
                        .await
                        .unwrap()
                        .expect("resource exists");
                    if entry.status == SessionResourceStatus::Completed {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                }
            }
        })
        .await
        .expect("blocking background runs should complete after release");

        assert!(
            !has_session_background_permits(session_id),
            "completed background runs should prune their per-session permit cache entry"
        );
    }

    #[tokio::test]
    async fn test_spawn_background_requires_file_store() {
        let session_id = SessionId::new();
        let resource_registry = Arc::new(TestSessionResourceRegistry::default());
        let storage_store = Arc::new(NoopStorageStore);
        let tool_registry = ToolRegistry::builder()
            .tool(SpawnBackgroundTool)
            .tool(TestBackgroundTool)
            .build();

        let context = ToolContext::with_storage_store(session_id, storage_store)
            .with_tool_registry(Arc::new(tool_registry))
            .with_session_resource_registry(resource_registry);

        let result = SpawnBackgroundTool
            .execute_with_context(
                json!({
                    "tool": "test_background",
                    "args": {}
                }),
                &context,
            )
            .await;

        let ToolExecutionResult::ToolError(message) = result else {
            panic!("spawn_background should reject missing file store");
        };
        assert!(message.contains("Session file store not available"));
    }

    #[tokio::test]
    async fn test_spawn_background_caps_output_log_size() {
        let session_id = SessionId::new();
        let resource_registry = Arc::new(TestSessionResourceRegistry::default());
        let file_store = Arc::new(TestFileStore::default());
        let storage_store = Arc::new(NoopStorageStore);
        let tool_registry = ToolRegistry::builder()
            .tool(SpawnBackgroundTool)
            .tool(TestLargeOutputBackgroundTool)
            .build();

        let context = ToolContext::with_stores(session_id, file_store.clone(), storage_store)
            .with_tool_registry(Arc::new(tool_registry))
            .with_session_resource_registry(resource_registry.clone());

        let result = SpawnBackgroundTool
            .execute_with_context(
                json!({
                    "tool": "test_background_large_output",
                    "args": {}
                }),
                &context,
            )
            .await;

        let ToolExecutionResult::Success(value) = result else {
            panic!("spawn_background should succeed");
        };
        let run_id = value["run_id"].as_str().unwrap().to_string();

        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                let entry = resource_registry
                    .get(session_id, &run_id)
                    .await
                    .unwrap()
                    .expect("resource exists");
                if entry.status == SessionResourceStatus::Completed {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("background run should complete");

        let log_content = file_store
            .read_file(session_id, &format!("/.background/{run_id}/output.log"))
            .await
            .unwrap()
            .expect("log file")
            .content
            .unwrap_or_default();

        assert!(log_content.contains("[system] background output truncated"));
        assert!(log_content.chars().count() <= MAX_BACKGROUND_OUTPUT_LOG_CHARS + 128);
    }

    #[tokio::test]
    async fn test_spawn_background_can_create_scheduled_monitor() {
        let session_id = SessionId::new();
        let schedule_store = Arc::new(TestScheduleStore::default());
        let storage_store = Arc::new(NoopStorageStore);
        let tool_registry = ToolRegistry::builder()
            .tool(SpawnBackgroundTool)
            .tool(TestBackgroundTool)
            .build();

        let context = ToolContext::with_storage_store(session_id, storage_store)
            .with_tool_registry(Arc::new(tool_registry))
            .with_schedule_store(schedule_store.clone());

        let result = SpawnBackgroundTool
            .execute_with_context(
                json!({
                    "tool": "test_background",
                    "title": "Watch PR 1319",
                    "args": { "summary": "Background complete" },
                    "schedule": {
                        "cron_expression": "*/10 * * * *",
                        "timezone": "America/Chicago"
                    }
                }),
                &context,
            )
            .await;

        let ToolExecutionResult::Success(value) = result else {
            panic!("spawn_background should create a schedule: {result:?}");
        };

        assert_eq!(value["status"], "scheduled");
        assert_eq!(value["title"], "Watch PR 1319");
        assert_eq!(value["cron_expression"], "*/10 * * * *");
        assert_eq!(value["timezone"], "America/Chicago");

        let schedules = schedule_store.list_schedules(session_id).await.unwrap();
        assert_eq!(schedules.len(), 1);
        assert_eq!(
            schedules[0].cron_expression.as_deref(),
            Some("*/10 * * * *")
        );
        assert!(schedules[0].description.contains("Monitor: Watch PR 1319"));
        assert!(
            schedules[0]
                .description
                .contains("\"summary\": \"Background complete\"")
        );
    }

    #[tokio::test]
    async fn test_spawn_background_rejects_invalid_scheduled_at() {
        let session_id = SessionId::new();
        let storage_store = Arc::new(NoopStorageStore);
        let tool_registry = ToolRegistry::builder()
            .tool(SpawnBackgroundTool)
            .tool(TestBackgroundTool)
            .build();
        let context = ToolContext::with_storage_store(session_id, storage_store)
            .with_tool_registry(Arc::new(tool_registry));

        let result = SpawnBackgroundTool
            .execute_with_context(
                json!({
                    "tool": "test_background",
                    "args": {},
                    "schedule": {
                        "scheduled_at": "tomorrow at noon"
                    }
                }),
                &context,
            )
            .await;

        let ToolExecutionResult::ToolError(message) = result else {
            panic!("spawn_background should reject invalid scheduled_at");
        };
        assert!(message.contains("scheduled_at must be RFC3339"));
    }

    #[tokio::test]
    async fn test_spawn_background_rejects_ambiguous_schedule_shape() {
        let session_id = SessionId::new();
        let storage_store = Arc::new(NoopStorageStore);
        let tool_registry = ToolRegistry::builder()
            .tool(SpawnBackgroundTool)
            .tool(TestBackgroundTool)
            .build();
        let context = ToolContext::with_storage_store(session_id, storage_store)
            .with_tool_registry(Arc::new(tool_registry));

        let result = SpawnBackgroundTool
            .execute_with_context(
                json!({
                    "tool": "test_background",
                    "args": {},
                    "schedule": {
                        "cron_expression": "*/10 * * * *",
                        "scheduled_at": "2026-04-16T15:30:00Z"
                    }
                }),
                &context,
            )
            .await;

        let ToolExecutionResult::ToolError(message) = result else {
            panic!("spawn_background should reject ambiguous schedule shape");
        };
        assert!(message.contains("must not include both cron_expression and scheduled_at"));
    }
}

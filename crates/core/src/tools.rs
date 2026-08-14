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

use crate::background::BackgroundExecutableTool;
use crate::tool_types::{
    BuiltinTool, DeferrablePolicy, ToolCall, ToolDefinition, ToolHints, ToolPolicy, ToolResult,
    ToolResultImage,
};
use crate::{
    tool_context::ToolContext, tool_context::ToolContextService, tool_context::ToolContextServices,
};

use crate::error::Result;
use crate::tool_execution::ToolExecutor;
// EVE-888: `spawn_background`, its session-task mirroring, the background
// event sink and the reattach path moved to `everruns-platform`
// (`background_run`). Creating session tasks and schedules is hosted behaviour;
// the kernel keeps the neutral `BackgroundExecutableTool`/`BackgroundEventSink`
// contracts in `crate::background` and runs whatever a host supplies.

// ============================================================================
// Tool Execution Result - Error Handling Contract
// ============================================================================

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

    /// Runtime services that must be present before this tool can be exposed.
    ///
    /// Context-aware tools should declare hard requirements here. Optional
    /// services that only enable extra behavior should not be listed.
    fn required_context_services(&self) -> &'static [ToolContextService] {
        &[]
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

    /// Returns backend-authored narration for a call to this tool, e.g.
    /// "Read AGENTS.md".
    ///
    /// The owning capability's default [`crate::capabilities::Capability::narrate`]
    /// dispatches here for the tool whose `name()` matches the call. Return
    /// `None` to accept the generic `narration_noun`/display-name fallback.
    /// Implementations should use the phrasing helpers in
    /// [`crate::tool_narration`] (`narrate_read_file`, `narrate_shell_exec`, …)
    /// so wording and localization stay consistent.
    fn narrate(
        &self,
        _tool_call: &crate::tool_types::ToolCall,
        _phase: crate::tool_narration::ToolNarrationPhase,
        _locale: Option<&str>,
        _ctx: crate::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        None
    }

    /// Returns native background execution support when this tool opts into
    /// detached execution via `hints().supports_background`.
    fn as_background_executable(&self) -> Option<&dyn BackgroundExecutableTool> {
        None
    }

    /// Deferral policy for progressive tool-schema disclosure (tool search).
    /// Hot-path or "consult-first" tools can return [`DeferrablePolicy::Never`]
    /// to always keep their full schema directly callable. Defaults to
    /// [`DeferrablePolicy::Automatic`].
    fn deferrable_policy(&self) -> DeferrablePolicy {
        DeferrablePolicy::default()
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
            deferrable: self.deferrable_policy(),
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
    /// This includes `report_progress`, the neutral progress-reporting
    /// contract tool. Test doubles such as echo tools belong to test-support
    /// or the test that owns them.
    ///
    /// Test fixture tools (test math/weather) are NOT included: they moved to
    /// the `everruns-test-support` crate (EVE-875) and are registered
    /// explicitly by tests that need them.
    pub fn with_defaults() -> Self {
        use crate::progress_reporting::ReportProgressTool;

        let builder = ToolRegistry::builder()
            // NOTE: `spawn_background` is intentionally NOT a default tool —
            // it is contributed by the `background_execution` capability,
            // which is auto-activated by
            // `collect_capabilities_with_configs` whenever a collected tool
            // declares `ToolHints::supports_background = Some(true)`. Keeping
            // it out of defaults preserves the lockstep contract between
            // model-visible tools and the worker execution registry: the
            // executor only knows about `spawn_background` when the model
            // can also see it.
            .tool(ReportProgressTool);

        builder.build()
    }

    /// Create a tool registry for autonomous scheduled monitor probes.
    ///
    /// Probe execution currently uses a scheduler-local [`ToolContext`] instead
    /// of the fully populated worker/API executor context. Keep this registry to
    /// context-free tools so scheduled probes cannot bypass session-scoped
    /// controls such as network ACLs, egress routing, storage, or filesystem
    /// mediation.
    pub fn with_monitor_probe_defaults() -> Self {
        Self::new()
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

    /// Fail configuration before model exposure when a registered tool is
    /// missing a runtime service it declares as required.
    pub fn validate_context_services(&self, services: &ToolContextServices) -> Result<()> {
        let mut tools: Vec<_> = self.tools.values().collect();
        tools.sort_by_key(|tool| tool.name());
        for tool in tools {
            for service in tool.required_context_services() {
                if !services.provides(*service) {
                    return Err(crate::error::AgentLoopError::config(format!(
                        "tool \"{}\" requires unavailable ToolContext service {}",
                        tool.name(),
                        service.name(),
                    )));
                }
            }
        }
        Ok(())
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

/// A tool that echoes back its arguments (useful for testing).
#[cfg(test)]
pub struct EchoTool;

#[cfg(test)]
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

/// A tool that always fails (useful for testing error handling).
#[cfg(test)]
pub struct FailingTool {
    error_message: String,
    use_internal_error: bool,
}

#[cfg(test)]
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

#[cfg(test)]
impl Default for FailingTool {
    fn default() -> Self {
        Self::with_tool_error("Tool execution failed")
    }
}

#[cfg(test)]
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

    struct RequiresOrgId;

    #[async_trait]
    impl Tool for RequiresOrgId {
        fn name(&self) -> &str {
            "requires_org_id"
        }

        fn description(&self) -> &str {
            "Exercises required ToolContext service validation"
        }

        fn parameters_schema(&self) -> Value {
            serde_json::json!({"type": "object", "additionalProperties": false})
        }

        fn required_context_services(&self) -> &'static [ToolContextService] {
            &[ToolContextService::OrgId]
        }

        async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
            ToolExecutionResult::success(Value::Null)
        }
    }

    #[test]
    fn required_context_service_validation_is_structured() {
        let mut registry = ToolRegistry::new();
        registry.register(RequiresOrgId);

        let error = registry
            .validate_context_services(&ToolContextServices::default())
            .expect_err("missing required service must fail before tool exposure");

        assert!(matches!(
            error,
            crate::AgentLoopError::Configuration(message)
                if message.contains("requires_org_id") && message.contains("OrgId")
        ));
    }

    #[test]
    fn required_context_service_validation_accepts_supplied_service() {
        let mut registry = ToolRegistry::new();
        registry.register(RequiresOrgId);
        let services = ToolContextServices {
            org_id: Some(crate::typed_id::OrgId::from_seed(1)),
            ..ToolContextServices::default()
        };

        registry
            .validate_context_services(&services)
            .expect("advertised required service should validate");
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
        registry.register(EchoTool);

        assert_eq!(registry.len(), 1);
        assert!(registry.has("echo"));
        assert!(!registry.has("nonexistent"));

        let definitions = registry.tool_definitions();
        assert_eq!(definitions.len(), 1);
    }

    #[tokio::test]
    async fn test_tool_registry_builder() {
        let registry = ToolRegistry::builder().tool(EchoTool).build();

        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn test_tool_display_name_in_definition() {
        let tool = EchoTool;
        assert_eq!(tool.display_name(), Some("Echo"));

        let def = tool.to_definition();
        assert_eq!(def.display_name(), Some("Echo"));
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
        let tool = EchoTool;
        let def = tool.to_definition();

        let ToolDefinition::Builtin(builtin) = def else {
            panic!("expected Builtin variant");
        };
        assert_eq!(builtin.name, "echo");
        assert_eq!(builtin.policy, ToolPolicy::Auto);
    }

    #[test]
    fn test_with_defaults_has_expected_tools() {
        let registry = ToolRegistry::with_defaults();

        // Core execution-contract tools only.
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

        // Test fixture tools moved to everruns-test-support (EVE-875) and
        // must NOT be in defaults.
        assert!(!registry.has("add"), "add must NOT be in defaults");
        assert!(
            !registry.has("get_weather"),
            "get_weather must NOT be in defaults"
        );

        // Environment-backed tools are composed by the host, not core defaults.
        for tool in ["read_file", "write_file", "bash", "web_fetch"] {
            assert!(!registry.has(tool), "`{tool}` must not be a core default");
        }

        assert_eq!(registry.len(), 1, "should have one core default tool");
    }

    #[tokio::test]
    async fn test_with_defaults_tools_are_executable() {
        let registry = ToolRegistry::with_defaults();

        // The neutral progress contract remains executable as a core default.
        let tool_call = ToolCall {
            id: "call_1".to_string(),
            name: "report_progress".to_string(),
            arguments: serde_json::json!({
                "status": "completed",
                "summary": "Boundary audit complete"
            }),
        };

        let tool_def = registry.get("report_progress").unwrap().to_definition();
        let result = registry.execute(&tool_call, &tool_def).await.unwrap();

        assert!(result.error.is_none());
        assert_eq!(result.result.unwrap()["summary"], "Boundary audit complete");
    }

    /// Regression: with_defaults() must NOT include capability-provided tools like
    /// 'bash'. These tools come from capabilities and must be registered separately.
    /// If bash were in defaults, the harness capability fallback would be masked.
    #[test]
    fn test_with_defaults_excludes_capability_only_tools() {
        let registry = ToolRegistry::with_defaults();

        // bash comes from bashkit_shell capability, not defaults
        assert!(
            !registry.has("bash"),
            "bash must not be in defaults — it comes from bashkit_shell capability"
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

    // =========================================================================
    // Cooperative cancellation tests
    // =========================================================================

    // Minimal in-memory SessionTaskRegistry for cancel tests.
    // (Mirrors the double in capabilities/session_tasks.rs — kept local because
    //  that module is private.)

    // -------------------------------------------------------------------------
    // reattach_background_run early-guard tests
    // -------------------------------------------------------------------------
}

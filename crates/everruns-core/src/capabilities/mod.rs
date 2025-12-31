//! Capabilities Module for Agent Loop
//!
//! This module provides the capabilities abstraction that allows composing
//! agent functionality through modular units. Each capability can contribute:
//! - System prompt additions
//! - Tools for the agent
//! - Behavior modifications (future)
//!
//! Design decisions:
//! - Capabilities are defined via the Capability trait for flexibility
//! - CapabilityRegistry holds all available capability implementations
//! - apply_capabilities() merges capability contributions into AgentConfig
//! - The agent-loop remains execution-focused; capabilities are applied before execution
//!
//! Each capability is in its own file with collocated tools.

use crate::config::AgentConfig;
use crate::tool_types::ToolDefinition;
use crate::tools::{Tool, ToolRegistry};
use std::collections::HashMap;
use std::sync::Arc;

// Re-export capability types from capability_types module
pub use crate::capability_types::{CapabilityId, CapabilityStatus};

// ============================================================================
// Capability Modules
// ============================================================================

mod current_time;
mod file_system;
mod noop;
mod research;
mod sandbox;
mod stateless_todo_list;
mod test_math;
mod test_weather;
mod web_fetch;

// Re-export capabilities
pub use current_time::{CurrentTimeCapability, GetCurrentTimeTool};
pub use file_system::{
    DeleteFileTool, FileSystemCapability, GrepFilesTool, ListDirectoryTool, ReadFileTool,
    StatFileTool, WriteFileTool,
};
pub use noop::NoopCapability;
pub use research::ResearchCapability;
pub use sandbox::SandboxCapability;
pub use stateless_todo_list::{StatelessTodoListCapability, WriteTodosTool};
pub use test_math::{AddTool, DivideTool, MultiplyTool, SubtractTool, TestMathCapability};
pub use test_weather::{GetForecastTool, GetWeatherTool, TestWeatherCapability};
pub use web_fetch::{WebFetchCapability, WebFetchTool};

// ============================================================================
// Capability Trait
// ============================================================================

/// Trait for implementing capabilities that extend agent functionality.
///
/// A capability can contribute:
/// - System prompt additions (prepended to agent's system prompt)
/// - Tools (added to agent's available tools)
///
/// # Example
///
/// ```ignore
/// use everruns_core::capabilities::{Capability, CapabilityId};
///
/// struct CurrentTimeCapability;
///
/// impl Capability for CurrentTimeCapability {
///     fn id(&self) -> &str {
///         CapabilityId::CURRENT_TIME
///     }
///
///     fn name(&self) -> &str {
///         "Current Time"
///     }
///
///     fn description(&self) -> &str {
///         "Provides tools to get the current date and time."
///     }
///
///     fn tools(&self) -> Vec<Box<dyn Tool>> {
///         vec![Box::new(GetCurrentTimeTool)]
///     }
/// }
/// ```
pub trait Capability: Send + Sync {
    /// Returns the unique capability identifier as a string
    fn id(&self) -> &str;

    /// Returns the display name
    fn name(&self) -> &str;

    /// Returns a description of what this capability provides
    fn description(&self) -> &str;

    /// Returns the current status of this capability
    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    /// Returns the icon name for UI rendering (optional)
    fn icon(&self) -> Option<&str> {
        None
    }

    /// Returns the category for grouping in UI (optional)
    fn category(&self) -> Option<&str> {
        None
    }

    /// Returns text to prepend to the agent's system prompt (optional)
    fn system_prompt_addition(&self) -> Option<&str> {
        None
    }

    /// Returns tool implementations provided by this capability
    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![]
    }

    /// Returns tool definitions for the agent config
    /// By default, converts tools() to definitions
    fn tool_definitions(&self) -> Vec<ToolDefinition> {
        self.tools().iter().map(|t| t.to_definition()).collect()
    }
}

// ============================================================================
// Capability Registry
// ============================================================================

/// Registry that holds all available capability implementations.
///
/// The registry provides access to capabilities by ID and allows
/// applying multiple capabilities to build an AgentConfig.
///
/// # Example
///
/// ```
/// use everruns_core::capabilities::{CapabilityRegistry, CapabilityId};
///
/// let registry = CapabilityRegistry::with_builtins();
///
/// // Get a capability by ID
/// if let Some(cap) = registry.get(CapabilityId::CURRENT_TIME) {
///     println!("Capability: {}", cap.name());
/// }
///
/// // List all available capabilities
/// for cap in registry.list() {
///     println!("{}: {}", cap.id(), cap.name());
/// }
/// ```
#[derive(Clone)]
pub struct CapabilityRegistry {
    capabilities: HashMap<String, Arc<dyn Capability>>,
}

impl CapabilityRegistry {
    /// Create a new empty registry
    pub fn new() -> Self {
        Self {
            capabilities: HashMap::new(),
        }
    }

    /// Create a registry with all built-in capabilities registered
    pub fn with_builtins() -> Self {
        let mut registry = Self::new();
        registry.register(NoopCapability);
        registry.register(CurrentTimeCapability);
        registry.register(ResearchCapability);
        registry.register(SandboxCapability);
        registry.register(FileSystemCapability);
        registry.register(TestMathCapability);
        registry.register(TestWeatherCapability);
        registry.register(StatelessTodoListCapability);
        registry.register(WebFetchCapability);
        registry
    }

    /// Register a capability
    pub fn register(&mut self, capability: impl Capability + 'static) {
        self.capabilities
            .insert(capability.id().to_string(), Arc::new(capability));
    }

    /// Register a boxed capability
    pub fn register_boxed(&mut self, capability: Box<dyn Capability>) {
        self.capabilities
            .insert(capability.id().to_string(), Arc::from(capability));
    }

    /// Register an Arc-wrapped capability
    pub fn register_arc(&mut self, capability: Arc<dyn Capability>) {
        self.capabilities
            .insert(capability.id().to_string(), capability);
    }

    /// Get a capability by ID
    pub fn get(&self, id: &str) -> Option<&Arc<dyn Capability>> {
        self.capabilities.get(id)
    }

    /// Check if a capability is registered
    pub fn has(&self, id: &str) -> bool {
        self.capabilities.contains_key(id)
    }

    /// Get all registered capabilities
    pub fn list(&self) -> Vec<&Arc<dyn Capability>> {
        self.capabilities.values().collect()
    }

    /// Get the number of registered capabilities
    pub fn len(&self) -> usize {
        self.capabilities.len()
    }

    /// Check if the registry is empty
    pub fn is_empty(&self) -> bool {
        self.capabilities.is_empty()
    }

    /// Create a builder for fluent capability registration
    pub fn builder() -> CapabilityRegistryBuilder {
        CapabilityRegistryBuilder::new()
    }
}

impl Default for CapabilityRegistry {
    fn default() -> Self {
        Self::with_builtins()
    }
}

impl std::fmt::Debug for CapabilityRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let ids: Vec<_> = self.capabilities.keys().collect();
        f.debug_struct("CapabilityRegistry")
            .field("capabilities", &ids)
            .finish()
    }
}

/// Builder for creating a CapabilityRegistry with a fluent API
pub struct CapabilityRegistryBuilder {
    registry: CapabilityRegistry,
}

impl CapabilityRegistryBuilder {
    /// Create a new builder with an empty registry
    pub fn new() -> Self {
        Self {
            registry: CapabilityRegistry::new(),
        }
    }

    /// Create a new builder with built-in capabilities
    pub fn with_builtins() -> Self {
        Self {
            registry: CapabilityRegistry::with_builtins(),
        }
    }

    /// Add a capability
    pub fn capability(mut self, capability: impl Capability + 'static) -> Self {
        self.registry.register(capability);
        self
    }

    /// Build the registry
    pub fn build(self) -> CapabilityRegistry {
        self.registry
    }
}

impl Default for CapabilityRegistryBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Apply Capabilities to AgentConfig
// ============================================================================

/// Result of applying capabilities to a base config
pub struct AppliedCapabilities {
    /// The modified agent config with capability contributions merged
    pub config: AgentConfig,
    /// Tool registry containing all capability tools
    pub tool_registry: ToolRegistry,
    /// IDs of capabilities that were applied
    pub applied_ids: Vec<String>,
}

/// Apply capabilities to a base agent configuration.
///
/// This function:
/// 1. Collects system prompt additions from capabilities (in order)
/// 2. Prepends them to the agent's base system prompt
/// 3. Collects all tools from capabilities
/// 4. Returns the modified config and a tool registry
///
/// # Arguments
///
/// * `base_config` - The agent's base configuration
/// * `capability_ids` - Ordered list of capability IDs to apply
/// * `registry` - The capability registry containing implementations
///
/// # Returns
///
/// An `AppliedCapabilities` struct containing the modified config,
/// tool registry, and list of applied capability IDs.
///
/// # Example
///
/// ```
/// use everruns_core::capabilities::{apply_capabilities, CapabilityRegistry, CapabilityId};
/// use everruns_core::config::AgentConfig;
///
/// let registry = CapabilityRegistry::with_builtins();
/// let base_config = AgentConfig::new("You are a helpful assistant.", "gpt-5.2");
///
/// let capability_ids = vec![CapabilityId::CURRENT_TIME.to_string()];
/// let applied = apply_capabilities(base_config, &capability_ids, &registry);
///
/// // The config now includes CurrentTime tool
/// assert!(!applied.tool_registry.is_empty());
/// ```
pub fn apply_capabilities(
    base_config: AgentConfig,
    capability_ids: &[String],
    registry: &CapabilityRegistry,
) -> AppliedCapabilities {
    let mut system_prompt_parts: Vec<String> = Vec::new();
    let mut tool_registry = ToolRegistry::new();
    let mut tool_definitions: Vec<ToolDefinition> = Vec::new();
    let mut applied_ids: Vec<String> = Vec::new();

    // Apply capabilities in order
    for cap_id in capability_ids {
        if let Some(capability) = registry.get(cap_id) {
            // Only apply available capabilities
            if capability.status() != CapabilityStatus::Available {
                continue;
            }

            // Collect system prompt addition
            if let Some(addition) = capability.system_prompt_addition() {
                system_prompt_parts.push(addition.to_string());
            }

            // Collect tools
            for tool in capability.tools() {
                tool_registry.register_boxed(tool);
            }

            // Collect tool definitions
            tool_definitions.extend(capability.tool_definitions());

            applied_ids.push(cap_id.clone());
        }
    }

    // Build final system prompt: capability additions + base prompt
    let mut final_system_prompt = String::new();
    if !system_prompt_parts.is_empty() {
        final_system_prompt.push_str(&system_prompt_parts.join("\n\n"));
        final_system_prompt.push_str("\n\n");
    }
    final_system_prompt.push_str(&base_config.system_prompt);

    // Create modified config
    let config = AgentConfig {
        system_prompt: final_system_prompt,
        model: base_config.model,
        tools: tool_definitions,
        max_iterations: base_config.max_iterations,
        temperature: base_config.temperature,
        max_tokens: base_config.max_tokens,
    };

    AppliedCapabilities {
        config,
        tool_registry,
        applied_ids,
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::ToolExecutionResult;

    #[test]
    fn test_capability_registry_with_builtins() {
        let registry = CapabilityRegistry::with_builtins();

        assert!(registry.has(CapabilityId::NOOP));
        assert!(registry.has(CapabilityId::CURRENT_TIME));
        assert!(registry.has(CapabilityId::RESEARCH));
        assert!(registry.has(CapabilityId::SANDBOX));
        assert!(registry.has(CapabilityId::FILE_SYSTEM));
        assert!(registry.has(CapabilityId::TEST_MATH));
        assert!(registry.has(CapabilityId::TEST_WEATHER));
        assert!(registry.has(CapabilityId::STATELESS_TODO_LIST));
        assert!(registry.has(CapabilityId::WEB_FETCH));
        assert_eq!(registry.len(), 9);
    }

    #[test]
    fn test_capability_registry_get() {
        let registry = CapabilityRegistry::with_builtins();

        let noop = registry.get(CapabilityId::NOOP).unwrap();
        assert_eq!(noop.id(), CapabilityId::NOOP);
        assert_eq!(noop.name(), "No-Op");
        assert_eq!(noop.status(), CapabilityStatus::Available);
    }

    #[test]
    fn test_capability_status() {
        let registry = CapabilityRegistry::with_builtins();

        let current_time = registry.get(CapabilityId::CURRENT_TIME).unwrap();
        assert_eq!(current_time.status(), CapabilityStatus::Available);

        let research = registry.get(CapabilityId::RESEARCH).unwrap();
        assert_eq!(research.status(), CapabilityStatus::ComingSoon);
    }

    #[test]
    fn test_current_time_capability_has_tools() {
        let registry = CapabilityRegistry::with_builtins();

        let current_time = registry.get(CapabilityId::CURRENT_TIME).unwrap();
        let tools = current_time.tools();

        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name(), "get_current_time");
    }

    #[test]
    fn test_apply_capabilities_empty() {
        let registry = CapabilityRegistry::with_builtins();
        let base_config = AgentConfig::new("You are a helpful assistant.", "gpt-5.2");

        let applied = apply_capabilities(base_config.clone(), &[], &registry);

        assert_eq!(applied.config.system_prompt, base_config.system_prompt);
        assert!(applied.tool_registry.is_empty());
        assert!(applied.applied_ids.is_empty());
    }

    #[test]
    fn test_apply_capabilities_noop() {
        let registry = CapabilityRegistry::with_builtins();
        let base_config = AgentConfig::new("You are a helpful assistant.", "gpt-5.2");

        let applied = apply_capabilities(
            base_config.clone(),
            &[CapabilityId::NOOP.to_string()],
            &registry,
        );

        // Noop has no system prompt addition or tools
        assert_eq!(applied.config.system_prompt, base_config.system_prompt);
        assert!(applied.tool_registry.is_empty());
        assert_eq!(applied.applied_ids, vec![CapabilityId::NOOP]);
    }

    #[test]
    fn test_apply_capabilities_current_time() {
        let registry = CapabilityRegistry::with_builtins();
        let base_config = AgentConfig::new("You are a helpful assistant.", "gpt-5.2");

        let applied = apply_capabilities(
            base_config.clone(),
            &[CapabilityId::CURRENT_TIME.to_string()],
            &registry,
        );

        // CurrentTime has no system prompt addition but has a tool
        assert_eq!(applied.config.system_prompt, base_config.system_prompt);
        assert!(applied.tool_registry.has("get_current_time"));
        assert_eq!(applied.tool_registry.len(), 1);
        assert_eq!(applied.applied_ids, vec![CapabilityId::CURRENT_TIME]);
    }

    #[test]
    fn test_apply_capabilities_skips_coming_soon() {
        let registry = CapabilityRegistry::with_builtins();
        let base_config = AgentConfig::new("You are a helpful assistant.", "gpt-5.2");

        // Research is ComingSoon, so it should be skipped
        let applied = apply_capabilities(
            base_config.clone(),
            &[CapabilityId::RESEARCH.to_string()],
            &registry,
        );

        // System prompt should not have the research addition
        assert_eq!(applied.config.system_prompt, base_config.system_prompt);
        assert!(applied.applied_ids.is_empty()); // Research was not applied
    }

    #[test]
    fn test_apply_capabilities_multiple() {
        let registry = CapabilityRegistry::with_builtins();
        let base_config = AgentConfig::new("You are a helpful assistant.", "gpt-5.2");

        let applied = apply_capabilities(
            base_config.clone(),
            &[
                CapabilityId::NOOP.to_string(),
                CapabilityId::CURRENT_TIME.to_string(),
            ],
            &registry,
        );

        assert!(applied.tool_registry.has("get_current_time"));
        assert_eq!(
            applied.applied_ids,
            vec![CapabilityId::NOOP, CapabilityId::CURRENT_TIME]
        );
    }

    #[test]
    fn test_apply_capabilities_preserves_order() {
        let registry = CapabilityRegistry::with_builtins();
        let base_config = AgentConfig::new("Base prompt.", "gpt-5.2");

        // Order should be preserved in applied_ids
        let applied = apply_capabilities(
            base_config,
            &[
                CapabilityId::CURRENT_TIME.to_string(),
                CapabilityId::NOOP.to_string(),
            ],
            &registry,
        );

        assert_eq!(
            applied.applied_ids,
            vec![CapabilityId::CURRENT_TIME, CapabilityId::NOOP]
        );
    }

    #[test]
    fn test_capability_registry_builder() {
        let registry = CapabilityRegistry::builder()
            .capability(NoopCapability)
            .capability(CurrentTimeCapability)
            .build();

        assert!(registry.has(CapabilityId::NOOP));
        assert!(registry.has(CapabilityId::CURRENT_TIME));
        assert_eq!(registry.len(), 2);
    }

    #[test]
    fn test_capability_icons_and_categories() {
        let registry = CapabilityRegistry::with_builtins();

        let noop = registry.get(CapabilityId::NOOP).unwrap();
        assert_eq!(noop.icon(), Some("circle-off"));
        assert_eq!(noop.category(), Some("Testing"));

        let current_time = registry.get(CapabilityId::CURRENT_TIME).unwrap();
        assert_eq!(current_time.icon(), Some("clock"));
        assert_eq!(current_time.category(), Some("Utilities"));
    }

    #[tokio::test]
    async fn test_get_current_time_tool_iso8601() {
        let tool = GetCurrentTimeTool;
        let result = tool.execute(serde_json::json!({})).await;

        if let ToolExecutionResult::Success(value) = result {
            assert!(value.get("datetime").is_some());
            assert_eq!(value.get("format").unwrap().as_str().unwrap(), "iso8601");
        } else {
            panic!("Expected success");
        }
    }

    #[tokio::test]
    async fn test_get_current_time_tool_unix() {
        let tool = GetCurrentTimeTool;
        let result = tool.execute(serde_json::json!({"format": "unix"})).await;

        if let ToolExecutionResult::Success(value) = result {
            assert!(value.get("timestamp").is_some());
            assert_eq!(value.get("format").unwrap().as_str().unwrap(), "unix");
        } else {
            panic!("Expected success");
        }
    }

    #[tokio::test]
    async fn test_get_current_time_tool_human() {
        let tool = GetCurrentTimeTool;
        let result = tool.execute(serde_json::json!({"format": "human"})).await;

        if let ToolExecutionResult::Success(value) = result {
            assert!(value.get("datetime").is_some());
            assert_eq!(value.get("format").unwrap().as_str().unwrap(), "human");
            // Human format should contain "at" for time
            let datetime = value.get("datetime").unwrap().as_str().unwrap();
            assert!(datetime.contains("at"));
        } else {
            panic!("Expected success");
        }
    }

    // TestMath capability tests
    #[test]
    fn test_test_math_capability_has_tools() {
        let registry = CapabilityRegistry::with_builtins();
        let math = registry.get(CapabilityId::TEST_MATH).unwrap();
        let tools = math.tools();

        assert_eq!(tools.len(), 4);
        let tool_names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
        assert!(tool_names.contains(&"add"));
        assert!(tool_names.contains(&"subtract"));
        assert!(tool_names.contains(&"multiply"));
        assert!(tool_names.contains(&"divide"));
    }

    #[tokio::test]
    async fn test_add_tool() {
        let tool = AddTool;
        let result = tool.execute(serde_json::json!({"a": 5, "b": 3})).await;

        if let ToolExecutionResult::Success(value) = result {
            assert_eq!(value.get("result").unwrap().as_f64().unwrap(), 8.0);
            assert_eq!(value.get("operation").unwrap().as_str().unwrap(), "add");
        } else {
            panic!("Expected success");
        }
    }

    #[tokio::test]
    async fn test_subtract_tool() {
        let tool = SubtractTool;
        let result = tool.execute(serde_json::json!({"a": 10, "b": 4})).await;

        if let ToolExecutionResult::Success(value) = result {
            assert_eq!(value.get("result").unwrap().as_f64().unwrap(), 6.0);
            assert_eq!(
                value.get("operation").unwrap().as_str().unwrap(),
                "subtract"
            );
        } else {
            panic!("Expected success");
        }
    }

    #[tokio::test]
    async fn test_multiply_tool() {
        let tool = MultiplyTool;
        let result = tool.execute(serde_json::json!({"a": 6, "b": 7})).await;

        if let ToolExecutionResult::Success(value) = result {
            assert_eq!(value.get("result").unwrap().as_f64().unwrap(), 42.0);
            assert_eq!(
                value.get("operation").unwrap().as_str().unwrap(),
                "multiply"
            );
        } else {
            panic!("Expected success");
        }
    }

    #[tokio::test]
    async fn test_divide_tool() {
        let tool = DivideTool;
        let result = tool.execute(serde_json::json!({"a": 20, "b": 4})).await;

        if let ToolExecutionResult::Success(value) = result {
            assert_eq!(value.get("result").unwrap().as_f64().unwrap(), 5.0);
            assert_eq!(value.get("operation").unwrap().as_str().unwrap(), "divide");
        } else {
            panic!("Expected success");
        }
    }

    #[tokio::test]
    async fn test_divide_by_zero() {
        let tool = DivideTool;
        let result = tool.execute(serde_json::json!({"a": 10, "b": 0})).await;

        if let ToolExecutionResult::ToolError(msg) = result {
            assert!(msg.contains("divide by zero"));
        } else {
            panic!("Expected tool error for division by zero");
        }
    }

    // TestWeather capability tests
    #[test]
    fn test_test_weather_capability_has_tools() {
        let registry = CapabilityRegistry::with_builtins();
        let weather = registry.get(CapabilityId::TEST_WEATHER).unwrap();
        let tools = weather.tools();

        assert_eq!(tools.len(), 2);
        let tool_names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
        assert!(tool_names.contains(&"get_weather"));
        assert!(tool_names.contains(&"get_forecast"));
    }

    #[tokio::test]
    async fn test_get_weather_tool() {
        let tool = GetWeatherTool;
        let result = tool
            .execute(serde_json::json!({"location": "New York"}))
            .await;

        if let ToolExecutionResult::Success(value) = result {
            assert_eq!(value.get("location").unwrap().as_str().unwrap(), "New York");
            assert!(value.get("temperature").is_some());
            assert!(value.get("conditions").is_some());
            assert!(value.get("humidity").is_some());
        } else {
            panic!("Expected success");
        }
    }

    #[tokio::test]
    async fn test_get_weather_fahrenheit() {
        let tool = GetWeatherTool;
        let result = tool
            .execute(serde_json::json!({"location": "London", "units": "fahrenheit"}))
            .await;

        if let ToolExecutionResult::Success(value) = result {
            assert_eq!(value.get("units").unwrap().as_str().unwrap(), "fahrenheit");
            // Fahrenheit temps should be higher than Celsius
            let temp = value.get("temperature").unwrap().as_f64().unwrap();
            assert!(temp > 30.0); // At least 30°F
        } else {
            panic!("Expected success");
        }
    }

    #[tokio::test]
    async fn test_get_forecast_tool() {
        let tool = GetForecastTool;
        let result = tool
            .execute(serde_json::json!({"location": "Tokyo", "days": 5}))
            .await;

        if let ToolExecutionResult::Success(value) = result {
            assert_eq!(value.get("location").unwrap().as_str().unwrap(), "Tokyo");
            assert_eq!(value.get("days").unwrap().as_u64().unwrap(), 5);
            let forecast = value.get("forecast").unwrap().as_array().unwrap();
            assert_eq!(forecast.len(), 5);
            // Check first day has expected fields
            let first_day = &forecast[0];
            assert!(first_day.get("date").is_some());
            assert!(first_day.get("high").is_some());
            assert!(first_day.get("low").is_some());
            assert!(first_day.get("conditions").is_some());
        } else {
            panic!("Expected success");
        }
    }

    #[test]
    fn test_apply_capabilities_test_math() {
        let registry = CapabilityRegistry::with_builtins();
        let base_config = AgentConfig::new("You are a helpful assistant.", "gpt-5.2");

        let applied = apply_capabilities(
            base_config.clone(),
            &[CapabilityId::TEST_MATH.to_string()],
            &registry,
        );

        // TestMath has system prompt addition and 4 tools
        assert!(applied.config.system_prompt.contains("math tools"));
        assert!(applied.tool_registry.has("add"));
        assert!(applied.tool_registry.has("subtract"));
        assert!(applied.tool_registry.has("multiply"));
        assert!(applied.tool_registry.has("divide"));
        assert_eq!(applied.tool_registry.len(), 4);
    }

    #[test]
    fn test_apply_capabilities_test_weather() {
        let registry = CapabilityRegistry::with_builtins();
        let base_config = AgentConfig::new("You are a helpful assistant.", "gpt-5.2");

        let applied = apply_capabilities(
            base_config.clone(),
            &[CapabilityId::TEST_WEATHER.to_string()],
            &registry,
        );

        // TestWeather has system prompt addition and 2 tools
        assert!(applied.config.system_prompt.contains("weather tools"));
        assert!(applied.tool_registry.has("get_weather"));
        assert!(applied.tool_registry.has("get_forecast"));
        assert_eq!(applied.tool_registry.len(), 2);
    }

    #[test]
    fn test_apply_capabilities_test_math_and_test_weather() {
        let registry = CapabilityRegistry::with_builtins();
        let base_config = AgentConfig::new("You are a helpful assistant.", "gpt-5.2");

        let applied = apply_capabilities(
            base_config.clone(),
            &[
                CapabilityId::TEST_MATH.to_string(),
                CapabilityId::TEST_WEATHER.to_string(),
            ],
            &registry,
        );

        // Should have both sets of tools
        assert_eq!(applied.tool_registry.len(), 6); // 4 math + 2 weather
        assert!(applied.tool_registry.has("add"));
        assert!(applied.tool_registry.has("get_weather"));
    }

    // StatelessTodoList capability tests
    #[test]
    fn test_stateless_todo_list_capability_has_tools() {
        let registry = CapabilityRegistry::with_builtins();
        let capability = registry.get(CapabilityId::STATELESS_TODO_LIST).unwrap();
        let tools = capability.tools();

        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name(), "write_todos");
    }

    #[test]
    fn test_stateless_todo_list_capability_has_system_prompt() {
        let registry = CapabilityRegistry::with_builtins();
        let capability = registry.get(CapabilityId::STATELESS_TODO_LIST).unwrap();

        let system_prompt = capability.system_prompt_addition().unwrap();
        assert!(system_prompt.contains("Task Management"));
        assert!(system_prompt.contains("write_todos"));
        assert!(system_prompt.contains("in_progress"));
        assert!(system_prompt.contains("completed"));
    }

    #[test]
    fn test_stateless_todo_list_capability_metadata() {
        let registry = CapabilityRegistry::with_builtins();
        let capability = registry.get(CapabilityId::STATELESS_TODO_LIST).unwrap();

        assert_eq!(capability.name(), "Task Management");
        assert_eq!(capability.icon(), Some("list-checks"));
        assert_eq!(capability.category(), Some("Productivity"));
        assert_eq!(capability.status(), CapabilityStatus::Available);
    }

    #[test]
    fn test_apply_capabilities_stateless_todo_list() {
        let registry = CapabilityRegistry::with_builtins();
        let base_config = AgentConfig::new("You are a helpful assistant.", "gpt-5.2");

        let applied = apply_capabilities(
            base_config.clone(),
            &[CapabilityId::STATELESS_TODO_LIST.to_string()],
            &registry,
        );

        // StatelessTodoList has system prompt addition and 1 tool
        assert!(applied.config.system_prompt.contains("Task Management"));
        assert!(applied.config.system_prompt.contains("write_todos"));
        assert!(applied.tool_registry.has("write_todos"));
        assert_eq!(applied.tool_registry.len(), 1);
    }

    // WebFetch capability tests
    #[test]
    fn test_web_fetch_capability_has_tools() {
        let registry = CapabilityRegistry::with_builtins();
        let capability = registry.get(CapabilityId::WEB_FETCH).unwrap();
        let tools = capability.tools();

        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name(), "web_fetch");
    }

    #[test]
    fn test_web_fetch_capability_no_system_prompt() {
        let registry = CapabilityRegistry::with_builtins();
        let capability = registry.get(CapabilityId::WEB_FETCH).unwrap();

        // WebFetch should not have a system prompt addition
        assert!(capability.system_prompt_addition().is_none());
    }

    #[test]
    fn test_web_fetch_capability_metadata() {
        let registry = CapabilityRegistry::with_builtins();
        let capability = registry.get(CapabilityId::WEB_FETCH).unwrap();

        assert_eq!(capability.name(), "Web Fetch");
        assert_eq!(capability.icon(), Some("globe"));
        assert_eq!(capability.category(), Some("Network"));
        assert_eq!(capability.status(), CapabilityStatus::Available);
    }

    #[test]
    fn test_apply_capabilities_web_fetch() {
        let registry = CapabilityRegistry::with_builtins();
        let base_config = AgentConfig::new("You are a helpful assistant.", "gpt-5.2");

        let applied = apply_capabilities(
            base_config.clone(),
            &[CapabilityId::WEB_FETCH.to_string()],
            &registry,
        );

        // WebFetch has no system prompt addition but has 1 tool
        assert_eq!(applied.config.system_prompt, base_config.system_prompt);
        assert!(applied.tool_registry.has("web_fetch"));
        assert_eq!(applied.tool_registry.len(), 1);
    }

    #[tokio::test]
    async fn test_web_fetch_tool_missing_url() {
        let tool = WebFetchTool;
        let result = tool.execute(serde_json::json!({})).await;

        if let ToolExecutionResult::ToolError(msg) = result {
            assert!(msg.contains("url"));
        } else {
            panic!("Expected tool error for missing URL");
        }
    }

    #[tokio::test]
    async fn test_web_fetch_tool_invalid_url() {
        let tool = WebFetchTool;
        let result = tool.execute(serde_json::json!({"url": "not-a-url"})).await;

        if let ToolExecutionResult::ToolError(msg) = result {
            assert!(msg.contains("Invalid URL"));
        } else {
            panic!("Expected tool error for invalid URL");
        }
    }

    #[tokio::test]
    async fn test_web_fetch_tool_invalid_method() {
        let tool = WebFetchTool;
        let result = tool
            .execute(serde_json::json!({"url": "https://example.com", "method": "DELETE"}))
            .await;

        if let ToolExecutionResult::ToolError(msg) = result {
            assert!(msg.contains("Invalid method"));
        } else {
            panic!("Expected tool error for invalid method");
        }
    }
}

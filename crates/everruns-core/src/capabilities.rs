// Capabilities Module for Agent Loop
//
// This module provides the capabilities abstraction that allows composing
// agent functionality through modular units. Each capability can contribute:
// - System prompt additions
// - Tools for the agent
// - Behavior modifications (future)
//
// Design decisions:
// - Capabilities are defined via the Capability trait for flexibility
// - CapabilityRegistry holds all available capability implementations
// - apply_capabilities() merges capability contributions into AgentConfig
// - The agent-loop remains execution-focused; capabilities are applied before execution

use crate::config::AgentConfig;
use crate::tool_types::ToolDefinition;
use crate::tools::{Tool, ToolRegistry};
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

// Re-export capability types for convenience
pub use crate::capability_types::{CapabilityId, CapabilityStatus};

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
///     fn id(&self) -> CapabilityId {
///         CapabilityId::CurrentTime
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
    /// Returns the unique capability identifier
    fn id(&self) -> CapabilityId;

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
/// if let Some(cap) = registry.get(CapabilityId::CurrentTime) {
///     println!("Capability: {}", cap.name());
/// }
///
/// // List all available capabilities
/// for cap in registry.list() {
///     println!("{}: {}", cap.id(), cap.name());
/// }
/// ```
pub struct CapabilityRegistry {
    capabilities: HashMap<CapabilityId, Arc<dyn Capability>>,
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
        registry
    }

    /// Register a capability
    pub fn register(&mut self, capability: impl Capability + 'static) {
        self.capabilities
            .insert(capability.id(), Arc::new(capability));
    }

    /// Register a boxed capability
    pub fn register_boxed(&mut self, capability: Box<dyn Capability>) {
        self.capabilities
            .insert(capability.id(), Arc::from(capability));
    }

    /// Register an Arc-wrapped capability
    pub fn register_arc(&mut self, capability: Arc<dyn Capability>) {
        self.capabilities.insert(capability.id(), capability);
    }

    /// Get a capability by ID
    pub fn get(&self, id: CapabilityId) -> Option<&Arc<dyn Capability>> {
        self.capabilities.get(&id)
    }

    /// Check if a capability is registered
    pub fn has(&self, id: CapabilityId) -> bool {
        self.capabilities.contains_key(&id)
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
    pub applied_ids: Vec<CapabilityId>,
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
/// let capability_ids = vec![CapabilityId::CurrentTime];
/// let applied = apply_capabilities(base_config, &capability_ids, &registry);
///
/// // The config now includes CurrentTime tool
/// assert!(!applied.tool_registry.is_empty());
/// ```
pub fn apply_capabilities(
    base_config: AgentConfig,
    capability_ids: &[CapabilityId],
    registry: &CapabilityRegistry,
) -> AppliedCapabilities {
    let mut system_prompt_parts: Vec<String> = Vec::new();
    let mut tool_registry = ToolRegistry::new();
    let mut tool_definitions: Vec<ToolDefinition> = Vec::new();
    let mut applied_ids: Vec<CapabilityId> = Vec::new();

    // Apply capabilities in order
    for cap_id in capability_ids {
        if let Some(capability) = registry.get(*cap_id) {
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

            applied_ids.push(*cap_id);
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
// Built-in Capabilities
// ============================================================================

/// Noop capability - for testing and demonstration purposes
pub struct NoopCapability;

impl Capability for NoopCapability {
    fn id(&self) -> CapabilityId {
        CapabilityId::Noop
    }

    fn name(&self) -> &str {
        "No-Op"
    }

    fn description(&self) -> &str {
        "A no-operation capability for testing and demonstration purposes. Does not add any functionality."
    }

    fn icon(&self) -> Option<&str> {
        Some("circle-off")
    }

    fn category(&self) -> Option<&str> {
        Some("Testing")
    }
}

/// CurrentTime capability - provides tools to get current date and time
pub struct CurrentTimeCapability;

impl Capability for CurrentTimeCapability {
    fn id(&self) -> CapabilityId {
        CapabilityId::CurrentTime
    }

    fn name(&self) -> &str {
        "Current Time"
    }

    fn description(&self) -> &str {
        "Adds a tool to get the current date and time in various formats and timezones."
    }

    fn icon(&self) -> Option<&str> {
        Some("clock")
    }

    fn category(&self) -> Option<&str> {
        Some("Utilities")
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![Box::new(GetCurrentTimeTool)]
    }
}

/// Research capability - for deep research with organized findings (coming soon)
pub struct ResearchCapability;

impl Capability for ResearchCapability {
    fn id(&self) -> CapabilityId {
        CapabilityId::Research
    }

    fn name(&self) -> &str {
        "Deep Research"
    }

    fn description(&self) -> &str {
        "Enables deep research capabilities with a scratchpad for notes, web search tools, and structured thinking."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::ComingSoon
    }

    fn icon(&self) -> Option<&str> {
        Some("search")
    }

    fn category(&self) -> Option<&str> {
        Some("AI")
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some("You have access to a research scratchpad. Use it to organize your thoughts and findings.")
    }
}

/// Sandbox capability - for sandboxed code execution (coming soon)
pub struct SandboxCapability;

impl Capability for SandboxCapability {
    fn id(&self) -> CapabilityId {
        CapabilityId::Sandbox
    }

    fn name(&self) -> &str {
        "Sandboxed Execution"
    }

    fn description(&self) -> &str {
        "Enables sandboxed code execution environment for running code safely."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::ComingSoon
    }

    fn icon(&self) -> Option<&str> {
        Some("box")
    }

    fn category(&self) -> Option<&str> {
        Some("Execution")
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some(
            "You can execute code in a sandboxed environment. Use the execute_code tool to run code safely.",
        )
    }
}

/// FileSystem capability - for file system access (coming soon)
pub struct FileSystemCapability;

impl Capability for FileSystemCapability {
    fn id(&self) -> CapabilityId {
        CapabilityId::FileSystem
    }

    fn name(&self) -> &str {
        "File System Access"
    }

    fn description(&self) -> &str {
        "Adds tools to access and manipulate files - read, write, grep, and more."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::ComingSoon
    }

    fn icon(&self) -> Option<&str> {
        Some("folder")
    }

    fn category(&self) -> Option<&str> {
        Some("File Operations")
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some("You have access to file system tools. You can read, write, and search files.")
    }
}

/// TestMath capability - calculator tools for testing tool calling
pub struct TestMathCapability;

impl Capability for TestMathCapability {
    fn id(&self) -> CapabilityId {
        CapabilityId::TestMath
    }

    fn name(&self) -> &str {
        "Test Math"
    }

    fn description(&self) -> &str {
        "Testing capability: adds calculator tools (add, subtract, multiply, divide) for tool calling tests."
    }

    fn icon(&self) -> Option<&str> {
        Some("calculator")
    }

    fn category(&self) -> Option<&str> {
        Some("Testing")
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some("You have access to math tools. Use them for calculations: add, subtract, multiply, divide.")
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![
            Box::new(AddTool),
            Box::new(SubtractTool),
            Box::new(MultiplyTool),
            Box::new(DivideTool),
        ]
    }
}

/// TestWeather capability - mock weather tools for testing tool calling
pub struct TestWeatherCapability;

impl Capability for TestWeatherCapability {
    fn id(&self) -> CapabilityId {
        CapabilityId::TestWeather
    }

    fn name(&self) -> &str {
        "Test Weather"
    }

    fn description(&self) -> &str {
        "Testing capability: adds mock weather tools (get_weather, get_forecast) for tool calling tests."
    }

    fn icon(&self) -> Option<&str> {
        Some("cloud-sun")
    }

    fn category(&self) -> Option<&str> {
        Some("Testing")
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some("You have access to weather tools. Use get_weather for current conditions and get_forecast for multi-day forecasts.")
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![Box::new(GetWeatherTool), Box::new(GetForecastTool)]
    }
}

// ============================================================================
// Capability Tools
// ============================================================================

/// Tool that returns the current date and time
pub struct GetCurrentTimeTool;

#[async_trait]
impl Tool for GetCurrentTimeTool {
    fn name(&self) -> &str {
        "get_current_time"
    }

    fn description(&self) -> &str {
        "Get the current date and time. Can return time in different formats and timezones."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "timezone": {
                    "type": "string",
                    "description": "Timezone to return the time in (e.g., 'UTC', 'America/New_York', 'Europe/London'). Defaults to UTC."
                },
                "format": {
                    "type": "string",
                    "enum": ["iso8601", "unix", "human"],
                    "description": "Output format: 'iso8601' for ISO 8601 format, 'unix' for Unix timestamp, 'human' for human-readable format. Defaults to 'iso8601'."
                }
            },
            "additionalProperties": false
        })
    }

    async fn execute(&self, arguments: Value) -> crate::tools::ToolExecutionResult {
        let format = arguments
            .get("format")
            .and_then(|v| v.as_str())
            .unwrap_or("iso8601");

        let _timezone = arguments
            .get("timezone")
            .and_then(|v| v.as_str())
            .unwrap_or("UTC");

        // Note: For simplicity, we're using UTC. Full timezone support would require
        // the chrono-tz crate which adds significant dependencies.
        let now = chrono::Utc::now();

        let result = match format {
            "unix" => serde_json::json!({
                "timestamp": now.timestamp(),
                "format": "unix",
                "timezone": "UTC"
            }),
            "human" => serde_json::json!({
                "datetime": now.format("%A, %B %d, %Y at %H:%M:%S UTC").to_string(),
                "format": "human",
                "timezone": "UTC"
            }),
            _ => serde_json::json!({
                "datetime": now.to_rfc3339(),
                "format": "iso8601",
                "timezone": "UTC"
            }),
        };

        crate::tools::ToolExecutionResult::success(result)
    }
}

// ============================================================================
// Math Tools
// ============================================================================

/// Tool that adds two numbers
pub struct AddTool;

#[async_trait]
impl Tool for AddTool {
    fn name(&self) -> &str {
        "add"
    }

    fn description(&self) -> &str {
        "Add two numbers together and return the result."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "a": {
                    "type": "number",
                    "description": "The first number"
                },
                "b": {
                    "type": "number",
                    "description": "The second number"
                }
            },
            "required": ["a", "b"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, arguments: Value) -> crate::tools::ToolExecutionResult {
        let a = arguments.get("a").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let b = arguments.get("b").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let result = a + b;

        crate::tools::ToolExecutionResult::success(serde_json::json!({
            "result": result,
            "operation": "add",
            "a": a,
            "b": b
        }))
    }
}

/// Tool that subtracts two numbers
pub struct SubtractTool;

#[async_trait]
impl Tool for SubtractTool {
    fn name(&self) -> &str {
        "subtract"
    }

    fn description(&self) -> &str {
        "Subtract the second number from the first and return the result."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "a": {
                    "type": "number",
                    "description": "The number to subtract from"
                },
                "b": {
                    "type": "number",
                    "description": "The number to subtract"
                }
            },
            "required": ["a", "b"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, arguments: Value) -> crate::tools::ToolExecutionResult {
        let a = arguments.get("a").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let b = arguments.get("b").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let result = a - b;

        crate::tools::ToolExecutionResult::success(serde_json::json!({
            "result": result,
            "operation": "subtract",
            "a": a,
            "b": b
        }))
    }
}

/// Tool that multiplies two numbers
pub struct MultiplyTool;

#[async_trait]
impl Tool for MultiplyTool {
    fn name(&self) -> &str {
        "multiply"
    }

    fn description(&self) -> &str {
        "Multiply two numbers together and return the result."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "a": {
                    "type": "number",
                    "description": "The first number"
                },
                "b": {
                    "type": "number",
                    "description": "The second number"
                }
            },
            "required": ["a", "b"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, arguments: Value) -> crate::tools::ToolExecutionResult {
        let a = arguments.get("a").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let b = arguments.get("b").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let result = a * b;

        crate::tools::ToolExecutionResult::success(serde_json::json!({
            "result": result,
            "operation": "multiply",
            "a": a,
            "b": b
        }))
    }
}

/// Tool that divides two numbers
pub struct DivideTool;

#[async_trait]
impl Tool for DivideTool {
    fn name(&self) -> &str {
        "divide"
    }

    fn description(&self) -> &str {
        "Divide the first number by the second and return the result. Returns an error if dividing by zero."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "a": {
                    "type": "number",
                    "description": "The dividend (number to be divided)"
                },
                "b": {
                    "type": "number",
                    "description": "The divisor (number to divide by)"
                }
            },
            "required": ["a", "b"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, arguments: Value) -> crate::tools::ToolExecutionResult {
        let a = arguments.get("a").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let b = arguments.get("b").and_then(|v| v.as_f64()).unwrap_or(0.0);

        if b == 0.0 {
            return crate::tools::ToolExecutionResult::tool_error("Cannot divide by zero");
        }

        let result = a / b;

        crate::tools::ToolExecutionResult::success(serde_json::json!({
            "result": result,
            "operation": "divide",
            "a": a,
            "b": b
        }))
    }
}

// ============================================================================
// Weather Tools (Mocked for Testing)
// ============================================================================

/// Tool that returns mock weather data for a location
pub struct GetWeatherTool;

#[async_trait]
impl Tool for GetWeatherTool {
    fn name(&self) -> &str {
        "get_weather"
    }

    fn description(&self) -> &str {
        "Get the current weather for a location. Returns temperature, conditions, humidity, and wind speed."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "location": {
                    "type": "string",
                    "description": "The city or location name (e.g., 'New York', 'London', 'Tokyo')"
                },
                "units": {
                    "type": "string",
                    "enum": ["celsius", "fahrenheit"],
                    "description": "Temperature units. Defaults to 'celsius'."
                }
            },
            "required": ["location"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, arguments: Value) -> crate::tools::ToolExecutionResult {
        let location = arguments
            .get("location")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown");

        let units = arguments
            .get("units")
            .and_then(|v| v.as_str())
            .unwrap_or("celsius");

        // Generate deterministic mock weather based on location hash
        let hash = location
            .bytes()
            .fold(0u32, |acc, b| acc.wrapping_add(b as u32));
        let temp_c = ((hash % 35) as i32) + 5; // 5-40°C range
        let temp = if units == "fahrenheit" {
            (temp_c as f64 * 9.0 / 5.0) + 32.0
        } else {
            temp_c as f64
        };

        let conditions = match hash % 5 {
            0 => "sunny",
            1 => "partly cloudy",
            2 => "cloudy",
            3 => "rainy",
            _ => "windy",
        };

        let humidity = (hash % 50) + 30; // 30-80%
        let wind_speed = (hash % 30) + 5; // 5-35 km/h

        crate::tools::ToolExecutionResult::success(serde_json::json!({
            "location": location,
            "temperature": temp,
            "units": units,
            "conditions": conditions,
            "humidity": humidity,
            "wind_speed_kmh": wind_speed,
            "timestamp": chrono::Utc::now().to_rfc3339()
        }))
    }
}

/// Tool that returns mock weather forecast for a location
pub struct GetForecastTool;

#[async_trait]
impl Tool for GetForecastTool {
    fn name(&self) -> &str {
        "get_forecast"
    }

    fn description(&self) -> &str {
        "Get the weather forecast for a location for the next several days."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "location": {
                    "type": "string",
                    "description": "The city or location name (e.g., 'New York', 'London', 'Tokyo')"
                },
                "days": {
                    "type": "integer",
                    "description": "Number of days to forecast (1-7). Defaults to 3."
                },
                "units": {
                    "type": "string",
                    "enum": ["celsius", "fahrenheit"],
                    "description": "Temperature units. Defaults to 'celsius'."
                }
            },
            "required": ["location"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, arguments: Value) -> crate::tools::ToolExecutionResult {
        let location = arguments
            .get("location")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown");

        let days = arguments
            .get("days")
            .and_then(|v| v.as_u64())
            .unwrap_or(3)
            .min(7) as usize;

        let units = arguments
            .get("units")
            .and_then(|v| v.as_str())
            .unwrap_or("celsius");

        // Generate deterministic mock forecast based on location hash
        let hash = location
            .bytes()
            .fold(0u32, |acc, b| acc.wrapping_add(b as u32));

        let today = chrono::Utc::now().date_naive();
        let mut forecast_days = Vec::new();

        for day_offset in 0..days {
            let day_hash = hash.wrapping_add(day_offset as u32 * 7);
            let temp_c = ((day_hash % 35) as i32) + 5;
            let temp_high = if units == "fahrenheit" {
                (temp_c as f64 * 9.0 / 5.0) + 32.0
            } else {
                temp_c as f64
            };
            let temp_low = temp_high - 8.0 - ((day_hash % 5) as f64);

            let conditions = match day_hash % 5 {
                0 => "sunny",
                1 => "partly cloudy",
                2 => "cloudy",
                3 => "rainy",
                _ => "windy",
            };

            let date = today + chrono::Duration::days(day_offset as i64);

            forecast_days.push(serde_json::json!({
                "date": date.to_string(),
                "high": temp_high,
                "low": temp_low,
                "conditions": conditions,
                "precipitation_chance": (day_hash % 100) as i32
            }));
        }

        crate::tools::ToolExecutionResult::success(serde_json::json!({
            "location": location,
            "units": units,
            "days": days,
            "forecast": forecast_days
        }))
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_capability_registry_with_builtins() {
        let registry = CapabilityRegistry::with_builtins();

        assert!(registry.has(CapabilityId::Noop));
        assert!(registry.has(CapabilityId::CurrentTime));
        assert!(registry.has(CapabilityId::Research));
        assert!(registry.has(CapabilityId::Sandbox));
        assert!(registry.has(CapabilityId::FileSystem));
        assert!(registry.has(CapabilityId::TestMath));
        assert!(registry.has(CapabilityId::TestWeather));
        assert_eq!(registry.len(), 7);
    }

    #[test]
    fn test_capability_registry_get() {
        let registry = CapabilityRegistry::with_builtins();

        let noop = registry.get(CapabilityId::Noop).unwrap();
        assert_eq!(noop.id(), CapabilityId::Noop);
        assert_eq!(noop.name(), "No-Op");
        assert_eq!(noop.status(), CapabilityStatus::Available);
    }

    #[test]
    fn test_capability_status() {
        let registry = CapabilityRegistry::with_builtins();

        let current_time = registry.get(CapabilityId::CurrentTime).unwrap();
        assert_eq!(current_time.status(), CapabilityStatus::Available);

        let research = registry.get(CapabilityId::Research).unwrap();
        assert_eq!(research.status(), CapabilityStatus::ComingSoon);
    }

    #[test]
    fn test_current_time_capability_has_tools() {
        let registry = CapabilityRegistry::with_builtins();

        let current_time = registry.get(CapabilityId::CurrentTime).unwrap();
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

        let applied = apply_capabilities(base_config.clone(), &[CapabilityId::Noop], &registry);

        // Noop has no system prompt addition or tools
        assert_eq!(applied.config.system_prompt, base_config.system_prompt);
        assert!(applied.tool_registry.is_empty());
        assert_eq!(applied.applied_ids, vec![CapabilityId::Noop]);
    }

    #[test]
    fn test_apply_capabilities_current_time() {
        let registry = CapabilityRegistry::with_builtins();
        let base_config = AgentConfig::new("You are a helpful assistant.", "gpt-5.2");

        let applied =
            apply_capabilities(base_config.clone(), &[CapabilityId::CurrentTime], &registry);

        // CurrentTime has no system prompt addition but has a tool
        assert_eq!(applied.config.system_prompt, base_config.system_prompt);
        assert!(applied.tool_registry.has("get_current_time"));
        assert_eq!(applied.tool_registry.len(), 1);
        assert_eq!(applied.applied_ids, vec![CapabilityId::CurrentTime]);
    }

    #[test]
    fn test_apply_capabilities_skips_coming_soon() {
        let registry = CapabilityRegistry::with_builtins();
        let base_config = AgentConfig::new("You are a helpful assistant.", "gpt-5.2");

        // Research is ComingSoon, so it should be skipped
        let applied = apply_capabilities(base_config.clone(), &[CapabilityId::Research], &registry);

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
            &[CapabilityId::Noop, CapabilityId::CurrentTime],
            &registry,
        );

        assert!(applied.tool_registry.has("get_current_time"));
        assert_eq!(
            applied.applied_ids,
            vec![CapabilityId::Noop, CapabilityId::CurrentTime]
        );
    }

    #[test]
    fn test_apply_capabilities_preserves_order() {
        let registry = CapabilityRegistry::with_builtins();
        let base_config = AgentConfig::new("Base prompt.", "gpt-5.2");

        // Order should be preserved in applied_ids
        let applied = apply_capabilities(
            base_config,
            &[CapabilityId::CurrentTime, CapabilityId::Noop],
            &registry,
        );

        assert_eq!(
            applied.applied_ids,
            vec![CapabilityId::CurrentTime, CapabilityId::Noop]
        );
    }

    #[test]
    fn test_capability_registry_builder() {
        let registry = CapabilityRegistry::builder()
            .capability(NoopCapability)
            .capability(CurrentTimeCapability)
            .build();

        assert!(registry.has(CapabilityId::Noop));
        assert!(registry.has(CapabilityId::CurrentTime));
        assert_eq!(registry.len(), 2);
    }

    #[test]
    fn test_capability_icons_and_categories() {
        let registry = CapabilityRegistry::with_builtins();

        let noop = registry.get(CapabilityId::Noop).unwrap();
        assert_eq!(noop.icon(), Some("circle-off"));
        assert_eq!(noop.category(), Some("Testing"));

        let current_time = registry.get(CapabilityId::CurrentTime).unwrap();
        assert_eq!(current_time.icon(), Some("clock"));
        assert_eq!(current_time.category(), Some("Utilities"));
    }

    #[tokio::test]
    async fn test_get_current_time_tool_iso8601() {
        let tool = GetCurrentTimeTool;
        let result = tool.execute(serde_json::json!({})).await;

        if let crate::tools::ToolExecutionResult::Success(value) = result {
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

        if let crate::tools::ToolExecutionResult::Success(value) = result {
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

        if let crate::tools::ToolExecutionResult::Success(value) = result {
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
        let math = registry.get(CapabilityId::TestMath).unwrap();
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

        if let crate::tools::ToolExecutionResult::Success(value) = result {
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

        if let crate::tools::ToolExecutionResult::Success(value) = result {
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

        if let crate::tools::ToolExecutionResult::Success(value) = result {
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

        if let crate::tools::ToolExecutionResult::Success(value) = result {
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

        if let crate::tools::ToolExecutionResult::ToolError(msg) = result {
            assert!(msg.contains("divide by zero"));
        } else {
            panic!("Expected tool error for division by zero");
        }
    }

    // TestWeather capability tests
    #[test]
    fn test_test_weather_capability_has_tools() {
        let registry = CapabilityRegistry::with_builtins();
        let weather = registry.get(CapabilityId::TestWeather).unwrap();
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

        if let crate::tools::ToolExecutionResult::Success(value) = result {
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

        if let crate::tools::ToolExecutionResult::Success(value) = result {
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

        if let crate::tools::ToolExecutionResult::Success(value) = result {
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

        let applied = apply_capabilities(base_config.clone(), &[CapabilityId::TestMath], &registry);

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

        let applied =
            apply_capabilities(base_config.clone(), &[CapabilityId::TestWeather], &registry);

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
            &[CapabilityId::TestMath, CapabilityId::TestWeather],
            &registry,
        );

        // Should have both sets of tools
        assert_eq!(applied.tool_registry.len(), 6); // 4 math + 2 weather
        assert!(applied.tool_registry.has("add"));
        assert!(applied.tool_registry.has("get_weather"));
    }
}

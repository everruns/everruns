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
//! - apply_capabilities() merges capability contributions into RuntimeAgent
//! - The agent-loop remains execution-focused; capabilities are applied before execution
//!
//! Each capability is in its own file with collocated tools.

use crate::runtime_agent::RuntimeAgent;
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
mod fake_aws;
mod fake_crm;
mod fake_financial;
mod fake_warehouse;
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
pub use fake_aws::{
    AwsCreateEc2InstanceTool, AwsCreateIamUserTool, AwsCreateRdsDatabaseTool,
    AwsCreateS3BucketTool, AwsGetCloudWatchMetricsTool, AwsListEc2InstancesTool,
    AwsListIamUsersTool, AwsListRdsDatabasesTool, AwsListS3BucketsTool, AwsListSecurityGroupsTool,
    AwsStopEc2InstanceTool, FakeAwsCapability,
};
pub use fake_crm::{
    CrmAddInteractionTool, CrmCreateCustomerTool, CrmCreateTicketTool, CrmGetCustomerTool,
    CrmListCustomersTool, CrmListTicketsTool, CrmSearchCustomersTool, CrmUpdateTicketTool,
    FakeCrmCapability,
};
pub use fake_financial::{
    FakeFinancialCapability, FinanceCreateBudgetTool, FinanceCreateTransactionTool,
    FinanceForecastCashFlowTool, FinanceGetBalanceTool, FinanceGetExpenseReportTool,
    FinanceGetRevenueReportTool, FinanceListBudgetsTool, FinanceListTransactionsTool,
};
pub use fake_warehouse::{
    FakeWarehouseCapability, WarehouseCreateInvoiceTool, WarehouseCreateOrderTool,
    WarehouseCreateShipmentTool, WarehouseGetInventoryTool, WarehouseInventoryReportTool,
    WarehouseListOrdersTool, WarehouseListShipmentsTool, WarehouseProcessReturnTool,
    WarehouseUpdateInventoryTool, WarehouseUpdateShipmentStatusTool,
};
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
/// applying multiple capabilities to build a RuntimeAgent.
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
        // Fake demo capabilities
        registry.register(FakeWarehouseCapability);
        registry.register(FakeAwsCapability);
        registry.register(FakeCrmCapability);
        registry.register(FakeFinancialCapability);
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
// Collect Capabilities Helper
// ============================================================================

/// Collected data from capabilities before applying to config.
///
/// This intermediate struct allows sharing the capability collection logic
/// between `apply_capabilities` and `apply_capabilities_to_builder`.
pub struct CollectedCapabilities {
    /// System prompt additions (in order)
    pub system_prompt_parts: Vec<String>,
    /// Tool implementations for the registry
    pub tools: Vec<Box<dyn Tool>>,
    /// Tool definitions for config
    pub tool_definitions: Vec<ToolDefinition>,
    /// IDs of capabilities that were collected
    pub applied_ids: Vec<String>,
}

impl CollectedCapabilities {
    /// Returns the combined system prompt prefix from all capabilities.
    /// Returns None if no capabilities contributed system prompt additions.
    pub fn system_prompt_prefix(&self) -> Option<String> {
        if self.system_prompt_parts.is_empty() {
            None
        } else {
            Some(self.system_prompt_parts.join("\n\n"))
        }
    }
}

/// Collect contributions from capabilities without applying them.
///
/// This extracts system prompt additions, tools, and tool definitions from
/// the given capabilities. Use this when you need the raw capability data
/// before applying it to a config or builder.
///
/// # Arguments
///
/// * `capability_ids` - Ordered list of capability IDs to collect
/// * `registry` - The capability registry containing implementations
pub fn collect_capabilities(
    capability_ids: &[String],
    registry: &CapabilityRegistry,
) -> CollectedCapabilities {
    let mut system_prompt_parts: Vec<String> = Vec::new();
    let mut tools: Vec<Box<dyn Tool>> = Vec::new();
    let mut tool_definitions: Vec<ToolDefinition> = Vec::new();
    let mut applied_ids: Vec<String> = Vec::new();

    for cap_id in capability_ids {
        if let Some(capability) = registry.get(cap_id) {
            // Only collect from available capabilities
            if capability.status() != CapabilityStatus::Available {
                continue;
            }

            // Collect system prompt addition
            if let Some(addition) = capability.system_prompt_addition() {
                system_prompt_parts.push(addition.to_string());
            }

            // Collect tools
            tools.extend(capability.tools());

            // Collect tool definitions
            tool_definitions.extend(capability.tool_definitions());

            applied_ids.push(cap_id.clone());
        }
    }

    CollectedCapabilities {
        system_prompt_parts,
        tools,
        tool_definitions,
        applied_ids,
    }
}

// ============================================================================
// Apply Capabilities to RuntimeAgent
// ============================================================================

/// Result of applying capabilities to a base runtime agent
pub struct AppliedCapabilities {
    /// The modified runtime agent with capability contributions merged
    pub runtime_agent: RuntimeAgent,
    /// Tool registry containing all capability tools
    pub tool_registry: ToolRegistry,
    /// IDs of capabilities that were applied
    pub applied_ids: Vec<String>,
}

/// Apply capabilities to a base runtime agent configuration.
///
/// This function:
/// 1. Collects system prompt additions from capabilities (in order)
/// 2. Prepends them to the agent's base system prompt
/// 3. Collects all tools from capabilities
/// 4. Returns the modified runtime agent and a tool registry
///
/// # Arguments
///
/// * `base_runtime_agent` - The agent's base runtime configuration
/// * `capability_ids` - Ordered list of capability IDs to apply
/// * `registry` - The capability registry containing implementations
///
/// # Returns
///
/// An `AppliedCapabilities` struct containing the modified runtime agent,
/// tool registry, and list of applied capability IDs.
///
/// # Example
///
/// ```
/// use everruns_core::capabilities::{apply_capabilities, CapabilityRegistry, CapabilityId};
/// use everruns_core::runtime_agent::RuntimeAgent;
///
/// let registry = CapabilityRegistry::with_builtins();
/// let base_runtime_agent = RuntimeAgent::new("You are a helpful assistant.", "gpt-5.2");
///
/// let capability_ids = vec![CapabilityId::CURRENT_TIME.to_string()];
/// let applied = apply_capabilities(base_runtime_agent, &capability_ids, &registry);
///
/// // The runtime agent now includes CurrentTime tool
/// assert!(!applied.tool_registry.is_empty());
/// ```
pub fn apply_capabilities(
    base_runtime_agent: RuntimeAgent,
    capability_ids: &[String],
    registry: &CapabilityRegistry,
) -> AppliedCapabilities {
    let collected = collect_capabilities(capability_ids, registry);

    // Build final system prompt: capability additions + base prompt
    let final_system_prompt = match collected.system_prompt_prefix() {
        Some(prefix) => format!("{}\n\n{}", prefix, base_runtime_agent.system_prompt),
        None => base_runtime_agent.system_prompt,
    };

    // Build tool registry from collected tools
    let mut tool_registry = ToolRegistry::new();
    for tool in collected.tools {
        tool_registry.register_boxed(tool);
    }

    // Create modified runtime agent
    let runtime_agent = RuntimeAgent {
        system_prompt: final_system_prompt,
        model: base_runtime_agent.model,
        tools: collected.tool_definitions,
        max_iterations: base_runtime_agent.max_iterations,
        temperature: base_runtime_agent.temperature,
        max_tokens: base_runtime_agent.max_tokens,
    };

    AppliedCapabilities {
        runtime_agent,
        tool_registry,
        applied_ids: collected.applied_ids,
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // CapabilityRegistry tests
    // =========================================================================

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
        assert!(registry.has(CapabilityId::FAKE_WAREHOUSE));
        assert!(registry.has(CapabilityId::FAKE_AWS));
        assert!(registry.has(CapabilityId::FAKE_CRM));
        assert!(registry.has(CapabilityId::FAKE_FINANCIAL));
        assert_eq!(registry.len(), 13);
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
    fn test_capability_status() {
        let registry = CapabilityRegistry::with_builtins();

        let current_time = registry.get(CapabilityId::CURRENT_TIME).unwrap();
        assert_eq!(current_time.status(), CapabilityStatus::Available);

        let research = registry.get(CapabilityId::RESEARCH).unwrap();
        assert_eq!(research.status(), CapabilityStatus::ComingSoon);
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

    // =========================================================================
    // apply_capabilities tests
    // =========================================================================

    #[test]
    fn test_apply_capabilities_empty() {
        let registry = CapabilityRegistry::with_builtins();
        let base_runtime_agent = RuntimeAgent::new("You are a helpful assistant.", "gpt-5.2");

        let applied = apply_capabilities(base_runtime_agent.clone(), &[], &registry);

        assert_eq!(
            applied.runtime_agent.system_prompt,
            base_runtime_agent.system_prompt
        );
        assert!(applied.tool_registry.is_empty());
        assert!(applied.applied_ids.is_empty());
    }

    #[test]
    fn test_apply_capabilities_noop() {
        let registry = CapabilityRegistry::with_builtins();
        let base_runtime_agent = RuntimeAgent::new("You are a helpful assistant.", "gpt-5.2");

        let applied = apply_capabilities(
            base_runtime_agent.clone(),
            &[CapabilityId::NOOP.to_string()],
            &registry,
        );

        // Noop has no system prompt addition or tools
        assert_eq!(
            applied.runtime_agent.system_prompt,
            base_runtime_agent.system_prompt
        );
        assert!(applied.tool_registry.is_empty());
        assert_eq!(applied.applied_ids, vec![CapabilityId::NOOP]);
    }

    #[test]
    fn test_apply_capabilities_current_time() {
        let registry = CapabilityRegistry::with_builtins();
        let base_runtime_agent = RuntimeAgent::new("You are a helpful assistant.", "gpt-5.2");

        let applied = apply_capabilities(
            base_runtime_agent.clone(),
            &[CapabilityId::CURRENT_TIME.to_string()],
            &registry,
        );

        // CurrentTime has no system prompt addition but has a tool
        assert_eq!(
            applied.runtime_agent.system_prompt,
            base_runtime_agent.system_prompt
        );
        assert!(applied.tool_registry.has("get_current_time"));
        assert_eq!(applied.tool_registry.len(), 1);
        assert_eq!(applied.applied_ids, vec![CapabilityId::CURRENT_TIME]);
    }

    #[test]
    fn test_apply_capabilities_skips_coming_soon() {
        let registry = CapabilityRegistry::with_builtins();
        let base_runtime_agent = RuntimeAgent::new("You are a helpful assistant.", "gpt-5.2");

        // Research is ComingSoon, so it should be skipped
        let applied = apply_capabilities(
            base_runtime_agent.clone(),
            &[CapabilityId::RESEARCH.to_string()],
            &registry,
        );

        // System prompt should not have the research addition
        assert_eq!(
            applied.runtime_agent.system_prompt,
            base_runtime_agent.system_prompt
        );
        assert!(applied.applied_ids.is_empty()); // Research was not applied
    }

    #[test]
    fn test_apply_capabilities_multiple() {
        let registry = CapabilityRegistry::with_builtins();
        let base_runtime_agent = RuntimeAgent::new("You are a helpful assistant.", "gpt-5.2");

        let applied = apply_capabilities(
            base_runtime_agent.clone(),
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
        let base_runtime_agent = RuntimeAgent::new("Base prompt.", "gpt-5.2");

        // Order should be preserved in applied_ids
        let applied = apply_capabilities(
            base_runtime_agent,
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
    fn test_apply_capabilities_test_math() {
        let registry = CapabilityRegistry::with_builtins();
        let base_runtime_agent = RuntimeAgent::new("You are a helpful assistant.", "gpt-5.2");

        let applied = apply_capabilities(
            base_runtime_agent.clone(),
            &[CapabilityId::TEST_MATH.to_string()],
            &registry,
        );

        // TestMath has system prompt addition and 4 tools
        assert!(applied.runtime_agent.system_prompt.contains("math tools"));
        assert!(applied.tool_registry.has("add"));
        assert!(applied.tool_registry.has("subtract"));
        assert!(applied.tool_registry.has("multiply"));
        assert!(applied.tool_registry.has("divide"));
        assert_eq!(applied.tool_registry.len(), 4);
    }

    #[test]
    fn test_apply_capabilities_test_weather() {
        let registry = CapabilityRegistry::with_builtins();
        let base_runtime_agent = RuntimeAgent::new("You are a helpful assistant.", "gpt-5.2");

        let applied = apply_capabilities(
            base_runtime_agent.clone(),
            &[CapabilityId::TEST_WEATHER.to_string()],
            &registry,
        );

        // TestWeather has system prompt addition and 2 tools
        assert!(applied
            .runtime_agent
            .system_prompt
            .contains("weather tools"));
        assert!(applied.tool_registry.has("get_weather"));
        assert!(applied.tool_registry.has("get_forecast"));
        assert_eq!(applied.tool_registry.len(), 2);
    }

    #[test]
    fn test_apply_capabilities_test_math_and_test_weather() {
        let registry = CapabilityRegistry::with_builtins();
        let base_runtime_agent = RuntimeAgent::new("You are a helpful assistant.", "gpt-5.2");

        let applied = apply_capabilities(
            base_runtime_agent.clone(),
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

    #[test]
    fn test_apply_capabilities_stateless_todo_list() {
        let registry = CapabilityRegistry::with_builtins();
        let base_runtime_agent = RuntimeAgent::new("You are a helpful assistant.", "gpt-5.2");

        let applied = apply_capabilities(
            base_runtime_agent.clone(),
            &[CapabilityId::STATELESS_TODO_LIST.to_string()],
            &registry,
        );

        // StatelessTodoList has system prompt addition and 1 tool
        assert!(applied
            .runtime_agent
            .system_prompt
            .contains("Task Management"));
        assert!(applied.runtime_agent.system_prompt.contains("write_todos"));
        assert!(applied.tool_registry.has("write_todos"));
        assert_eq!(applied.tool_registry.len(), 1);
    }

    #[test]
    fn test_apply_capabilities_web_fetch() {
        let registry = CapabilityRegistry::with_builtins();
        let base_runtime_agent = RuntimeAgent::new("You are a helpful assistant.", "gpt-5.2");

        let applied = apply_capabilities(
            base_runtime_agent.clone(),
            &[CapabilityId::WEB_FETCH.to_string()],
            &registry,
        );

        // WebFetch has no system prompt addition but has 1 tool
        assert_eq!(
            applied.runtime_agent.system_prompt,
            base_runtime_agent.system_prompt
        );
        assert!(applied.tool_registry.has("web_fetch"));
        assert_eq!(applied.tool_registry.len(), 1);
    }
}

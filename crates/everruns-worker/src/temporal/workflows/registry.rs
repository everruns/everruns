// Workflow registry for dynamic workflow creation
// Decision: Factory functions allow runtime workflow creation from JSON input
// Decision: Builder pattern for fluent registration API
// Decision: with_defaults() registers TemporalSessionWorkflow

use std::collections::HashMap;

use anyhow::{anyhow, Result};

use super::traits::{Workflow, WorkflowInput};

/// Factory function for creating workflow instances from JSON input.
pub type WorkflowFactory =
    Box<dyn Fn(serde_json::Value) -> Result<Box<dyn Workflow>> + Send + Sync>;

/// Registry that maps workflow type names to their factory functions.
///
/// # Example
///
/// ```ignore
/// let registry = WorkflowRegistry::builder()
///     .workflow::<TemporalSessionWorkflow>()
///     .build();
///
/// let workflow = registry.create("session_workflow", input_json)?;
/// ```
pub struct WorkflowRegistry {
    factories: HashMap<&'static str, WorkflowFactory>,
}

impl WorkflowRegistry {
    /// Create a new empty registry
    pub fn new() -> Self {
        Self {
            factories: HashMap::new(),
        }
    }

    /// Create a registry with default workflows (TemporalSessionWorkflow) registered
    pub fn with_defaults() -> Self {
        Self::builder()
            .workflow::<super::TemporalSessionWorkflow>()
            .build()
    }

    /// Register a workflow factory for a workflow type
    pub fn register(&mut self, workflow_type: &'static str, factory: WorkflowFactory) {
        self.factories.insert(workflow_type, factory);
    }

    /// Create a workflow instance from JSON input
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The workflow type is not registered
    /// - The input JSON cannot be parsed for this workflow type
    pub fn create(
        &self,
        workflow_type: &str,
        input: serde_json::Value,
    ) -> Result<Box<dyn Workflow>> {
        let factory = self.factories.get(workflow_type).ok_or_else(|| {
            anyhow!(
                "Unknown workflow type: '{}'. Registered types: {:?}",
                workflow_type,
                self.types()
            )
        })?;

        factory(input)
    }

    /// Check if a workflow type is registered
    pub fn has(&self, workflow_type: &str) -> bool {
        self.factories.contains_key(workflow_type)
    }

    /// Get all registered workflow types
    pub fn types(&self) -> Vec<&str> {
        self.factories.keys().copied().collect()
    }

    /// Get the number of registered workflow types
    pub fn len(&self) -> usize {
        self.factories.len()
    }

    /// Check if the registry is empty
    pub fn is_empty(&self) -> bool {
        self.factories.is_empty()
    }

    /// Create a builder for fluent registration
    pub fn builder() -> WorkflowRegistryBuilder {
        WorkflowRegistryBuilder::new()
    }
}

impl Default for WorkflowRegistry {
    fn default() -> Self {
        Self::with_defaults()
    }
}

impl std::fmt::Debug for WorkflowRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkflowRegistry")
            .field("workflow_types", &self.types())
            .finish()
    }
}

/// Builder for creating a WorkflowRegistry with a fluent API.
pub struct WorkflowRegistryBuilder {
    registry: WorkflowRegistry,
}

impl WorkflowRegistryBuilder {
    /// Create a new builder with an empty registry
    pub fn new() -> Self {
        Self {
            registry: WorkflowRegistry::new(),
        }
    }

    /// Register a workflow type using its WorkflowInput implementation
    pub fn workflow<W: WorkflowInput>(mut self) -> Self {
        self.registry.register(W::WORKFLOW_TYPE, W::factory());
        self
    }

    /// Register a workflow with a custom factory
    pub fn workflow_factory(
        mut self,
        workflow_type: &'static str,
        factory: WorkflowFactory,
    ) -> Self {
        self.registry.register(workflow_type, factory);
        self
    }

    /// Build the registry
    pub fn build(self) -> WorkflowRegistry {
        self.registry
    }
}

impl Default for WorkflowRegistryBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::temporal::types::SessionWorkflowInput;
    use uuid::Uuid;

    #[test]
    fn test_registry_new_is_empty() {
        let registry = WorkflowRegistry::new();
        assert!(registry.is_empty());
        assert_eq!(registry.len(), 0);
    }

    #[test]
    fn test_registry_with_defaults() {
        let registry = WorkflowRegistry::with_defaults();
        assert!(registry.has("session_workflow"));
        assert!(!registry.is_empty());
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn test_registry_create_workflow() {
        let registry = WorkflowRegistry::with_defaults();

        let input = serde_json::to_value(SessionWorkflowInput {
            session_id: Uuid::now_v7(),
            agent_id: Uuid::now_v7(),
        })
        .unwrap();

        let workflow = registry.create("session_workflow", input);
        assert!(workflow.is_ok());
        assert_eq!(workflow.unwrap().workflow_type(), "session_workflow");
    }

    #[test]
    fn test_registry_unknown_type() {
        let registry = WorkflowRegistry::with_defaults();

        let result = registry.create("unknown_workflow", serde_json::json!({}));
        assert!(result.is_err());
        let error = result.unwrap_err().to_string();
        assert!(error.contains("Unknown workflow type"));
        assert!(error.contains("unknown_workflow"));
    }

    #[test]
    fn test_registry_invalid_input() {
        let registry = WorkflowRegistry::with_defaults();

        // Missing required fields
        let result = registry.create("session_workflow", serde_json::json!({"invalid": true}));
        assert!(result.is_err());
        let error = result.unwrap_err().to_string();
        assert!(error.contains("Failed to parse"));
    }

    #[test]
    fn test_registry_builder() {
        let registry = WorkflowRegistry::builder()
            .workflow::<super::super::TemporalSessionWorkflow>()
            .build();

        assert!(registry.has("session_workflow"));
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn test_registry_types() {
        let registry = WorkflowRegistry::with_defaults();
        let types = registry.types();
        assert!(types.contains(&"session_workflow"));
    }

    #[test]
    fn test_registry_debug() {
        let registry = WorkflowRegistry::with_defaults();
        let debug_str = format!("{:?}", registry);
        assert!(debug_str.contains("WorkflowRegistry"));
        assert!(debug_str.contains("session_workflow"));
    }

    #[test]
    fn test_registry_default_impl() {
        let registry = WorkflowRegistry::default();
        assert!(registry.has("session_workflow"));
    }
}

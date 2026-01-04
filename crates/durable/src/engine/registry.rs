//! Workflow registry for type-erased workflow creation
//!
//! The registry allows registering workflow factories that create workflow
//! instances from JSON input without knowing the concrete type at runtime.

use std::collections::HashMap;
use std::fmt;

use serde_json::Value;

use crate::activity::ActivityError;
use crate::workflow::{Workflow, WorkflowAction, WorkflowError, WorkflowSignal};

/// Type-erased workflow interface
///
/// This trait allows the executor to work with workflows without knowing
/// their concrete types. All method parameters and return values are JSON.
pub trait AnyWorkflow: Send + Sync {
    /// Get the workflow type identifier
    fn workflow_type(&self) -> &'static str;

    /// Called when workflow starts
    fn on_start(&mut self) -> Vec<WorkflowAction>;

    /// Called when an activity completes
    fn on_activity_completed(
        &mut self,
        activity_id: &str,
        result: Value,
    ) -> Vec<WorkflowAction>;

    /// Called when an activity fails
    fn on_activity_failed(
        &mut self,
        activity_id: &str,
        error: &ActivityError,
    ) -> Vec<WorkflowAction>;

    /// Called when a timer fires
    fn on_timer_fired(&mut self, timer_id: &str) -> Vec<WorkflowAction>;

    /// Called when a signal is received
    fn on_signal(&mut self, signal: &WorkflowSignal) -> Vec<WorkflowAction>;

    /// Check if workflow has completed
    fn is_completed(&self) -> bool;

    /// Get the result as JSON (if completed successfully)
    fn result_json(&self) -> Option<Value>;

    /// Get the error (if failed)
    fn error(&self) -> Option<WorkflowError>;
}

/// Wrapper to implement AnyWorkflow for any Workflow
struct WorkflowWrapper<W: Workflow> {
    inner: W,
}

impl<W: Workflow> AnyWorkflow for WorkflowWrapper<W> {
    fn workflow_type(&self) -> &'static str {
        W::TYPE
    }

    fn on_start(&mut self) -> Vec<WorkflowAction> {
        self.inner.on_start()
    }

    fn on_activity_completed(
        &mut self,
        activity_id: &str,
        result: Value,
    ) -> Vec<WorkflowAction> {
        self.inner.on_activity_completed(activity_id, result)
    }

    fn on_activity_failed(
        &mut self,
        activity_id: &str,
        error: &ActivityError,
    ) -> Vec<WorkflowAction> {
        self.inner.on_activity_failed(activity_id, error)
    }

    fn on_timer_fired(&mut self, timer_id: &str) -> Vec<WorkflowAction> {
        self.inner.on_timer_fired(timer_id)
    }

    fn on_signal(&mut self, signal: &WorkflowSignal) -> Vec<WorkflowAction> {
        self.inner.on_signal(signal)
    }

    fn is_completed(&self) -> bool {
        self.inner.is_completed()
    }

    fn result_json(&self) -> Option<Value> {
        self.inner.result().map(|r| {
            serde_json::to_value(r).unwrap_or(Value::Null)
        })
    }

    fn error(&self) -> Option<WorkflowError> {
        self.inner.error()
    }
}

/// Factory function type for creating workflows from JSON input
pub type WorkflowFactory = Box<dyn Fn(Value) -> Result<Box<dyn AnyWorkflow>, serde_json::Error> + Send + Sync>;

/// Registry of workflow factories
///
/// The registry maps workflow type names to factory functions that create
/// workflow instances from JSON input.
pub struct WorkflowRegistry {
    factories: HashMap<String, WorkflowFactory>,
}

impl Default for WorkflowRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl WorkflowRegistry {
    /// Create a new empty registry
    pub fn new() -> Self {
        Self {
            factories: HashMap::new(),
        }
    }

    /// Register a workflow type
    ///
    /// # Example
    ///
    /// ```ignore
    /// let mut registry = WorkflowRegistry::new();
    /// registry.register::<MyWorkflow>();
    /// ```
    pub fn register<W: Workflow>(&mut self) {
        let factory: WorkflowFactory = Box::new(|input: Value| {
            let typed_input: W::Input = serde_json::from_value(input)?;
            let workflow = W::new(typed_input);
            Ok(Box::new(WorkflowWrapper { inner: workflow }) as Box<dyn AnyWorkflow>)
        });

        self.factories.insert(W::TYPE.to_string(), factory);
    }

    /// Check if a workflow type is registered
    pub fn contains(&self, workflow_type: &str) -> bool {
        self.factories.contains_key(workflow_type)
    }

    /// Create a workflow instance from type name and JSON input
    pub fn create(
        &self,
        workflow_type: &str,
        input: Value,
    ) -> Result<Box<dyn AnyWorkflow>, RegistryError> {
        let factory = self
            .factories
            .get(workflow_type)
            .ok_or_else(|| RegistryError::UnknownWorkflowType(workflow_type.to_string()))?;

        factory(input).map_err(RegistryError::Deserialization)
    }

    /// Get the number of registered workflow types
    pub fn len(&self) -> usize {
        self.factories.len()
    }

    /// Check if registry is empty
    pub fn is_empty(&self) -> bool {
        self.factories.is_empty()
    }

    /// Get all registered workflow type names
    pub fn workflow_types(&self) -> impl Iterator<Item = &str> {
        self.factories.keys().map(|s| s.as_str())
    }
}

impl fmt::Debug for WorkflowRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WorkflowRegistry")
            .field("workflow_types", &self.factories.keys().collect::<Vec<_>>())
            .finish()
    }
}

/// Errors from registry operations
#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    /// Workflow type not registered
    #[error("unknown workflow type: {0}")]
    UnknownWorkflowType(String),

    /// Failed to deserialize workflow input
    #[error("failed to deserialize workflow input: {0}")]
    Deserialization(#[source] serde_json::Error),
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize)]
    struct TestInput {
        value: i32,
    }

    #[derive(Debug, Serialize, Deserialize)]
    struct TestOutput {
        result: i32,
    }

    struct TestWorkflow {
        input: TestInput,
        completed: bool,
    }

    impl Workflow for TestWorkflow {
        const TYPE: &'static str = "test_workflow";
        type Input = TestInput;
        type Output = TestOutput;

        fn new(input: Self::Input) -> Self {
            Self {
                input,
                completed: false,
            }
        }

        fn on_start(&mut self) -> Vec<WorkflowAction> {
            vec![WorkflowAction::schedule_activity(
                "compute",
                "compute_activity",
                serde_json::json!({ "n": self.input.value }),
            )]
        }

        fn on_activity_completed(
            &mut self,
            _activity_id: &str,
            result: Value,
        ) -> Vec<WorkflowAction> {
            self.completed = true;
            let r: i32 = serde_json::from_value(result).unwrap_or(0);
            vec![WorkflowAction::complete(serde_json::json!({ "result": r * 2 }))]
        }

        fn on_activity_failed(
            &mut self,
            _activity_id: &str,
            error: &ActivityError,
        ) -> Vec<WorkflowAction> {
            vec![WorkflowAction::fail(WorkflowError::new(&error.message))]
        }

        fn is_completed(&self) -> bool {
            self.completed
        }

        fn result(&self) -> Option<Self::Output> {
            if self.completed {
                Some(TestOutput { result: self.input.value * 2 })
            } else {
                None
            }
        }
    }

    #[test]
    fn test_register_and_create() {
        let mut registry = WorkflowRegistry::new();
        registry.register::<TestWorkflow>();

        assert!(registry.contains("test_workflow"));
        assert!(!registry.contains("unknown"));

        let workflow = registry
            .create("test_workflow", serde_json::json!({ "value": 42 }))
            .expect("should create workflow");

        assert_eq!(workflow.workflow_type(), "test_workflow");
        assert!(!workflow.is_completed());
    }

    #[test]
    fn test_unknown_workflow_type() {
        let registry = WorkflowRegistry::new();
        let result = registry.create("unknown", serde_json::json!({}));

        assert!(matches!(result, Err(RegistryError::UnknownWorkflowType(_))));
    }

    #[test]
    fn test_invalid_input() {
        let mut registry = WorkflowRegistry::new();
        registry.register::<TestWorkflow>();

        // Missing required field
        let result = registry.create("test_workflow", serde_json::json!({}));
        assert!(matches!(result, Err(RegistryError::Deserialization(_))));
    }

    #[test]
    fn test_workflow_execution() {
        let mut registry = WorkflowRegistry::new();
        registry.register::<TestWorkflow>();

        let mut workflow = registry
            .create("test_workflow", serde_json::json!({ "value": 10 }))
            .unwrap();

        // Start workflow
        let actions = workflow.on_start();
        assert_eq!(actions.len(), 1);
        assert!(matches!(actions[0], WorkflowAction::ScheduleActivity { .. }));

        // Complete activity
        let actions = workflow.on_activity_completed("compute", serde_json::json!(5));
        assert_eq!(actions.len(), 1);
        assert!(matches!(actions[0], WorkflowAction::CompleteWorkflow { .. }));

        assert!(workflow.is_completed());
    }

    #[test]
    fn test_registry_debug() {
        let mut registry = WorkflowRegistry::new();
        registry.register::<TestWorkflow>();

        let debug_str = format!("{:?}", registry);
        assert!(debug_str.contains("test_workflow"));
    }

    #[test]
    fn test_workflow_types_iterator() {
        let mut registry = WorkflowRegistry::new();
        registry.register::<TestWorkflow>();

        let types: Vec<_> = registry.workflow_types().collect();
        assert_eq!(types, vec!["test_workflow"]);
    }
}

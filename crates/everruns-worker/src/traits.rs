// Workflow trait and related abstractions
// Decision: Trait is object-safe for storage in HashMap<String, Box<dyn Workflow>>
// Decision: Uses Send + Sync bounds for async Temporal execution
// Decision: WorkflowInput trait enables automatic factory generation

use serde::de::DeserializeOwned;

use crate::types::WorkflowAction;
use crate::workflow_registry::WorkflowFactory;

/// Trait for workflow implementations in the Temporal worker.
///
/// Workflows are state machines that process activations from Temporal
/// and return commands. Each workflow type handles its own state transitions
/// and activity scheduling.
///
/// All implementations must be deterministic and replayable from Temporal history.
pub trait Workflow: Send + Sync + std::fmt::Debug {
    /// Returns the workflow type name (e.g., "session_workflow")
    fn workflow_type(&self) -> &'static str;

    /// Called when the workflow starts
    fn on_start(&mut self) -> Vec<WorkflowAction>;

    /// Called when an activity completes successfully
    fn on_activity_completed(
        &mut self,
        activity_id: &str,
        result: serde_json::Value,
    ) -> Vec<WorkflowAction>;

    /// Called when an activity fails
    fn on_activity_failed(&mut self, activity_id: &str, error: &str) -> Vec<WorkflowAction>;

    /// Returns true if the workflow is in a terminal state (Completed or Failed)
    fn is_completed(&self) -> bool;
}

/// Trait for workflows with typed input, enabling automatic factory generation.
///
/// Implement this trait to allow registration via `WorkflowRegistry::builder().workflow::<T>()`.
///
/// # Example
///
/// ```ignore
/// impl WorkflowInput for MyWorkflow {
///     const WORKFLOW_TYPE: &'static str = "my_workflow";
///     type Input = MyWorkflowInput;
///
///     fn from_input(input: Self::Input) -> Self {
///         MyWorkflow::new(input)
///     }
/// }
/// ```
pub trait WorkflowInput: Workflow + Sized + 'static {
    /// The workflow type name used in Temporal
    const WORKFLOW_TYPE: &'static str;

    /// The input type for this workflow (must be deserializable from JSON)
    type Input: DeserializeOwned;

    /// Create a workflow instance from parsed input
    fn from_input(input: Self::Input) -> Self;

    /// Create a factory function for this workflow type
    ///
    /// This is automatically implemented and creates a factory that:
    /// 1. Deserializes the JSON input to `Self::Input`
    /// 2. Calls `Self::from_input` to create the workflow
    fn factory() -> WorkflowFactory {
        Box::new(|value: serde_json::Value| {
            let input: Self::Input = serde_json::from_value(value).map_err(|e| {
                anyhow::anyhow!("Failed to parse {} input: {}", Self::WORKFLOW_TYPE, e)
            })?;
            Ok(Box::new(Self::from_input(input)) as Box<dyn Workflow>)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize)]
    struct TestInput {
        value: String,
    }

    #[derive(Debug)]
    struct TestWorkflow {
        _input: TestInput,
        started: bool,
    }

    impl Workflow for TestWorkflow {
        fn workflow_type(&self) -> &'static str {
            "test_workflow"
        }

        fn on_start(&mut self) -> Vec<WorkflowAction> {
            self.started = true;
            vec![WorkflowAction::CompleteWorkflow {
                result: Some(serde_json::json!({"started": true})),
            }]
        }

        fn on_activity_completed(
            &mut self,
            _activity_id: &str,
            _result: serde_json::Value,
        ) -> Vec<WorkflowAction> {
            vec![]
        }

        fn on_activity_failed(&mut self, _activity_id: &str, error: &str) -> Vec<WorkflowAction> {
            vec![WorkflowAction::FailWorkflow {
                reason: error.to_string(),
            }]
        }

        fn is_completed(&self) -> bool {
            self.started
        }
    }

    impl WorkflowInput for TestWorkflow {
        const WORKFLOW_TYPE: &'static str = "test_workflow";
        type Input = TestInput;

        fn from_input(input: Self::Input) -> Self {
            TestWorkflow {
                _input: input,
                started: false,
            }
        }
    }

    #[test]
    fn test_workflow_trait_implementation() {
        let input = TestInput {
            value: "hello".to_string(),
        };
        let mut workflow = TestWorkflow::from_input(input);

        assert_eq!(workflow.workflow_type(), "test_workflow");
        assert!(!workflow.is_completed());

        let actions = workflow.on_start();
        assert_eq!(actions.len(), 1);
        assert!(workflow.is_completed());
    }

    #[test]
    fn test_workflow_factory() {
        let factory = TestWorkflow::factory();
        let input_json = serde_json::json!({"value": "test"});

        let workflow = factory(input_json).expect("Factory should create workflow");
        assert_eq!(workflow.workflow_type(), "test_workflow");
    }

    #[test]
    fn test_workflow_factory_invalid_input() {
        let factory = TestWorkflow::factory();
        let invalid_json = serde_json::json!({"wrong_field": 123});

        let result = factory(invalid_json);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Failed to parse"));
    }
}

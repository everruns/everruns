//! Workflow trait definition

use serde::{de::DeserializeOwned, Serialize};

use super::{WorkflowAction, WorkflowSignal};
use crate::activity::ActivityError;

/// Error type for workflow failures
#[derive(Debug, Clone, Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct WorkflowError {
    /// Error message
    pub message: String,

    /// Error code for programmatic handling
    pub code: Option<String>,

    /// Whether this error is retryable
    pub retryable: bool,
}

impl WorkflowError {
    /// Create a new workflow error
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code: None,
            retryable: false,
        }
    }

    /// Create a retryable error
    pub fn retryable(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code: None,
            retryable: true,
        }
    }

    /// Set the error code
    pub fn with_code(mut self, code: impl Into<String>) -> Self {
        self.code = Some(code.into());
        self
    }
}

impl std::fmt::Display for WorkflowError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for WorkflowError {}

/// A workflow is a deterministic state machine driven by events
///
/// Workflows are the core abstraction for durable execution. They define:
/// - How to start execution (`on_start`)
/// - How to handle activity completions (`on_activity_completed`, `on_activity_failed`)
/// - How to handle timers (`on_timer_fired`)
/// - How to handle external signals (`on_signal`)
///
/// # Determinism
///
/// Workflows must be deterministic - given the same sequence of events, they must
/// produce the same sequence of actions. This enables replay-based recovery.
///
/// # Example
///
/// ```ignore
/// use everruns_durable::prelude::*;
///
/// struct OrderWorkflow {
///     state: OrderState,
///     order_id: String,
/// }
///
/// impl Workflow for OrderWorkflow {
///     const TYPE: &'static str = "order_workflow";
///     type Input = OrderInput;
///     type Output = OrderResult;
///
///     fn new(input: Self::Input) -> Self {
///         Self {
///             state: OrderState::Created,
///             order_id: input.order_id,
///         }
///     }
///
///     fn on_start(&mut self) -> Vec<WorkflowAction> {
///         vec![WorkflowAction::ScheduleActivity {
///             activity_id: "validate".into(),
///             activity_type: "validate_order".into(),
///             input: json!({ "order_id": self.order_id }),
///             options: ActivityOptions::default(),
///         }]
///     }
///
///     // ... implement other methods
/// }
/// ```
pub trait Workflow: Send + Sync + 'static {
    /// Unique type identifier for this workflow
    ///
    /// This is used to look up the workflow in the registry during replay.
    const TYPE: &'static str;

    /// Input type for starting the workflow
    type Input: Serialize + DeserializeOwned + Send + Clone;

    /// Output type when workflow completes successfully
    type Output: Serialize + DeserializeOwned + Send;

    /// Create a new workflow instance from input
    ///
    /// This is called both when starting a new workflow and when replaying.
    fn new(input: Self::Input) -> Self;

    /// Called when workflow starts (or replays from beginning)
    ///
    /// Return a list of actions to schedule initial work.
    fn on_start(&mut self) -> Vec<WorkflowAction>;

    /// Called when an activity completes successfully
    ///
    /// The result is the JSON value returned by the activity.
    fn on_activity_completed(
        &mut self,
        activity_id: &str,
        result: serde_json::Value,
    ) -> Vec<WorkflowAction>;

    /// Called when an activity fails (after all retries exhausted)
    fn on_activity_failed(
        &mut self,
        activity_id: &str,
        error: &ActivityError,
    ) -> Vec<WorkflowAction>;

    /// Called when a timer fires
    fn on_timer_fired(&mut self, timer_id: &str) -> Vec<WorkflowAction> {
        let _ = timer_id;
        vec![]
    }

    /// Called when an external signal is received
    fn on_signal(&mut self, signal: &WorkflowSignal) -> Vec<WorkflowAction> {
        let _ = signal;
        vec![]
    }

    /// Check if workflow has reached a terminal state
    fn is_completed(&self) -> bool;

    /// Get the workflow result (if completed successfully)
    fn result(&self) -> Option<Self::Output>;

    /// Get the workflow error (if failed)
    fn error(&self) -> Option<WorkflowError> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_workflow_error_display() {
        let error = WorkflowError::new("something went wrong");
        assert_eq!(error.to_string(), "something went wrong");
    }

    #[test]
    fn test_workflow_error_with_code() {
        let error = WorkflowError::new("not found").with_code("NOT_FOUND");
        assert_eq!(error.code, Some("NOT_FOUND".to_string()));
    }

    #[test]
    fn test_workflow_error_retryable() {
        let error = WorkflowError::retryable("temporary failure");
        assert!(error.retryable);
    }
}

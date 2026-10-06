//! Workflow actions

use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::WorkflowError;
use super::options::{ActivityOptions, duration_serde};

/// Actions a workflow can request
///
/// These are the commands a workflow can issue in response to events.
/// Each action is persisted as a [`WorkflowEvent`](super::WorkflowEvent) before execution.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WorkflowAction {
    /// Schedule an activity for execution
    ScheduleActivity {
        /// Unique identifier for this activity within the workflow
        activity_id: String,

        /// Type of activity to execute (used to look up in registry)
        activity_type: String,

        /// Input data for the activity (JSON)
        input: serde_json::Value,

        /// Execution options (retries, timeouts, etc.)
        options: ActivityOptions,
    },

    /// Start a timer that fires after the specified duration
    StartTimer {
        /// Unique identifier for this timer within the workflow
        timer_id: String,

        /// Duration to wait before firing
        #[serde(with = "duration_serde")]
        duration: Duration,
    },

    /// Complete the workflow successfully with a result
    CompleteWorkflow {
        /// Result value (JSON)
        result: serde_json::Value,
    },

    /// Fail the workflow with an error
    FailWorkflow {
        /// Error details
        error: WorkflowError,
    },

    /// Start a child workflow and get its outcome back through
    /// [`Workflow::on_child_workflow_completed`](super::Workflow::on_child_workflow_completed)
    /// or [`Workflow::on_child_workflow_failed`](super::Workflow::on_child_workflow_failed).
    ScheduleChildWorkflow {
        /// The child's id within this workflow. The engine derives the child's
        /// workflow UUID from it, so scheduling the same id twice starts one child.
        workflow_id: String,

        /// Type of workflow to start
        workflow_type: String,

        /// Input for the child workflow
        input: serde_json::Value,
    },

    /// Request cancellation of a pending activity
    CancelActivity {
        /// ID of the activity to cancel
        activity_id: String,
    },

    /// No action (used when event handling doesn't trigger new work)
    None,
}

impl WorkflowAction {
    /// Create a schedule activity action with default options
    pub fn schedule_activity(
        activity_id: impl Into<String>,
        activity_type: impl Into<String>,
        input: serde_json::Value,
    ) -> Self {
        Self::ScheduleActivity {
            activity_id: activity_id.into(),
            activity_type: activity_type.into(),
            input,
            options: ActivityOptions::default(),
        }
    }

    /// Create a complete workflow action
    pub fn complete(result: serde_json::Value) -> Self {
        Self::CompleteWorkflow { result }
    }

    /// Create a fail workflow action
    pub fn fail(error: WorkflowError) -> Self {
        Self::FailWorkflow { error }
    }

    /// Start a child workflow of a registered type.
    ///
    /// ```
    /// use everruns_durable::WorkflowAction;
    /// use serde_json::json;
    ///
    /// let action = WorkflowAction::child_workflow("invoice", "billing", json!({ "order": 7 }));
    /// assert!(matches!(action, WorkflowAction::ScheduleChildWorkflow { .. }));
    /// ```
    pub fn child_workflow(
        child_id: impl Into<String>,
        workflow_type: impl Into<String>,
        input: serde_json::Value,
    ) -> Self {
        Self::ScheduleChildWorkflow {
            workflow_id: child_id.into(),
            workflow_type: workflow_type.into(),
            input,
        }
    }

    /// Create a timer action
    pub fn timer(timer_id: impl Into<String>, duration: Duration) -> Self {
        Self::StartTimer {
            timer_id: timer_id.into(),
            duration,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_schedule_activity_action() {
        let action =
            WorkflowAction::schedule_activity("step-1", "my_activity", json!({"key": "value"}));

        match action {
            WorkflowAction::ScheduleActivity {
                activity_id,
                activity_type,
                input,
                ..
            } => {
                assert_eq!(activity_id, "step-1");
                assert_eq!(activity_type, "my_activity");
                assert_eq!(input, json!({"key": "value"}));
            }
            _ => panic!("Expected ScheduleActivity"),
        }
    }

    #[test]
    fn test_workflow_action_serialization() {
        let action = WorkflowAction::ScheduleActivity {
            activity_id: "test".to_string(),
            activity_type: "my_activity".to_string(),
            input: json!({}),
            options: ActivityOptions::default(),
        };

        let json = serde_json::to_string(&action).unwrap();
        assert!(json.contains("\"type\":\"schedule_activity\""));

        let parsed: WorkflowAction = serde_json::from_str(&json).unwrap();
        assert_eq!(action, parsed);
    }

    #[test]
    fn test_timer_action() {
        let action = WorkflowAction::timer("delay", Duration::from_secs(60));

        match action {
            WorkflowAction::StartTimer { timer_id, duration } => {
                assert_eq!(timer_id, "delay");
                assert_eq!(duration, Duration::from_secs(60));
            }
            _ => panic!("Expected StartTimer"),
        }
    }
}

//! Workflow events for persistence

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{ActivityOptions, WorkflowError, WorkflowSignal};
use crate::activity::ActivityError;

/// Types of timeouts that can occur
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TimeoutType {
    /// Activity was not claimed within schedule_to_start_timeout
    ScheduleToStart,

    /// Activity did not complete within start_to_close_timeout
    StartToClose,

    /// Worker did not send heartbeat within heartbeat_timeout
    Heartbeat,
}

/// Events stored in the durable_workflow_events table
///
/// These events form the append-only log for a workflow. They are used for:
/// - Persisting workflow progress
/// - Replaying workflows after recovery
/// - Auditing and debugging
///
/// Events are immutable once written. The workflow state is reconstructed
/// by replaying all events in sequence order.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WorkflowEvent {
    // =========================================================================
    // Workflow Lifecycle Events
    // =========================================================================
    /// Workflow was started with the given input
    WorkflowStarted {
        /// The input provided when starting the workflow
        input: serde_json::Value,
    },

    /// Workflow completed successfully
    WorkflowCompleted {
        /// The result value
        result: serde_json::Value,
    },

    /// Workflow failed with an error
    WorkflowFailed {
        /// Error details
        error: WorkflowError,
    },

    /// Workflow was cancelled (via signal or admin action)
    WorkflowCancelled {
        /// Reason for cancellation
        reason: String,
    },

    // =========================================================================
    // Activity Lifecycle Events
    // =========================================================================
    /// Activity was scheduled for execution
    ActivityScheduled {
        /// Unique activity identifier within the workflow
        activity_id: String,

        /// Type of activity to execute
        activity_type: String,

        /// Input for the activity
        input: serde_json::Value,

        /// Execution options
        options: ActivityOptions,
    },

    /// Activity execution started (claimed by a worker)
    ActivityStarted {
        /// Activity identifier
        activity_id: String,

        /// Current attempt number (1-based)
        attempt: u32,

        /// ID of the worker executing the activity
        worker_id: String,
    },

    /// Activity completed successfully
    ActivityCompleted {
        /// Activity identifier
        activity_id: String,

        /// Result returned by the activity
        result: serde_json::Value,
    },

    /// Activity failed (may or may not retry)
    ActivityFailed {
        /// Activity identifier
        activity_id: String,

        /// Error details
        error: ActivityError,

        /// Whether the activity will be retried
        will_retry: bool,
    },

    /// Activity timed out
    ActivityTimedOut {
        /// Activity identifier
        activity_id: String,

        /// Type of timeout that occurred
        timeout_type: TimeoutType,
    },

    /// Activity was cancelled
    ActivityCancelled {
        /// Activity identifier
        activity_id: String,

        /// Reason for cancellation
        reason: String,
    },

    // =========================================================================
    // Timer Events
    // =========================================================================
    /// Timer was started
    TimerStarted {
        /// Timer identifier
        timer_id: String,

        /// Duration in milliseconds
        duration_ms: u64,
    },

    /// Timer fired (duration elapsed)
    TimerFired {
        /// Timer identifier
        timer_id: String,
    },

    /// Timer was cancelled
    TimerCancelled {
        /// Timer identifier
        timer_id: String,
    },

    // =========================================================================
    // Signal Events
    // =========================================================================
    /// External signal was received
    SignalReceived {
        /// The signal that was received
        signal: WorkflowSignal,
    },

    // =========================================================================
    // Child Workflow Events
    // =========================================================================
    /// Child workflow was started
    ChildWorkflowStarted {
        /// Child workflow ID
        workflow_id: Uuid,

        /// Type of the child workflow
        workflow_type: String,
    },

    /// Child workflow completed successfully
    ChildWorkflowCompleted {
        /// Child workflow ID
        workflow_id: Uuid,

        /// Result from the child workflow
        result: serde_json::Value,
    },

    /// Child workflow failed
    ChildWorkflowFailed {
        /// Child workflow ID
        workflow_id: Uuid,

        /// Error from the child workflow
        error: WorkflowError,
    },
}

impl WorkflowEvent {
    /// Get the activity_id if this is an activity-related event
    pub fn activity_id(&self) -> Option<&str> {
        match self {
            Self::ActivityScheduled { activity_id, .. }
            | Self::ActivityStarted { activity_id, .. }
            | Self::ActivityCompleted { activity_id, .. }
            | Self::ActivityFailed { activity_id, .. }
            | Self::ActivityTimedOut { activity_id, .. }
            | Self::ActivityCancelled { activity_id, .. } => Some(activity_id),
            _ => None,
        }
    }

    /// Check if this is a terminal workflow event
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::WorkflowCompleted { .. }
                | Self::WorkflowFailed { .. }
                | Self::WorkflowCancelled { .. }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_workflow_event_serialization() {
        let event = WorkflowEvent::WorkflowStarted {
            input: json!({"order_id": "123"}),
        };

        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"type\":\"workflow_started\""));

        let parsed: WorkflowEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(event, parsed);
    }

    #[test]
    fn test_activity_event_serialization() {
        let event = WorkflowEvent::ActivityCompleted {
            activity_id: "step-1".to_string(),
            result: json!({"status": "ok"}),
        };

        let json = serde_json::to_string(&event).unwrap();
        let parsed: WorkflowEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(event, parsed);
    }

    #[test]
    fn test_activity_id_extraction() {
        let event = WorkflowEvent::ActivityStarted {
            activity_id: "my-activity".to_string(),
            attempt: 1,
            worker_id: "worker-1".to_string(),
        };

        assert_eq!(event.activity_id(), Some("my-activity"));

        let start_event = WorkflowEvent::WorkflowStarted {
            input: json!({}),
        };
        assert_eq!(start_event.activity_id(), None);
    }

    #[test]
    fn test_is_terminal() {
        assert!(WorkflowEvent::WorkflowCompleted { result: json!({}) }.is_terminal());
        assert!(WorkflowEvent::WorkflowFailed {
            error: WorkflowError::new("error")
        }
        .is_terminal());
        assert!(WorkflowEvent::WorkflowCancelled {
            reason: "cancelled".to_string()
        }
        .is_terminal());

        assert!(!WorkflowEvent::WorkflowStarted { input: json!({}) }.is_terminal());
        assert!(!WorkflowEvent::ActivityCompleted {
            activity_id: "x".to_string(),
            result: json!({})
        }
        .is_terminal());
    }
}

//! Workflow actions and activity options

use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::WorkflowError;
use crate::reliability::{CircuitBreakerConfig, RetryPolicy};

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

    /// Schedule a child workflow
    ScheduleChildWorkflow {
        /// Unique identifier for the child workflow
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

    /// Create a timer action
    pub fn timer(timer_id: impl Into<String>, duration: Duration) -> Self {
        Self::StartTimer {
            timer_id: timer_id.into(),
            duration,
        }
    }
}

/// Options for activity execution
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ActivityOptions {
    /// Retry policy for this activity
    pub retry_policy: RetryPolicy,

    /// Maximum time to wait for activity to be claimed by a worker
    #[serde(with = "duration_serde")]
    pub schedule_to_start_timeout: Duration,

    /// Maximum time for activity execution (from start to completion)
    #[serde(with = "duration_serde")]
    pub start_to_close_timeout: Duration,

    /// Heartbeat interval for long-running activities
    /// If set, workers must send heartbeats within this interval
    #[serde(with = "option_duration_serde")]
    pub heartbeat_timeout: Option<Duration>,

    /// Circuit breaker configuration for this activity
    pub circuit_breaker: Option<CircuitBreakerConfig>,

    /// Priority (higher values = higher priority, claimed first)
    pub priority: i32,
}

impl Default for ActivityOptions {
    fn default() -> Self {
        Self {
            retry_policy: RetryPolicy::default(),
            schedule_to_start_timeout: Duration::from_secs(60),
            start_to_close_timeout: Duration::from_secs(300),
            heartbeat_timeout: None,
            circuit_breaker: None,
            priority: 0,
        }
    }
}

impl ActivityOptions {
    /// Create options with a specific retry policy
    pub fn with_retry(mut self, policy: RetryPolicy) -> Self {
        self.retry_policy = policy;
        self
    }

    /// Set the schedule-to-start timeout
    pub fn with_schedule_to_start_timeout(mut self, timeout: Duration) -> Self {
        self.schedule_to_start_timeout = timeout;
        self
    }

    /// Set the start-to-close timeout
    pub fn with_start_to_close_timeout(mut self, timeout: Duration) -> Self {
        self.start_to_close_timeout = timeout;
        self
    }

    /// Enable heartbeating with the specified timeout
    pub fn with_heartbeat(mut self, timeout: Duration) -> Self {
        self.heartbeat_timeout = Some(timeout);
        self
    }

    /// Set the priority
    pub fn with_priority(mut self, priority: i32) -> Self {
        self.priority = priority;
        self
    }
}

/// Serde support for Duration (as milliseconds)
mod duration_serde {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::time::Duration;

    pub fn serialize<S>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        duration.as_millis().serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Duration, D::Error>
    where
        D: Deserializer<'de>,
    {
        let millis = u64::deserialize(deserializer)?;
        Ok(Duration::from_millis(millis))
    }
}

/// Serde support for Option<Duration>
mod option_duration_serde {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::time::Duration;

    pub fn serialize<S>(duration: &Option<Duration>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match duration {
            Some(d) => d.as_millis().serialize(serializer),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<Duration>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let millis: Option<u64> = Option::deserialize(deserializer)?;
        Ok(millis.map(Duration::from_millis))
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
    fn test_activity_options_serialization() {
        let options = ActivityOptions::default()
            .with_priority(10)
            .with_heartbeat(Duration::from_secs(30));

        let json = serde_json::to_string(&options).unwrap();
        let parsed: ActivityOptions = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.priority, 10);
        assert_eq!(parsed.heartbeat_timeout, Some(Duration::from_secs(30)));
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

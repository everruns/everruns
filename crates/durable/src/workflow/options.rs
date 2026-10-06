//! Activity options

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::reliability::{CircuitBreakerConfig, RetryPolicy};

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

    /// Delay before the task becomes claimable. Durable timers use this; an
    /// activity can too, to run no earlier than a point in time.
    #[serde(default, with = "option_duration_serde")]
    pub start_delay: Option<Duration>,

    /// Idempotent enqueue keyed by `(workflow_id, activity_id)`.
    ///
    /// When set on a workflow task, enqueueing returns the id of an existing
    /// task with the same `activity_id` in the same workflow, in any status,
    /// instead of creating a second one; such a replay is also exempt from the
    /// per-workflow pending-task limit. Use it for tasks that several callers
    /// may race to enqueue for one logical event. Standalone tasks (no
    /// workflow) ignore it. Under concurrent enqueues the PostgreSQL store is
    /// race-free only where a unique index covers those `activity_id`s; without
    /// one the check is best-effort.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub dedupe_by_activity_id: bool,
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
            start_delay: None,
            dedupe_by_activity_id: false,
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

    /// Keep the task unclaimable until `delay` has passed.
    ///
    /// ```
    /// use std::time::Duration;
    /// use everruns_durable::ActivityOptions;
    ///
    /// let options = ActivityOptions::default().with_start_delay(Duration::from_secs(30));
    /// assert_eq!(options.start_delay, Some(Duration::from_secs(30)));
    /// ```
    pub fn with_start_delay(mut self, delay: Duration) -> Self {
        self.start_delay = Some(delay);
        self
    }

    /// Make enqueueing idempotent per `(workflow_id, activity_id)`; see
    /// [`ActivityOptions::dedupe_by_activity_id`].
    pub fn with_dedupe_by_activity_id(mut self) -> Self {
        self.dedupe_by_activity_id = true;
        self
    }
}

/// Serde support for Duration (as milliseconds)
pub(super) mod duration_serde {
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

/// Serde support for `Option<Duration>`
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
}

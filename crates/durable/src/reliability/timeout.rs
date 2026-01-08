//! Activity timeout management
//!
//! Provides timeout enforcement for activities with configurable timeouts.

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::persistence::{StoreError, WorkflowEventStore};

/// Timeout-related errors
#[derive(Debug, Error)]
pub enum TimeoutError {
    /// Activity exceeded schedule-to-start timeout
    #[error("activity timed out waiting to start (waited {elapsed:?}, limit {limit:?})")]
    ScheduleToStartTimeout { elapsed: Duration, limit: Duration },

    /// Activity exceeded start-to-close timeout
    #[error("activity execution timed out (ran for {elapsed:?}, limit {limit:?})")]
    StartToCloseTimeout { elapsed: Duration, limit: Duration },

    /// Heartbeat timeout exceeded
    #[error("activity heartbeat timed out (no heartbeat for {elapsed:?}, limit {limit:?})")]
    HeartbeatTimeout { elapsed: Duration, limit: Duration },

    /// Store error
    #[error("store error: {0}")]
    Store(#[from] StoreError),
}

/// Timeout configuration for activities
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TimeoutConfig {
    /// Maximum time from scheduling to start of execution
    #[serde(with = "duration_millis")]
    pub schedule_to_start: Duration,

    /// Maximum time from start to completion
    #[serde(with = "duration_millis")]
    pub start_to_close: Duration,

    /// Maximum time between heartbeats (None = no heartbeat required)
    #[serde(with = "option_duration_millis")]
    pub heartbeat: Option<Duration>,
}

impl Default for TimeoutConfig {
    fn default() -> Self {
        Self {
            schedule_to_start: Duration::from_secs(60), // 1 minute to start
            start_to_close: Duration::from_secs(300),   // 5 minutes to complete
            heartbeat: None,                            // No heartbeat by default
        }
    }
}

impl TimeoutConfig {
    /// Create a new timeout configuration
    pub fn new() -> Self {
        Self::default()
    }

    /// Set schedule-to-start timeout
    pub fn with_schedule_to_start(mut self, timeout: Duration) -> Self {
        self.schedule_to_start = timeout;
        self
    }

    /// Set start-to-close timeout
    pub fn with_start_to_close(mut self, timeout: Duration) -> Self {
        self.start_to_close = timeout;
        self
    }

    /// Set heartbeat timeout
    pub fn with_heartbeat(mut self, timeout: Duration) -> Self {
        self.heartbeat = Some(timeout);
        self
    }

    /// Remove heartbeat timeout
    pub fn without_heartbeat(mut self) -> Self {
        self.heartbeat = None;
        self
    }
}

/// Information about a task's timing
#[derive(Debug, Clone)]
pub struct TaskTimingInfo {
    /// Task ID
    pub task_id: Uuid,
    /// When the task was scheduled
    pub scheduled_at: DateTime<Utc>,
    /// When the task started (was claimed)
    pub started_at: Option<DateTime<Utc>>,
    /// Last heartbeat time
    pub last_heartbeat_at: Option<DateTime<Utc>>,
    /// Timeout configuration
    pub timeout_config: TimeoutConfig,
}

/// Manages activity timeouts
///
/// The TimeoutManager checks tasks for timeout violations and can
/// trigger appropriate actions (mark as failed, requeue, etc.).
///
/// # Example
///
/// ```ignore
/// use everruns_durable::reliability::TimeoutManager;
///
/// let manager = TimeoutManager::new(store);
///
/// // Check for timed out tasks
/// let timed_out = manager.find_timed_out_tasks().await?;
///
/// for task in timed_out {
///     manager.handle_timeout(task.task_id, task.timeout_type).await?;
/// }
/// ```
pub struct TimeoutManager {
    store: Arc<dyn WorkflowEventStore>,
}

/// Type of timeout that occurred
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeoutType {
    /// Task waited too long to be claimed
    ScheduleToStart,
    /// Task took too long to complete
    StartToClose,
    /// No heartbeat received in time
    Heartbeat,
}

/// A task that has timed out
#[derive(Debug, Clone)]
#[allow(dead_code)] // Part of public API, will be used in timeout scanning loop
pub struct TimedOutTask {
    /// Task ID
    pub task_id: Uuid,
    /// Type of timeout
    pub timeout_type: TimeoutType,
    /// How long the timeout was exceeded by
    pub exceeded_by: Duration,
}

impl TimeoutManager {
    /// Create a new timeout manager
    pub fn new(store: Arc<dyn WorkflowEventStore>) -> Self {
        Self { store }
    }

    /// Check if a task has exceeded its schedule-to-start timeout
    pub fn check_schedule_to_start(
        &self,
        scheduled_at: DateTime<Utc>,
        started_at: Option<DateTime<Utc>>,
        config: &TimeoutConfig,
    ) -> Option<TimeoutError> {
        // Only check if task hasn't started yet
        if started_at.is_some() {
            return None;
        }

        let elapsed = Utc::now()
            .signed_duration_since(scheduled_at)
            .to_std()
            .unwrap_or(Duration::ZERO);

        if elapsed > config.schedule_to_start {
            Some(TimeoutError::ScheduleToStartTimeout {
                elapsed,
                limit: config.schedule_to_start,
            })
        } else {
            None
        }
    }

    /// Check if a task has exceeded its start-to-close timeout
    pub fn check_start_to_close(
        &self,
        started_at: Option<DateTime<Utc>>,
        config: &TimeoutConfig,
    ) -> Option<TimeoutError> {
        let started = started_at?;

        let elapsed = Utc::now()
            .signed_duration_since(started)
            .to_std()
            .unwrap_or(Duration::ZERO);

        if elapsed > config.start_to_close {
            Some(TimeoutError::StartToCloseTimeout {
                elapsed,
                limit: config.start_to_close,
            })
        } else {
            None
        }
    }

    /// Check if a task has exceeded its heartbeat timeout
    pub fn check_heartbeat(
        &self,
        started_at: Option<DateTime<Utc>>,
        last_heartbeat_at: Option<DateTime<Utc>>,
        config: &TimeoutConfig,
    ) -> Option<TimeoutError> {
        let heartbeat_timeout = config.heartbeat?;

        // Only check if task has started
        started_at?;

        // Use last heartbeat time, or started_at if no heartbeat yet
        let last_beat = last_heartbeat_at.or(started_at)?;

        let elapsed = Utc::now()
            .signed_duration_since(last_beat)
            .to_std()
            .unwrap_or(Duration::ZERO);

        if elapsed > heartbeat_timeout {
            Some(TimeoutError::HeartbeatTimeout {
                elapsed,
                limit: heartbeat_timeout,
            })
        } else {
            None
        }
    }

    /// Check all timeout conditions for a task
    pub fn check_task_timeout(
        &self,
        timing: &TaskTimingInfo,
    ) -> Option<(TimeoutType, TimeoutError)> {
        // Check schedule-to-start first (task not started)
        if let Some(err) = self.check_schedule_to_start(
            timing.scheduled_at,
            timing.started_at,
            &timing.timeout_config,
        ) {
            return Some((TimeoutType::ScheduleToStart, err));
        }

        // Check heartbeat (if configured)
        if let Some(err) = self.check_heartbeat(
            timing.started_at,
            timing.last_heartbeat_at,
            &timing.timeout_config,
        ) {
            return Some((TimeoutType::Heartbeat, err));
        }

        // Check start-to-close
        if let Some(err) = self.check_start_to_close(timing.started_at, &timing.timeout_config) {
            return Some((TimeoutType::StartToClose, err));
        }

        None
    }

    /// Handle a timed out task by failing it
    pub async fn handle_timeout(
        &self,
        task_id: Uuid,
        timeout_type: TimeoutType,
    ) -> Result<(), TimeoutError> {
        let error_message = match timeout_type {
            TimeoutType::ScheduleToStart => "Task timed out waiting to start",
            TimeoutType::StartToClose => "Task execution timed out",
            TimeoutType::Heartbeat => "Task heartbeat timed out",
        };

        self.store.fail_task(task_id, error_message).await?;
        Ok(())
    }

    /// Calculate remaining time for a timeout
    pub fn remaining_time(&self, started_at: DateTime<Utc>, timeout: Duration) -> Option<Duration> {
        let elapsed = Utc::now()
            .signed_duration_since(started_at)
            .to_std()
            .unwrap_or(Duration::ZERO);

        timeout.checked_sub(elapsed)
    }
}

/// Serde support for Duration as milliseconds
mod duration_millis {
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

/// Serde support for Option<Duration> as milliseconds
mod option_duration_millis {
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
    use crate::persistence::InMemoryWorkflowEventStore;
    use chrono::Duration as ChronoDuration;

    fn create_test_manager() -> TimeoutManager {
        let store = Arc::new(InMemoryWorkflowEventStore::new());
        TimeoutManager::new(store)
    }

    #[test]
    fn test_timeout_config_defaults() {
        let config = TimeoutConfig::default();
        assert_eq!(config.schedule_to_start, Duration::from_secs(60));
        assert_eq!(config.start_to_close, Duration::from_secs(300));
        assert!(config.heartbeat.is_none());
    }

    #[test]
    fn test_timeout_config_builder() {
        let config = TimeoutConfig::new()
            .with_schedule_to_start(Duration::from_secs(30))
            .with_start_to_close(Duration::from_secs(600))
            .with_heartbeat(Duration::from_secs(10));

        assert_eq!(config.schedule_to_start, Duration::from_secs(30));
        assert_eq!(config.start_to_close, Duration::from_secs(600));
        assert_eq!(config.heartbeat, Some(Duration::from_secs(10)));
    }

    #[test]
    fn test_schedule_to_start_not_started() {
        let manager = create_test_manager();
        let scheduled_at = Utc::now() - ChronoDuration::seconds(120);
        let config = TimeoutConfig::default(); // 60s schedule-to-start

        let result = manager.check_schedule_to_start(scheduled_at, None, &config);
        assert!(result.is_some());
        assert!(matches!(
            result,
            Some(TimeoutError::ScheduleToStartTimeout { .. })
        ));
    }

    #[test]
    fn test_schedule_to_start_already_started() {
        let manager = create_test_manager();
        let scheduled_at = Utc::now() - ChronoDuration::seconds(120);
        let started_at = Some(Utc::now() - ChronoDuration::seconds(60));
        let config = TimeoutConfig::default();

        let result = manager.check_schedule_to_start(scheduled_at, started_at, &config);
        assert!(result.is_none());
    }

    #[test]
    fn test_start_to_close_not_started() {
        let manager = create_test_manager();
        let config = TimeoutConfig::default();

        let result = manager.check_start_to_close(None, &config);
        assert!(result.is_none());
    }

    #[test]
    fn test_start_to_close_timeout() {
        let manager = create_test_manager();
        let started_at = Some(Utc::now() - ChronoDuration::seconds(600));
        let config = TimeoutConfig::default(); // 300s start-to-close

        let result = manager.check_start_to_close(started_at, &config);
        assert!(result.is_some());
        assert!(matches!(
            result,
            Some(TimeoutError::StartToCloseTimeout { .. })
        ));
    }

    #[test]
    fn test_heartbeat_no_config() {
        let manager = create_test_manager();
        let started_at = Some(Utc::now() - ChronoDuration::seconds(600));
        let config = TimeoutConfig::default(); // No heartbeat

        let result = manager.check_heartbeat(started_at, None, &config);
        assert!(result.is_none());
    }

    #[test]
    fn test_heartbeat_timeout() {
        let manager = create_test_manager();
        let started_at = Some(Utc::now() - ChronoDuration::seconds(60));
        let last_heartbeat = Some(Utc::now() - ChronoDuration::seconds(30));
        let config = TimeoutConfig::new().with_heartbeat(Duration::from_secs(10));

        let result = manager.check_heartbeat(started_at, last_heartbeat, &config);
        assert!(result.is_some());
        assert!(matches!(
            result,
            Some(TimeoutError::HeartbeatTimeout { .. })
        ));
    }

    #[test]
    fn test_remaining_time() {
        let manager = create_test_manager();
        let started_at = Utc::now() - ChronoDuration::seconds(10);
        let timeout = Duration::from_secs(60);

        let remaining = manager.remaining_time(started_at, timeout);
        assert!(remaining.is_some());
        assert!(remaining.unwrap() > Duration::from_secs(40));
        assert!(remaining.unwrap() < Duration::from_secs(60));
    }

    #[test]
    fn test_remaining_time_expired() {
        let manager = create_test_manager();
        let started_at = Utc::now() - ChronoDuration::seconds(120);
        let timeout = Duration::from_secs(60);

        let remaining = manager.remaining_time(started_at, timeout);
        assert!(remaining.is_none());
    }

    #[test]
    fn test_timeout_config_serialization() {
        let config = TimeoutConfig::new()
            .with_schedule_to_start(Duration::from_secs(30))
            .with_heartbeat(Duration::from_secs(10));

        let json = serde_json::to_string(&config).unwrap();
        let parsed: TimeoutConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(config, parsed);
    }
}

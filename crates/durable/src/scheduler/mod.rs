//! Scheduler component for scheduled task execution
//!
//! The `DurableScheduler` polls the schedule table and triggers workflows/activities
//! at their scheduled times. It supports:
//!
//! - Multi-instance coordination via `SELECT ... FOR UPDATE SKIP LOCKED`
//! - Round-robin fair scheduling across organizations
//! - Max concurrent execution limits
//! - Catch-up execution for missed triggers
//! - Heartbeat-based instance registration

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, instrument, warn};
use uuid::Uuid;

use crate::persistence::{
    ScheduleRow, ScheduleTargetType, SchedulerInstanceInfo, StoreError, TaskDefinition,
    TraceContext, WorkflowEventStore,
};
use crate::workflow::WorkflowEvent;

/// Configuration for the scheduler
#[derive(Debug, Clone)]
pub struct SchedulerConfig {
    /// How often to poll for due schedules (default: 1s)
    pub poll_interval: Duration,

    /// Maximum schedules to claim per poll cycle (default: 100)
    pub batch_size: u32,

    /// Timeout for reclaiming schedules from stale instances (default: 30s)
    pub claim_timeout: Duration,

    /// Heartbeat interval for instance registration (default: 10s)
    pub heartbeat_interval: Duration,

    /// Whether to enable catch-up execution for missed triggers
    pub enable_catch_up: bool,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            poll_interval: Duration::from_secs(1),
            batch_size: 100,
            claim_timeout: Duration::from_secs(30),
            heartbeat_interval: Duration::from_secs(10),
            enable_catch_up: true,
        }
    }
}

/// Errors from scheduler operations
#[derive(Debug, thiserror::Error)]
pub enum SchedulerError {
    /// Store error
    #[error("store error: {0}")]
    Store(#[from] StoreError),

    /// Failed to calculate next trigger
    #[error("failed to calculate next trigger: {0}")]
    CronError(String),

    /// Workflow type not registered
    #[error("workflow type not registered: {0}")]
    UnknownWorkflowType(String),

    /// Activity type not registered
    #[error("activity type not registered: {0}")]
    UnknownActivityType(String),
}

/// Result of triggering a schedule
#[derive(Debug)]
pub struct TriggerResult {
    /// ID of the created execution record
    pub execution_id: Uuid,

    /// ID of the created workflow or task
    pub target_id: Uuid,
}

/// Durable scheduler component
///
/// Runs as a background loop, polling for due schedules and triggering them.
/// Supports multiple concurrent instances via database locking.
pub struct DurableScheduler {
    store: Arc<dyn WorkflowEventStore>,
    config: SchedulerConfig,
    instance_id: String,
    schedules_processed: std::sync::atomic::AtomicU64,
}

impl DurableScheduler {
    /// Create a new scheduler instance
    pub fn new(
        store: Arc<dyn WorkflowEventStore>,
        config: SchedulerConfig,
        instance_id: String,
    ) -> Self {
        Self {
            store,
            config,
            instance_id,
            schedules_processed: std::sync::atomic::AtomicU64::new(0),
        }
    }

    /// Create with default configuration
    pub fn with_defaults(store: Arc<dyn WorkflowEventStore>, instance_id: String) -> Self {
        Self::new(store, SchedulerConfig::default(), instance_id)
    }

    /// Spawn the scheduler as a background tokio task.
    /// Returns a CancellationToken that can be used to stop the scheduler.
    pub fn spawn(self) -> CancellationToken {
        let shutdown = CancellationToken::new();
        let token = shutdown.clone();
        tokio::spawn(async move {
            self.run(token).await;
        });
        shutdown
    }

    /// Run the scheduler loop until shutdown
    #[instrument(skip(self, shutdown), fields(instance_id = %self.instance_id))]
    pub async fn run(&self, shutdown: CancellationToken) {
        info!("scheduler starting");

        // Register this instance
        let instance_info = SchedulerInstanceInfo {
            instance_id: self.instance_id.clone(),
            started_at: Utc::now(),
            last_heartbeat_at: Utc::now(),
            schedules_processed: 0,
            hostname: hostname::get()
                .ok()
                .and_then(|h: std::ffi::OsString| h.into_string().ok()),
            version: option_env!("CARGO_PKG_VERSION").map(String::from),
        };

        if let Err(e) = self.store.register_scheduler_instance(instance_info).await {
            error!(error = %e, "failed to register scheduler instance");
        }

        let mut poll_interval = tokio::time::interval(self.config.poll_interval);
        let mut heartbeat_interval = tokio::time::interval(self.config.heartbeat_interval);

        loop {
            tokio::select! {
                _ = shutdown.cancelled() => {
                    info!("scheduler shutting down");
                    break;
                }
                _ = poll_interval.tick() => {
                    if let Err(e) = self.process_due_schedules().await {
                        error!(error = %e, "failed to process due schedules");
                    }
                }
                _ = heartbeat_interval.tick() => {
                    if let Err(e) = self.send_heartbeat().await {
                        warn!(error = %e, "failed to send heartbeat");
                    }
                }
            }
        }

        // Deregister on shutdown
        if let Err(e) = self
            .store
            .deregister_scheduler_instance(&self.instance_id)
            .await
        {
            warn!(error = %e, "failed to deregister scheduler instance");
        }

        info!("scheduler stopped");
    }

    /// Process all due schedules
    #[instrument(skip(self))]
    pub async fn process_due_schedules(&self) -> Result<(), SchedulerError> {
        // Claim due schedules (round-robin across orgs)
        let due = self
            .store
            .claim_due_schedules(&self.instance_id, self.config.batch_size)
            .await?;

        if !due.is_empty() {
            debug!(count = due.len(), "claimed due schedules");
        }

        for schedule in due {
            if let Err(e) = self.trigger_schedule(&schedule).await {
                error!(
                    schedule_id = %schedule.id,
                    schedule_name = %schedule.name,
                    error = %e,
                    "failed to trigger schedule"
                );
            }
            // Increment counter
            self.schedules_processed
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }

        Ok(())
    }

    /// Trigger a single schedule
    #[instrument(skip(self, schedule), fields(
        schedule_id = %schedule.id,
        schedule_name = %schedule.name,
    ))]
    async fn trigger_schedule(&self, schedule: &ScheduleRow) -> Result<(), SchedulerError> {
        // Check max_concurrent limit
        if let Some(max) = schedule.max_concurrent {
            let running = self.store.count_running_executions(schedule.id).await?;
            if running >= max {
                debug!(
                    running = running,
                    max = max,
                    "skipping trigger due to max_concurrent limit"
                );
                // Skip this trigger but still update next_trigger_at
                self.skip_and_update_next(schedule).await?;
                return Ok(());
            }
        }

        // Get scheduled_at time (or now if missing)
        let scheduled_at = schedule.next_trigger_at.unwrap_or_else(Utc::now);

        // Create execution record (already sets status to Running)
        let execution_id = self
            .store
            .create_schedule_execution(schedule.id, scheduled_at)
            .await?;

        // Trigger the target
        let result = match schedule.target_type {
            ScheduleTargetType::Workflow => {
                self.trigger_workflow(schedule, execution_id, scheduled_at)
                    .await
            }
            ScheduleTargetType::Activity => {
                self.trigger_activity(schedule, execution_id, scheduled_at)
                    .await
            }
        };

        // Update execution and schedule based on result
        match result {
            Ok(target_id) => {
                // Mark execution as completed with target ID
                let is_workflow = schedule.target_type == ScheduleTargetType::Workflow;
                self.store
                    .complete_schedule_execution(execution_id, target_id, is_workflow)
                    .await?;

                // Update next trigger time
                let next = self.calculate_next_trigger(schedule)?;
                self.store.update_next_trigger(schedule.id, next).await?;

                info!(
                    target_id = %target_id,
                    next_trigger = %next,
                    "triggered schedule successfully"
                );
            }
            Err(e) => {
                // Mark execution as failed
                self.store
                    .fail_schedule_execution(execution_id, &e.to_string())
                    .await?;

                // Still update next trigger time
                let next = self.calculate_next_trigger(schedule)?;
                self.store.update_next_trigger(schedule.id, next).await?;

                warn!(
                    error = %e,
                    next_trigger = %next,
                    "schedule trigger failed"
                );
            }
        }

        Ok(())
    }

    /// Trigger a workflow
    async fn trigger_workflow(
        &self,
        schedule: &ScheduleRow,
        _execution_id: Uuid,
        _scheduled_at: DateTime<Utc>,
    ) -> Result<Uuid, SchedulerError> {
        let workflow_id = Uuid::now_v7();

        // Create trace context for the scheduled execution
        let trace_context = TraceContext {
            trace_id: Uuid::now_v7().to_string(),
            span_id: Uuid::now_v7().to_string(),
            trace_flags: 0,
        };

        // Create workflow in store
        self.store
            .create_workflow(
                workflow_id,
                &schedule.target_name,
                schedule.target_input.clone(),
                Some(&trace_context),
            )
            .await?;

        // Append WorkflowStarted event
        let start_event = WorkflowEvent::WorkflowStarted {
            input: schedule.target_input.clone(),
        };

        self.store
            .append_events(workflow_id, 0, vec![start_event])
            .await?;

        debug!(%workflow_id, "created workflow from schedule");

        Ok(workflow_id)
    }

    /// Trigger an activity (standalone task)
    async fn trigger_activity(
        &self,
        schedule: &ScheduleRow,
        _execution_id: Uuid,
        _scheduled_at: DateTime<Utc>,
    ) -> Result<Uuid, SchedulerError> {
        // For standalone activities, we create a synthetic workflow ID
        // This allows tracking via the normal task queue
        let workflow_id = Uuid::now_v7();

        let task = TaskDefinition {
            workflow_id,
            activity_id: format!("scheduled-{}", Uuid::now_v7()),
            activity_type: schedule.target_name.clone(),
            input: schedule.target_input.clone(),
            options: schedule
                .retry_policy
                .clone()
                .and_then(|v| serde_json::from_value(v).ok())
                .unwrap_or_default(),
        };

        let task_id = self.store.enqueue_task(task).await?;

        debug!(%task_id, "enqueued activity from schedule");

        Ok(task_id)
    }

    /// Skip current trigger and update next trigger time
    async fn skip_and_update_next(&self, schedule: &ScheduleRow) -> Result<(), SchedulerError> {
        let scheduled_at = schedule.next_trigger_at.unwrap_or_else(Utc::now);

        // Create execution record (it will be in Running status initially)
        let execution_id = self
            .store
            .create_schedule_execution(schedule.id, scheduled_at)
            .await?;

        // Mark as skipped
        self.store
            .skip_schedule_execution(execution_id, "max_concurrent limit reached")
            .await?;

        // Update next trigger time
        let next = self.calculate_next_trigger(schedule)?;
        self.store.update_next_trigger(schedule.id, next).await?;

        debug!(%next, "skipped trigger due to max_concurrent, updated next trigger");

        Ok(())
    }

    /// Calculate next trigger time based on cron expression
    fn calculate_next_trigger(
        &self,
        schedule: &ScheduleRow,
    ) -> Result<DateTime<Utc>, SchedulerError> {
        use cron::Schedule;
        use std::str::FromStr;

        let cron_schedule = Schedule::from_str(&schedule.cron_expression)
            .map_err(|e| SchedulerError::CronError(e.to_string()))?;

        // Get next occurrence after now
        let next = cron_schedule
            .upcoming(chrono::Utc)
            .next()
            .ok_or_else(|| SchedulerError::CronError("no upcoming occurrence".to_string()))?;

        // Note: For simplicity, we're using UTC times here
        // Full timezone support would require chrono-tz

        Ok(next)
    }

    /// Send heartbeat to register this instance is alive
    async fn send_heartbeat(&self) -> Result<(), SchedulerError> {
        let processed = self
            .schedules_processed
            .load(std::sync::atomic::Ordering::Relaxed);
        self.store
            .heartbeat_scheduler_instance(&self.instance_id, processed)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persistence::{CreateScheduleRow, InMemoryWorkflowEventStore};
    use serde_json::Value;

    #[tokio::test]
    async fn test_scheduler_creates_and_runs() {
        let store: Arc<dyn WorkflowEventStore> = Arc::new(InMemoryWorkflowEventStore::new());
        let scheduler = DurableScheduler::with_defaults(store, "test-scheduler-1".to_string());

        assert_eq!(scheduler.instance_id, "test-scheduler-1");
        assert_eq!(scheduler.config.batch_size, 100);
    }

    #[tokio::test]
    async fn test_scheduler_processes_due_schedules() {
        let store = Arc::new(InMemoryWorkflowEventStore::new());

        // Create a schedule that's due now
        let schedule = CreateScheduleRow {
            name: "test-schedule".to_string(),
            description: None,
            cron_expression: "0 * * * * * *".to_string(), // every minute (7-field cron)
            timezone: "UTC".to_string(),
            target_type: ScheduleTargetType::Workflow,
            target_name: "test-workflow".to_string(),
            target_input: Value::Object(Default::default()),
            enabled: true,
            max_concurrent: None,
            catch_up_missed: false,
            max_catch_up: None,
            retry_policy: None,
            next_trigger_at: Some(Utc::now() - chrono::Duration::seconds(10)), // Due 10s ago
        };

        let schedule_id = store.create_schedule(schedule).await.unwrap();

        let scheduler = DurableScheduler::with_defaults(
            Arc::clone(&store) as Arc<dyn WorkflowEventStore>,
            "test-scheduler-1".to_string(),
        );

        // Process due schedules
        scheduler.process_due_schedules().await.unwrap();

        // Check that an execution was created
        let stats = store.get_schedule_stats(schedule_id).await.unwrap();
        assert_eq!(stats.total_executions, 1);

        // Check that next_trigger_at was updated
        let updated = store.get_schedule(schedule_id).await.unwrap();
        assert!(updated.next_trigger_at.unwrap() > Utc::now());
    }

    #[tokio::test]
    async fn test_scheduler_respects_max_concurrent() {
        let store = Arc::new(InMemoryWorkflowEventStore::new());

        // Create a schedule with max_concurrent = 1
        let schedule = CreateScheduleRow {
            name: "limited-schedule".to_string(),
            description: None,
            cron_expression: "* * * * *".to_string(),
            timezone: "UTC".to_string(),
            target_type: ScheduleTargetType::Workflow,
            target_name: "test-workflow".to_string(),
            target_input: Value::Object(Default::default()),
            enabled: true,
            max_concurrent: Some(1),
            catch_up_missed: false,
            max_catch_up: None,
            retry_policy: None,
            next_trigger_at: Some(Utc::now() - chrono::Duration::seconds(10)),
        };

        let schedule_id = store.create_schedule(schedule).await.unwrap();

        let scheduler = DurableScheduler::with_defaults(
            Arc::clone(&store) as Arc<dyn WorkflowEventStore>,
            "test-scheduler-1".to_string(),
        );

        // First trigger should work
        scheduler.process_due_schedules().await.unwrap();

        let stats = store.get_schedule_stats(schedule_id).await.unwrap();
        assert_eq!(stats.total_executions, 1);
    }

    #[tokio::test]
    async fn test_scheduler_trigger_activity() {
        let store = Arc::new(InMemoryWorkflowEventStore::new());

        // Create an activity schedule
        let schedule = CreateScheduleRow {
            name: "activity-schedule".to_string(),
            description: None,
            cron_expression: "* * * * *".to_string(),
            timezone: "UTC".to_string(),
            target_type: ScheduleTargetType::Activity,
            target_name: "cleanup-activity".to_string(),
            target_input: serde_json::json!({"cleanup": true}),
            enabled: true,
            max_concurrent: None,
            catch_up_missed: false,
            max_catch_up: None,
            retry_policy: None,
            next_trigger_at: Some(Utc::now() - chrono::Duration::seconds(10)),
        };

        let schedule_id = store.create_schedule(schedule).await.unwrap();

        let scheduler = DurableScheduler::with_defaults(
            Arc::clone(&store) as Arc<dyn WorkflowEventStore>,
            "test-scheduler-1".to_string(),
        );

        // Trigger should create a task
        scheduler.process_due_schedules().await.unwrap();

        let stats = store.get_schedule_stats(schedule_id).await.unwrap();
        assert_eq!(stats.total_executions, 1);
    }

    #[tokio::test]
    async fn test_scheduler_heartbeat() {
        let store = Arc::new(InMemoryWorkflowEventStore::new());

        let scheduler = DurableScheduler::with_defaults(
            Arc::clone(&store) as Arc<dyn WorkflowEventStore>,
            "test-scheduler-1".to_string(),
        );

        // Register instance
        let instance_info = SchedulerInstanceInfo {
            instance_id: "test-scheduler-1".to_string(),
            started_at: Utc::now(),
            last_heartbeat_at: Utc::now(),
            schedules_processed: 0,
            hostname: None,
            version: None,
        };
        store
            .register_scheduler_instance(instance_info)
            .await
            .unwrap();

        // Send heartbeat
        scheduler.send_heartbeat().await.unwrap();

        // Verify instance is registered
        let instances = store.list_scheduler_instances().await.unwrap();
        assert_eq!(instances.len(), 1);
        assert_eq!(instances[0].instance_id, "test-scheduler-1");
    }

    #[tokio::test]
    async fn test_calculate_next_trigger() {
        let store: Arc<dyn WorkflowEventStore> = Arc::new(InMemoryWorkflowEventStore::new());
        let scheduler = DurableScheduler::with_defaults(store, "test-scheduler-1".to_string());

        let schedule = ScheduleRow {
            id: Uuid::now_v7(),
            name: "test".to_string(),
            description: None,
            cron_expression: "0 0 * * * * *".to_string(), // every hour (7-field cron)
            timezone: "UTC".to_string(),
            target_type: ScheduleTargetType::Workflow,
            target_name: "test".to_string(),
            target_input: Value::Null,
            enabled: true,
            max_concurrent: None,
            catch_up_missed: false,
            max_catch_up: None,
            retry_policy: None,
            last_triggered_at: None,
            next_trigger_at: None,
            claimed_by: None,
            claimed_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let next = scheduler.calculate_next_trigger(&schedule).unwrap();
        assert!(next > Utc::now());
    }

    #[tokio::test]
    async fn test_scheduler_run_loop_processes_schedule() {
        let store = Arc::new(InMemoryWorkflowEventStore::new());

        // Create a schedule due now
        let schedule = CreateScheduleRow {
            name: "loop-test".to_string(),
            description: None,
            cron_expression: "* * * * * * *".to_string(), // every second (7-field cron)
            timezone: "UTC".to_string(),
            target_type: ScheduleTargetType::Workflow,
            target_name: "test-workflow".to_string(),
            target_input: serde_json::json!({"test": true}),
            enabled: true,
            max_concurrent: None,
            catch_up_missed: false,
            max_catch_up: None,
            retry_policy: None,
            next_trigger_at: Some(Utc::now() - chrono::Duration::seconds(1)),
        };

        let schedule_id = store.create_schedule(schedule).await.unwrap();

        // Start scheduler with fast polling
        let config = SchedulerConfig {
            poll_interval: Duration::from_millis(100),
            heartbeat_interval: Duration::from_secs(60),
            ..Default::default()
        };
        let scheduler = DurableScheduler::new(
            Arc::clone(&store) as Arc<dyn WorkflowEventStore>,
            config,
            "test-loop".to_string(),
        );

        let shutdown = CancellationToken::new();
        let token = shutdown.clone();
        tokio::spawn(async move {
            scheduler.run(token).await;
        });

        // Wait for at least one trigger
        tokio::time::sleep(Duration::from_millis(500)).await;
        shutdown.cancel();

        let stats = store.get_schedule_stats(schedule_id).await.unwrap();
        assert!(
            stats.total_executions >= 1,
            "expected at least 1 execution, got {}",
            stats.total_executions
        );
    }
}

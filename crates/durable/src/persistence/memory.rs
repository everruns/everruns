//! In-memory implementation of WorkflowEventStore for testing

use std::collections::HashMap;
#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use parking_lot::RwLock;
use uuid::Uuid;

use super::store::*;
use crate::workflow::{WorkflowError, WorkflowEvent, WorkflowSignal};

mod admin;
mod circuit_breakers;
mod dlq;
mod event_log;
mod schedules;
mod signals;
mod task_queue;
mod task_table;
mod workers;
use task_table::{TaskState, TaskTable};

/// Internal workflow state
#[allow(dead_code)] // Fields stored for debugging/future use
struct WorkflowState {
    workflow_type: String,
    status: WorkflowStatus,
    input: serde_json::Value,
    result: Option<serde_json::Value>,
    error: Option<WorkflowError>,
    events: Vec<WorkflowEvent>,
    signals: Vec<WorkflowSignal>,
    created_at: chrono::DateTime<chrono::Utc>,
    started_at: Option<chrono::DateTime<chrono::Utc>>,
    completed_at: Option<chrono::DateTime<chrono::Utc>>,
    continued_as_new_id: Option<Uuid>,
}

/// Circuit breaker state in memory
struct CircuitBreakerMemState {
    state: crate::reliability::CircuitState,
    failure_count: u32,
    success_count: u32,
    opened_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Schedule state in memory
struct ScheduleMemState {
    row: ScheduleRow,
}

/// Schedule execution state in memory
struct ScheduleExecutionMemState {
    row: ScheduleExecutionRow,
}

/// Snapshot state in memory
struct SnapshotMemState {
    sequence_num: i32,
    snapshot_data: Vec<u8>,
    created_at: chrono::DateTime<chrono::Utc>,
}

/// In-memory implementation of WorkflowEventStore
///
/// This is primarily for testing. It stores all data in memory and
/// provides the same semantics as the PostgreSQL implementation.
///
/// # Example
///
/// ```
/// use everruns_durable::InMemoryWorkflowEventStore;
///
/// let store = InMemoryWorkflowEventStore::new();
/// ```
pub struct InMemoryWorkflowEventStore {
    workflows: RwLock<HashMap<Uuid, WorkflowState>>,
    tasks: RwLock<TaskTable>,
    dlq: RwLock<HashMap<Uuid, DlqEntry>>,
    circuit_breakers: RwLock<HashMap<String, CircuitBreakerMemState>>,
    workers: RwLock<HashMap<String, WorkerInfo>>,
    /// Snapshots keyed by workflow_id -> list of snapshots (sorted by sequence_num)
    snapshots: RwLock<HashMap<Uuid, Vec<SnapshotMemState>>>,
    schedules: RwLock<HashMap<Uuid, ScheduleMemState>>,
    schedule_executions: RwLock<HashMap<Uuid, ScheduleExecutionMemState>>,
    scheduler_instances: RwLock<HashMap<String, SchedulerInstanceInfo>>,
    #[cfg(test)]
    load_events_calls: AtomicUsize,
    #[cfg(test)]
    count_events_calls: AtomicUsize,
    max_pending_tasks_per_workflow: u32,
}

impl InMemoryWorkflowEventStore {
    /// Create a new in-memory store
    pub fn new() -> Self {
        Self {
            workflows: RwLock::new(HashMap::new()),
            tasks: RwLock::new(TaskTable::default()),
            dlq: RwLock::new(HashMap::new()),
            circuit_breakers: RwLock::new(HashMap::new()),
            workers: RwLock::new(HashMap::new()),
            snapshots: RwLock::new(HashMap::new()),
            schedules: RwLock::new(HashMap::new()),
            schedule_executions: RwLock::new(HashMap::new()),
            scheduler_instances: RwLock::new(HashMap::new()),
            #[cfg(test)]
            load_events_calls: AtomicUsize::new(0),
            #[cfg(test)]
            count_events_calls: AtomicUsize::new(0),
            max_pending_tasks_per_workflow: super::store::max_pending_tasks_per_workflow_from_env(),
        }
    }

    /// Create with a custom max pending tasks per workflow limit (for testing)
    #[cfg(test)]
    pub fn with_max_pending_tasks(limit: u32) -> Self {
        let mut store = Self::new();
        store.max_pending_tasks_per_workflow = limit;
        store
    }

    #[cfg(test)]
    pub fn load_events_call_count(&self) -> usize {
        self.load_events_calls.load(Ordering::Relaxed)
    }

    #[cfg(test)]
    pub fn count_events_call_count(&self) -> usize {
        self.count_events_calls.load(Ordering::Relaxed)
    }

    /// Get the number of workflows
    pub fn workflow_count(&self) -> usize {
        self.workflows.read().len()
    }

    /// Get the number of pending tasks
    pub fn pending_task_count(&self) -> usize {
        self.tasks.read().pending_total()
    }

    /// Make a claimed task look abandoned, so the next
    /// [`reclaim_stale_tasks`](super::TaskQueue::reclaim_stale_tasks) returns
    /// it whatever the threshold. Stands in for a worker that stopped
    /// heartbeating.
    pub fn expire_claim(&self, task_id: Uuid) {
        self.tasks.write().update(task_id, |task| {
            task.heartbeat_at = Some(chrono::DateTime::<Utc>::MIN_UTC);
        });
    }

    /// Get the number of DLQ entries
    pub fn dlq_count(&self) -> usize {
        self.dlq.read().len()
    }

    /// Clear all data (for testing)
    pub fn clear(&self) {
        self.workflows.write().clear();
        self.tasks.write().clear();
        self.workers.write().clear();
        self.dlq.write().clear();
    }
}

impl Default for InMemoryWorkflowEventStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    /// Register workers so the store lets them claim, as PostgreSQL requires.
    fn register_workers(store: &InMemoryWorkflowEventStore, ids: &[&str]) {
        let mut workers = store.workers.write();
        for id in ids {
            workers.insert(id.to_string(), WorkerInfo::new(*id, Vec::<String>::new()));
        }
    }

    use super::*;
    use crate::workflow::ActivityOptions;

    #[tokio::test]
    async fn test_create_and_get_workflow() {
        let store = InMemoryWorkflowEventStore::new();
        let workflow_id = Uuid::now_v7();

        store
            .create_workflow(
                workflow_id,
                "test_workflow",
                serde_json::json!({"key": "value"}),
                None,
            )
            .await
            .unwrap();

        let status = store.get_workflow_status(workflow_id).await.unwrap();
        assert_eq!(status, WorkflowStatus::Pending);
    }

    #[tokio::test]
    async fn test_append_and_load_events() {
        let store = InMemoryWorkflowEventStore::new();
        let workflow_id = Uuid::now_v7();

        store
            .create_workflow(workflow_id, "test", serde_json::json!({}), None)
            .await
            .unwrap();

        // Append first event
        let seq = store
            .append_events(
                workflow_id,
                0,
                vec![WorkflowEvent::started(serde_json::json!({}))],
            )
            .await
            .unwrap();
        assert_eq!(seq, 1);

        // Append second event
        let seq = store
            .append_events(
                workflow_id,
                1,
                vec![WorkflowEvent::ActivityScheduled {
                    activity_id: "step-1".to_string(),
                    activity_type: "test_activity".to_string(),
                    input: serde_json::json!({}),
                    options: ActivityOptions::default(),
                }],
            )
            .await
            .unwrap();
        assert_eq!(seq, 2);

        // Load events
        let events = store.load_events(workflow_id).await.unwrap();
        assert_eq!(events.len(), 2);
    }

    #[tokio::test]
    async fn test_concurrency_conflict() {
        let store = InMemoryWorkflowEventStore::new();
        let workflow_id = Uuid::now_v7();

        store
            .create_workflow(workflow_id, "test", serde_json::json!({}), None)
            .await
            .unwrap();

        // Append with wrong sequence should fail
        let result = store
            .append_events(
                workflow_id,
                5, // Wrong sequence
                vec![WorkflowEvent::started(serde_json::json!({}))],
            )
            .await;

        assert!(matches!(
            result,
            Err(StoreError::ConcurrencyConflict { .. })
        ));
    }

    #[tokio::test]
    async fn test_task_lifecycle() {
        let store = InMemoryWorkflowEventStore::new();
        register_workers(&store, &["worker-1"]);
        let workflow_id = Uuid::now_v7();

        store
            .create_workflow(workflow_id, "test", serde_json::json!({}), None)
            .await
            .unwrap();

        // Enqueue task
        let task_id = store
            .enqueue_task(TaskDefinition {
                workflow_id: Some(workflow_id),
                activity_id: "step-1".to_string(),
                activity_type: "test_activity".to_string(),
                input: serde_json::json!({}),
                options: ActivityOptions::default(),
            })
            .await
            .unwrap();

        assert_eq!(store.pending_task_count(), 1);

        // Claim task
        let claimed = store
            .claim_task("worker-1", &["test_activity".to_string()], 1)
            .await
            .unwrap();
        assert_eq!(claimed.len(), 1);
        assert_eq!(claimed[0].id, task_id);

        // Complete task (pass worker_id that claimed it)
        store
            .complete_task(task_id, "worker-1", serde_json::json!({"result": "ok"}))
            .await
            .unwrap();

        // Task should no longer be pending
        assert_eq!(store.pending_task_count(), 0);
    }

    #[tokio::test]
    async fn test_task_retry() {
        let store = InMemoryWorkflowEventStore::new();
        register_workers(&store, &["worker-1"]);
        let workflow_id = Uuid::now_v7();

        store
            .create_workflow(workflow_id, "test", serde_json::json!({}), None)
            .await
            .unwrap();

        // Enqueue task with 3 max attempts
        let options = ActivityOptions::default();
        let task_id = store
            .enqueue_task(TaskDefinition {
                workflow_id: Some(workflow_id),
                activity_id: "step-1".to_string(),
                activity_type: "test_activity".to_string(),
                input: serde_json::json!({}),
                options,
            })
            .await
            .unwrap();

        // Claim and fail
        store
            .claim_task("worker-1", &["test_activity".to_string()], 1)
            .await
            .unwrap();

        let outcome = store.fail_task(task_id, "error 1").await.unwrap();
        assert!(matches!(outcome, TaskFailureOutcome::WillRetry { .. }));

        // Task should be pending again
        assert_eq!(store.pending_task_count(), 1);
    }

    #[tokio::test]
    async fn test_signals() {
        let store = InMemoryWorkflowEventStore::new();
        let workflow_id = Uuid::now_v7();

        store
            .create_workflow(workflow_id, "test", serde_json::json!({}), None)
            .await
            .unwrap();

        // Send signal
        store
            .send_signal(workflow_id, WorkflowSignal::cancel("user cancelled"))
            .await
            .unwrap();

        // Get pending signals
        let signals = store.get_pending_signals(workflow_id).await.unwrap();
        assert_eq!(signals.len(), 1);
        assert!(signals[0].is_cancel());

        // Mark as processed
        store.mark_signals_processed(workflow_id, 1).await.unwrap();

        let signals = store.get_pending_signals(workflow_id).await.unwrap();
        assert_eq!(signals.len(), 0);
    }

    // ========================================================================
    // Task ownership verification tests (duplicate scheduling prevention)
    // ========================================================================

    #[tokio::test]
    async fn test_complete_task_wrong_worker_rejected() {
        // Scenario: Worker A claims task, Worker B tries to complete it
        let store = InMemoryWorkflowEventStore::new();
        register_workers(&store, &["worker-A"]);
        let workflow_id = Uuid::now_v7();

        store
            .create_workflow(workflow_id, "test", serde_json::json!({}), None)
            .await
            .unwrap();

        let task_id = store
            .enqueue_task(TaskDefinition {
                workflow_id: Some(workflow_id),
                activity_id: "step-1".to_string(),
                activity_type: "test_activity".to_string(),
                input: serde_json::json!({}),
                options: ActivityOptions::default(),
            })
            .await
            .unwrap();

        // Worker A claims the task
        let claimed = store
            .claim_task("worker-A", &["test_activity".to_string()], 1)
            .await
            .unwrap();
        assert_eq!(claimed.len(), 1);
        assert_eq!(claimed[0].id, task_id);

        // Worker B tries to complete the task (should fail)
        let result = store
            .complete_task(task_id, "worker-B", serde_json::json!({"result": "ok"}))
            .await;

        assert!(
            matches!(result, Err(StoreError::TaskNotOwned(_))),
            "Expected TaskNotOwned error, got: {:?}",
            result
        );

        // Worker A can still complete it
        store
            .complete_task(task_id, "worker-A", serde_json::json!({"result": "ok"}))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_complete_task_already_completed_rejected() {
        // Scenario: Worker A completes task, then tries to complete again
        let store = InMemoryWorkflowEventStore::new();
        register_workers(&store, &["worker-A"]);
        let workflow_id = Uuid::now_v7();

        store
            .create_workflow(workflow_id, "test", serde_json::json!({}), None)
            .await
            .unwrap();

        let task_id = store
            .enqueue_task(TaskDefinition {
                workflow_id: Some(workflow_id),
                activity_id: "step-1".to_string(),
                activity_type: "test_activity".to_string(),
                input: serde_json::json!({}),
                options: ActivityOptions::default(),
            })
            .await
            .unwrap();

        // Claim and complete
        store
            .claim_task("worker-A", &["test_activity".to_string()], 1)
            .await
            .unwrap();

        store
            .complete_task(task_id, "worker-A", serde_json::json!({"result": "ok"}))
            .await
            .unwrap();

        // Try to complete again (should fail)
        let result = store
            .complete_task(task_id, "worker-A", serde_json::json!({"result": "ok2"}))
            .await;

        assert!(
            matches!(result, Err(StoreError::TaskNotOwned(_))),
            "Expected TaskNotOwned error for already completed task, got: {:?}",
            result
        );
    }

    #[tokio::test]
    async fn test_two_workers_race_condition_prevented() {
        // Scenario: Simulates the race condition that causes duplicate atoms
        // 1. Worker A claims task
        // 2. Task heartbeat times out, task is reclaimed (simulated by failing)
        // 3. Worker B claims the same task
        // 4. Worker B completes the task
        // 5. Worker A tries to complete (should be rejected)
        let store = InMemoryWorkflowEventStore::new();
        register_workers(&store, &["worker-A", "worker-B"]);
        let workflow_id = Uuid::now_v7();

        store
            .create_workflow(workflow_id, "test", serde_json::json!({}), None)
            .await
            .unwrap();

        let task_id = store
            .enqueue_task(TaskDefinition {
                workflow_id: Some(workflow_id),
                activity_id: "act-1".to_string(),
                activity_type: "act".to_string(),
                input: serde_json::json!({"step": 1}),
                options: ActivityOptions::default(),
            })
            .await
            .unwrap();

        // Step 1: Worker A claims the task
        let claimed_a = store
            .claim_task("worker-A", &["act".to_string()], 1)
            .await
            .unwrap();
        assert_eq!(claimed_a.len(), 1);

        // Step 2: Simulate heartbeat timeout - task goes back to pending
        // (In real scenario, reclaim_stale_tasks would do this)
        {
            let mut tasks = store.tasks.write();
            tasks.update(task_id, |task| task.release(chrono::Utc::now()));
        }

        // Step 3: Worker B claims the same task
        let claimed_b = store
            .claim_task("worker-B", &["act".to_string()], 1)
            .await
            .unwrap();
        assert_eq!(claimed_b.len(), 1);
        assert_eq!(claimed_b[0].id, task_id);

        // Step 4: Worker B completes the task successfully
        store
            .complete_task(task_id, "worker-B", serde_json::json!({"result": "from B"}))
            .await
            .unwrap();

        // Step 5: Worker A (late) tries to complete - should be REJECTED
        let result_a = store
            .complete_task(task_id, "worker-A", serde_json::json!({"result": "from A"}))
            .await;

        assert!(
            matches!(result_a, Err(StoreError::TaskNotOwned(_))),
            "Worker A should be rejected since task was reclaimed and completed by Worker B. Got: {:?}",
            result_a
        );
    }

    #[tokio::test]
    async fn test_duplicate_scheduling_prevention_workflow() {
        // End-to-end scenario testing the fix prevents duplicate atom scheduling
        // This simulates the full workflow that was causing duplicate atoms
        let store = InMemoryWorkflowEventStore::new();
        register_workers(&store, &["worker-A", "worker-B"]);
        let workflow_id = Uuid::now_v7();

        store
            .create_workflow(
                workflow_id,
                "message_processing",
                serde_json::json!({}),
                None,
            )
            .await
            .unwrap();

        // Enqueue an "act" task (triggered by user message)
        let task_id = store
            .enqueue_task(TaskDefinition {
                workflow_id: Some(workflow_id),
                activity_id: "act-0".to_string(),
                activity_type: "act".to_string(),
                input: serde_json::json!({"message": "hello"}),
                options: ActivityOptions::default(),
            })
            .await
            .unwrap();

        // Worker A claims and starts processing
        let _ = store
            .claim_task("worker-A", &["act".to_string()], 1)
            .await
            .unwrap();

        // Simulate: Worker A takes too long, task reclaimed
        {
            let mut tasks = store.tasks.write();
            tasks.update(task_id, |task| task.release(chrono::Utc::now()));
        }

        // Worker B claims and completes
        let _ = store
            .claim_task("worker-B", &["act".to_string()], 1)
            .await
            .unwrap();

        // Worker B completes successfully
        let result_b = store
            .complete_task(
                task_id,
                "worker-B",
                serde_json::json!({"turn_complete": true}),
            )
            .await;
        assert!(result_b.is_ok(), "Worker B should complete successfully");

        // Now we simulate what WOULD happen in the old buggy code:
        // Worker A finishes processing and tries to complete
        let result_a = store
            .complete_task(
                task_id,
                "worker-A",
                serde_json::json!({"turn_complete": true}),
            )
            .await;

        // The fix ensures Worker A is REJECTED
        assert!(
            matches!(result_a, Err(StoreError::TaskNotOwned(_))),
            "Worker A must be rejected to prevent duplicate atom scheduling"
        );

        // In the real workflow, after complete_task fails, the worker
        // should NOT call schedule_next_activity, preventing duplicate atoms
    }

    // =========================================================================
    // Schedule Tests
    // =========================================================================

    #[tokio::test]
    async fn test_schedule_create_and_get() {
        let store = InMemoryWorkflowEventStore::new();

        let schedule = CreateScheduleRow {
            name: "test-schedule".to_string(),
            description: Some("Test description".to_string()),
            cron_expression: "*/5 * * * *".to_string(),
            timezone: "UTC".to_string(),
            target_type: ScheduleTargetType::Workflow,
            target_name: "test_workflow".to_string(),
            target_input: serde_json::json!({"key": "value"}),
            enabled: true,
            max_concurrent: Some(2),
            catch_up_missed: false,
            max_catch_up: Some(1),
            retry_policy: None,
            next_trigger_at: Some(Utc::now()),
        };

        let id = store.create_schedule(schedule.clone()).await.unwrap();
        let retrieved = store.get_schedule(id).await.unwrap();

        assert_eq!(retrieved.name, "test-schedule");
        assert_eq!(retrieved.cron_expression, "*/5 * * * *");
        assert_eq!(retrieved.target_type, ScheduleTargetType::Workflow);
        assert!(retrieved.enabled);
    }

    #[tokio::test]
    async fn test_schedule_list_and_filter() {
        let store = InMemoryWorkflowEventStore::new();

        let schedule = CreateScheduleRow {
            name: "test-schedule".to_string(),
            description: None,
            cron_expression: "*/5 * * * *".to_string(),
            timezone: "UTC".to_string(),
            target_type: ScheduleTargetType::Workflow,
            target_name: "test_workflow".to_string(),
            target_input: serde_json::json!({}),
            enabled: true,
            max_concurrent: None,
            catch_up_missed: false,
            max_catch_up: None,
            retry_policy: None,
            next_trigger_at: None,
        };

        let _id = store.create_schedule(schedule).await.unwrap();

        // List all schedules
        let all = store
            .list_schedules(
                ScheduleFilter::default(),
                Pagination {
                    limit: 100,
                    offset: 0,
                },
            )
            .await
            .unwrap();
        assert_eq!(all.len(), 1);

        // Filter by enabled
        let enabled = store
            .list_schedules(
                ScheduleFilter {
                    enabled: Some(true),
                    target_type: None,
                },
                Pagination {
                    limit: 100,
                    offset: 0,
                },
            )
            .await
            .unwrap();
        assert_eq!(enabled.len(), 1);
    }

    #[tokio::test]
    async fn test_schedule_claim_due() {
        let store = InMemoryWorkflowEventStore::new();
        let now = Utc::now();

        // Create multiple due schedules
        for i in 0..3 {
            let schedule = CreateScheduleRow {
                name: format!("schedule-{}", i),
                description: None,
                cron_expression: "* * * * *".to_string(),
                timezone: "UTC".to_string(),
                target_type: ScheduleTargetType::Workflow,
                target_name: "test".to_string(),
                target_input: serde_json::json!({}),
                enabled: true,
                max_concurrent: None,
                catch_up_missed: false,
                max_catch_up: None,
                retry_policy: None,
                next_trigger_at: Some(now - chrono::Duration::minutes(1)),
            };
            store.create_schedule(schedule).await.unwrap();
        }

        // Claim with limit 10 - should get all 3
        let claimed = store.claim_due_schedules("scheduler-1", 10).await.unwrap();
        assert_eq!(claimed.len(), 3);

        // Second claim should get none (all claimed)
        let claimed2 = store.claim_due_schedules("scheduler-2", 10).await.unwrap();
        assert_eq!(claimed2.len(), 0);
    }

    #[tokio::test]
    async fn test_schedule_execution_lifecycle() {
        let store = InMemoryWorkflowEventStore::new();
        let now = Utc::now();

        // Create schedule
        let schedule = CreateScheduleRow {
            name: "test-schedule".to_string(),
            description: None,
            cron_expression: "*/5 * * * *".to_string(),
            timezone: "UTC".to_string(),
            target_type: ScheduleTargetType::Workflow,
            target_name: "test_workflow".to_string(),
            target_input: serde_json::json!({}),
            enabled: true,
            max_concurrent: None,
            catch_up_missed: false,
            max_catch_up: None,
            retry_policy: None,
            next_trigger_at: Some(now),
        };
        let schedule_id = store.create_schedule(schedule).await.unwrap();

        // Create execution
        let exec_id = store
            .create_schedule_execution(schedule_id, now)
            .await
            .unwrap();

        // Verify running
        let exec = store.get_schedule_execution(exec_id).await.unwrap();
        assert_eq!(exec.status, ScheduleExecutionStatus::Running);

        // Check running count
        let running = store.count_running_executions(schedule_id).await.unwrap();
        assert_eq!(running, 1);

        // Complete execution
        let workflow_id = Uuid::now_v7();
        store
            .complete_schedule_execution(exec_id, workflow_id, true)
            .await
            .unwrap();

        // Verify completed
        let exec = store.get_schedule_execution(exec_id).await.unwrap();
        assert_eq!(exec.status, ScheduleExecutionStatus::Completed);
        assert_eq!(exec.workflow_id, Some(workflow_id));
        assert!(exec.duration_ms.is_some());

        // Running count should be 0
        let running = store.count_running_executions(schedule_id).await.unwrap();
        assert_eq!(running, 0);
    }

    #[tokio::test]
    async fn test_schedule_stats() {
        let store = InMemoryWorkflowEventStore::new();
        let now = Utc::now();

        // Create schedule
        let schedule = CreateScheduleRow {
            name: "test-schedule".to_string(),
            description: None,
            cron_expression: "*/5 * * * *".to_string(),
            timezone: "UTC".to_string(),
            target_type: ScheduleTargetType::Workflow,
            target_name: "test_workflow".to_string(),
            target_input: serde_json::json!({}),
            enabled: true,
            max_concurrent: None,
            catch_up_missed: false,
            max_catch_up: None,
            retry_policy: None,
            next_trigger_at: Some(now),
        };
        let schedule_id = store.create_schedule(schedule).await.unwrap();

        // Create and complete some executions
        for _ in 0..3 {
            let exec_id = store
                .create_schedule_execution(schedule_id, now)
                .await
                .unwrap();
            store
                .complete_schedule_execution(exec_id, Uuid::now_v7(), true)
                .await
                .unwrap();
        }

        // Create a failed execution
        let exec_id = store
            .create_schedule_execution(schedule_id, now)
            .await
            .unwrap();
        store
            .fail_schedule_execution(exec_id, "Test error")
            .await
            .unwrap();

        // Get stats
        let stats = store.get_schedule_stats(schedule_id).await.unwrap();
        assert_eq!(stats.total_executions, 4);
        assert_eq!(stats.successful_executions, 3);
        assert_eq!(stats.failed_executions, 1);
        assert_eq!(
            stats.last_execution_status,
            Some(ScheduleExecutionStatus::Failed)
        );
    }

    #[tokio::test]
    async fn test_task_queue_limit_per_workflow() {
        let store = InMemoryWorkflowEventStore::with_max_pending_tasks(3);
        let workflow_id = Uuid::now_v7();

        store
            .create_workflow(workflow_id, "test", serde_json::json!({}), None)
            .await
            .unwrap();

        // Enqueue up to the limit
        for i in 0..3 {
            store
                .enqueue_task(TaskDefinition {
                    workflow_id: Some(workflow_id),
                    activity_id: format!("act_{i}"),
                    activity_type: "test_activity".to_string(),
                    input: serde_json::json!({}),
                    options: ActivityOptions::default(),
                })
                .await
                .unwrap();
        }

        // Fourth should fail
        let result = store
            .enqueue_task(TaskDefinition {
                workflow_id: Some(workflow_id),
                activity_id: "act_overflow".to_string(),
                activity_type: "test_activity".to_string(),
                input: serde_json::json!({}),
                options: ActivityOptions::default(),
            })
            .await;

        assert!(
            matches!(result, Err(StoreError::TaskQueueLimitExceeded { .. })),
            "expected TaskQueueLimitExceeded, got: {:?}",
            result
        );
    }

    #[tokio::test]
    async fn test_task_queue_limit_does_not_block_other_workflows() {
        let store = InMemoryWorkflowEventStore::with_max_pending_tasks(2);
        let workflow_a = Uuid::now_v7();
        let workflow_b = Uuid::now_v7();

        store
            .create_workflow(workflow_a, "test", serde_json::json!({}), None)
            .await
            .unwrap();
        store
            .create_workflow(workflow_b, "test", serde_json::json!({}), None)
            .await
            .unwrap();

        // Fill workflow A to limit
        for i in 0..2 {
            store
                .enqueue_task(TaskDefinition {
                    workflow_id: Some(workflow_a),
                    activity_id: format!("a_{i}"),
                    activity_type: "test".to_string(),
                    input: serde_json::json!({}),
                    options: ActivityOptions::default(),
                })
                .await
                .unwrap();
        }

        // Workflow B should still work
        store
            .enqueue_task(TaskDefinition {
                workflow_id: Some(workflow_b),
                activity_id: "b_0".to_string(),
                activity_type: "test".to_string(),
                input: serde_json::json!({}),
                options: ActivityOptions::default(),
            })
            .await
            .unwrap();

        // Workflow A should fail
        let result = store
            .enqueue_task(TaskDefinition {
                workflow_id: Some(workflow_a),
                activity_id: "a_overflow".to_string(),
                activity_type: "test".to_string(),
                input: serde_json::json!({}),
                options: ActivityOptions::default(),
            })
            .await;

        assert!(matches!(
            result,
            Err(StoreError::TaskQueueLimitExceeded { .. })
        ));
    }

    #[tokio::test]
    async fn test_standalone_task_enqueue_and_claim() {
        let store = InMemoryWorkflowEventStore::new();
        register_workers(&store, &["worker-1"]);

        // Enqueue standalone task (no workflow)
        let task_id = store
            .enqueue_task(TaskDefinition {
                workflow_id: None,
                activity_id: "standalone-1".to_string(),
                activity_type: "email_send".to_string(),
                input: serde_json::json!({"to": "user@example.com"}),
                options: ActivityOptions::default(),
            })
            .await
            .unwrap();

        // Verify task info has no workflow_id
        let task = store.get_task(task_id).await.unwrap();
        assert!(task.workflow_id.is_none());
        assert_eq!(task.activity_type, "email_send");

        // Claim it
        let claimed = store
            .claim_task("worker-1", &["email_send".to_string()], 1)
            .await
            .unwrap();
        assert_eq!(claimed.len(), 1);
        assert_eq!(claimed[0].id, task_id);
        assert!(claimed[0].workflow_id.is_none());
    }

    #[tokio::test]
    async fn test_standalone_task_list_filter() {
        let store = InMemoryWorkflowEventStore::new();
        let workflow_id = Uuid::now_v7();

        store
            .create_workflow(workflow_id, "test", serde_json::json!({}), None)
            .await
            .unwrap();

        // Enqueue one workflow task and one standalone task
        store
            .enqueue_task(TaskDefinition {
                workflow_id: Some(workflow_id),
                activity_id: "wf-task".to_string(),
                activity_type: "test".to_string(),
                input: serde_json::json!({}),
                options: ActivityOptions::default(),
            })
            .await
            .unwrap();

        store
            .enqueue_task(TaskDefinition {
                workflow_id: None,
                activity_id: "standalone-task".to_string(),
                activity_type: "test".to_string(),
                input: serde_json::json!({}),
                options: ActivityOptions::default(),
            })
            .await
            .unwrap();

        // List all tasks
        let all = store
            .list_tasks(
                TaskFilter {
                    status: None,
                    activity_type: None,
                    workflow_id: None,
                    standalone_only: false,
                },
                Pagination {
                    offset: 0,
                    limit: 100,
                },
            )
            .await
            .unwrap();
        assert_eq!(all.len(), 2);

        // List standalone only
        let standalone = store
            .list_tasks(
                TaskFilter {
                    status: None,
                    activity_type: None,
                    workflow_id: None,
                    standalone_only: true,
                },
                Pagination {
                    offset: 0,
                    limit: 100,
                },
            )
            .await
            .unwrap();
        assert_eq!(standalone.len(), 1);
        assert!(standalone[0].workflow_id.is_none());
    }

    #[tokio::test]
    async fn test_standalone_task_queue_limit() {
        let store = InMemoryWorkflowEventStore::new();
        register_workers(&store, &["worker"]);
        let definition = TaskDefinition {
            workflow_id: None,
            activity_id: "standalone".into(),
            activity_type: "test".into(),
            input: serde_json::json!({}),
            options: ActivityOptions::default(),
        };
        // Seed just below the cap directly to avoid quadratic enqueue setup.
        {
            let mut tasks = store.tasks.write();
            for _ in 0..DEFAULT_MAX_PENDING_STANDALONE_TASKS - 1 {
                tasks.insert(Uuid::now_v7(), TaskState::pending(definition.clone()));
            }
        }
        store
            .enqueue_task(definition.clone())
            .await
            .expect("last available slot");
        let error = store.enqueue_task(definition.clone()).await.unwrap_err();
        assert!(
            matches!(error, StoreError::StandaloneTaskQueueLimitExceeded { current, limit }
            if current == DEFAULT_MAX_PENDING_STANDALONE_TASKS && limit == DEFAULT_MAX_PENDING_STANDALONE_TASKS)
        );
        let claimed = store
            .claim_task("worker", &["test".into()], 1)
            .await
            .unwrap();
        assert_eq!(claimed.len(), 1);
        store
            .enqueue_task(definition)
            .await
            .expect("claim frees a pending slot");
    }

    // =========================================================================
    // System Health Tests
    // =========================================================================

    #[tokio::test]
    async fn test_system_health_empty() {
        let store = InMemoryWorkflowEventStore::new();
        let health = store.get_system_health().await.unwrap();

        assert_eq!(health.total_workers, 0);
        assert_eq!(health.active_workers, 0);
        assert_eq!(health.workers_accepting, 0);
        assert_eq!(health.total_capacity, 0);
        assert_eq!(health.current_load, 0);
        assert_eq!(health.pending_tasks, 0);
        assert_eq!(health.claimed_tasks, 0);
        assert_eq!(health.completed_tasks, 0);
        assert_eq!(health.failed_tasks, 0);
        assert_eq!(health.started_tasks, 0);
        assert_eq!(health.running_workflows, 0);
        assert_eq!(health.pending_workflows, 0);
        assert_eq!(health.completed_workflows, 0);
        assert_eq!(health.failed_workflows, 0);
        assert_eq!(health.started_workflows, 0);
        assert_eq!(health.dlq_size, 0);
    }

    #[tokio::test]
    async fn test_system_health_worker_counts() {
        let store = InMemoryWorkflowEventStore::new();

        // Register two active workers
        store
            .register_worker(WorkerInfo {
                id: "w1".to_string(),
                worker_group: Some("default".to_string()),
                activity_types: vec!["act".to_string()],
                max_concurrency: 10,
                current_load: 3,
                status: "active".to_string(),
                accepting_tasks: true,
                backpressure_reason: None,
                started_at: Utc::now(),
                last_heartbeat_at: Utc::now(),
                hostname: None,
                version: None,
                metadata: None,
                tasks_completed: 0,
                tasks_failed: 0,
                avg_task_duration_ms: None,
            })
            .await
            .unwrap();

        store
            .register_worker(WorkerInfo {
                id: "w2".to_string(),
                worker_group: Some("default".to_string()),
                activity_types: vec!["act".to_string()],
                max_concurrency: 5,
                current_load: 2,
                status: "active".to_string(),
                accepting_tasks: false,
                backpressure_reason: Some("overloaded".to_string()),
                started_at: Utc::now(),
                last_heartbeat_at: Utc::now(),
                hostname: None,
                version: None,
                metadata: None,
                tasks_completed: 0,
                tasks_failed: 0,
                avg_task_duration_ms: None,
            })
            .await
            .unwrap();

        let health = store.get_system_health().await.unwrap();

        assert_eq!(health.total_workers, 2);
        assert_eq!(health.active_workers, 2);
        assert_eq!(health.workers_accepting, 1); // only w1 accepts
        assert_eq!(health.total_capacity, 15); // 10 + 5
        assert_eq!(health.current_load, 5); // 3 + 2
    }

    #[tokio::test]
    async fn test_system_health_draining_workers_excluded_from_capacity() {
        let store = InMemoryWorkflowEventStore::new();

        store
            .register_worker(WorkerInfo {
                id: "w-active".to_string(),
                worker_group: Some("default".to_string()),
                activity_types: vec!["act".to_string()],
                max_concurrency: 10,
                current_load: 2,
                status: "active".to_string(),
                accepting_tasks: true,
                backpressure_reason: None,
                started_at: Utc::now(),
                last_heartbeat_at: Utc::now(),
                hostname: None,
                version: None,
                metadata: None,
                tasks_completed: 0,
                tasks_failed: 0,
                avg_task_duration_ms: None,
            })
            .await
            .unwrap();

        store
            .register_worker(WorkerInfo {
                id: "w-draining".to_string(),
                worker_group: Some("default".to_string()),
                activity_types: vec!["act".to_string()],
                max_concurrency: 10,
                current_load: 5,
                status: "draining".to_string(),
                accepting_tasks: false,
                backpressure_reason: None,
                started_at: Utc::now(),
                last_heartbeat_at: Utc::now(),
                hostname: None,
                version: None,
                metadata: None,
                tasks_completed: 0,
                tasks_failed: 0,
                avg_task_duration_ms: None,
            })
            .await
            .unwrap();

        let health = store.get_system_health().await.unwrap();

        assert_eq!(health.total_workers, 2);
        assert_eq!(health.active_workers, 1);
        assert_eq!(health.total_capacity, 10); // only active worker
        assert_eq!(health.current_load, 2); // only active worker
    }

    #[tokio::test]
    async fn test_system_health_stale_workers_excluded() {
        let store = InMemoryWorkflowEventStore::new();

        // Register a worker with a stale heartbeat (> 60s ago)
        store
            .register_worker(WorkerInfo {
                id: "w-stale".to_string(),
                worker_group: Some("default".to_string()),
                activity_types: vec!["act".to_string()],
                max_concurrency: 10,
                current_load: 5,
                status: "active".to_string(),
                accepting_tasks: true,
                backpressure_reason: None,
                started_at: Utc::now(),
                last_heartbeat_at: Utc::now() - chrono::Duration::seconds(120),
                hostname: None,
                version: None,
                metadata: None,
                tasks_completed: 0,
                tasks_failed: 0,
                avg_task_duration_ms: None,
            })
            .await
            .unwrap();

        let health = store.get_system_health().await.unwrap();

        assert_eq!(health.total_workers, 1); // still counted as total
        assert_eq!(health.active_workers, 0); // but not active (stale heartbeat)
        assert_eq!(health.workers_accepting, 0);
        assert_eq!(health.total_capacity, 0);
        assert_eq!(health.current_load, 0);
    }

    #[tokio::test]
    async fn test_system_health_workflow_counts() {
        let store = InMemoryWorkflowEventStore::new();

        // Create workflows in different states
        let wf_pending = Uuid::now_v7();
        store
            .create_workflow(wf_pending, "test", serde_json::json!({}), None)
            .await
            .unwrap();
        // Stays pending (default)

        let wf_running = Uuid::now_v7();
        store
            .create_workflow(wf_running, "test", serde_json::json!({}), None)
            .await
            .unwrap();
        store
            .update_workflow_status(wf_running, WorkflowStatus::Running, None, None)
            .await
            .unwrap();

        let wf_completed = Uuid::now_v7();
        store
            .create_workflow(wf_completed, "test", serde_json::json!({}), None)
            .await
            .unwrap();
        store
            .update_workflow_status(
                wf_completed,
                WorkflowStatus::Completed,
                Some(serde_json::json!({})),
                None,
            )
            .await
            .unwrap();

        let wf_failed = Uuid::now_v7();
        store
            .create_workflow(wf_failed, "test", serde_json::json!({}), None)
            .await
            .unwrap();
        store
            .update_workflow_status(
                wf_failed,
                WorkflowStatus::Failed,
                None,
                Some(crate::workflow::WorkflowError::new("test failure")),
            )
            .await
            .unwrap();

        let wf_cancelled = Uuid::now_v7();
        store
            .create_workflow(wf_cancelled, "test", serde_json::json!({}), None)
            .await
            .unwrap();
        store
            .update_workflow_status(wf_cancelled, WorkflowStatus::Cancelled, None, None)
            .await
            .unwrap();

        let health = store.get_system_health().await.unwrap();

        assert_eq!(health.pending_workflows, 1);
        assert_eq!(health.running_workflows, 1);
        assert_eq!(health.completed_workflows, 1);
        assert_eq!(health.failed_workflows, 2); // failed + cancelled
    }

    #[tokio::test]
    async fn test_system_health_task_counts() {
        let store = InMemoryWorkflowEventStore::new();
        register_workers(&store, &["w1"]);
        let workflow_id = Uuid::now_v7();
        store
            .create_workflow(workflow_id, "test", serde_json::json!({}), None)
            .await
            .unwrap();

        // Enqueue two tasks
        store
            .enqueue_task(TaskDefinition {
                workflow_id: Some(workflow_id),
                activity_id: "a1".to_string(),
                activity_type: "act".to_string(),
                input: serde_json::json!({}),
                options: ActivityOptions::default(),
            })
            .await
            .unwrap();

        let _task2 = store
            .enqueue_task(TaskDefinition {
                workflow_id: Some(workflow_id),
                activity_id: "a2".to_string(),
                activity_type: "act".to_string(),
                input: serde_json::json!({}),
                options: ActivityOptions::default(),
            })
            .await
            .unwrap();

        // Initial: 2 pending, 0 claimed
        let health = store.get_system_health().await.unwrap();
        assert_eq!(health.pending_tasks, 2);
        assert_eq!(health.claimed_tasks, 0);
        assert_eq!(health.started_tasks, 0);

        // Claim one task
        let claimed = store
            .claim_task("w1", &["act".to_string()], 1)
            .await
            .unwrap();
        let claimed_id = claimed[0].id;

        let health = store.get_system_health().await.unwrap();
        assert_eq!(health.pending_tasks, 1);
        assert_eq!(health.claimed_tasks, 1);
        assert_eq!(health.started_tasks, 1); // claimed_at is set

        // Complete the claimed task
        store
            .complete_task(claimed_id, "w1", serde_json::json!({}))
            .await
            .unwrap();

        let health = store.get_system_health().await.unwrap();
        assert_eq!(health.pending_tasks, 1);
        assert_eq!(health.claimed_tasks, 0);
        assert_eq!(health.completed_tasks, 1);
        assert_eq!(health.started_tasks, 1); // still 1 (ever started)
    }

    #[tokio::test]
    async fn test_system_health_dlq_size() {
        let store = InMemoryWorkflowEventStore::new();
        register_workers(&store, &["w1"]);
        let workflow_id = Uuid::now_v7();
        store
            .create_workflow(workflow_id, "test", serde_json::json!({}), None)
            .await
            .unwrap();

        // Create a task, claim, and move to DLQ
        let task_id = store
            .enqueue_task(TaskDefinition {
                workflow_id: Some(workflow_id),
                activity_id: "a1".to_string(),
                activity_type: "act".to_string(),
                input: serde_json::json!({}),
                options: ActivityOptions::default(),
            })
            .await
            .unwrap();

        store
            .claim_task("w1", &["act".to_string()], 1)
            .await
            .unwrap();
        store
            .move_to_dlq(task_id, vec!["err1".to_string(), "err2".to_string()])
            .await
            .unwrap();

        let health = store.get_system_health().await.unwrap();
        assert_eq!(health.dlq_size, 1);
    }

    #[tokio::test]
    async fn test_system_health_started_workflows_counts_started_at() {
        let store = InMemoryWorkflowEventStore::new();

        // Create a pending workflow (never started)
        let wf1 = Uuid::now_v7();
        store
            .create_workflow(wf1, "test", serde_json::json!({}), None)
            .await
            .unwrap();

        // Create a running workflow (has started_at)
        let wf2 = Uuid::now_v7();
        store
            .create_workflow(wf2, "test", serde_json::json!({}), None)
            .await
            .unwrap();
        store
            .update_workflow_status(wf2, WorkflowStatus::Running, None, None)
            .await
            .unwrap();

        // Create a completed workflow (has started_at)
        let wf3 = Uuid::now_v7();
        store
            .create_workflow(wf3, "test", serde_json::json!({}), None)
            .await
            .unwrap();
        store
            .update_workflow_status(wf3, WorkflowStatus::Running, None, None)
            .await
            .unwrap();
        store
            .update_workflow_status(
                wf3,
                WorkflowStatus::Completed,
                Some(serde_json::json!({})),
                None,
            )
            .await
            .unwrap();

        let health = store.get_system_health().await.unwrap();
        // started_workflows counts workflows where started_at IS NOT NULL
        // wf2 (running) and wf3 (completed after running) should have started_at set
        assert_eq!(health.started_workflows, 2);
    }

    // EVE-455: previously, the API verified workflow existence by reading the
    // first 1000 rows of `list_workflows`, ordered by `created_at DESC`. Once
    // the store had > 1000 workflows, the oldest one fell off the page and
    // its detail/SSE endpoint silently returned 404. The direct
    // `get_workflow_extended` lookup must resolve any workflow regardless of
    // how many newer workflows exist.
    #[tokio::test]
    async fn test_get_workflow_extended_resolves_past_first_page() {
        let store = InMemoryWorkflowEventStore::new();

        // Create the workflow we will look up first.
        let target = Uuid::now_v7();
        store
            .create_workflow(target, "old", serde_json::json!({}), None)
            .await
            .unwrap();

        // Pile on > 1000 newer workflows.
        for _ in 0..1100 {
            store
                .create_workflow(Uuid::now_v7(), "filler", serde_json::json!({}), None)
                .await
                .unwrap();
        }

        // Force the target's `created_at` strictly older than every filler.
        // Without this the regression guard would be flaky: in a tight loop
        // two `create_workflow` calls can land on the same `Utc::now()` and
        // `list_workflows` sorts only by `created_at` (no tie-breaker), so
        // the target could end up inside the first 1000 by HashMap-iteration
        // luck even with > 1000 workflows. (Copilot review on PR #1760.)
        {
            let mut workflows = store.workflows.write();
            let target_state = workflows.get_mut(&target).expect("target workflow exists");
            target_state.created_at = chrono::Utc::now() - chrono::Duration::days(1);
        }

        // The legacy first-page scan would not find this workflow.
        let first_page = store
            .list_workflows(
                WorkflowFilter::default(),
                Pagination {
                    offset: 0,
                    limit: 1000,
                },
            )
            .await
            .unwrap();
        assert_eq!(first_page.len(), 1000);
        assert!(
            !first_page.iter().any(|w| w.id == target),
            "regression guard: target workflow must be off the first page"
        );

        // The direct lookup must resolve it anyway.
        let resolved = store
            .get_workflow_extended(target)
            .await
            .unwrap()
            .expect("get_workflow_extended must find old workflow");
        assert_eq!(resolved.id, target);
        assert_eq!(resolved.workflow_type, "old");
    }

    #[tokio::test]
    async fn test_get_workflow_extended_returns_none_for_unknown_id() {
        let store = InMemoryWorkflowEventStore::new();
        let unknown = Uuid::now_v7();
        let resolved = store.get_workflow_extended(unknown).await.unwrap();
        assert!(resolved.is_none());
    }

    // ---- EVE-534: forward-progress guard (no DB) ----

    async fn enqueue_reason_task(
        store: &InMemoryWorkflowEventStore,
        workflow_id: Uuid,
        max_attempts: u32,
    ) -> Uuid {
        let options = ActivityOptions {
            retry_policy: crate::reliability::RetryPolicy::exponential()
                .with_max_attempts(max_attempts),
            ..Default::default()
        };
        store
            .enqueue_task(TaskDefinition {
                workflow_id: Some(workflow_id),
                activity_id: "reason_task".to_string(),
                activity_type: "reason".to_string(),
                input: serde_json::json!({"org_id": 1}),
                options,
            })
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn test_no_progress_turn_is_sealed_in_memory() {
        // Relies on the default seal threshold (3); avoid mutating the
        // process-global env, which is flaky under parallel test execution.
        let threshold = DEFAULT_NO_PROGRESS_SEAL_THRESHOLD;
        let store = InMemoryWorkflowEventStore::new();
        register_workers(&store, &["w1"]);
        let workflow_id = Uuid::now_v7();
        store
            .create_workflow(workflow_id, "seal_test", serde_json::json!({}), None)
            .await
            .unwrap();
        // max_attempts generous so the seal fires on progress, not attempts.
        let task_id = enqueue_reason_task(&store, workflow_id, 50).await;

        let mut sealed = false;
        for cycle in 1..=threshold {
            // Claim (simulate worker picking it up then crashing) — no genuine
            // forward-progress event recorded.
            let claimed = store
                .claim_task("w1", &["reason".to_string()], 1)
                .await
                .unwrap();
            assert_eq!(claimed.len(), 1, "cycle {cycle}: claimable before seal");

            // Replicate the Postgres claim_task flow: a real claim writes an
            // 'activity_started' event to durable_workflow_events on EVERY
            // (re)claim, advancing the event sequence. The seal guard MUST NOT
            // treat that bookkeeping event as forward progress (EVE-534) —
            // otherwise the token advances every cycle and the crash-looping
            // turn is never sealed (the original bug). We append it here so the
            // in-memory test exercises the same event stream as Postgres.
            store
                .append_events(
                    workflow_id,
                    store.count_events(workflow_id).await.unwrap() as i32,
                    vec![WorkflowEvent::ActivityStarted {
                        activity_id: "reason_task".to_string(),
                        attempt: cycle,
                        worker_id: "w1".to_string(),
                    }],
                )
                .await
                .unwrap();

            store.expire_claim(task_id);

            let result = store
                .reclaim_stale_tasks(Duration::from_secs(30))
                .await
                .unwrap();
            if cycle < threshold {
                assert_eq!(result.reclaimed_ids.len(), 1, "cycle {cycle}: requeued");
                assert!(result.sealed_tasks.is_empty());
            } else {
                assert_eq!(result.sealed_tasks.len(), 1, "sealed at threshold");
                let s = &result.sealed_tasks[0];
                assert_eq!(s.task_id, task_id);
                assert_eq!(s.reason, "no_progress");
                assert_eq!(s.no_progress_count, threshold);
                sealed = true;
            }
        }
        assert!(sealed);

        let task = store.get_task(task_id).await.unwrap();
        assert_eq!(task.status, TaskStatus::Dead, "sealed => dead (DLQ)");
        // Not re-claimable: no more re-billing.
        let claimed = store
            .claim_task("w1", &["reason".to_string()], 1)
            .await
            .unwrap();
        assert!(claimed.is_empty(), "sealed task must not be re-claimed");
        assert!(
            task.attempt <= threshold,
            "seal fired on no-progress, not max_attempts"
        );
    }

    #[tokio::test]
    async fn test_progress_resets_no_progress_counter_in_memory() {
        // Relies on the default seal threshold (3); see note above re: env.
        let store = InMemoryWorkflowEventStore::new();
        register_workers(&store, &["w1"]);
        let workflow_id = Uuid::now_v7();
        store
            .create_workflow(workflow_id, "progress_test", serde_json::json!({}), None)
            .await
            .unwrap();
        let task_id = enqueue_reason_task(&store, workflow_id, 50).await;

        for cycle in 0..6u32 {
            let _ = store
                .claim_task("w1", &["reason".to_string()], 1)
                .await
                .unwrap();
            // Mirror Postgres claim_task: an 'activity_started' bookkeeping
            // event on every (re)claim. On its own this must NOT count as
            // forward progress (EVE-534).
            store
                .append_events(
                    workflow_id,
                    store.count_events(workflow_id).await.unwrap() as i32,
                    vec![WorkflowEvent::ActivityStarted {
                        activity_id: "reason_task".to_string(),
                        attempt: cycle,
                        worker_id: "w1".to_string(),
                    }],
                )
                .await
                .unwrap();
            // ...but the turn also records a genuine progress event each cycle
            // (e.g. the activity completed), which MUST advance the token and
            // reset the no-progress counter so the turn is never sealed.
            store
                .append_events(
                    workflow_id,
                    store.count_events(workflow_id).await.unwrap() as i32,
                    vec![WorkflowEvent::ActivityCompleted {
                        activity_id: "reason_task".to_string(),
                        result: serde_json::json!({"cycle": cycle}),
                    }],
                )
                .await
                .unwrap();

            store.expire_claim(task_id);

            let result = store
                .reclaim_stale_tasks(Duration::from_secs(30))
                .await
                .unwrap();
            assert!(
                result.sealed_tasks.is_empty(),
                "progressing => never sealed"
            );
            assert_eq!(result.reclaimed_ids.len(), 1);
        }
        let task = store.get_task(task_id).await.unwrap();
        assert_eq!(task.status, TaskStatus::Pending);
    }

    /// EVE-534: standalone tasks (workflow_id IS NULL) have no workflow event
    /// stream, so the derived progress token is always 0. They must be exempt
    /// from the seal guard — re-queued to pending across many reclaims and only
    /// DLQ'd via max-attempts, never sealed for "no progress".
    #[tokio::test]
    async fn test_standalone_task_is_not_sealed_in_memory() {
        let store = InMemoryWorkflowEventStore::new();
        register_workers(&store, &["w1"]);
        let options = ActivityOptions {
            // Generous max_attempts so attempt-exhaustion never fires within the
            // loop below; only the seal guard could end the task early.
            retry_policy: crate::reliability::RetryPolicy::exponential().with_max_attempts(50),
            ..Default::default()
        };
        let task_id = store
            .enqueue_task(TaskDefinition {
                workflow_id: None,
                activity_id: "standalone-seal".to_string(),
                activity_type: "standalone".to_string(),
                input: serde_json::json!({"org_id": 1}),
                options,
            })
            .await
            .unwrap();

        // Reclaim far more than the seal threshold; a standalone task must never
        // be sealed and must always be requeued.
        let cycles = DEFAULT_NO_PROGRESS_SEAL_THRESHOLD + 5;
        for cycle in 1..=cycles {
            let claimed = store
                .claim_task("w1", &["standalone".to_string()], 1)
                .await
                .unwrap();
            assert_eq!(claimed.len(), 1, "cycle {cycle}: claimable");

            store.expire_claim(task_id);

            let result = store
                .reclaim_stale_tasks(Duration::from_secs(30))
                .await
                .unwrap();
            assert!(
                result.sealed_tasks.is_empty(),
                "cycle {cycle}: standalone task must never be sealed"
            );
            assert_eq!(
                result.reclaimed_ids.len(),
                1,
                "cycle {cycle}: standalone task requeued to pending"
            );

            let task = store.get_task(task_id).await.unwrap();
            assert_eq!(task.status, TaskStatus::Pending);

            // Inspect internal bookkeeping: the guard must leave standalone
            // tasks untouched (no token, no accumulated no-progress count).
            let tasks = store.tasks.read();
            let internal = tasks.get(&task_id).unwrap();
            assert_eq!(
                internal.no_progress_count, 0,
                "cycle {cycle}: no_progress_count must stay 0 for standalone tasks"
            );
            assert!(
                internal.progress_token.is_none(),
                "cycle {cycle}: standalone tasks must not track a progress token"
            );
        }
    }

    #[tokio::test]
    async fn deterministic_failure_bypasses_retries_and_releases_workflow() {
        let store = InMemoryWorkflowEventStore::new();
        register_workers(&store, &["worker"]);
        let workflow_id = Uuid::now_v7();
        store
            .create_workflow(workflow_id, "turn", serde_json::json!({}), None)
            .await
            .unwrap();
        store
            .update_workflow_status(workflow_id, WorkflowStatus::Running, None, None)
            .await
            .unwrap();
        let task_id = store
            .enqueue_task(TaskDefinition {
                workflow_id: Some(workflow_id),
                activity_id: "reason".to_string(),
                activity_type: "reason".to_string(),
                input: serde_json::json!({"session_id": "session_test"}),
                options: ActivityOptions {
                    retry_policy: crate::reliability::RetryPolicy::exponential()
                        .with_max_attempts(5),
                    ..Default::default()
                },
            })
            .await
            .unwrap();
        store
            .claim_task("worker", &["reason".to_string()], 1)
            .await
            .unwrap();

        let outcome = store
            .fail_task_with_retry(task_id, "Model not configured", false)
            .await
            .unwrap();
        assert!(matches!(outcome, TaskFailureOutcome::MovedToDlq));
        assert_eq!(store.get_task(task_id).await.unwrap().attempt, 1);

        let error = WorkflowError::new("Model not configured");
        assert!(
            store
                .try_fail_workflow(workflow_id, error.clone())
                .await
                .unwrap()
        );
        assert!(!store.try_fail_workflow(workflow_id, error).await.unwrap());
        assert!(
            store
                .try_claim_workflow_for_new_turn(workflow_id)
                .await
                .unwrap()
        );
        assert_eq!(
            store.get_workflow_status(workflow_id).await.unwrap(),
            WorkflowStatus::Running
        );

        let follow_up_task_id = store
            .enqueue_task(TaskDefinition {
                workflow_id: Some(workflow_id),
                activity_id: "reason-follow-up".to_string(),
                activity_type: "reason".to_string(),
                input: serde_json::json!({"model_id": "model_valid"}),
                options: ActivityOptions::default(),
            })
            .await
            .unwrap();
        let claimed = store
            .claim_task("worker", &["reason".to_string()], 1)
            .await
            .unwrap();
        assert_eq!(claimed.len(), 1);
        assert_eq!(claimed[0].id, follow_up_task_id);
    }

    #[tokio::test]
    async fn retryable_failure_keeps_workflow_running_before_final_attempt() {
        let store = InMemoryWorkflowEventStore::new();
        register_workers(&store, &["worker"]);
        let workflow_id = Uuid::now_v7();
        store
            .create_workflow(workflow_id, "turn", serde_json::json!({}), None)
            .await
            .unwrap();
        store
            .update_workflow_status(workflow_id, WorkflowStatus::Running, None, None)
            .await
            .unwrap();
        let task_id = store
            .enqueue_task(TaskDefinition {
                workflow_id: Some(workflow_id),
                activity_id: "reason".to_string(),
                activity_type: "reason".to_string(),
                input: serde_json::json!({}),
                options: ActivityOptions {
                    retry_policy: crate::reliability::RetryPolicy::exponential()
                        .with_max_attempts(5),
                    ..Default::default()
                },
            })
            .await
            .unwrap();
        store
            .claim_task("worker", &["reason".to_string()], 1)
            .await
            .unwrap();

        let outcome = store.fail_task(task_id, "transient").await.unwrap();
        assert!(matches!(outcome, TaskFailureOutcome::WillRetry { .. }));
        assert_eq!(
            store.get_workflow_status(workflow_id).await.unwrap(),
            WorkflowStatus::Running
        );
    }

    #[tokio::test]
    async fn worker_reaper_race_elects_exactly_one_terminal_effect_owner() {
        let store = std::sync::Arc::new(InMemoryWorkflowEventStore::new());
        let workflow_id = Uuid::now_v7();
        store
            .create_workflow(workflow_id, "turn", serde_json::json!({}), None)
            .await
            .unwrap();
        store
            .update_workflow_status(workflow_id, WorkflowStatus::Running, None, None)
            .await
            .unwrap();

        let worker = store.clone();
        let reaper = store.clone();
        let (worker_won, reaper_won) = tokio::join!(
            async {
                let won = worker
                    .try_fail_workflow(workflow_id, WorkflowError::new("worker failure"))
                    .await?;
                if won {
                    crate::task_events::record_workflow_failed(
                        worker.as_ref(),
                        workflow_id,
                        "worker failure".to_string(),
                    )
                    .await;
                }
                Ok::<_, StoreError>(won)
            },
            async {
                let won = reaper
                    .try_fail_workflow(workflow_id, WorkflowError::new("reaper failure"))
                    .await?;
                if won {
                    crate::task_events::record_workflow_failed(
                        reaper.as_ref(),
                        workflow_id,
                        "reaper failure".to_string(),
                    )
                    .await;
                }
                Ok::<_, StoreError>(won)
            },
        );

        assert_eq!(worker_won.unwrap() as u8 + reaper_won.unwrap() as u8, 1);
        let events = store.get_workflow_events(workflow_id).await.unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|event| event.event_type == "workflow_failed")
                .count(),
            1
        );
    }
}

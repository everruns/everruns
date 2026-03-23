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

/// Internal task state
struct TaskState {
    definition: TaskDefinition,
    status: TaskStatus,
    attempt: u32,
    claimed_by: Option<String>,
    last_error: Option<String>,
    error_history: Vec<String>,
    created_at: chrono::DateTime<chrono::Utc>,
    claimed_at: Option<chrono::DateTime<chrono::Utc>>,
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
    tasks: RwLock<HashMap<Uuid, TaskState>>,
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
            tasks: RwLock::new(HashMap::new()),
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
        self.tasks
            .read()
            .values()
            .filter(|t| t.status == TaskStatus::Pending)
            .count()
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

#[async_trait]
impl WorkflowEventStore for InMemoryWorkflowEventStore {
    async fn create_workflow(
        &self,
        workflow_id: Uuid,
        workflow_type: &str,
        input: serde_json::Value,
        _trace_context: Option<&TraceContext>,
    ) -> Result<(), StoreError> {
        let mut workflows = self.workflows.write();
        workflows.insert(
            workflow_id,
            WorkflowState {
                workflow_type: workflow_type.to_string(),
                status: WorkflowStatus::Pending,
                input,
                result: None,
                error: None,
                events: vec![],
                signals: vec![],
                created_at: Utc::now(),
                started_at: None,
                completed_at: None,
                continued_as_new_id: None,
            },
        );
        Ok(())
    }

    async fn get_workflow_status(&self, workflow_id: Uuid) -> Result<WorkflowStatus, StoreError> {
        let workflows = self.workflows.read();
        workflows
            .get(&workflow_id)
            .map(|w| w.status)
            .ok_or(StoreError::WorkflowNotFound(workflow_id))
    }

    async fn get_workflow_info(&self, workflow_id: Uuid) -> Result<WorkflowInfo, StoreError> {
        let workflows = self.workflows.read();
        let workflow = workflows
            .get(&workflow_id)
            .ok_or(StoreError::WorkflowNotFound(workflow_id))?;

        Ok(WorkflowInfo {
            id: workflow_id,
            workflow_type: workflow.workflow_type.clone(),
            status: workflow.status,
            input: workflow.input.clone(),
            result: workflow.result.clone(),
            error: workflow.error.clone(),
            continued_as_new_id: workflow.continued_as_new_id,
        })
    }

    async fn append_events(
        &self,
        workflow_id: Uuid,
        expected_sequence: i32,
        events: Vec<WorkflowEvent>,
    ) -> Result<i32, StoreError> {
        let mut workflows = self.workflows.write();
        let workflow = workflows
            .get_mut(&workflow_id)
            .ok_or(StoreError::WorkflowNotFound(workflow_id))?;

        let current_sequence = workflow.events.len() as i32;
        if current_sequence != expected_sequence {
            return Err(StoreError::ConcurrencyConflict {
                expected: expected_sequence,
                actual: current_sequence,
            });
        }

        workflow.events.extend(events);
        Ok(workflow.events.len() as i32)
    }

    async fn load_events(
        &self,
        workflow_id: Uuid,
    ) -> Result<Vec<(i32, WorkflowEvent)>, StoreError> {
        #[cfg(test)]
        self.load_events_calls.fetch_add(1, Ordering::Relaxed);

        let workflows = self.workflows.read();
        let workflow = workflows
            .get(&workflow_id)
            .ok_or(StoreError::WorkflowNotFound(workflow_id))?;

        Ok(workflow
            .events
            .iter()
            .enumerate()
            .map(|(i, e)| (i as i32, e.clone()))
            .collect())
    }

    async fn count_events(&self, workflow_id: Uuid) -> Result<usize, StoreError> {
        #[cfg(test)]
        self.count_events_calls.fetch_add(1, Ordering::Relaxed);

        let workflows = self.workflows.read();
        let workflow = workflows
            .get(&workflow_id)
            .ok_or(StoreError::WorkflowNotFound(workflow_id))?;

        Ok(workflow.events.len())
    }

    async fn count_events_after(
        &self,
        workflow_id: Uuid,
        after_sequence: i32,
    ) -> Result<usize, StoreError> {
        let workflows = self.workflows.read();
        let workflow = workflows
            .get(&workflow_id)
            .ok_or(StoreError::WorkflowNotFound(workflow_id))?;

        Ok(workflow
            .events
            .iter()
            .enumerate()
            .filter(|(i, _)| (*i as i32) > after_sequence)
            .count())
    }

    async fn load_events_after(
        &self,
        workflow_id: Uuid,
        after_sequence: i32,
    ) -> Result<Vec<(i32, WorkflowEvent)>, StoreError> {
        let workflows = self.workflows.read();
        let workflow = workflows
            .get(&workflow_id)
            .ok_or(StoreError::WorkflowNotFound(workflow_id))?;

        Ok(workflow
            .events
            .iter()
            .enumerate()
            .filter(|(i, _)| (*i as i32) > after_sequence)
            .map(|(i, e)| (i as i32, e.clone()))
            .collect())
    }

    async fn save_snapshot(
        &self,
        workflow_id: Uuid,
        sequence_num: i32,
        snapshot_data: Vec<u8>,
    ) -> Result<(), StoreError> {
        let mut snapshots = self.snapshots.write();
        let entry = snapshots.entry(workflow_id).or_default();

        // UPSERT: replace if same sequence_num exists
        if let Some(existing) = entry.iter_mut().find(|s| s.sequence_num == sequence_num) {
            existing.snapshot_data = snapshot_data;
            existing.created_at = Utc::now();
        } else {
            entry.push(SnapshotMemState {
                sequence_num,
                snapshot_data,
                created_at: Utc::now(),
            });
            entry.sort_by_key(|s| s.sequence_num);
        }

        Ok(())
    }

    async fn load_latest_snapshot(
        &self,
        workflow_id: Uuid,
    ) -> Result<Option<WorkflowSnapshot>, StoreError> {
        let snapshots = self.snapshots.read();
        let snapshot = snapshots
            .get(&workflow_id)
            .and_then(|entries| entries.last())
            .map(|s| WorkflowSnapshot {
                workflow_id,
                sequence_num: s.sequence_num,
                snapshot_data: s.snapshot_data.clone(),
                created_at: s.created_at,
            });
        Ok(snapshot)
    }

    async fn delete_snapshots(&self, workflow_id: Uuid) -> Result<(), StoreError> {
        self.snapshots.write().remove(&workflow_id);
        Ok(())
    }

    async fn update_workflow_status(
        &self,
        workflow_id: Uuid,
        status: WorkflowStatus,
        result: Option<serde_json::Value>,
        error: Option<WorkflowError>,
    ) -> Result<(), StoreError> {
        let mut workflows = self.workflows.write();
        let workflow = workflows
            .get_mut(&workflow_id)
            .ok_or(StoreError::WorkflowNotFound(workflow_id))?;

        workflow.status = status;
        if matches!(status, WorkflowStatus::Pending) {
            // Clear transient fields when resetting for a new turn
            workflow.result = None;
            workflow.error = None;
            workflow.started_at = None;
            workflow.completed_at = None;
        } else {
            workflow.result = result;
            workflow.error = error;
            match status {
                WorkflowStatus::Running => {
                    if workflow.started_at.is_none() {
                        workflow.started_at = Some(Utc::now());
                    }
                }
                WorkflowStatus::Completed
                | WorkflowStatus::Failed
                | WorkflowStatus::Cancelled
                | WorkflowStatus::ContinuedAsNew => {
                    if workflow.completed_at.is_none() {
                        workflow.completed_at = Some(Utc::now());
                    }
                }
                WorkflowStatus::Pending => {} // handled above
            }
        }
        Ok(())
    }

    async fn continue_as_new(
        &self,
        old_workflow_id: Uuid,
        workflow_type: &str,
        input: serde_json::Value,
        snapshot_data: Vec<u8>,
    ) -> Result<Uuid, StoreError> {
        let new_workflow_id = Uuid::now_v7();
        let mut workflows = self.workflows.write();

        // Verify old workflow exists and is running
        let old_workflow = workflows
            .get_mut(&old_workflow_id)
            .ok_or(StoreError::WorkflowNotFound(old_workflow_id))?;

        if old_workflow.status.is_terminal() {
            return Err(StoreError::Database(format!(
                "workflow {old_workflow_id} is already terminal"
            )));
        }

        // Mark old workflow as continued
        old_workflow.status = WorkflowStatus::ContinuedAsNew;
        old_workflow.completed_at = Some(Utc::now());
        old_workflow.continued_as_new_id = Some(new_workflow_id);

        // Clear old events (archive)
        old_workflow.events.clear();

        // Create new workflow with a WorkflowStarted event
        let now = Utc::now();
        workflows.insert(
            new_workflow_id,
            WorkflowState {
                workflow_type: workflow_type.to_string(),
                status: WorkflowStatus::Running,
                input: input.clone(),
                result: None,
                error: None,
                events: vec![WorkflowEvent::WorkflowStarted {
                    input: input.clone(),
                }],
                signals: vec![],
                created_at: now,
                started_at: Some(now),
                completed_at: None,
                continued_as_new_id: None,
            },
        );

        // Save snapshot on the new workflow at sequence 0 (before the start event)
        drop(workflows);
        let mut snapshots = self.snapshots.write();
        // Delete old workflow snapshots
        snapshots.remove(&old_workflow_id);
        // Save snapshot on new workflow
        snapshots
            .entry(new_workflow_id)
            .or_default()
            .push(SnapshotMemState {
                sequence_num: 0,
                snapshot_data,
                created_at: now,
            });

        Ok(new_workflow_id)
    }

    async fn enqueue_task(&self, task: TaskDefinition) -> Result<Uuid, StoreError> {
        let task_id = Uuid::now_v7();
        let mut tasks = self.tasks.write();

        // Check pending task limits
        if let Some(wf_id) = task.workflow_id {
            let limit = self.max_pending_tasks_per_workflow;
            let pending_count = tasks
                .values()
                .filter(|t| {
                    t.definition.workflow_id == Some(wf_id) && t.status == TaskStatus::Pending
                })
                .count() as u32;

            if pending_count >= limit {
                return Err(StoreError::TaskQueueLimitExceeded {
                    workflow_id: wf_id,
                    current: pending_count,
                    limit,
                });
            }
        } else {
            let limit = DEFAULT_MAX_PENDING_STANDALONE_TASKS;
            let pending_count = tasks
                .values()
                .filter(|t| t.definition.workflow_id.is_none() && t.status == TaskStatus::Pending)
                .count() as u32;

            if pending_count >= limit {
                return Err(StoreError::StandaloneTaskQueueLimitExceeded {
                    current: pending_count,
                    limit,
                });
            }
        }

        tasks.insert(
            task_id,
            TaskState {
                definition: task,
                status: TaskStatus::Pending,
                attempt: 0,
                claimed_by: None,
                last_error: None,
                error_history: vec![],
                created_at: Utc::now(),
                claimed_at: None,
            },
        );
        Ok(task_id)
    }

    async fn claim_task(
        &self,
        worker_id: &str,
        activity_types: &[String],
        max_tasks: usize,
    ) -> Result<Vec<ClaimedTask>, StoreError> {
        let mut tasks = self.tasks.write();
        let mut claimed = vec![];

        for (task_id, task) in tasks.iter_mut() {
            if claimed.len() >= max_tasks {
                break;
            }

            // Check attempt < max_attempts to prevent infinite retries when workers panic
            // without calling fail_task (mirroring PostgreSQL fix)
            let max_attempts = task.definition.options.retry_policy.max_attempts;
            if task.status == TaskStatus::Pending
                && activity_types.contains(&task.definition.activity_type)
                && task.attempt < max_attempts
            {
                task.status = TaskStatus::Claimed;
                task.claimed_by = Some(worker_id.to_string());
                task.claimed_at = Some(Utc::now());
                task.attempt += 1;

                claimed.push(ClaimedTask {
                    id: *task_id,
                    workflow_id: task.definition.workflow_id,
                    activity_id: task.definition.activity_id.clone(),
                    activity_type: task.definition.activity_type.clone(),
                    input: task.definition.input.clone(),
                    options: task.definition.options.clone(),
                    attempt: task.attempt,
                    max_attempts: task.definition.options.retry_policy.max_attempts,
                });
            }
        }

        Ok(claimed)
    }

    async fn heartbeat_task(
        &self,
        task_id: Uuid,
        _worker_id: &str,
        _details: Option<serde_json::Value>,
    ) -> Result<HeartbeatResponse, StoreError> {
        let tasks = self.tasks.read();
        if !tasks.contains_key(&task_id) {
            return Err(StoreError::TaskNotFound(task_id));
        }

        Ok(HeartbeatResponse {
            accepted: true,
            should_cancel: false,
        })
    }

    async fn complete_task(
        &self,
        task_id: Uuid,
        worker_id: &str,
        _result: serde_json::Value,
    ) -> Result<(), StoreError> {
        let mut tasks = self.tasks.write();
        let task = tasks
            .get_mut(&task_id)
            .ok_or(StoreError::TaskNotFound(task_id))?;

        // Verify the task is still claimed by this worker
        if task.status != TaskStatus::Claimed {
            return Err(StoreError::TaskNotOwned(task_id));
        }
        if task.claimed_by.as_deref() != Some(worker_id) {
            return Err(StoreError::TaskNotOwned(task_id));
        }

        task.status = TaskStatus::Completed;
        Ok(())
    }

    async fn fail_task(
        &self,
        task_id: Uuid,
        error: &str,
    ) -> Result<TaskFailureOutcome, StoreError> {
        let mut tasks = self.tasks.write();
        let task = tasks
            .get_mut(&task_id)
            .ok_or(StoreError::TaskNotFound(task_id))?;

        task.error_history.push(error.to_string());
        task.last_error = Some(error.to_string());

        let max_attempts = task.definition.options.retry_policy.max_attempts;
        if task.attempt < max_attempts {
            // Requeue for retry
            task.status = TaskStatus::Pending;
            task.claimed_by = None;

            let delay = task
                .definition
                .options
                .retry_policy
                .delay_for_attempt(task.attempt + 1);

            Ok(TaskFailureOutcome::WillRetry {
                next_attempt: task.attempt + 1,
                delay,
            })
        } else {
            // Move to DLQ
            task.status = TaskStatus::Dead;
            Ok(TaskFailureOutcome::MovedToDlq)
        }
    }

    async fn try_claim_workflow_for_new_turn(&self, workflow_id: Uuid) -> Result<bool, StoreError> {
        let mut workflows = self.workflows.write();
        let workflow = workflows
            .get_mut(&workflow_id)
            .ok_or(StoreError::WorkflowNotFound(workflow_id))?;

        if workflow.status == WorkflowStatus::Running {
            return Ok(false);
        }

        // Claim: set to Running, clear transient fields
        workflow.status = WorkflowStatus::Running;
        workflow.result = None;
        workflow.error = None;
        workflow.started_at = Some(chrono::Utc::now());
        workflow.completed_at = None;
        drop(workflows);

        // Cancel stale pending tasks
        let mut tasks = self.tasks.write();
        for task in tasks.values_mut() {
            if task.definition.workflow_id == Some(workflow_id)
                && task.status == TaskStatus::Pending
            {
                task.status = TaskStatus::Cancelled;
            }
        }

        Ok(true)
    }

    async fn cancel_pending_tasks_for_workflow(
        &self,
        workflow_id: Uuid,
    ) -> Result<u64, StoreError> {
        let mut tasks = self.tasks.write();
        let mut count = 0u64;
        for task in tasks.values_mut() {
            if task.definition.workflow_id == Some(workflow_id)
                && task.status == TaskStatus::Pending
            {
                task.status = TaskStatus::Cancelled;
                count += 1;
            }
        }
        Ok(count)
    }

    async fn get_task(&self, task_id: Uuid) -> Result<TaskInfo, StoreError> {
        let tasks = self.tasks.read();
        let task = tasks
            .get(&task_id)
            .ok_or(StoreError::TaskNotFound(task_id))?;

        Ok(TaskInfo {
            id: task_id,
            workflow_id: task.definition.workflow_id,
            activity_id: task.definition.activity_id.clone(),
            activity_type: task.definition.activity_type.clone(),
            status: task.status,
            priority: task.definition.options.priority,
            attempt: task.attempt,
            max_attempts: task.definition.options.retry_policy.max_attempts,
            claimed_by: task.claimed_by.clone(),
            last_error: task.last_error.clone(),
            created_at: task.created_at,
            claimed_at: task.claimed_at,
        })
    }

    async fn reclaim_stale_tasks(
        &self,
        _stale_threshold: Duration,
    ) -> Result<ReclaimResult, StoreError> {
        // In-memory implementation doesn't track timestamps
        Ok(ReclaimResult::default())
    }

    async fn send_signal(
        &self,
        workflow_id: Uuid,
        signal: WorkflowSignal,
    ) -> Result<(), StoreError> {
        let mut workflows = self.workflows.write();
        let workflow = workflows
            .get_mut(&workflow_id)
            .ok_or(StoreError::WorkflowNotFound(workflow_id))?;

        workflow.signals.push(signal);
        Ok(())
    }

    async fn get_pending_signals(
        &self,
        workflow_id: Uuid,
    ) -> Result<Vec<WorkflowSignal>, StoreError> {
        let workflows = self.workflows.read();
        let workflow = workflows
            .get(&workflow_id)
            .ok_or(StoreError::WorkflowNotFound(workflow_id))?;

        Ok(workflow.signals.clone())
    }

    async fn mark_signals_processed(
        &self,
        workflow_id: Uuid,
        count: usize,
    ) -> Result<(), StoreError> {
        let mut workflows = self.workflows.write();
        let workflow = workflows
            .get_mut(&workflow_id)
            .ok_or(StoreError::WorkflowNotFound(workflow_id))?;

        workflow.signals = workflow
            .signals
            .split_off(count.min(workflow.signals.len()));
        Ok(())
    }

    async fn consume_pending_signals(
        &self,
        workflow_id: Uuid,
    ) -> Result<Vec<WorkflowSignal>, StoreError> {
        let mut workflows = self.workflows.write();
        let workflow = workflows
            .get_mut(&workflow_id)
            .ok_or(StoreError::WorkflowNotFound(workflow_id))?;

        let signals = std::mem::take(&mut workflow.signals);
        Ok(signals)
    }

    async fn move_to_dlq(
        &self,
        task_id: Uuid,
        error_history: Vec<String>,
    ) -> Result<(), StoreError> {
        let tasks = self.tasks.read();
        let task = tasks
            .get(&task_id)
            .ok_or(StoreError::TaskNotFound(task_id))?;

        let entry = DlqEntry {
            id: Uuid::now_v7(),
            original_task_id: task_id,
            workflow_id: task.definition.workflow_id,
            activity_id: task.definition.activity_id.clone(),
            activity_type: task.definition.activity_type.clone(),
            input: task.definition.input.clone(),
            attempts: task.attempt,
            last_error: task.last_error.clone().unwrap_or_default(),
            error_history,
            dead_at: Utc::now(),
        };

        drop(tasks);
        self.dlq.write().insert(entry.id, entry);
        Ok(())
    }

    async fn requeue_from_dlq(&self, dlq_id: Uuid) -> Result<Uuid, StoreError> {
        let mut dlq = self.dlq.write();
        let entry = dlq
            .remove(&dlq_id)
            .ok_or(StoreError::TaskNotFound(dlq_id))?;

        drop(dlq);

        // Create new task from DLQ entry
        let task_id = Uuid::now_v7();
        let mut tasks = self.tasks.write();

        // We need to recreate options - use defaults for simplicity in test
        let options = crate::workflow::ActivityOptions::default();

        tasks.insert(
            task_id,
            TaskState {
                definition: TaskDefinition {
                    workflow_id: entry.workflow_id,
                    activity_id: entry.activity_id,
                    activity_type: entry.activity_type,
                    input: entry.input,
                    options,
                },
                status: TaskStatus::Pending,
                attempt: 0,
                claimed_by: None,
                last_error: None,
                error_history: vec![],
                created_at: Utc::now(),
                claimed_at: None,
            },
        );

        Ok(task_id)
    }

    async fn list_dlq(
        &self,
        filter: DlqFilter,
        pagination: Pagination,
    ) -> Result<Vec<DlqEntry>, StoreError> {
        let dlq = self.dlq.read();
        let mut entries: Vec<_> = dlq
            .values()
            .filter(|e| {
                if let Some(wid) = filter.workflow_id
                    && e.workflow_id != Some(wid)
                {
                    return false;
                }
                if let Some(ref at) = filter.activity_type
                    && &e.activity_type != at
                {
                    return false;
                }
                true
            })
            .cloned()
            .collect();

        entries.sort_by(|a, b| b.dead_at.cmp(&a.dead_at));

        let start = pagination.offset as usize;
        let end = (pagination.offset + pagination.limit) as usize;

        Ok(entries.into_iter().skip(start).take(end - start).collect())
    }

    async fn create_circuit_breaker(
        &self,
        key: &str,
        _config: &crate::reliability::CircuitBreakerConfig,
    ) -> Result<(), StoreError> {
        let mut breakers = self.circuit_breakers.write();
        breakers.insert(
            key.to_string(),
            CircuitBreakerMemState {
                state: crate::reliability::CircuitState::Closed,
                failure_count: 0,
                success_count: 0,
                opened_at: None,
            },
        );
        Ok(())
    }

    async fn get_circuit_breaker(
        &self,
        key: &str,
    ) -> Result<Option<CircuitBreakerState>, StoreError> {
        let breakers = self.circuit_breakers.read();
        Ok(breakers.get(key).map(|b| CircuitBreakerState {
            key: key.to_string(),
            state: b.state,
            failure_count: b.failure_count,
            success_count: b.success_count,
            last_failure_at: None,
            opened_at: b.opened_at,
            half_open_at: None,
            updated_at: Utc::now(),
        }))
    }

    async fn update_circuit_breaker(
        &self,
        key: &str,
        state: crate::reliability::CircuitState,
        failure_count: u32,
        success_count: u32,
    ) -> Result<(), StoreError> {
        let mut breakers = self.circuit_breakers.write();
        let breaker = breakers.get_mut(key);

        match breaker {
            Some(b) => {
                let opened_at = if state == crate::reliability::CircuitState::Open
                    && b.state != crate::reliability::CircuitState::Open
                {
                    Some(Utc::now())
                } else if state == crate::reliability::CircuitState::Closed {
                    None
                } else {
                    b.opened_at
                };

                b.state = state;
                b.failure_count = failure_count;
                b.success_count = success_count;
                b.opened_at = opened_at;
            }
            None => {
                // Create if doesn't exist
                breakers.insert(
                    key.to_string(),
                    CircuitBreakerMemState {
                        state,
                        failure_count,
                        success_count,
                        opened_at: if state == crate::reliability::CircuitState::Open {
                            Some(Utc::now())
                        } else {
                            None
                        },
                    },
                );
            }
        }
        Ok(())
    }

    async fn list_circuit_breakers(&self) -> Result<Vec<CircuitBreakerState>, StoreError> {
        let breakers = self.circuit_breakers.read();
        Ok(breakers
            .iter()
            .map(|(k, b)| CircuitBreakerState {
                key: k.clone(),
                state: b.state,
                failure_count: b.failure_count,
                success_count: b.success_count,
                last_failure_at: None,
                opened_at: b.opened_at,
                half_open_at: None,
                updated_at: Utc::now(),
            })
            .collect())
    }

    async fn force_open_circuit_breaker(&self, key: &str) -> Result<(), StoreError> {
        let mut breakers = self.circuit_breakers.write();
        breakers.insert(
            key.to_string(),
            CircuitBreakerMemState {
                state: crate::reliability::CircuitState::Open,
                failure_count: 0,
                success_count: 0,
                opened_at: Some(Utc::now()),
            },
        );
        Ok(())
    }

    async fn force_close_circuit_breaker(&self, key: &str) -> Result<(), StoreError> {
        let mut breakers = self.circuit_breakers.write();
        if let Some(b) = breakers.get_mut(key) {
            b.state = crate::reliability::CircuitState::Closed;
            b.failure_count = 0;
            b.success_count = 0;
            b.opened_at = None;
            Ok(())
        } else {
            Err(StoreError::CircuitBreakerNotFound(key.to_string()))
        }
    }

    async fn delete_circuit_breaker(&self, key: &str) -> Result<(), StoreError> {
        let mut breakers = self.circuit_breakers.write();
        if breakers.remove(key).is_some() {
            Ok(())
        } else {
            Err(StoreError::CircuitBreakerNotFound(key.to_string()))
        }
    }

    // Worker management methods
    async fn register_worker(&self, worker: WorkerInfo) -> Result<(), StoreError> {
        let mut workers = self.workers.write();
        workers.insert(worker.id.clone(), worker);
        Ok(())
    }

    async fn worker_heartbeat(
        &self,
        worker_id: &str,
        current_load: usize,
        accepting_tasks: bool,
    ) -> Result<(), StoreError> {
        let mut workers = self.workers.write();
        if let Some(worker) = workers.get_mut(worker_id) {
            worker.current_load = current_load as u32;
            worker.accepting_tasks = accepting_tasks;
            worker.last_heartbeat_at = Utc::now();
        }
        Ok(())
    }

    async fn deregister_worker(&self, worker_id: &str) -> Result<usize, StoreError> {
        let mut workers = self.workers.write();
        workers.remove(worker_id);
        // In DEV_MODE, we don't reclaim tasks since there's only one worker
        Ok(0)
    }

    async fn get_capacity_snapshot(&self) -> Result<CapacitySnapshot, StoreError> {
        let workers = self.workers.read();
        let heartbeat_threshold =
            Utc::now() - chrono::Duration::seconds(WORKER_HEARTBEAT_TIMEOUT_SECS);

        let mut total_available: u32 = 0;
        let mut active_workers: u32 = 0;
        for w in workers.values() {
            if w.status == "active"
                && w.accepting_tasks
                && w.last_heartbeat_at > heartbeat_threshold
            {
                total_available += w.max_concurrency.saturating_sub(w.current_load);
                active_workers += 1;
            }
        }
        Ok(CapacitySnapshot {
            total_available,
            active_workers,
        })
    }

    async fn list_workers(&self, filter: WorkerFilter) -> Result<Vec<WorkerInfo>, StoreError> {
        let workers = self.workers.read();
        let mut result: Vec<_> = workers
            .values()
            .filter(|w| {
                // Apply status filter
                if let Some(ref status) = filter.status
                    && &w.status != status
                {
                    return false;
                }
                // Apply worker_group filter
                if let Some(ref group) = filter.worker_group
                    && w.worker_group.as_ref() != Some(group)
                {
                    return false;
                }
                true
            })
            .cloned()
            .collect();
        result.sort_by(|a, b| b.started_at.cmp(&a.started_at));
        Ok(result)
    }

    async fn get_system_health(&self) -> Result<SystemHealth, StoreError> {
        let workers = self.workers.read();
        let heartbeat_threshold =
            Utc::now() - chrono::Duration::seconds(WORKER_HEARTBEAT_TIMEOUT_SECS);

        let total_workers = workers.len();
        let active_workers = workers
            .values()
            .filter(|w| w.status == "active" && w.last_heartbeat_at > heartbeat_threshold)
            .count();
        let workers_accepting = workers
            .values()
            .filter(|w| {
                w.status == "active"
                    && w.accepting_tasks
                    && w.last_heartbeat_at > heartbeat_threshold
            })
            .count();
        let total_capacity: usize = workers
            .values()
            .filter(|w| w.status == "active" && w.last_heartbeat_at > heartbeat_threshold)
            .map(|w| w.max_concurrency as usize)
            .sum();
        let current_load: usize = workers
            .values()
            .filter(|w| w.status == "active" && w.last_heartbeat_at > heartbeat_threshold)
            .map(|w| w.current_load as usize)
            .sum();
        drop(workers);

        let tasks = self.tasks.read();
        let pending_tasks = tasks
            .values()
            .filter(|t| t.status == TaskStatus::Pending)
            .count();
        let claimed_tasks = tasks
            .values()
            .filter(|t| t.status == TaskStatus::Claimed)
            .count();
        let completed_tasks = tasks
            .values()
            .filter(|t| t.status == TaskStatus::Completed)
            .count();
        let failed_tasks = tasks
            .values()
            .filter(|t| matches!(t.status, TaskStatus::Failed | TaskStatus::Dead))
            .count();
        let started_tasks = tasks.values().filter(|t| t.claimed_at.is_some()).count();
        drop(tasks);

        let workflows = self.workflows.read();
        let running_workflows = workflows
            .values()
            .filter(|w| w.status == WorkflowStatus::Running)
            .count();
        let pending_workflows = workflows
            .values()
            .filter(|w| w.status == WorkflowStatus::Pending)
            .count();
        let completed_workflows = workflows
            .values()
            .filter(|w| w.status == WorkflowStatus::Completed)
            .count();
        let failed_workflows = workflows
            .values()
            .filter(|w| matches!(w.status, WorkflowStatus::Failed | WorkflowStatus::Cancelled))
            .count();
        let started_workflows = workflows
            .values()
            .filter(|w| w.started_at.is_some())
            .count();
        drop(workflows);

        let dlq_size = self.dlq.read().len();

        Ok(SystemHealth {
            total_workers,
            active_workers,
            workers_accepting,
            total_capacity,
            current_load,
            pending_tasks,
            claimed_tasks,
            completed_tasks,
            failed_tasks,
            started_tasks,
            running_workflows,
            pending_workflows,
            completed_workflows,
            failed_workflows,
            started_workflows,
            dlq_size,
        })
    }

    async fn list_workflows(
        &self,
        filter: WorkflowFilter,
        pagination: Pagination,
    ) -> Result<Vec<WorkflowInfoExtended>, StoreError> {
        let workflows = self.workflows.read();
        let mut result: Vec<_> = workflows
            .iter()
            .filter(|(_, w)| {
                // Apply status filter
                if let Some(ref status) = filter.status
                    && &w.status != status
                {
                    return false;
                }
                // Apply workflow_type filter
                if let Some(ref wf_type) = filter.workflow_type
                    && &w.workflow_type != wf_type
                {
                    return false;
                }
                true
            })
            .map(|(id, w)| WorkflowInfoExtended {
                id: *id,
                workflow_type: w.workflow_type.clone(),
                status: w.status,
                input: w.input.clone(),
                result: w.result.clone(),
                error: w.error.clone(),
                created_at: w.created_at,
                started_at: w.started_at,
                completed_at: w.completed_at,
                continued_as_new_id: w.continued_as_new_id,
            })
            .collect();
        result.sort_by(|a, b| b.created_at.cmp(&a.created_at));

        let start = pagination.offset as usize;
        let end = (pagination.offset + pagination.limit) as usize;
        Ok(result.into_iter().skip(start).take(end - start).collect())
    }

    async fn list_tasks(
        &self,
        filter: TaskFilter,
        pagination: Pagination,
    ) -> Result<Vec<TaskInfo>, StoreError> {
        let tasks = self.tasks.read();
        let mut result: Vec<_> = tasks
            .iter()
            .filter(|(_, t)| {
                // Apply status filter
                if let Some(ref status) = filter.status
                    && &t.status != status
                {
                    return false;
                }
                // Apply activity_type filter
                if let Some(ref activity_type) = filter.activity_type
                    && &t.definition.activity_type != activity_type
                {
                    return false;
                }
                // Apply workflow_id filter
                if let Some(ref wf_id) = filter.workflow_id
                    && t.definition.workflow_id.as_ref() != Some(wf_id)
                {
                    return false;
                }
                // Apply standalone_only filter
                if filter.standalone_only && t.definition.workflow_id.is_some() {
                    return false;
                }
                true
            })
            .map(|(id, t)| TaskInfo {
                id: *id,
                workflow_id: t.definition.workflow_id,
                activity_id: t.definition.activity_id.clone(),
                activity_type: t.definition.activity_type.clone(),
                status: t.status,
                priority: t.definition.options.priority,
                attempt: t.attempt,
                max_attempts: t.definition.options.retry_policy.max_attempts,
                claimed_by: t.claimed_by.clone(),
                last_error: t.last_error.clone(),
                created_at: t.created_at,
                claimed_at: t.claimed_at,
            })
            .collect();
        // Sort by created_at ascending (oldest first) to show execution order
        result.sort_by(|a, b| a.created_at.cmp(&b.created_at));

        let start = pagination.offset as usize;
        let end = (pagination.offset + pagination.limit) as usize;
        Ok(result.into_iter().skip(start).take(end - start).collect())
    }

    // =========================================================================
    // Schedule Operations
    // =========================================================================

    async fn create_schedule(&self, schedule: CreateScheduleRow) -> Result<Uuid, StoreError> {
        let id = Uuid::now_v7();
        let now = Utc::now();

        let row = ScheduleRow {
            id,
            name: schedule.name,
            description: schedule.description,
            cron_expression: schedule.cron_expression,
            timezone: schedule.timezone,
            target_type: schedule.target_type,
            target_name: schedule.target_name,
            target_input: schedule.target_input,
            enabled: schedule.enabled,
            max_concurrent: schedule.max_concurrent,
            catch_up_missed: schedule.catch_up_missed,
            max_catch_up: schedule.max_catch_up,
            retry_policy: schedule.retry_policy,
            last_triggered_at: None,
            next_trigger_at: schedule.next_trigger_at,
            claimed_by: None,
            claimed_at: None,
            created_at: now,
            updated_at: now,
        };

        let mut schedules = self.schedules.write();
        schedules.insert(id, ScheduleMemState { row });
        Ok(id)
    }

    async fn get_schedule(&self, id: Uuid) -> Result<ScheduleRow, StoreError> {
        let schedules = self.schedules.read();
        schedules
            .get(&id)
            .map(|s| s.row.clone())
            .ok_or(StoreError::ScheduleNotFound(id))
    }

    async fn list_schedules(
        &self,
        filter: ScheduleFilter,
        pagination: Pagination,
    ) -> Result<Vec<ScheduleRow>, StoreError> {
        let schedules = self.schedules.read();
        let mut result: Vec<_> = schedules
            .values()
            .filter(|s| {
                if filter.enabled.is_some_and(|e| s.row.enabled != e) {
                    return false;
                }
                if filter.target_type.is_some_and(|t| s.row.target_type != t) {
                    return false;
                }
                true
            })
            .map(|s| s.row.clone())
            .collect();
        result.sort_by(|a, b| b.created_at.cmp(&a.created_at));

        let start = pagination.offset as usize;
        let end = (pagination.offset + pagination.limit) as usize;
        Ok(result.into_iter().skip(start).take(end - start).collect())
    }

    async fn count_schedules(&self, filter: ScheduleFilter) -> Result<u64, StoreError> {
        let schedules = self.schedules.read();
        let count = schedules
            .values()
            .filter(|s| {
                if filter.enabled.is_some_and(|e| s.row.enabled != e) {
                    return false;
                }
                if filter.target_type.is_some_and(|t| s.row.target_type != t) {
                    return false;
                }
                true
            })
            .count();
        Ok(count as u64)
    }

    async fn update_schedule(&self, id: Uuid, update: UpdateSchedule) -> Result<(), StoreError> {
        let mut schedules = self.schedules.write();
        let state = schedules
            .get_mut(&id)
            .ok_or(StoreError::ScheduleNotFound(id))?;

        if let Some(name) = update.name {
            state.row.name = name;
        }
        update.description.apply(&mut state.row.description);
        if let Some(cron_expression) = update.cron_expression {
            state.row.cron_expression = cron_expression;
        }
        if let Some(timezone) = update.timezone {
            state.row.timezone = timezone;
        }
        if let Some(target_type) = update.target_type {
            state.row.target_type = target_type;
        }
        if let Some(target_name) = update.target_name {
            state.row.target_name = target_name;
        }
        if let Some(target_input) = update.target_input {
            state.row.target_input = target_input;
        }
        if let Some(enabled) = update.enabled {
            state.row.enabled = enabled;
        }
        update.max_concurrent.apply(&mut state.row.max_concurrent);
        if let Some(catch_up_missed) = update.catch_up_missed {
            state.row.catch_up_missed = catch_up_missed;
        }
        update.max_catch_up.apply(&mut state.row.max_catch_up);
        update.retry_policy.apply(&mut state.row.retry_policy);
        update.next_trigger_at.apply(&mut state.row.next_trigger_at);
        state.row.updated_at = Utc::now();
        Ok(())
    }

    async fn delete_schedule(&self, id: Uuid) -> Result<(), StoreError> {
        let mut schedules = self.schedules.write();
        schedules
            .remove(&id)
            .ok_or(StoreError::ScheduleNotFound(id))?;

        // Also remove executions for this schedule
        let mut executions = self.schedule_executions.write();
        executions.retain(|_, e| e.row.schedule_id != id);
        Ok(())
    }

    // =========================================================================
    // Scheduler Operations
    // =========================================================================

    async fn claim_due_schedules(
        &self,
        scheduler_id: &str,
        limit: u32,
    ) -> Result<Vec<ScheduleRow>, StoreError> {
        let now = Utc::now();
        let mut schedules = self.schedules.write();

        // Find due schedules sorted by next_trigger_at
        let mut candidates: Vec<_> = schedules
            .iter_mut()
            .filter(|(_, s)| {
                s.row.enabled
                    && s.row.next_trigger_at.is_some_and(|t| t <= now)
                    && s.row.claimed_by.is_none()
            })
            .collect();
        candidates.sort_by(|a, b| a.1.row.next_trigger_at.cmp(&b.1.row.next_trigger_at));

        let mut claimed = Vec::new();
        for (_, state) in candidates {
            if claimed.len() >= limit as usize {
                break;
            }
            state.row.claimed_by = Some(scheduler_id.to_string());
            state.row.claimed_at = Some(now);
            claimed.push(state.row.clone());
        }

        Ok(claimed)
    }

    async fn update_next_trigger(
        &self,
        id: Uuid,
        next: chrono::DateTime<Utc>,
    ) -> Result<(), StoreError> {
        let mut schedules = self.schedules.write();
        let state = schedules
            .get_mut(&id)
            .ok_or(StoreError::ScheduleNotFound(id))?;
        state.row.next_trigger_at = Some(next);
        state.row.last_triggered_at = Some(Utc::now());
        state.row.claimed_by = None;
        state.row.claimed_at = None;
        state.row.updated_at = Utc::now();
        Ok(())
    }

    async fn skip_schedule_trigger(&self, id: Uuid) -> Result<(), StoreError> {
        // Just release the claim, keep next_trigger_at the same
        let mut schedules = self.schedules.write();
        let state = schedules
            .get_mut(&id)
            .ok_or(StoreError::ScheduleNotFound(id))?;
        state.row.claimed_by = None;
        state.row.claimed_at = None;
        Ok(())
    }

    async fn release_schedule(&self, id: Uuid) -> Result<(), StoreError> {
        self.skip_schedule_trigger(id).await
    }

    // =========================================================================
    // Schedule Execution Operations
    // =========================================================================

    async fn create_schedule_execution(
        &self,
        schedule_id: Uuid,
        scheduled_at: chrono::DateTime<Utc>,
    ) -> Result<Uuid, StoreError> {
        let id = Uuid::now_v7();
        let now = Utc::now();

        let row = ScheduleExecutionRow {
            id,
            schedule_id,
            scheduled_at,
            started_at: now,
            completed_at: None,
            status: ScheduleExecutionStatus::Running,
            workflow_id: None,
            task_id: None,
            error: None,
            duration_ms: None,
            created_at: now,
        };

        let mut executions = self.schedule_executions.write();
        executions.insert(id, ScheduleExecutionMemState { row });
        Ok(id)
    }

    async fn get_schedule_execution(&self, id: Uuid) -> Result<ScheduleExecutionRow, StoreError> {
        let executions = self.schedule_executions.read();
        executions
            .get(&id)
            .map(|e| e.row.clone())
            .ok_or(StoreError::ScheduleExecutionNotFound(id))
    }

    async fn complete_schedule_execution(
        &self,
        execution_id: Uuid,
        target_id: Uuid,
        is_workflow: bool,
    ) -> Result<(), StoreError> {
        let mut executions = self.schedule_executions.write();
        let state = executions
            .get_mut(&execution_id)
            .ok_or(StoreError::ScheduleExecutionNotFound(execution_id))?;

        let now = Utc::now();
        state.row.status = ScheduleExecutionStatus::Completed;
        state.row.completed_at = Some(now);
        state.row.duration_ms = Some((now - state.row.started_at).num_milliseconds() as i32);
        if is_workflow {
            state.row.workflow_id = Some(target_id);
        } else {
            state.row.task_id = Some(target_id);
        }
        Ok(())
    }

    async fn fail_schedule_execution(
        &self,
        execution_id: Uuid,
        error: &str,
    ) -> Result<(), StoreError> {
        let mut executions = self.schedule_executions.write();
        let state = executions
            .get_mut(&execution_id)
            .ok_or(StoreError::ScheduleExecutionNotFound(execution_id))?;

        let now = Utc::now();
        state.row.status = ScheduleExecutionStatus::Failed;
        state.row.completed_at = Some(now);
        state.row.duration_ms = Some((now - state.row.started_at).num_milliseconds() as i32);
        state.row.error = Some(error.to_string());
        Ok(())
    }

    async fn skip_schedule_execution(
        &self,
        execution_id: Uuid,
        reason: &str,
    ) -> Result<(), StoreError> {
        let mut executions = self.schedule_executions.write();
        let state = executions
            .get_mut(&execution_id)
            .ok_or(StoreError::ScheduleExecutionNotFound(execution_id))?;

        let now = Utc::now();
        state.row.status = ScheduleExecutionStatus::Skipped;
        state.row.completed_at = Some(now);
        state.row.duration_ms = Some((now - state.row.started_at).num_milliseconds() as i32);
        state.row.error = Some(reason.to_string());
        Ok(())
    }

    async fn list_schedule_executions(
        &self,
        filter: ScheduleExecutionFilter,
        pagination: Pagination,
    ) -> Result<Vec<ScheduleExecutionRow>, StoreError> {
        let executions = self.schedule_executions.read();
        let mut result: Vec<_> = executions
            .values()
            .filter(|e| {
                if filter.schedule_id.is_some_and(|id| e.row.schedule_id != id) {
                    return false;
                }
                if filter.status.is_some_and(|s| e.row.status != s) {
                    return false;
                }
                true
            })
            .map(|e| e.row.clone())
            .collect();
        result.sort_by(|a, b| b.created_at.cmp(&a.created_at));

        let start = pagination.offset as usize;
        let end = (pagination.offset + pagination.limit) as usize;
        Ok(result.into_iter().skip(start).take(end - start).collect())
    }

    async fn count_running_executions(&self, schedule_id: Uuid) -> Result<u32, StoreError> {
        let executions = self.schedule_executions.read();
        let count = executions
            .values()
            .filter(|e| {
                e.row.schedule_id == schedule_id && e.row.status == ScheduleExecutionStatus::Running
            })
            .count();
        Ok(count as u32)
    }

    async fn get_schedule_stats(&self, schedule_id: Uuid) -> Result<ScheduleStats, StoreError> {
        let executions = self.schedule_executions.read();
        let schedule_execs: Vec<_> = executions
            .values()
            .filter(|e| e.row.schedule_id == schedule_id)
            .collect();

        let total_executions = schedule_execs.len() as u64;
        let successful_executions = schedule_execs
            .iter()
            .filter(|e| e.row.status == ScheduleExecutionStatus::Completed)
            .count() as u64;
        let failed_executions = schedule_execs
            .iter()
            .filter(|e| e.row.status == ScheduleExecutionStatus::Failed)
            .count() as u64;
        let skipped_executions = schedule_execs
            .iter()
            .filter(|e| e.row.status == ScheduleExecutionStatus::Skipped)
            .count() as u64;

        let durations: Vec<_> = schedule_execs
            .iter()
            .filter_map(|e| e.row.duration_ms)
            .collect();
        let avg_duration_ms = if durations.is_empty() {
            None
        } else {
            Some(durations.iter().map(|d| *d as u64).sum::<u64>() / durations.len() as u64)
        };

        let last_execution_status = schedule_execs
            .iter()
            .max_by_key(|e| e.row.created_at)
            .map(|e| e.row.status);

        Ok(ScheduleStats {
            total_executions,
            successful_executions,
            failed_executions,
            skipped_executions,
            avg_duration_ms,
            last_execution_status,
        })
    }

    // =========================================================================
    // Rate Limit Operations
    // =========================================================================
    // Scheduler Instance Operations
    // =========================================================================

    async fn register_scheduler_instance(
        &self,
        instance: SchedulerInstanceInfo,
    ) -> Result<(), StoreError> {
        let mut instances = self.scheduler_instances.write();
        instances.insert(instance.instance_id.clone(), instance);
        Ok(())
    }

    async fn heartbeat_scheduler_instance(
        &self,
        instance_id: &str,
        schedules_processed: u64,
    ) -> Result<(), StoreError> {
        let mut instances = self.scheduler_instances.write();
        if let Some(instance) = instances.get_mut(instance_id) {
            instance.last_heartbeat_at = Utc::now();
            instance.schedules_processed = schedules_processed;
        }
        Ok(())
    }

    async fn list_scheduler_instances(&self) -> Result<Vec<SchedulerInstanceInfo>, StoreError> {
        let instances = self.scheduler_instances.read();
        Ok(instances.values().cloned().collect())
    }

    async fn deregister_scheduler_instance(&self, instance_id: &str) -> Result<(), StoreError> {
        let mut instances = self.scheduler_instances.write();
        instances.remove(instance_id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
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
                vec![WorkflowEvent::WorkflowStarted {
                    input: serde_json::json!({}),
                }],
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
                vec![WorkflowEvent::WorkflowStarted {
                    input: serde_json::json!({}),
                }],
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
            let task = tasks.get_mut(&task_id).unwrap();
            task.status = TaskStatus::Pending;
            task.claimed_by = None;
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
            let task = tasks.get_mut(&task_id).unwrap();
            task.status = TaskStatus::Pending;
            task.claimed_by = None;
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
        // Use a small standalone limit for testing
        let store = InMemoryWorkflowEventStore::new();

        // Enqueue standalone tasks up to the limit - won't actually hit 10k in test,
        // just verify the code path works for a few tasks
        for i in 0..5 {
            store
                .enqueue_task(TaskDefinition {
                    workflow_id: None,
                    activity_id: format!("standalone-{i}"),
                    activity_type: "test".to_string(),
                    input: serde_json::json!({}),
                    options: ActivityOptions::default(),
                })
                .await
                .unwrap();
        }

        // Verify all 5 enqueued successfully
        let tasks = store
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
        assert_eq!(tasks.len(), 5);
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
}

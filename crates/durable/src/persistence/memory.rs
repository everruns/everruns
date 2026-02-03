//! In-memory implementation of WorkflowEventStore for testing

use std::collections::HashMap;
use std::sync::atomic::AtomicI32;
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

/// Rate limit window in memory
struct RateLimitWindow {
    count: u32,
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
    schedules: RwLock<HashMap<Uuid, ScheduleMemState>>,
    schedule_executions: RwLock<HashMap<Uuid, ScheduleExecutionMemState>>,
    rate_limits: RwLock<HashMap<(i64, chrono::DateTime<chrono::Utc>), RateLimitWindow>>,
    scheduler_instances: RwLock<HashMap<String, SchedulerInstanceInfo>>,
    #[allow(dead_code)] // Reserved for future global sequence counter
    sequence_counter: AtomicI32,
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
            schedules: RwLock::new(HashMap::new()),
            schedule_executions: RwLock::new(HashMap::new()),
            rate_limits: RwLock::new(HashMap::new()),
            scheduler_instances: RwLock::new(HashMap::new()),
            sequence_counter: AtomicI32::new(0),
        }
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
        workflow.result = result;
        workflow.error = error;
        Ok(())
    }

    async fn enqueue_task(&self, task: TaskDefinition) -> Result<Uuid, StoreError> {
        let task_id = Uuid::now_v7();
        let mut tasks = self.tasks.write();
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
    ) -> Result<Vec<Uuid>, StoreError> {
        // In-memory implementation doesn't track timestamps
        Ok(vec![])
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
                    && e.workflow_id != wid
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
                    && &t.definition.workflow_id != wf_id
                {
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
            org_id: schedule.org_id,
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

    async fn get_schedule_for_org(&self, org_id: i64, id: Uuid) -> Result<ScheduleRow, StoreError> {
        let schedules = self.schedules.read();
        schedules
            .get(&id)
            .filter(|s| s.row.org_id == org_id)
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
                if filter.org_id.is_some_and(|id| s.row.org_id != id) {
                    return false;
                }
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
                if filter.org_id.is_some_and(|id| s.row.org_id != id) {
                    return false;
                }
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
        if let Some(description) = update.description {
            state.row.description = description;
        }
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
        if let Some(max_concurrent) = update.max_concurrent {
            state.row.max_concurrent = max_concurrent;
        }
        if let Some(catch_up_missed) = update.catch_up_missed {
            state.row.catch_up_missed = catch_up_missed;
        }
        if let Some(max_catch_up) = update.max_catch_up {
            state.row.max_catch_up = max_catch_up;
        }
        if let Some(retry_policy) = update.retry_policy {
            state.row.retry_policy = retry_policy;
        }
        if let Some(next_trigger_at) = update.next_trigger_at {
            state.row.next_trigger_at = next_trigger_at;
        }
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

        // Find due schedules, one per org (round-robin fairness)
        let mut seen_orgs = std::collections::HashSet::new();
        let mut claimed = Vec::new();

        // Sort by org_id, then by next_trigger_at to get earliest per org
        let mut candidates: Vec<_> = schedules
            .iter_mut()
            .filter(|(_, s)| {
                s.row.enabled
                    && s.row.next_trigger_at.is_some_and(|t| t <= now)
                    && s.row.claimed_by.is_none()
            })
            .collect();
        candidates.sort_by(|a, b| {
            let org_cmp = a.1.row.org_id.cmp(&b.1.row.org_id);
            if org_cmp != std::cmp::Ordering::Equal {
                return org_cmp;
            }
            a.1.row.next_trigger_at.cmp(&b.1.row.next_trigger_at)
        });

        for (_, state) in candidates {
            if claimed.len() >= limit as usize {
                break;
            }
            if seen_orgs.contains(&state.row.org_id) {
                continue;
            }
            seen_orgs.insert(state.row.org_id);
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
        org_id: i64,
        schedule_id: Uuid,
        scheduled_at: chrono::DateTime<Utc>,
    ) -> Result<Uuid, StoreError> {
        let id = Uuid::now_v7();
        let now = Utc::now();

        let row = ScheduleExecutionRow {
            id,
            org_id,
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
                if filter.org_id.is_some_and(|id| e.row.org_id != id) {
                    return false;
                }
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

    async fn check_and_increment_rate_limit(
        &self,
        org_id: i64,
        limit: u32,
    ) -> Result<(u32, u32), StoreError> {
        use chrono::Timelike;
        let now = Utc::now();
        let window_start = now
            .with_minute(0)
            .and_then(|t| t.with_second(0))
            .and_then(|t| t.with_nanosecond(0))
            .unwrap_or(now);

        let mut rate_limits = self.rate_limits.write();
        let key = (org_id, window_start);
        let window = rate_limits
            .entry(key)
            .or_insert(RateLimitWindow { count: 0 });
        window.count += 1;
        Ok((window.count, limit))
    }

    async fn get_rate_limit_count(&self, org_id: i64) -> Result<u32, StoreError> {
        use chrono::Timelike;
        let now = Utc::now();
        let window_start = now
            .with_minute(0)
            .and_then(|t| t.with_second(0))
            .and_then(|t| t.with_nanosecond(0))
            .unwrap_or(now);

        let rate_limits = self.rate_limits.read();
        let key = (org_id, window_start);
        Ok(rate_limits.get(&key).map(|w| w.count).unwrap_or(0))
    }

    async fn cleanup_rate_limit_windows(
        &self,
        older_than: chrono::DateTime<Utc>,
    ) -> Result<u64, StoreError> {
        let mut rate_limits = self.rate_limits.write();
        let before_count = rate_limits.len();
        rate_limits.retain(|(_, window_start), _| *window_start >= older_than);
        let removed = before_count - rate_limits.len();
        Ok(removed as u64)
    }

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
                workflow_id,
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
                workflow_id,
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
                workflow_id,
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
                workflow_id,
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
                workflow_id,
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
                workflow_id,
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
            org_id: 1,
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
        assert_eq!(retrieved.org_id, 1);
        assert_eq!(retrieved.cron_expression, "*/5 * * * *");
        assert_eq!(retrieved.target_type, ScheduleTargetType::Workflow);
        assert!(retrieved.enabled);
    }

    #[tokio::test]
    async fn test_schedule_org_isolation() {
        let store = InMemoryWorkflowEventStore::new();

        let schedule = CreateScheduleRow {
            org_id: 1,
            name: "org1-schedule".to_string(),
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

        let id = store.create_schedule(schedule).await.unwrap();

        // Can get with correct org
        let result = store.get_schedule_for_org(1, id).await;
        assert!(result.is_ok());

        // Cannot get with wrong org
        let result = store.get_schedule_for_org(2, id).await;
        assert!(matches!(result, Err(StoreError::ScheduleNotFound(_))));
    }

    #[tokio::test]
    async fn test_schedule_claim_round_robin() {
        let store = InMemoryWorkflowEventStore::new();
        let now = Utc::now();

        // Create schedules for org 1 (3 schedules)
        for i in 0..3 {
            let schedule = CreateScheduleRow {
                org_id: 1,
                name: format!("org1-schedule-{}", i),
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

        // Create schedule for org 2 (1 schedule)
        let schedule = CreateScheduleRow {
            org_id: 2,
            name: "org2-schedule".to_string(),
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

        // Claim with limit 10 - should get one per org (round-robin)
        let claimed = store.claim_due_schedules("scheduler-1", 10).await.unwrap();

        // Should get exactly 2 (one from org 1, one from org 2)
        assert_eq!(claimed.len(), 2);

        // Verify we got one from each org
        let org_ids: std::collections::HashSet<_> = claimed.iter().map(|s| s.org_id).collect();
        assert!(org_ids.contains(&1));
        assert!(org_ids.contains(&2));
    }

    #[tokio::test]
    async fn test_schedule_execution_lifecycle() {
        let store = InMemoryWorkflowEventStore::new();
        let now = Utc::now();

        // Create schedule
        let schedule = CreateScheduleRow {
            org_id: 1,
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
            .create_schedule_execution(1, schedule_id, now)
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
            org_id: 1,
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
                .create_schedule_execution(1, schedule_id, now)
                .await
                .unwrap();
            store
                .complete_schedule_execution(exec_id, Uuid::now_v7(), true)
                .await
                .unwrap();
        }

        // Create a failed execution
        let exec_id = store
            .create_schedule_execution(1, schedule_id, now)
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
    async fn test_rate_limit() {
        let store = InMemoryWorkflowEventStore::new();

        // Increment rate limit multiple times
        for i in 1..=5 {
            let (count, limit) = store.check_and_increment_rate_limit(1, 1000).await.unwrap();
            assert_eq!(count, i);
            assert_eq!(limit, 1000);
        }

        // Check current count
        let count = store.get_rate_limit_count(1).await.unwrap();
        assert_eq!(count, 5);

        // Different org should have 0
        let count = store.get_rate_limit_count(2).await.unwrap();
        assert_eq!(count, 0);
    }
}

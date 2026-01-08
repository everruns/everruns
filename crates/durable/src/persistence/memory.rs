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
}

/// Internal task state
struct TaskState {
    definition: TaskDefinition,
    status: TaskStatus,
    attempt: u32,
    claimed_by: Option<String>,
    last_error: Option<String>,
    error_history: Vec<String>,
}

/// Circuit breaker state in memory
struct CircuitBreakerMemState {
    state: crate::reliability::CircuitState,
    failure_count: u32,
    success_count: u32,
    opened_at: Option<chrono::DateTime<chrono::Utc>>,
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

            if task.status == TaskStatus::Pending
                && activity_types.contains(&task.definition.activity_type)
            {
                task.status = TaskStatus::Claimed;
                task.claimed_by = Some(worker_id.to_string());
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
        _result: serde_json::Value,
    ) -> Result<(), StoreError> {
        let mut tasks = self.tasks.write();
        let task = tasks
            .get_mut(&task_id)
            .ok_or(StoreError::TaskNotFound(task_id))?;

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
                if let Some(wid) = filter.workflow_id {
                    if e.workflow_id != wid {
                        return false;
                    }
                }
                if let Some(ref at) = filter.activity_type {
                    if &e.activity_type != at {
                        return false;
                    }
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

        // Complete task
        store
            .complete_task(task_id, serde_json::json!({"result": "ok"}))
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
}

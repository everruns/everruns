//! Workflow executor with replay support
//!
//! The `WorkflowExecutor` is responsible for:
//! - Starting new workflows
//! - Replaying workflows from event history
//! - Processing workflow actions (scheduling activities, timers, etc.)
//! - Handling signals

use std::sync::Arc;

use tracing::{debug, error, info, instrument, warn};
use uuid::Uuid;

use crate::activity::ActivityError;
use crate::persistence::{
    StoreError, TaskDefinition, TraceContext, WorkflowEventStore, WorkflowStatus,
};
use crate::workflow::{WorkflowAction, WorkflowEvent, WorkflowSignal};

use super::registry::{AnyWorkflow, RegistryError, WorkflowRegistry};

/// Configuration for the workflow executor
#[derive(Debug, Clone)]
pub struct ExecutorConfig {
    /// Maximum events per workflow (for safety)
    pub max_events_per_workflow: usize,

    /// Whether to validate actions before persisting
    pub validate_actions: bool,
}

impl Default for ExecutorConfig {
    fn default() -> Self {
        Self {
            max_events_per_workflow: 10000,
            validate_actions: true,
        }
    }
}

/// Errors from executor operations
#[derive(Debug, thiserror::Error)]
pub enum ExecutorError {
    /// Store error
    #[error("store error: {0}")]
    Store(#[from] StoreError),

    /// Registry error
    #[error("registry error: {0}")]
    Registry(#[from] RegistryError),

    /// Workflow already completed
    #[error("workflow {0} already completed")]
    WorkflowCompleted(Uuid),

    /// Workflow not found
    #[error("workflow not found: {0}")]
    WorkflowNotFound(Uuid),

    /// Replay error (non-determinism detected)
    #[error("replay error: {0}")]
    ReplayError(String),

    /// Too many events
    #[error("workflow {0} has too many events ({1} > {2})")]
    TooManyEvents(Uuid, usize, usize),

    /// Invalid action
    #[error("invalid action: {0}")]
    InvalidAction(String),

    /// Serialization error
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}

/// Result of processing a workflow
#[derive(Debug)]
pub struct ProcessResult {
    /// Whether the workflow completed
    pub completed: bool,

    /// Number of new events written
    pub events_written: usize,

    /// Number of tasks enqueued
    pub tasks_enqueued: usize,

    /// Number of signals processed
    pub signals_processed: usize,
}

/// Workflow executor
///
/// The executor drives workflow state machines by replaying events and
/// processing actions. It uses optimistic concurrency control to handle
/// concurrent updates.
///
/// # Example
///
/// ```ignore
/// use everruns_durable::prelude::*;
///
/// let store = InMemoryWorkflowEventStore::new();
/// let mut executor = WorkflowExecutor::new(store);
/// executor.register::<MyWorkflow>();
///
/// // Start a new workflow
/// let workflow_id = executor.start_workflow::<MyWorkflow>(input).await?;
///
/// // Process the workflow (after activities complete)
/// executor.process_workflow(workflow_id).await?;
/// ```
pub struct WorkflowExecutor<S: WorkflowEventStore> {
    store: Arc<S>,
    registry: WorkflowRegistry,
    config: ExecutorConfig,
}

impl<S: WorkflowEventStore> WorkflowExecutor<S> {
    /// Create a new executor with the given store
    pub fn new(store: S) -> Self {
        Self {
            store: Arc::new(store),
            registry: WorkflowRegistry::new(),
            config: ExecutorConfig::default(),
        }
    }

    /// Create a new executor with custom config
    pub fn with_config(store: S, config: ExecutorConfig) -> Self {
        Self {
            store: Arc::new(store),
            registry: WorkflowRegistry::new(),
            config,
        }
    }

    /// Register a workflow type
    pub fn register<W: crate::workflow::Workflow>(&mut self) {
        self.registry.register::<W>();
        info!(workflow_type = W::TYPE, "registered workflow type");
    }

    /// Get a reference to the store
    pub fn store(&self) -> &S {
        &self.store
    }

    /// Start a new workflow
    ///
    /// Creates the workflow instance, persists the start event, and
    /// processes initial actions.
    #[instrument(skip(self, input, trace_context), fields(workflow_type = W::TYPE))]
    pub async fn start_workflow<W: crate::workflow::Workflow>(
        &self,
        input: W::Input,
        trace_context: Option<TraceContext>,
    ) -> Result<Uuid, ExecutorError> {
        let workflow_id = Uuid::now_v7();
        let input_json = serde_json::to_value(&input)?;

        info!(%workflow_id, "starting new workflow");

        // Create workflow in store
        self.store
            .create_workflow(
                workflow_id,
                W::TYPE,
                input_json.clone(),
                trace_context.as_ref(),
            )
            .await?;

        // Append WorkflowStarted event
        let start_event = WorkflowEvent::WorkflowStarted {
            input: input_json.clone(),
        };

        self.store
            .append_events(workflow_id, 0, vec![start_event])
            .await?;

        // Create workflow instance and process on_start
        let mut workflow = W::new(input);
        let actions = workflow.on_start();

        // Check if workflow completes immediately
        let completes_immediately = actions.iter().any(|a| {
            matches!(
                a,
                WorkflowAction::CompleteWorkflow { .. } | WorkflowAction::FailWorkflow { .. }
            )
        });

        // Process initial actions
        self.process_actions(workflow_id, 1, actions).await?;

        // Only update status to Running if workflow didn't complete immediately
        if !completes_immediately {
            self.store
                .update_workflow_status(workflow_id, WorkflowStatus::Running, None, None)
                .await?;
        }

        Ok(workflow_id)
    }

    /// Process a workflow after external events (activity completions, signals, etc.)
    ///
    /// This replays the workflow from its event history and processes any
    /// new actions that result from recent events.
    #[instrument(skip(self))]
    pub async fn process_workflow(
        &self,
        workflow_id: Uuid,
    ) -> Result<ProcessResult, ExecutorError> {
        // Get workflow info including type and status
        let workflow_info = self.store.get_workflow_info(workflow_id).await?;

        if matches!(
            workflow_info.status,
            WorkflowStatus::Completed | WorkflowStatus::Failed | WorkflowStatus::Cancelled
        ) {
            debug!(%workflow_id, status = ?workflow_info.status, "workflow already in terminal state");
            return Ok(ProcessResult {
                completed: true,
                events_written: 0,
                tasks_enqueued: 0,
                signals_processed: 0,
            });
        }

        // Load all events
        let events = self.store.load_events(workflow_id).await?;

        if events.is_empty() {
            return Err(ExecutorError::WorkflowNotFound(workflow_id));
        }

        // Check event limit
        if events.len() > self.config.max_events_per_workflow {
            return Err(ExecutorError::TooManyEvents(
                workflow_id,
                events.len(),
                self.config.max_events_per_workflow,
            ));
        }

        // Verify first event is WorkflowStarted
        if !matches!(&events[0].1, WorkflowEvent::WorkflowStarted { .. }) {
            return Err(ExecutorError::ReplayError(
                "first event must be WorkflowStarted".to_string(),
            ));
        }

        // Create workflow instance using stored type and input
        let mut workflow = self
            .registry
            .create(&workflow_info.workflow_type, workflow_info.input.clone())?;

        // Track the current sequence (length = next expected sequence for appending)
        let mut current_sequence = events.len() as i32;
        let mut events_written = 0;
        let mut tasks_enqueued = 0;

        // Replay all events to rebuild state
        for (_seq, event) in &events {
            self.replay_event(&mut *workflow, event)?;
        }

        debug!(%workflow_id, current_sequence, "replayed events");

        // Check for pending signals
        let signals = self.store.get_pending_signals(workflow_id).await?;
        let signals_processed = signals.len();

        for signal in &signals {
            let actions = workflow.on_signal(signal);
            let signal_event = WorkflowEvent::SignalReceived {
                signal: signal.clone(),
            };

            // Append signal event
            current_sequence = self
                .store
                .append_events(workflow_id, current_sequence, vec![signal_event])
                .await?;
            events_written += 1;

            // Process resulting actions
            let (new_seq, written, enqueued) = self
                .process_actions_internal(workflow_id, current_sequence, actions)
                .await?;
            current_sequence = new_seq;
            events_written += written;
            tasks_enqueued += enqueued;
        }

        // Mark signals as processed
        if signals_processed > 0 {
            self.store
                .mark_signals_processed(workflow_id, signals_processed)
                .await?;
        }

        // Check if workflow is now complete
        let completed = workflow.is_completed();
        if completed {
            if let Some(result) = workflow.result_json() {
                self.store
                    .update_workflow_status(
                        workflow_id,
                        WorkflowStatus::Completed,
                        Some(result),
                        None,
                    )
                    .await?;
            } else if let Some(error) = workflow.error() {
                self.store
                    .update_workflow_status(workflow_id, WorkflowStatus::Failed, None, Some(error))
                    .await?;
            }
        }

        Ok(ProcessResult {
            completed,
            events_written,
            tasks_enqueued,
            signals_processed,
        })
    }

    /// Send a signal to a workflow
    #[instrument(skip(self, signal))]
    pub async fn send_signal(
        &self,
        workflow_id: Uuid,
        signal: WorkflowSignal,
    ) -> Result<(), ExecutorError> {
        // Verify workflow exists
        let status = self.store.get_workflow_status(workflow_id).await?;

        if matches!(
            status,
            WorkflowStatus::Completed | WorkflowStatus::Failed | WorkflowStatus::Cancelled
        ) {
            warn!(%workflow_id, ?status, "cannot send signal to completed workflow");
            return Err(ExecutorError::WorkflowCompleted(workflow_id));
        }

        self.store.send_signal(workflow_id, signal).await?;
        info!(%workflow_id, "signal sent");

        Ok(())
    }

    /// Handle activity completion
    ///
    /// Called by the worker pool when an activity completes successfully.
    #[instrument(skip(self, result))]
    pub async fn on_activity_completed(
        &self,
        workflow_id: Uuid,
        activity_id: &str,
        result: serde_json::Value,
    ) -> Result<ProcessResult, ExecutorError> {
        // Load events to get current sequence (length = next expected sequence)
        let events = self.store.load_events(workflow_id).await?;
        let current_sequence = events.len() as i32;

        // Append completion event
        let completion_event = WorkflowEvent::ActivityCompleted {
            activity_id: activity_id.to_string(),
            result,
        };

        self.store
            .append_events(workflow_id, current_sequence, vec![completion_event])
            .await?;

        // Process the workflow to handle the completion
        self.process_workflow(workflow_id).await
    }

    /// Handle activity failure
    ///
    /// Called by the worker pool when an activity fails.
    #[instrument(skip(self, error))]
    pub async fn on_activity_failed(
        &self,
        workflow_id: Uuid,
        activity_id: &str,
        error: ActivityError,
        will_retry: bool,
    ) -> Result<ProcessResult, ExecutorError> {
        // Load events to get current sequence (length = next expected sequence)
        let events = self.store.load_events(workflow_id).await?;
        let current_sequence = events.len() as i32;

        // Append failure event
        let failure_event = WorkflowEvent::ActivityFailed {
            activity_id: activity_id.to_string(),
            error,
            will_retry,
        };

        self.store
            .append_events(workflow_id, current_sequence, vec![failure_event])
            .await?;

        // Only process the workflow if this is the final failure (no more retries)
        if !will_retry {
            self.process_workflow(workflow_id).await
        } else {
            Ok(ProcessResult {
                completed: false,
                events_written: 1,
                tasks_enqueued: 0,
                signals_processed: 0,
            })
        }
    }

    /// Handle timer fired
    #[instrument(skip(self))]
    pub async fn on_timer_fired(
        &self,
        workflow_id: Uuid,
        timer_id: &str,
    ) -> Result<ProcessResult, ExecutorError> {
        // Load events to get current sequence (length = next expected sequence)
        let events = self.store.load_events(workflow_id).await?;
        let current_sequence = events.len() as i32;

        // Append timer fired event
        let timer_event = WorkflowEvent::TimerFired {
            timer_id: timer_id.to_string(),
        };

        self.store
            .append_events(workflow_id, current_sequence, vec![timer_event])
            .await?;

        // Process the workflow
        self.process_workflow(workflow_id).await
    }

    // =========================================================================
    // Internal Methods
    // =========================================================================

    /// Replay a single event on a workflow
    fn replay_event(
        &self,
        workflow: &mut dyn AnyWorkflow,
        event: &WorkflowEvent,
    ) -> Result<(), ExecutorError> {
        match event {
            WorkflowEvent::WorkflowStarted { .. } => {
                // on_start is called during workflow creation, not replay
                let _actions = workflow.on_start();
            }

            WorkflowEvent::ActivityCompleted {
                activity_id,
                result,
            } => {
                let _actions = workflow.on_activity_completed(activity_id, result.clone());
            }

            WorkflowEvent::ActivityFailed {
                activity_id,
                error,
                will_retry,
            } => {
                // Only notify workflow of final failure (when won't retry)
                if !will_retry {
                    let _actions = workflow.on_activity_failed(activity_id, error);
                }
            }

            WorkflowEvent::TimerFired { timer_id } => {
                let _actions = workflow.on_timer_fired(timer_id);
            }

            WorkflowEvent::SignalReceived { signal } => {
                let _actions = workflow.on_signal(signal);
            }

            // Events that don't affect workflow state during replay
            WorkflowEvent::WorkflowCompleted { .. }
            | WorkflowEvent::WorkflowFailed { .. }
            | WorkflowEvent::WorkflowCancelled { .. }
            | WorkflowEvent::ActivityScheduled { .. }
            | WorkflowEvent::ActivityStarted { .. }
            | WorkflowEvent::ActivityTimedOut { .. }
            | WorkflowEvent::ActivityCancelled { .. }
            | WorkflowEvent::TimerStarted { .. }
            | WorkflowEvent::TimerCancelled { .. }
            | WorkflowEvent::ChildWorkflowStarted { .. }
            | WorkflowEvent::ChildWorkflowCompleted { .. }
            | WorkflowEvent::ChildWorkflowFailed { .. } => {
                // These events are informational during replay
            }
        }

        Ok(())
    }

    /// Process actions from workflow, returning the new sequence number
    async fn process_actions(
        &self,
        workflow_id: Uuid,
        sequence: i32,
        actions: Vec<WorkflowAction>,
    ) -> Result<(), ExecutorError> {
        let (_new_seq, _written, _enqueued) = self
            .process_actions_internal(workflow_id, sequence, actions)
            .await?;
        Ok(())
    }

    /// Internal action processing that returns detailed results
    async fn process_actions_internal(
        &self,
        workflow_id: Uuid,
        mut sequence: i32,
        actions: Vec<WorkflowAction>,
    ) -> Result<(i32, usize, usize), ExecutorError> {
        let mut events_written = 0;
        let mut tasks_enqueued = 0;

        for action in actions {
            match action {
                WorkflowAction::ScheduleActivity {
                    activity_id,
                    activity_type,
                    input,
                    options,
                } => {
                    debug!(%workflow_id, %activity_id, %activity_type, "scheduling activity");

                    // Record the scheduling event
                    let event = WorkflowEvent::ActivityScheduled {
                        activity_id: activity_id.clone(),
                        activity_type: activity_type.clone(),
                        input: input.clone(),
                        options: options.clone(),
                    };

                    sequence = self
                        .store
                        .append_events(workflow_id, sequence, vec![event])
                        .await?;
                    events_written += 1;

                    // Enqueue the task
                    let task = TaskDefinition {
                        workflow_id,
                        activity_id,
                        activity_type,
                        input,
                        options,
                    };

                    self.store.enqueue_task(task).await?;
                    tasks_enqueued += 1;
                }

                WorkflowAction::StartTimer { timer_id, duration } => {
                    debug!(%workflow_id, %timer_id, ?duration, "starting timer");

                    let event = WorkflowEvent::TimerStarted {
                        timer_id,
                        duration_ms: duration.as_millis() as u64,
                    };

                    sequence = self
                        .store
                        .append_events(workflow_id, sequence, vec![event])
                        .await?;
                    events_written += 1;

                    // Timer scheduling would be handled by a separate timer service
                    // For now, we just record the event
                }

                WorkflowAction::CompleteWorkflow { result } => {
                    info!(%workflow_id, "completing workflow");

                    let event = WorkflowEvent::WorkflowCompleted {
                        result: result.clone(),
                    };

                    sequence = self
                        .store
                        .append_events(workflow_id, sequence, vec![event])
                        .await?;
                    events_written += 1;

                    self.store
                        .update_workflow_status(
                            workflow_id,
                            WorkflowStatus::Completed,
                            Some(result),
                            None,
                        )
                        .await?;
                }

                WorkflowAction::FailWorkflow { error } => {
                    error!(%workflow_id, error = %error.message, "failing workflow");

                    let event = WorkflowEvent::WorkflowFailed {
                        error: error.clone(),
                    };

                    sequence = self
                        .store
                        .append_events(workflow_id, sequence, vec![event])
                        .await?;
                    events_written += 1;

                    self.store
                        .update_workflow_status(
                            workflow_id,
                            WorkflowStatus::Failed,
                            None,
                            Some(error),
                        )
                        .await?;
                }

                WorkflowAction::ScheduleChildWorkflow {
                    workflow_id: child_id,
                    workflow_type,
                    input,
                } => {
                    debug!(%workflow_id, %child_id, %workflow_type, "scheduling child workflow");

                    // Record the event
                    let event = WorkflowEvent::ChildWorkflowStarted {
                        workflow_id: Uuid::now_v7(), // Generate child workflow ID
                        workflow_type,
                    };

                    sequence = self
                        .store
                        .append_events(workflow_id, sequence, vec![event])
                        .await?;
                    events_written += 1;

                    // Child workflow creation would be handled by a separate service
                    let _ = (child_id, input); // Suppress unused warnings
                }

                WorkflowAction::CancelActivity { activity_id } => {
                    debug!(%workflow_id, %activity_id, "cancelling activity");

                    let event = WorkflowEvent::ActivityCancelled {
                        activity_id,
                        reason: "cancelled by workflow".to_string(),
                    };

                    sequence = self
                        .store
                        .append_events(workflow_id, sequence, vec![event])
                        .await?;
                    events_written += 1;
                }

                WorkflowAction::None => {
                    // No action to process
                }
            }
        }

        Ok((sequence, events_written, tasks_enqueued))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persistence::InMemoryWorkflowEventStore;
    use serde::{Deserialize, Serialize};

    // Test workflow implementation
    #[derive(Debug, Clone, Serialize, Deserialize)]
    struct CounterInput {
        start: i32,
        target: i32,
    }

    #[derive(Debug, Serialize, Deserialize)]
    struct CounterOutput {
        final_value: i32,
    }

    struct CounterWorkflow {
        current: i32,
        target: i32,
        completed: bool,
        failed: bool,
        error_message: Option<String>,
    }

    impl crate::workflow::Workflow for CounterWorkflow {
        const TYPE: &'static str = "counter_workflow";
        type Input = CounterInput;
        type Output = CounterOutput;

        fn new(input: Self::Input) -> Self {
            Self {
                current: input.start,
                target: input.target,
                completed: false,
                failed: false,
                error_message: None,
            }
        }

        fn on_start(&mut self) -> Vec<WorkflowAction> {
            if self.current >= self.target {
                self.completed = true;
                vec![WorkflowAction::complete(
                    serde_json::json!({ "final_value": self.current }),
                )]
            } else {
                vec![WorkflowAction::schedule_activity(
                    format!("increment-{}", self.current),
                    "increment",
                    serde_json::json!({ "value": self.current }),
                )]
            }
        }

        fn on_activity_completed(
            &mut self,
            _activity_id: &str,
            result: serde_json::Value,
        ) -> Vec<WorkflowAction> {
            self.current = result.get("value").and_then(|v| v.as_i64()).unwrap_or(0) as i32;

            if self.current >= self.target {
                self.completed = true;
                vec![WorkflowAction::complete(
                    serde_json::json!({ "final_value": self.current }),
                )]
            } else {
                vec![WorkflowAction::schedule_activity(
                    format!("increment-{}", self.current),
                    "increment",
                    serde_json::json!({ "value": self.current }),
                )]
            }
        }

        fn on_activity_failed(
            &mut self,
            _activity_id: &str,
            error: &ActivityError,
        ) -> Vec<WorkflowAction> {
            self.failed = true;
            self.error_message = Some(error.message.clone());
            vec![WorkflowAction::fail(crate::WorkflowError::new(
                &error.message,
            ))]
        }

        fn is_completed(&self) -> bool {
            self.completed || self.failed
        }

        fn result(&self) -> Option<Self::Output> {
            if self.completed && !self.failed {
                Some(CounterOutput {
                    final_value: self.current,
                })
            } else {
                None
            }
        }

        fn error(&self) -> Option<crate::WorkflowError> {
            self.error_message.as_ref().map(crate::WorkflowError::new)
        }
    }

    #[tokio::test]
    async fn test_start_workflow() {
        let store = InMemoryWorkflowEventStore::new();
        let mut executor = WorkflowExecutor::new(store);
        executor.register::<CounterWorkflow>();

        let input = CounterInput {
            start: 0,
            target: 3,
        };
        let workflow_id = executor
            .start_workflow::<CounterWorkflow>(input, None)
            .await
            .expect("should start workflow");

        // Verify workflow was created
        let status = executor
            .store()
            .get_workflow_status(workflow_id)
            .await
            .expect("should get status");

        assert_eq!(status, WorkflowStatus::Running);

        // Verify events were written
        let events = executor
            .store()
            .load_events(workflow_id)
            .await
            .expect("should load events");

        assert!(events.len() >= 2); // WorkflowStarted + ActivityScheduled
        assert!(matches!(events[0].1, WorkflowEvent::WorkflowStarted { .. }));
        assert!(matches!(
            events[1].1,
            WorkflowEvent::ActivityScheduled { .. }
        ));
    }

    #[tokio::test]
    async fn test_immediate_completion() {
        let store = InMemoryWorkflowEventStore::new();
        let mut executor = WorkflowExecutor::new(store);
        executor.register::<CounterWorkflow>();

        // Start with current >= target, should complete immediately
        let input = CounterInput {
            start: 5,
            target: 3,
        };
        let workflow_id = executor
            .start_workflow::<CounterWorkflow>(input, None)
            .await
            .expect("should start workflow");

        // Verify workflow completed
        let status = executor
            .store()
            .get_workflow_status(workflow_id)
            .await
            .expect("should get status");

        assert_eq!(status, WorkflowStatus::Completed);
    }

    #[tokio::test]
    async fn test_activity_completion() {
        let store = InMemoryWorkflowEventStore::new();
        let mut executor = WorkflowExecutor::new(store);
        executor.register::<CounterWorkflow>();

        let input = CounterInput {
            start: 0,
            target: 2,
        };
        let workflow_id = executor
            .start_workflow::<CounterWorkflow>(input, None)
            .await
            .expect("should start workflow");

        // Complete first activity (increment 0 -> 1)
        let result = executor
            .on_activity_completed(
                workflow_id,
                "increment-0",
                serde_json::json!({ "value": 1 }),
            )
            .await
            .expect("should complete activity");

        assert!(!result.completed);

        // Complete second activity (increment 1 -> 2)
        let result = executor
            .on_activity_completed(
                workflow_id,
                "increment-1",
                serde_json::json!({ "value": 2 }),
            )
            .await
            .expect("should complete activity");

        assert!(result.completed);

        // Verify final status
        let status = executor
            .store()
            .get_workflow_status(workflow_id)
            .await
            .expect("should get status");

        assert_eq!(status, WorkflowStatus::Completed);
    }

    #[tokio::test]
    async fn test_activity_failure() {
        let store = InMemoryWorkflowEventStore::new();
        let mut executor = WorkflowExecutor::new(store);
        executor.register::<CounterWorkflow>();

        let input = CounterInput {
            start: 0,
            target: 5,
        };
        let workflow_id = executor
            .start_workflow::<CounterWorkflow>(input, None)
            .await
            .expect("should start workflow");

        // Fail the activity (final failure, no retry)
        let error = ActivityError::non_retryable("increment failed").with_type("INCREMENT_ERROR");
        let result = executor
            .on_activity_failed(workflow_id, "increment-0", error, false)
            .await
            .expect("should handle failure");

        assert!(result.completed);

        // Verify workflow failed
        let status = executor
            .store()
            .get_workflow_status(workflow_id)
            .await
            .expect("should get status");

        assert_eq!(status, WorkflowStatus::Failed);
    }

    #[tokio::test]
    async fn test_signal_handling() {
        let store = InMemoryWorkflowEventStore::new();
        let mut executor = WorkflowExecutor::new(store);
        executor.register::<CounterWorkflow>();

        let input = CounterInput {
            start: 0,
            target: 10,
        };
        let workflow_id = executor
            .start_workflow::<CounterWorkflow>(input, None)
            .await
            .expect("should start workflow");

        // Send a signal
        let signal = WorkflowSignal::new("test_signal", serde_json::json!({ "data": "hello" }));
        executor
            .send_signal(workflow_id, signal)
            .await
            .expect("should send signal");

        // Process workflow (should handle signal)
        let result = executor
            .process_workflow(workflow_id)
            .await
            .expect("should process");

        assert_eq!(result.signals_processed, 1);
    }

    #[tokio::test]
    async fn test_cannot_signal_completed_workflow() {
        let store = InMemoryWorkflowEventStore::new();
        let mut executor = WorkflowExecutor::new(store);
        executor.register::<CounterWorkflow>();

        // Start workflow that completes immediately
        let input = CounterInput {
            start: 10,
            target: 5,
        };
        let workflow_id = executor
            .start_workflow::<CounterWorkflow>(input, None)
            .await
            .expect("should start workflow");

        // Try to send signal to completed workflow
        let signal = WorkflowSignal::new("test", serde_json::json!({}));
        let result = executor.send_signal(workflow_id, signal).await;

        assert!(matches!(result, Err(ExecutorError::WorkflowCompleted(_))));
    }

    #[tokio::test]
    async fn test_replay_consistency() {
        let store = InMemoryWorkflowEventStore::new();
        let mut executor = WorkflowExecutor::new(store);
        executor.register::<CounterWorkflow>();

        let input = CounterInput {
            start: 0,
            target: 3,
        };
        let workflow_id = executor
            .start_workflow::<CounterWorkflow>(input, None)
            .await
            .expect("should start workflow");

        // Complete activities
        executor
            .on_activity_completed(
                workflow_id,
                "increment-0",
                serde_json::json!({ "value": 1 }),
            )
            .await
            .unwrap();
        executor
            .on_activity_completed(
                workflow_id,
                "increment-1",
                serde_json::json!({ "value": 2 }),
            )
            .await
            .unwrap();
        executor
            .on_activity_completed(
                workflow_id,
                "increment-2",
                serde_json::json!({ "value": 3 }),
            )
            .await
            .unwrap();

        // Process workflow again - should handle already completed state
        let result = executor.process_workflow(workflow_id).await.unwrap();
        assert!(result.completed);
    }
}

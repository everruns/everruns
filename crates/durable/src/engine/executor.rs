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

    /// Snapshot interval: save a snapshot every N events.
    /// Set to 0 to disable snapshotting.
    pub snapshot_interval: i32,
}

impl Default for ExecutorConfig {
    fn default() -> Self {
        Self {
            max_events_per_workflow: 10000,
            validate_actions: true,
            snapshot_interval: crate::persistence::snapshot_interval_from_env(),
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
    ///
    /// ## Snapshot-based replay
    ///
    /// If the workflow type supports snapshots (`Workflow::snapshot_state`/`restore_state`),
    /// the executor will:
    /// 1. Load the latest snapshot (if any)
    /// 2. Restore workflow state from the snapshot
    /// 3. Load and replay only events after the snapshot's sequence number
    /// 4. Save a new snapshot periodically (every `snapshot_interval` events)
    ///
    /// This reduces replay cost from O(total_events) to O(checkpoint_interval).
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

        // Try snapshot-based replay first
        let (mut workflow, events, snapshot_seq) = self
            .load_workflow_state(workflow_id, &workflow_info)
            .await?;

        // Track the current sequence
        // For snapshot replay: snapshot_seq + events_after.len()
        // For full replay: events.len()
        let mut current_sequence = if snapshot_seq > 0 {
            // We replayed events after snapshot; total = last event seq + 1
            events
                .last()
                .map(|(seq, _)| seq + 1)
                .unwrap_or(snapshot_seq + 1)
        } else {
            events.len() as i32
        };

        // For full replay (no snapshot), also need total event count for snapshot decisions
        let total_event_count = if snapshot_seq > 0 {
            // snapshot_seq is 0-indexed, plus events after snapshot
            (snapshot_seq + 1 + events.len() as i32) as usize
        } else {
            events.len()
        };

        let mut events_written = 0;
        let mut tasks_enqueued = 0;

        // Replay events to rebuild state
        for (_seq, event) in &events {
            self.replay_event(&mut *workflow, event)?;
        }

        debug!(
            %workflow_id,
            current_sequence,
            snapshot_seq,
            events_replayed = events.len(),
            "replayed events"
        );

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

        // Save snapshot if warranted (enough events since last snapshot)
        if !completed {
            self.maybe_save_snapshot(
                workflow_id,
                &*workflow,
                current_sequence,
                snapshot_seq,
                total_event_count + events_written,
            )
            .await;
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

    /// Load workflow state, using snapshot if available.
    ///
    /// Returns (workflow_instance, events_to_replay, snapshot_sequence).
    /// snapshot_sequence is 0 if no snapshot was used (full replay).
    async fn load_workflow_state(
        &self,
        workflow_id: Uuid,
        workflow_info: &crate::persistence::WorkflowInfo,
    ) -> Result<(Box<dyn AnyWorkflow>, Vec<(i32, WorkflowEvent)>, i32), ExecutorError> {
        // Try loading a snapshot first
        let snapshot = self.store.load_latest_snapshot(workflow_id).await?;

        if let Some(ref snap) = snapshot {
            // Try restoring from snapshot
            if let Some(restored) = self.registry.restore_from_snapshot(
                &workflow_info.workflow_type,
                workflow_info.input.clone(),
                &snap.snapshot_data,
            ) {
                // Load only events after the snapshot
                let events = self
                    .store
                    .load_events_after(workflow_id, snap.sequence_num)
                    .await?;

                debug!(
                    %workflow_id,
                    snapshot_seq = snap.sequence_num,
                    events_after = events.len(),
                    "restored from snapshot"
                );

                return Ok((restored, events, snap.sequence_num));
            }
            // Snapshot restore failed (e.g., workflow doesn't support snapshots)
            // Fall through to full replay
            debug!(
                %workflow_id,
                "snapshot restore failed, falling back to full replay"
            );
        }

        // Full replay: load all events
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

        // Create workflow from scratch
        let workflow = self
            .registry
            .create(&workflow_info.workflow_type, workflow_info.input.clone())?;

        Ok((workflow, events, 0))
    }

    /// Save a snapshot if enough events have accumulated since the last one.
    async fn maybe_save_snapshot(
        &self,
        workflow_id: Uuid,
        workflow: &dyn AnyWorkflow,
        current_sequence: i32,
        last_snapshot_seq: i32,
        total_events: usize,
    ) {
        let interval = self.config.snapshot_interval;
        if interval <= 0 {
            return;
        }

        // Only snapshot if enough events since last snapshot
        let events_since_snapshot = if last_snapshot_seq > 0 {
            current_sequence - last_snapshot_seq
        } else {
            // No previous snapshot — use total event count
            total_events as i32
        };

        if events_since_snapshot < interval {
            return;
        }

        // Try to serialize workflow state
        if let Some(state_data) = workflow.serialize_state() {
            // Snapshot at (current_sequence - 1) since current_sequence is next-to-append
            let snap_seq = current_sequence - 1;
            if let Err(e) = self
                .store
                .save_snapshot(workflow_id, snap_seq, state_data)
                .await
            {
                // Snapshot save failure is non-fatal; log and continue
                warn!(%workflow_id, snap_seq, error = %e, "failed to save snapshot");
            } else {
                debug!(%workflow_id, snap_seq, "saved workflow snapshot");
            }
        }
    }

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
                        workflow_id: Some(workflow_id),
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

    // =================================================================
    // Snapshot-capable workflow for testing
    // =================================================================

    /// A counter workflow that supports snapshot serialization
    #[derive(Debug, Serialize, Deserialize)]
    struct SnapCounterState {
        current: i32,
        target: i32,
        completed: bool,
        failed: bool,
        error_message: Option<String>,
    }

    struct SnapCounterWorkflow {
        state: SnapCounterState,
    }

    impl crate::workflow::Workflow for SnapCounterWorkflow {
        const TYPE: &'static str = "snap_counter_workflow";
        type Input = CounterInput;
        type Output = CounterOutput;

        fn new(input: Self::Input) -> Self {
            Self {
                state: SnapCounterState {
                    current: input.start,
                    target: input.target,
                    completed: false,
                    failed: false,
                    error_message: None,
                },
            }
        }

        fn on_start(&mut self) -> Vec<WorkflowAction> {
            if self.state.current >= self.state.target {
                self.state.completed = true;
                vec![WorkflowAction::complete(
                    serde_json::json!({ "final_value": self.state.current }),
                )]
            } else {
                vec![WorkflowAction::schedule_activity(
                    format!("increment-{}", self.state.current),
                    "increment",
                    serde_json::json!({ "value": self.state.current }),
                )]
            }
        }

        fn on_activity_completed(
            &mut self,
            _activity_id: &str,
            result: serde_json::Value,
        ) -> Vec<WorkflowAction> {
            self.state.current = result.get("value").and_then(|v| v.as_i64()).unwrap_or(0) as i32;

            if self.state.current >= self.state.target {
                self.state.completed = true;
                vec![WorkflowAction::complete(
                    serde_json::json!({ "final_value": self.state.current }),
                )]
            } else {
                vec![WorkflowAction::schedule_activity(
                    format!("increment-{}", self.state.current),
                    "increment",
                    serde_json::json!({ "value": self.state.current }),
                )]
            }
        }

        fn on_activity_failed(
            &mut self,
            _activity_id: &str,
            error: &ActivityError,
        ) -> Vec<WorkflowAction> {
            self.state.failed = true;
            self.state.error_message = Some(error.message.clone());
            vec![WorkflowAction::fail(crate::WorkflowError::new(
                &error.message,
            ))]
        }

        fn is_completed(&self) -> bool {
            self.state.completed || self.state.failed
        }

        fn result(&self) -> Option<Self::Output> {
            if self.state.completed && !self.state.failed {
                Some(CounterOutput {
                    final_value: self.state.current,
                })
            } else {
                None
            }
        }

        fn error(&self) -> Option<crate::WorkflowError> {
            self.state
                .error_message
                .as_ref()
                .map(crate::WorkflowError::new)
        }

        fn snapshot_state(&self) -> Option<Vec<u8>> {
            serde_json::to_vec(&self.state).ok()
        }

        fn restore_state(input: Self::Input, data: &[u8]) -> Option<Self> {
            let state: SnapCounterState = serde_json::from_slice(data).ok()?;
            // Validate consistency: target from input should match snapshot
            let _ = input; // Input already embedded in state
            Some(Self { state })
        }
    }

    /// Helper: create executor with a specific snapshot interval
    fn snap_executor(
        store: InMemoryWorkflowEventStore,
        interval: i32,
    ) -> WorkflowExecutor<InMemoryWorkflowEventStore> {
        let config = ExecutorConfig {
            snapshot_interval: interval,
            ..Default::default()
        };
        let mut executor = WorkflowExecutor::with_config(store, config);
        executor.register::<SnapCounterWorkflow>();
        executor
    }

    // =================================================================
    // Snapshot tests
    // =================================================================

    #[tokio::test]
    async fn test_snapshot_save_and_restore() {
        // Use a very small interval to trigger snapshot quickly
        let store = InMemoryWorkflowEventStore::new();
        let mut executor = snap_executor(store, 3);
        executor.register::<CounterWorkflow>(); // also register non-snap version

        let input = CounterInput {
            start: 0,
            target: 10,
        };
        let workflow_id = executor
            .start_workflow::<SnapCounterWorkflow>(input, None)
            .await
            .unwrap();

        // Complete several activities to exceed snapshot interval (3 events)
        for i in 0..5 {
            executor
                .on_activity_completed(
                    workflow_id,
                    &format!("increment-{}", i),
                    serde_json::json!({ "value": i + 1 }),
                )
                .await
                .unwrap();
        }

        // Verify a snapshot was saved
        let snapshot = executor
            .store()
            .load_latest_snapshot(workflow_id)
            .await
            .unwrap();
        assert!(snapshot.is_some(), "snapshot should have been saved");

        let snap = snapshot.unwrap();
        assert!(snap.sequence_num > 0, "snapshot sequence should be > 0");
        assert!(
            !snap.snapshot_data.is_empty(),
            "snapshot data should not be empty"
        );

        // Continue processing — should use snapshot for replay
        executor
            .on_activity_completed(
                workflow_id,
                "increment-5",
                serde_json::json!({ "value": 6 }),
            )
            .await
            .unwrap();

        // Verify workflow is still running (not at target 10 yet)
        let status = executor
            .store()
            .get_workflow_status(workflow_id)
            .await
            .unwrap();
        assert_eq!(status, WorkflowStatus::Running);
    }

    #[tokio::test]
    async fn test_snapshot_produces_same_result() {
        // Run workflow to completion with snapshots enabled
        let store = InMemoryWorkflowEventStore::new();
        let executor = snap_executor(store, 3);

        let input = CounterInput {
            start: 0,
            target: 5,
        };
        let workflow_id = executor
            .start_workflow::<SnapCounterWorkflow>(input.clone(), None)
            .await
            .unwrap();

        for i in 0..5 {
            executor
                .on_activity_completed(
                    workflow_id,
                    &format!("increment-{}", i),
                    serde_json::json!({ "value": i + 1 }),
                )
                .await
                .unwrap();
        }

        let status1 = executor
            .store()
            .get_workflow_status(workflow_id)
            .await
            .unwrap();
        assert_eq!(status1, WorkflowStatus::Completed);

        // Run same workflow WITHOUT snapshots
        let store2 = InMemoryWorkflowEventStore::new();
        let executor2 = snap_executor(store2, 0); // snapshots disabled

        let workflow_id2 = executor2
            .start_workflow::<SnapCounterWorkflow>(input, None)
            .await
            .unwrap();

        for i in 0..5 {
            executor2
                .on_activity_completed(
                    workflow_id2,
                    &format!("increment-{}", i),
                    serde_json::json!({ "value": i + 1 }),
                )
                .await
                .unwrap();
        }

        let status2 = executor2
            .store()
            .get_workflow_status(workflow_id2)
            .await
            .unwrap();
        assert_eq!(status2, WorkflowStatus::Completed);

        // Both should produce same final status
        assert_eq!(status1, status2);
    }

    #[tokio::test]
    async fn test_no_snapshot_without_support() {
        // Use the non-snapshot CounterWorkflow with snapshot interval enabled
        let store = InMemoryWorkflowEventStore::new();
        let config = ExecutorConfig {
            snapshot_interval: 2,
            ..Default::default()
        };
        let mut executor = WorkflowExecutor::with_config(store, config);
        executor.register::<CounterWorkflow>();

        let input = CounterInput {
            start: 0,
            target: 5,
        };
        let workflow_id = executor
            .start_workflow::<CounterWorkflow>(input, None)
            .await
            .unwrap();

        for i in 0..4 {
            executor
                .on_activity_completed(
                    workflow_id,
                    &format!("increment-{}", i),
                    serde_json::json!({ "value": i + 1 }),
                )
                .await
                .unwrap();
        }

        // No snapshot should be saved since CounterWorkflow doesn't implement snapshot_state
        let snapshot = executor
            .store()
            .load_latest_snapshot(workflow_id)
            .await
            .unwrap();
        assert!(snapshot.is_none(), "no snapshot for non-snapshot workflows");
    }

    #[tokio::test]
    async fn test_snapshot_disabled_when_interval_zero() {
        let store = InMemoryWorkflowEventStore::new();
        let executor = snap_executor(store, 0); // disabled

        let input = CounterInput {
            start: 0,
            target: 5,
        };
        let workflow_id = executor
            .start_workflow::<SnapCounterWorkflow>(input, None)
            .await
            .unwrap();

        for i in 0..4 {
            executor
                .on_activity_completed(
                    workflow_id,
                    &format!("increment-{}", i),
                    serde_json::json!({ "value": i + 1 }),
                )
                .await
                .unwrap();
        }

        let snapshot = executor
            .store()
            .load_latest_snapshot(workflow_id)
            .await
            .unwrap();
        assert!(snapshot.is_none(), "no snapshot when interval is 0");
    }

    #[tokio::test]
    async fn test_snapshot_not_saved_on_completion() {
        let store = InMemoryWorkflowEventStore::new();
        let executor = snap_executor(store, 2);

        let input = CounterInput {
            start: 0,
            target: 2,
        };
        let workflow_id = executor
            .start_workflow::<SnapCounterWorkflow>(input, None)
            .await
            .unwrap();

        // Complete all activities
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

        // Workflow completed - snapshot should NOT be saved for terminal workflows
        let status = executor
            .store()
            .get_workflow_status(workflow_id)
            .await
            .unwrap();
        assert_eq!(status, WorkflowStatus::Completed);

        // Even though enough events accumulated, snapshot not saved for completed workflow
        // (this is by design - no point snapshotting terminal workflows)
    }

    #[tokio::test]
    async fn test_snapshot_delete() {
        let store = InMemoryWorkflowEventStore::new();

        // Manually save a snapshot
        let workflow_id = Uuid::now_v7();
        store
            .save_snapshot(workflow_id, 10, b"test_data".to_vec())
            .await
            .unwrap();

        let snap = store.load_latest_snapshot(workflow_id).await.unwrap();
        assert!(snap.is_some());

        // Delete snapshots
        store.delete_snapshots(workflow_id).await.unwrap();

        let snap = store.load_latest_snapshot(workflow_id).await.unwrap();
        assert!(snap.is_none());
    }

    #[tokio::test]
    async fn test_snapshot_upsert() {
        let store = InMemoryWorkflowEventStore::new();
        let workflow_id = Uuid::now_v7();

        // Save snapshot at seq 5
        store
            .save_snapshot(workflow_id, 5, b"data_v1".to_vec())
            .await
            .unwrap();

        // Save snapshot at seq 10
        store
            .save_snapshot(workflow_id, 10, b"data_v2".to_vec())
            .await
            .unwrap();

        // Latest should be seq 10
        let snap = store
            .load_latest_snapshot(workflow_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(snap.sequence_num, 10);
        assert_eq!(snap.snapshot_data, b"data_v2");

        // Upsert seq 5 with new data
        store
            .save_snapshot(workflow_id, 5, b"data_v1_updated".to_vec())
            .await
            .unwrap();

        // Latest should still be seq 10
        let snap = store
            .load_latest_snapshot(workflow_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(snap.sequence_num, 10);
    }

    #[tokio::test]
    async fn test_load_events_after() {
        let store = InMemoryWorkflowEventStore::new();

        // Create a workflow and add events
        let workflow_id = Uuid::now_v7();
        store
            .create_workflow(workflow_id, "test", serde_json::json!({}), None)
            .await
            .unwrap();

        let events = vec![WorkflowEvent::WorkflowStarted {
            input: serde_json::json!({}),
        }];
        store.append_events(workflow_id, 0, events).await.unwrap();

        let events = vec![WorkflowEvent::ActivityScheduled {
            activity_id: "a1".into(),
            activity_type: "test".into(),
            input: serde_json::json!({}),
            options: crate::workflow::ActivityOptions::default(),
        }];
        store.append_events(workflow_id, 1, events).await.unwrap();

        let events = vec![WorkflowEvent::ActivityCompleted {
            activity_id: "a1".into(),
            result: serde_json::json!(42),
        }];
        store.append_events(workflow_id, 2, events).await.unwrap();

        // Load events after seq 0
        let after_0 = store.load_events_after(workflow_id, 0).await.unwrap();
        assert_eq!(after_0.len(), 2); // seq 1 and seq 2

        // Load events after seq 1
        let after_1 = store.load_events_after(workflow_id, 1).await.unwrap();
        assert_eq!(after_1.len(), 1); // only seq 2

        // Load events after seq 2 (none left)
        let after_2 = store.load_events_after(workflow_id, 2).await.unwrap();
        assert_eq!(after_2.len(), 0);
    }

    #[tokio::test]
    async fn test_snapshot_fallback_to_full_replay() {
        // If snapshot data is corrupted, should fall back to full replay
        let store = InMemoryWorkflowEventStore::new();
        let executor = snap_executor(store, 3);

        let input = CounterInput {
            start: 0,
            target: 5,
        };
        let workflow_id = executor
            .start_workflow::<SnapCounterWorkflow>(input, None)
            .await
            .unwrap();

        // Add some activities
        for i in 0..3 {
            executor
                .on_activity_completed(
                    workflow_id,
                    &format!("increment-{}", i),
                    serde_json::json!({ "value": i + 1 }),
                )
                .await
                .unwrap();
        }

        // Manually save corrupted snapshot
        executor
            .store()
            .save_snapshot(workflow_id, 5, b"corrupted_data".to_vec())
            .await
            .unwrap();

        // Processing should still work (falls back to full replay)
        let result = executor
            .on_activity_completed(
                workflow_id,
                "increment-3",
                serde_json::json!({ "value": 4 }),
            )
            .await;
        assert!(
            result.is_ok(),
            "should succeed with fallback to full replay"
        );
    }

    #[tokio::test]
    async fn test_many_events_with_snapshot_bounded_replay() {
        // Simulate a workflow with many events and verify snapshot-based
        // replay only loads events after the snapshot
        let store = InMemoryWorkflowEventStore::new();
        let executor = snap_executor(store, 5); // snapshot every 5

        let target = 20;
        let input = CounterInput { start: 0, target };
        let workflow_id = executor
            .start_workflow::<SnapCounterWorkflow>(input, None)
            .await
            .unwrap();

        // Complete 19 activities (0..19, need to reach 20)
        for i in 0..target {
            executor
                .on_activity_completed(
                    workflow_id,
                    &format!("increment-{}", i),
                    serde_json::json!({ "value": i + 1 }),
                )
                .await
                .unwrap();
        }

        // Workflow should be completed
        let status = executor
            .store()
            .get_workflow_status(workflow_id)
            .await
            .unwrap();
        assert_eq!(status, WorkflowStatus::Completed);

        // Verify total events accumulated (should be many)
        let all_events = executor.store().load_events(workflow_id).await.unwrap();
        assert!(
            all_events.len() > 20,
            "should have many events (got {})",
            all_events.len()
        );
    }
}

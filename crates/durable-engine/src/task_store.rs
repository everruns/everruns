//! Task queue adapter shared by durable runner backends and the worker loop.
//!
//! Every `WorkflowEventStore` gets this trait through the blanket impl below.
//! Transport-backed stores implement it from their own crate for their own
//! type (the worker does so for its gRPC client), which coherence accepts
//! because that type does not implement `WorkflowEventStore`.

use crate::durable::{
    ActivityOptions, ClaimedTask, Enqueued, EventLog, HeartbeatResponse, SignalStore, StoreError,
    TaskDefinition, TaskFailureOutcome, TaskQueue, WorkerInfo, WorkerRegistry, WorkflowError,
    WorkflowEvent, WorkflowEventStore, WorkflowStatus, append_event, record_activity_completed,
    record_activity_failed, record_activity_started, record_workflow_failed,
};
use async_trait::async_trait;
use uuid::Uuid;

/// Each item signals that new work may be claimable. Closure means resubscribe.
pub type TaskWakeups = tokio::sync::mpsc::Receiver<()>;

#[async_trait]
pub trait TaskStore: Send + Sync + 'static {
    async fn register_worker(&self, worker: WorkerInfo) -> Result<(), StoreError>;

    async fn worker_heartbeat(
        &self,
        worker_id: &str,
        current_load: usize,
        accepting_tasks: bool,
    ) -> Result<(), StoreError>;

    async fn deregister_worker(&self, worker_id: &str) -> Result<usize, StoreError>;

    async fn claim_task(
        &self,
        worker_id: &str,
        activity_types: &[String],
        max_tasks: usize,
    ) -> Result<Vec<ClaimedTask>, StoreError>;

    async fn heartbeat_task(
        &self,
        task_id: Uuid,
        worker_id: &str,
        details: Option<serde_json::Value>,
    ) -> Result<HeartbeatResponse, StoreError>;

    async fn get_workflow_status(&self, workflow_id: Uuid) -> Result<WorkflowStatus, StoreError>;

    async fn record_activity_started(&self, task: &ClaimedTask, worker_id: &str);

    async fn complete_task_and_record(
        &self,
        task: &ClaimedTask,
        worker_id: &str,
        output: serde_json::Value,
    ) -> Result<(), StoreError>;

    /// Complete `task`, then consume its workflow's pending signals of type
    /// `drain`. Returns how many were consumed, or `None` when this store did
    /// not drain (the caller then consumes them itself). The gRPC store folds
    /// both into one round trip; an in-process store gains nothing from it.
    async fn complete_task_and_drain(
        &self,
        task: &ClaimedTask,
        worker_id: &str,
        output: serde_json::Value,
        _drain: Option<&str>,
    ) -> Result<Option<usize>, StoreError> {
        self.complete_task_and_record(task, worker_id, output)
            .await?;
        Ok(None)
    }

    async fn fail_task_and_record(
        &self,
        task: &ClaimedTask,
        error: &str,
        retryable: bool,
    ) -> Result<TaskFailureOutcome, StoreError>;

    async fn enqueue_task_and_record(
        &self,
        workflow_id: Uuid,
        activity_id: String,
        activity_type: String,
        input: serde_json::Value,
    ) -> Result<Uuid, StoreError>;

    /// Enqueue and record a workflow's next step already claimed by
    /// `worker_id`, which then runs it without a wakeup and a claim (see
    /// [`TaskQueue::enqueue_claimed_task`]). Returns `None` when the task
    /// went to the queue instead; the default always does that.
    async fn enqueue_claimed_task_and_record(
        &self,
        workflow_id: Uuid,
        activity_id: String,
        activity_type: String,
        input: serde_json::Value,
        worker_id: &str,
    ) -> Result<Option<ClaimedTask>, StoreError> {
        let _ = worker_id;
        self.enqueue_task_and_record(workflow_id, activity_id, activity_type, input)
            .await?;
        Ok(None)
    }

    async fn update_workflow_status(
        &self,
        workflow_id: Uuid,
        status: WorkflowStatus,
        output: Option<serde_json::Value>,
        error: Option<WorkflowError>,
    ) -> Result<(), StoreError>;

    async fn complete_workflow(
        &self,
        workflow_id: Uuid,
        event_output: serde_json::Value,
        stored_output: Option<serde_json::Value>,
        error: Option<WorkflowError>,
    ) -> Result<(), StoreError>;

    async fn consume_pending_signals(
        &self,
        workflow_id: Uuid,
    ) -> Result<Vec<crate::durable::WorkflowSignal>, StoreError>;

    async fn consume_pending_signals_by_type(
        &self,
        workflow_id: Uuid,
        signal_type: &str,
    ) -> Result<Vec<crate::durable::WorkflowSignal>, StoreError>;

    /// Open a push channel that signals new claimable work.
    ///
    /// `Ok(None)` means the store has none and the worker polls only. See
    /// the worker wake-up listener for subscription recovery.
    async fn subscribe_task_wakeups(
        &self,
        _worker_id: &str,
        _activity_types: &[String],
    ) -> Result<Option<TaskWakeups>, StoreError> {
        Ok(None)
    }
}

#[async_trait]
impl<S> TaskStore for S
where
    S: WorkflowEventStore,
{
    async fn register_worker(&self, worker: WorkerInfo) -> Result<(), StoreError> {
        WorkerRegistry::register_worker(self, worker).await
    }

    async fn worker_heartbeat(
        &self,
        worker_id: &str,
        current_load: usize,
        accepting_tasks: bool,
    ) -> Result<(), StoreError> {
        WorkerRegistry::worker_heartbeat(self, worker_id, current_load, accepting_tasks).await
    }

    async fn deregister_worker(&self, worker_id: &str) -> Result<usize, StoreError> {
        WorkerRegistry::deregister_worker(self, worker_id).await
    }

    async fn claim_task(
        &self,
        worker_id: &str,
        activity_types: &[String],
        max_tasks: usize,
    ) -> Result<Vec<ClaimedTask>, StoreError> {
        TaskQueue::claim_task(self, worker_id, activity_types, max_tasks).await
    }

    async fn heartbeat_task(
        &self,
        task_id: Uuid,
        worker_id: &str,
        details: Option<serde_json::Value>,
    ) -> Result<HeartbeatResponse, StoreError> {
        TaskQueue::heartbeat_task(self, task_id, worker_id, details).await
    }

    async fn get_workflow_status(&self, workflow_id: Uuid) -> Result<WorkflowStatus, StoreError> {
        EventLog::get_workflow_status(self, workflow_id).await
    }

    async fn record_activity_started(&self, task: &ClaimedTask, worker_id: &str) {
        record_activity_started(
            self,
            task.workflow_id,
            task.activity_id.clone(),
            task.attempt,
            worker_id.to_string(),
        )
        .await;
    }

    async fn complete_task_and_record(
        &self,
        task: &ClaimedTask,
        worker_id: &str,
        output: serde_json::Value,
    ) -> Result<(), StoreError> {
        TaskQueue::complete_task(self, task.id, worker_id, output.clone()).await?;
        record_activity_completed(self, task.workflow_id, task.activity_id.clone(), output).await;
        Ok(())
    }

    async fn fail_task_and_record(
        &self,
        task: &ClaimedTask,
        error: &str,
        retryable: bool,
    ) -> Result<TaskFailureOutcome, StoreError> {
        let outcome = match TaskQueue::fail_task_with_retry(self, task.id, error, retryable).await {
            Ok(outcome) => outcome,
            Err(StoreError::TaskNotOwned(_)) => return Ok(TaskFailureOutcome::MovedToDlq),
            Err(error) => return Err(error),
        };
        let will_retry = matches!(outcome, TaskFailureOutcome::WillRetry { .. });
        record_activity_failed(
            self,
            task.workflow_id,
            task.activity_id.clone(),
            error.to_string(),
            will_retry,
        )
        .await;
        if matches!(outcome, TaskFailureOutcome::MovedToDlq)
            && let Some(workflow_id) = task.workflow_id
            && EventLog::try_fail_workflow(self, workflow_id, WorkflowError::new(error)).await?
        {
            record_workflow_failed(self, workflow_id, error.to_string()).await;
            return Ok(TaskFailureOutcome::ExhaustedRetries);
        }
        Ok(outcome)
    }

    async fn enqueue_task_and_record(
        &self,
        workflow_id: Uuid,
        activity_id: String,
        activity_type: String,
        input: serde_json::Value,
    ) -> Result<Uuid, StoreError> {
        let event = WorkflowEvent::ActivityScheduled {
            activity_id: activity_id.clone(),
            activity_type: activity_type.clone(),
            input: input.clone(),
            options: ActivityOptions::default(),
        };
        append_event(self, workflow_id, event).await?;
        TaskQueue::enqueue_task(
            self,
            TaskDefinition {
                workflow_id: Some(workflow_id),
                activity_id,
                activity_type,
                input,
                options: ActivityOptions::default(),
            },
        )
        .await
    }

    async fn enqueue_claimed_task_and_record(
        &self,
        workflow_id: Uuid,
        activity_id: String,
        activity_type: String,
        input: serde_json::Value,
        worker_id: &str,
    ) -> Result<Option<ClaimedTask>, StoreError> {
        let event = WorkflowEvent::ActivityScheduled {
            activity_id: activity_id.clone(),
            activity_type: activity_type.clone(),
            input: input.clone(),
            options: ActivityOptions::default(),
        };
        append_event(self, workflow_id, event).await?;
        let task = TaskDefinition {
            workflow_id: Some(workflow_id),
            activity_id,
            activity_type,
            input,
            options: ActivityOptions::default(),
        };
        TaskQueue::enqueue_claimed_task(self, task, worker_id)
            .await
            .map(Enqueued::into_claimed)
    }

    async fn update_workflow_status(
        &self,
        workflow_id: Uuid,
        status: WorkflowStatus,
        output: Option<serde_json::Value>,
        error: Option<WorkflowError>,
    ) -> Result<(), StoreError> {
        EventLog::update_workflow_status(self, workflow_id, status, output, error).await
    }

    async fn complete_workflow(
        &self,
        workflow_id: Uuid,
        event_output: serde_json::Value,
        stored_output: Option<serde_json::Value>,
        error: Option<WorkflowError>,
    ) -> Result<(), StoreError> {
        crate::durable::record_workflow_completed(self, workflow_id, event_output).await;
        EventLog::update_workflow_status(
            self,
            workflow_id,
            WorkflowStatus::Completed,
            stored_output,
            error,
        )
        .await
    }

    async fn consume_pending_signals(
        &self,
        workflow_id: Uuid,
    ) -> Result<Vec<crate::durable::WorkflowSignal>, StoreError> {
        SignalStore::consume_pending_signals(self, workflow_id).await
    }

    async fn consume_pending_signals_by_type(
        &self,
        workflow_id: Uuid,
        signal_type: &str,
    ) -> Result<Vec<crate::durable::WorkflowSignal>, StoreError> {
        SignalStore::consume_pending_signals_by_type(self, workflow_id, signal_type).await
    }
}

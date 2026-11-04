//! The store the worker's durable queue loop runs on.
//!
//! The queue carries only the activities that are not turns (leased-resource
//! cleanup, the session task reaper, scheduled channel and agent-trigger
//! invocations); turns run on session actor leases (`actor_runtime`). So the
//! worker needs the registry, claim, heartbeat and settle calls, and a push
//! channel for wake-ups, nothing about workflows or signals.
//!
//! Decision: every `everruns-durable` `WorkflowEventStore` is a `TaskStore`
//! through the blanket impl below (the in-process DEV_MODE worker runs on the
//! server's store), and the worker implements it for its own gRPC client
//! (`grpc_task_store`), which coherence accepts because that type does not
//! implement `WorkflowEventStore`.

use async_trait::async_trait;
use uuid::Uuid;

use crate::self_checks::SelfCheck;

use crate::durable::{
    ClaimedTask, EventLog, HeartbeatResponse, StoreError, TaskFailureOutcome, TaskQueue,
    WorkerHeartbeat, WorkerInfo, WorkerRegistry, WorkflowError, WorkflowEventStore,
    record_activity_completed, record_activity_failed, record_activity_started,
    record_workflow_failed,
};

/// Each item signals that new work may be claimable. Closure means resubscribe.
pub type TaskWakeups = tokio::sync::mpsc::Receiver<()>;

#[async_trait]
pub trait TaskStore: Send + Sync + 'static {
    async fn register_worker(&self, worker: WorkerInfo) -> Result<(), StoreError>;

    /// Records liveness and load; the reply says whether the worker is
    /// draining, so it can stop claiming.
    async fn worker_heartbeat(
        &self,
        worker_id: &str,
        current_load: usize,
        accepting_tasks: bool,
    ) -> Result<WorkerHeartbeat, StoreError>;

    /// Marks the worker draining: the queue hands it no new tasks. A worker
    /// calls it on itself when it starts shutting down.
    async fn drain_worker(&self, worker_id: &str) -> Result<(), StoreError>;

    async fn deregister_worker(&self, worker_id: &str) -> Result<usize, StoreError>;

    /// Keep the worker's checks on its own setup for the system doctor.
    async fn record_worker_checks(
        &self,
        _worker_id: &str,
        _checks: &[SelfCheck],
    ) -> Result<(), StoreError> {
        Ok(())
    }

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
    ) -> Result<HeartbeatResponse, StoreError>;

    /// Record `ActivityStarted` for a workflow-bound task. A store whose
    /// claim already records it (the control plane, for gRPC workers) does
    /// nothing.
    async fn record_activity_started(&self, task: &ClaimedTask, worker_id: &str);

    async fn complete_task(
        &self,
        task: &ClaimedTask,
        worker_id: &str,
        output: serde_json::Value,
    ) -> Result<(), StoreError>;

    async fn fail_task(
        &self,
        task: &ClaimedTask,
        error: &str,
        retryable: bool,
    ) -> Result<TaskFailureOutcome, StoreError>;

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
    ) -> Result<WorkerHeartbeat, StoreError> {
        WorkerRegistry::worker_heartbeat(self, worker_id, current_load, accepting_tasks).await
    }

    async fn drain_worker(&self, worker_id: &str) -> Result<(), StoreError> {
        WorkerRegistry::drain_worker(self, worker_id).await
    }

    async fn deregister_worker(&self, worker_id: &str) -> Result<usize, StoreError> {
        WorkerRegistry::deregister_worker(self, worker_id).await
    }

    async fn record_worker_checks(
        &self,
        worker_id: &str,
        checks: &[SelfCheck],
    ) -> Result<(), StoreError> {
        let checks = serde_json::to_value(checks)
            .map_err(|error| StoreError::Serialization(error.to_string()))?;
        WorkerRegistry::record_worker_checks(self, worker_id, checks).await
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
    ) -> Result<HeartbeatResponse, StoreError> {
        TaskQueue::heartbeat_task(self, task_id, worker_id, None).await
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

    async fn complete_task(
        &self,
        task: &ClaimedTask,
        worker_id: &str,
        output: serde_json::Value,
    ) -> Result<(), StoreError> {
        TaskQueue::complete_task(self, task.id, worker_id, output.clone()).await?;
        record_activity_completed(self, task.workflow_id, task.activity_id.clone(), output).await;
        Ok(())
    }

    async fn fail_task(
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
}

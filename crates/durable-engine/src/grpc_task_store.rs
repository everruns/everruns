// gRPC-backed TaskStore for standalone workers.
//
// The private durable entry owns both adapters; the worker loop consumes
// the shared TaskStore interface.

use crate::durable::{
    ClaimedTask, HeartbeatResponse, StoreError, TaskFailureOutcome, WorkerInfo, WorkflowError,
    WorkflowStatus,
};
use async_trait::async_trait;
use std::time::Duration;
use uuid::Uuid;

use crate::grpc_durable_store::{GrpcDurableStore, TaskNotificationEvent};
use crate::task_store::{TaskStore, TaskWakeups};

#[async_trait]
impl TaskStore for GrpcDurableStore {
    async fn register_worker(&self, worker: WorkerInfo) -> Result<(), StoreError> {
        let mut store = self.clone();
        GrpcDurableStore::register_worker(
            &mut store,
            &worker.id,
            worker.worker_group,
            worker.activity_types,
            worker.max_concurrency,
        )
        .await
        .map_err(store_error)
    }

    async fn worker_heartbeat(
        &self,
        worker_id: &str,
        current_load: usize,
        accepting_tasks: bool,
    ) -> Result<(), StoreError> {
        let mut store = self.clone();
        GrpcDurableStore::heartbeat_worker(
            &mut store,
            worker_id,
            current_load as u32,
            accepting_tasks,
        )
        .await
        .map_err(store_error)
    }

    async fn deregister_worker(&self, worker_id: &str) -> Result<usize, StoreError> {
        let mut store = self.clone();
        GrpcDurableStore::deregister_worker(&mut store, worker_id)
            .await
            .map_err(store_error)
    }

    async fn claim_task(
        &self,
        worker_id: &str,
        activity_types: &[String],
        max_tasks: usize,
    ) -> Result<Vec<ClaimedTask>, StoreError> {
        let mut store = self.clone();
        GrpcDurableStore::claim_tasks(&mut store, worker_id, activity_types, max_tasks)
            .await
            .map_err(store_error)
    }

    async fn heartbeat_task(
        &self,
        task_id: Uuid,
        worker_id: &str,
        details: Option<serde_json::Value>,
    ) -> Result<HeartbeatResponse, StoreError> {
        let mut store = self.clone();
        let response = GrpcDurableStore::heartbeat_task(&mut store, task_id, worker_id, details)
            .await
            .map_err(store_error)?;
        Ok(HeartbeatResponse {
            accepted: response.acknowledged,
            should_cancel: response.should_cancel,
        })
    }

    async fn get_workflow_status(&self, workflow_id: Uuid) -> Result<WorkflowStatus, StoreError> {
        let mut store = self.clone();
        let (status, _, _) = GrpcDurableStore::get_workflow_status(&mut store, workflow_id)
            .await
            .map_err(store_error)?;
        Ok(grpc_status_to_workflow_status(status))
    }

    async fn record_activity_started(&self, _task: &ClaimedTask, _worker_id: &str) {
        // The control plane owns workflow history for gRPC workers. Claim,
        // complete, fail, and enqueue RPCs record the durable task events.
    }

    async fn complete_task_and_record(
        &self,
        task: &ClaimedTask,
        worker_id: &str,
        output: serde_json::Value,
    ) -> Result<(), StoreError> {
        let mut store = self.clone();
        GrpcDurableStore::complete_task(&mut store, task.id, worker_id, output)
            .await
            .map_err(store_error)
    }

    async fn fail_task_and_record(
        &self,
        task: &ClaimedTask,
        error: &str,
        retryable: bool,
    ) -> Result<TaskFailureOutcome, StoreError> {
        let mut store = self.clone();
        let (will_retry, terminal_failure_owner) =
            GrpcDurableStore::fail_task(&mut store, task.id, error, retryable)
                .await
                .map_err(store_error)?;
        Ok(grpc_task_failure_outcome(
            will_retry,
            terminal_failure_owner,
            task.attempt,
        ))
    }

    async fn enqueue_task_and_record(
        &self,
        workflow_id: Uuid,
        activity_id: String,
        activity_type: String,
        input: serde_json::Value,
    ) -> Result<Uuid, StoreError> {
        let mut store = self.clone();
        GrpcDurableStore::enqueue_task(&mut store, workflow_id, activity_id, activity_type, input)
            .await
            .map_err(store_error)
    }

    async fn update_workflow_status(
        &self,
        workflow_id: Uuid,
        status: WorkflowStatus,
        output: Option<serde_json::Value>,
        error: Option<WorkflowError>,
    ) -> Result<(), StoreError> {
        let mut store = self.clone();
        GrpcDurableStore::update_workflow_status(
            &mut store,
            workflow_id,
            workflow_status_to_grpc_status(status),
            output,
            error.map(|err| err.message),
        )
        .await
        .map_err(store_error)
    }

    async fn complete_workflow(
        &self,
        workflow_id: Uuid,
        _event_output: serde_json::Value,
        stored_output: Option<serde_json::Value>,
        error: Option<WorkflowError>,
    ) -> Result<(), StoreError> {
        self.update_workflow_status(workflow_id, WorkflowStatus::Completed, stored_output, error)
            .await
    }

    async fn consume_pending_signals(
        &self,
        workflow_id: Uuid,
    ) -> Result<Vec<crate::durable::WorkflowSignal>, StoreError> {
        let mut store = self.clone();
        GrpcDurableStore::get_and_consume_signals(&mut store, workflow_id)
            .await
            .map_err(store_error)
    }

    async fn consume_pending_signals_by_type(
        &self,
        workflow_id: Uuid,
        signal_type: &str,
    ) -> Result<Vec<crate::durable::WorkflowSignal>, StoreError> {
        let mut store = self.clone();
        GrpcDurableStore::get_and_consume_signals_by_type(&mut store, workflow_id, signal_type)
            .await
            .map_err(store_error)
    }

    async fn subscribe_task_wakeups(
        &self,
        worker_id: &str,
        activity_types: &[String],
    ) -> Result<Option<TaskWakeups>, StoreError> {
        let mut store = self.clone();
        let mut stream = store
            .subscribe_task_notifications(worker_id, activity_types.to_vec())
            .await
            .map_err(store_error)?;
        let (tx, rx) = tokio::sync::mpsc::channel(1);
        tokio::spawn(async move {
            while let Some(event) = stream.recv().await {
                if tx.is_closed() {
                    return;
                }
                if matches!(event, TaskNotificationEvent::TaskAvailable { .. }) {
                    // Full channel means a wake-up is already pending; one is enough.
                    if let Err(tokio::sync::mpsc::error::TrySendError::Closed(())) = tx.try_send(())
                    {
                        return;
                    }
                }
            }
        });
        Ok(Some(rx))
    }
}

fn grpc_task_failure_outcome(
    will_retry: bool,
    terminal_failure_owner: bool,
    attempt: u32,
) -> TaskFailureOutcome {
    if terminal_failure_owner {
        TaskFailureOutcome::ExhaustedRetries
    } else if will_retry {
        TaskFailureOutcome::WillRetry {
            next_attempt: attempt + 1,
            delay: Duration::ZERO,
        }
    } else {
        TaskFailureOutcome::MovedToDlq
    }
}

fn store_error(error: anyhow::Error) -> StoreError {
    StoreError::Database(error.to_string())
}

fn grpc_status_to_workflow_status(
    status: crate::grpc_durable_store::WorkflowStatus,
) -> WorkflowStatus {
    match status {
        crate::grpc_durable_store::WorkflowStatus::Pending => WorkflowStatus::Pending,
        crate::grpc_durable_store::WorkflowStatus::Running => WorkflowStatus::Running,
        crate::grpc_durable_store::WorkflowStatus::Completed => WorkflowStatus::Completed,
        crate::grpc_durable_store::WorkflowStatus::Failed => WorkflowStatus::Failed,
        crate::grpc_durable_store::WorkflowStatus::Cancelled => WorkflowStatus::Cancelled,
        crate::grpc_durable_store::WorkflowStatus::ContinuedAsNew => WorkflowStatus::ContinuedAsNew,
    }
}

fn workflow_status_to_grpc_status(
    status: WorkflowStatus,
) -> crate::grpc_durable_store::WorkflowStatus {
    match status {
        WorkflowStatus::Pending => crate::grpc_durable_store::WorkflowStatus::Pending,
        WorkflowStatus::Running => crate::grpc_durable_store::WorkflowStatus::Running,
        WorkflowStatus::Completed => crate::grpc_durable_store::WorkflowStatus::Completed,
        WorkflowStatus::Failed => crate::grpc_durable_store::WorkflowStatus::Failed,
        WorkflowStatus::Cancelled => crate::grpc_durable_store::WorkflowStatus::Cancelled,
        WorkflowStatus::ContinuedAsNew => crate::grpc_durable_store::WorkflowStatus::ContinuedAsNew,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grpc_store_preserves_retry_and_terminal_ownership_outcomes() {
        assert!(matches!(
            grpc_task_failure_outcome(true, false, 2),
            TaskFailureOutcome::WillRetry {
                next_attempt: 3,
                ..
            }
        ));
        assert!(matches!(
            grpc_task_failure_outcome(false, true, 5),
            TaskFailureOutcome::ExhaustedRetries
        ));
        assert!(matches!(
            grpc_task_failure_outcome(false, false, 5),
            TaskFailureOutcome::MovedToDlq
        ));
    }
}

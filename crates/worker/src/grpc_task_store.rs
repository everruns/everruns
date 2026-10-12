// gRPC-backed TaskStore for standalone workers.
//
// The worker owns the TaskStore trait (with a blanket impl for every
// WorkflowEventStore) and implements it for its own gRPC client type, so
// coherence holds.
//
// Each call clones the store: a tonic client is a cheap handle over one
// shared channel, and the trait takes `&self` so callers need no lock.

use crate::durable::{
    ClaimedTask, HeartbeatResponse, StoreError, TaskFailureOutcome, WorkerHeartbeat, WorkerInfo,
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
    ) -> Result<WorkerHeartbeat, StoreError> {
        let mut store = self.clone();
        GrpcDurableStore::heartbeat_worker(
            &mut store,
            worker_id,
            current_load as u32,
            accepting_tasks,
        )
        .await
        .map(|draining| WorkerHeartbeat { draining })
        .map_err(store_error)
    }

    async fn drain_worker(&self, worker_id: &str) -> Result<(), StoreError> {
        let mut store = self.clone();
        GrpcDurableStore::drain_worker(&mut store, worker_id)
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
    ) -> Result<HeartbeatResponse, StoreError> {
        let mut store = self.clone();
        let response = GrpcDurableStore::heartbeat_task(&mut store, task_id, worker_id)
            .await
            .map_err(store_error)?;
        Ok(HeartbeatResponse {
            accepted: response.acknowledged,
            should_cancel: response.should_cancel,
        })
    }

    async fn record_activity_started(&self, _task: &ClaimedTask, _worker_id: &str) {
        // The control plane owns workflow history for gRPC workers. Claim,
        // complete, fail, and enqueue RPCs record the durable task events.
    }

    async fn complete_task(
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

    async fn fail_task(
        &self,
        task: &ClaimedTask,
        error: &str,
        retryable: bool,
    ) -> Result<TaskFailureOutcome, StoreError> {
        let mut store = self.clone();
        let will_retry = GrpcDurableStore::fail_task(&mut store, task.id, error, retryable)
            .await
            .map_err(store_error)?;
        Ok(if will_retry {
            TaskFailureOutcome::WillRetry {
                next_attempt: task.attempt + 1,
                delay: Duration::ZERO,
            }
        } else {
            TaskFailureOutcome::MovedToDlq
        })
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

fn store_error(error: anyhow::Error) -> StoreError {
    StoreError::Database(error.to_string())
}

// gRPC-backed TurnStore for standalone workers and their runner.
//
// durable-engine owns the TurnStore trait (with a blanket impl for every
// WorkflowEventStore); the worker implements it for its own gRPC client type,
// so the transport stays out of the engine and coherence holds.
//
// Each call clones the store: a tonic client is a cheap handle over one
// shared channel, and the trait takes `&self` so callers need no lock.

use crate::durable::{
    ClaimedTask, HeartbeatResponse, RunStart, StoreError, TaskFailureOutcome, WorkerInfo,
    WorkflowError, WorkflowSignal, WorkflowStatus,
};
use async_trait::async_trait;
use std::time::Duration;
use uuid::Uuid;

use crate::grpc_durable_store::{GrpcDurableStore, TaskNotificationEvent};
use crate::turn_store::{
    TaskWakeups, TurnHandOff, TurnNext, TurnStore, WorkflowSnapshot, finish_hand_off_in_steps,
};
use everruns_internal_protocol::proto;

#[async_trait]
impl TurnStore for GrpcDurableStore {
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
        GrpcDurableStore::complete_task(&mut store, task.id, worker_id, output, None)
            .await
            .map(|_| ())
            .map_err(store_error)
    }

    async fn complete_task_and_hand_off(
        &self,
        task: &ClaimedTask,
        worker_id: &str,
        output: serde_json::Value,
        hand_off: TurnHandOff,
    ) -> Result<Option<ClaimedTask>, StoreError> {
        let mut store = self.clone();
        let (handed_off, claimed) = GrpcDurableStore::complete_task(
            &mut store,
            task.id,
            worker_id,
            output,
            Some(hand_off_to_proto(&hand_off)),
        )
        .await
        .map_err(store_error)?;
        if handed_off {
            return Ok(claimed);
        }
        // A control plane that predates hand-offs only completed the task.
        finish_hand_off_in_steps(self, hand_off).await
    }

    async fn count_pending_signals(
        &self,
        workflow_id: Uuid,
        signal_type: &str,
    ) -> Result<usize, StoreError> {
        let mut store = self.clone();
        GrpcDurableStore::count_pending_signals(&mut store, workflow_id, signal_type)
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
        GrpcDurableStore::enqueue_task(
            &mut store,
            workflow_id,
            activity_id,
            activity_type,
            input,
            None,
        )
        .await
        .map(|(task_id, _)| task_id)
        .map_err(store_error)
    }

    async fn enqueue_claimed_task_and_record(
        &self,
        workflow_id: Uuid,
        activity_id: String,
        activity_type: String,
        input: serde_json::Value,
        worker_id: &str,
    ) -> Result<Option<ClaimedTask>, StoreError> {
        let mut store = self.clone();
        GrpcDurableStore::enqueue_task(
            &mut store,
            workflow_id,
            activity_id,
            activity_type,
            input,
            Some(worker_id),
        )
        .await
        .map(|(_, claimed)| claimed)
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

    async fn cancel_pending_tasks(&self, _workflow_id: Uuid) -> Result<u64, StoreError> {
        // The control plane cancels a session's turn; a worker never does.
        Ok(0)
    }

    async fn start_turn(
        &self,
        _workflow_id: Uuid,
        _workflow_type: &str,
        _input: serde_json::Value,
        _activity_id: String,
        _activity_type: String,
    ) -> Result<RunStart, StoreError> {
        // Turns start on the control plane, which owns the store. A worker's
        // runner only steers runs that already exist, so it reports one as
        // active and the runner signals it.
        Ok(RunStart::Active)
    }

    async fn get_workflow(&self, workflow_id: Uuid) -> Result<WorkflowSnapshot, StoreError> {
        let mut store = self.clone();
        let (status, output, error) =
            GrpcDurableStore::get_workflow_status(&mut store, workflow_id)
                .await
                .map_err(store_error)?;
        Ok(WorkflowSnapshot {
            status: grpc_status_to_workflow_status(status),
            output,
            error,
        })
    }

    async fn count_active_workflows(&self) -> Result<usize, StoreError> {
        let mut store = self.clone();
        GrpcDurableStore::count_active_workflows(&mut store)
            .await
            .map_err(store_error)
    }

    async fn send_signal(
        &self,
        workflow_id: Uuid,
        signal: WorkflowSignal,
    ) -> Result<(), StoreError> {
        let mut store = self.clone();
        GrpcDurableStore::send_signal(&mut store, workflow_id, signal)
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

fn hand_off_to_proto(hand_off: &TurnHandOff) -> proto::DurableHandOff {
    use everruns_internal_protocol::json_to_proto_struct;
    let next = match &hand_off.next {
        TurnNext::Step {
            activity_id,
            activity_type,
            input,
            claim_for,
        } => proto::durable_hand_off::Next::Step(proto::DurableHandOffStep {
            activity_id: activity_id.clone(),
            activity_type: activity_type.clone(),
            input: Some(json_to_proto_struct(input)),
            claim_for_worker_id: claim_for.clone(),
        }),
        TurnNext::Complete {
            event_output,
            stored_output,
            error,
        } => proto::durable_hand_off::Next::Complete(proto::DurableHandOffComplete {
            event_output: Some(json_to_proto_struct(event_output)),
            stored_output: stored_output.as_ref().map(json_to_proto_struct),
            error: error.as_ref().map(|error| error.message.clone()),
        }),
    };
    proto::DurableHandOff {
        drain_signal_type: hand_off
            .drain
            .as_ref()
            .map(|drain| drain.signal_type.clone()),
        drain_limit: hand_off
            .drain
            .as_ref()
            .map_or(0, |drain| u32::try_from(drain.limit).unwrap_or(u32::MAX)),
        next: Some(next),
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

pub(crate) fn grpc_status_to_workflow_status(
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

pub(crate) fn workflow_status_to_grpc_status(
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

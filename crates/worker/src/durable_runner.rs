//! Standalone-worker gRPC adapter for Scale's neutral durable store contract.

use anyhow::Result;
use async_trait::async_trait;
use everruns_durable::{WorkflowEvent, WorkflowSignal, WorkflowStatus};
use everruns_scale::DurableStoreBackend;
use uuid::Uuid;

use crate::grpc_durable_store::{GrpcDurableStore, WorkflowStatus as GrpcWorkflowStatus};

pub(crate) struct GrpcDurableStoreAdapter {
    store: GrpcDurableStore,
}

impl GrpcDurableStoreAdapter {
    pub(crate) async fn connect(address: &str) -> Result<Self> {
        Ok(Self {
            store: GrpcDurableStore::connect(address).await?,
        })
    }
}

#[async_trait]
impl DurableStoreBackend for GrpcDurableStoreAdapter {
    async fn get_workflow_status(
        &mut self,
        workflow_id: Uuid,
    ) -> Result<(WorkflowStatus, Option<serde_json::Value>, Option<String>)> {
        let (status, output, error) = self.store.get_workflow_status(workflow_id).await?;
        Ok((from_grpc(status), output, error))
    }

    async fn update_workflow_status(
        &mut self,
        workflow_id: Uuid,
        status: WorkflowStatus,
        output: Option<serde_json::Value>,
        error: Option<String>,
    ) -> Result<()> {
        self.store
            .update_workflow_status(workflow_id, to_grpc(status), output, error)
            .await
    }

    async fn enqueue_task(
        &mut self,
        workflow_id: Uuid,
        activity_id: String,
        activity_type: String,
        input: serde_json::Value,
    ) -> Result<Uuid> {
        self.store
            .enqueue_task(workflow_id, activity_id, activity_type, input)
            .await
    }

    async fn start_workflow_with_task(
        &mut self,
        workflow_id: Uuid,
        workflow_type: &str,
        input: serde_json::Value,
        activity_id: String,
        activity_type: String,
    ) -> Result<Uuid> {
        self.store
            .create_workflow(workflow_id, workflow_type, input.clone())
            .await?;
        let task_id = self
            .store
            .enqueue_task(workflow_id, activity_id, activity_type, input)
            .await?;
        self.store
            .update_workflow_status(workflow_id, GrpcWorkflowStatus::Running, None, None)
            .await?;
        Ok(task_id)
    }

    async fn count_active_workflows(&mut self) -> Result<usize> {
        self.store.count_active_workflows().await
    }

    async fn cancel_pending_tasks(&mut self, _workflow_id: Uuid) -> Result<u64> {
        Ok(0)
    }

    async fn append_events(
        &mut self,
        _workflow_id: Uuid,
        expected_sequence: i32,
        _events: Vec<WorkflowEvent>,
    ) -> Result<i32> {
        Ok(expected_sequence)
    }

    async fn try_claim_workflow_for_new_turn(&mut self, _workflow_id: Uuid) -> Result<bool> {
        Ok(false)
    }

    async fn send_signal(&mut self, workflow_id: Uuid, signal: WorkflowSignal) -> Result<()> {
        self.store.send_signal(workflow_id, signal).await
    }

    async fn get_and_consume_signals(&mut self, workflow_id: Uuid) -> Result<Vec<WorkflowSignal>> {
        self.store.get_and_consume_signals(workflow_id).await
    }
}

fn from_grpc(status: GrpcWorkflowStatus) -> WorkflowStatus {
    match status {
        GrpcWorkflowStatus::Pending => WorkflowStatus::Pending,
        GrpcWorkflowStatus::Running => WorkflowStatus::Running,
        GrpcWorkflowStatus::Completed => WorkflowStatus::Completed,
        GrpcWorkflowStatus::Failed => WorkflowStatus::Failed,
        GrpcWorkflowStatus::Cancelled => WorkflowStatus::Cancelled,
        GrpcWorkflowStatus::ContinuedAsNew => WorkflowStatus::ContinuedAsNew,
    }
}

fn to_grpc(status: WorkflowStatus) -> GrpcWorkflowStatus {
    match status {
        WorkflowStatus::Pending => GrpcWorkflowStatus::Pending,
        WorkflowStatus::Running => GrpcWorkflowStatus::Running,
        WorkflowStatus::Completed => GrpcWorkflowStatus::Completed,
        WorkflowStatus::Failed => GrpcWorkflowStatus::Failed,
        WorkflowStatus::Cancelled => GrpcWorkflowStatus::Cancelled,
        WorkflowStatus::ContinuedAsNew => GrpcWorkflowStatus::ContinuedAsNew,
    }
}

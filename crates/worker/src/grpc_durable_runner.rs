// gRPC durable runner: the worker's control-plane transport for `DurableRunner`.
//
// Decision: durable-engine owns `DurableRunner` and the `DurableStoreBackend`
// trait but carries no transport. The worker owns `GrpcDurableStore`, so it
// implements that trait here for its own type (no orphan or blanket-impl
// overlap) and provides the gRPC constructors the engine used to carry.

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use tracing::info;
use uuid::Uuid;

use crate::core::config::env_string_any;
use crate::durable::{WorkflowEvent, WorkflowSignal, WorkflowStatus};
use crate::durable_runner::{DurableRunner, DurableStoreBackend};
use crate::grpc_durable_store::GrpcDurableStore;
use crate::grpc_task_store::{grpc_status_to_workflow_status, workflow_status_to_grpc_status};
use crate::runner::AgentRunner;

/// Connect a durable runner to the control plane's gRPC durable service.
pub async fn connect_grpc_durable_runner(grpc_address: &str) -> Result<DurableRunner> {
    info!(
        grpc_address = %grpc_address,
        "Initializing durable runner (gRPC mode)"
    );
    let store = GrpcDurableStore::connect(grpc_address).await?;
    Ok(DurableRunner::from_store(store))
}

/// Connect a durable runner to the control plane named by
/// `SERVER_GRPC_ADDRESS` / `WORKER_GRPC_ADDRESS` (default `127.0.0.1:9001`).
pub async fn grpc_durable_runner_from_env() -> Result<DurableRunner> {
    let grpc_address = env_string_any(
        &["SERVER_GRPC_ADDRESS", "WORKER_GRPC_ADDRESS"],
        "127.0.0.1:9001",
    );
    connect_grpc_durable_runner(&grpc_address).await
}

/// Create an agent runner.
///
/// Pass a database pool for direct access (control-plane) or `None` for the
/// gRPC control-plane transport (workers).
pub async fn create_runner(
    db_pool: Option<crate::durable::PostgresPool>,
) -> Result<Arc<dyn AgentRunner>> {
    if let Some(pool) = db_pool {
        tracing::info!("Creating Durable execution engine runner (direct DB mode)");
        Ok(Arc::new(DurableRunner::new_with_pool(pool)))
    } else {
        tracing::info!("Creating Durable execution engine runner (gRPC mode)");
        let runner = grpc_durable_runner_from_env().await?;
        Ok(Arc::new(runner))
    }
}

#[async_trait]
impl DurableStoreBackend for GrpcDurableStore {
    async fn get_workflow_status(
        &mut self,
        workflow_id: Uuid,
    ) -> Result<(WorkflowStatus, Option<serde_json::Value>, Option<String>)> {
        let (status, output, error) =
            GrpcDurableStore::get_workflow_status(self, workflow_id).await?;
        Ok((grpc_status_to_workflow_status(status), output, error))
    }

    async fn create_workflow(
        &mut self,
        workflow_id: Uuid,
        workflow_type: &str,
        input: serde_json::Value,
    ) -> Result<Uuid> {
        GrpcDurableStore::create_workflow(self, workflow_id, workflow_type, input).await
    }

    async fn update_workflow_status(
        &mut self,
        workflow_id: Uuid,
        status: WorkflowStatus,
        output: Option<serde_json::Value>,
        error: Option<String>,
    ) -> Result<()> {
        GrpcDurableStore::update_workflow_status(
            self,
            workflow_id,
            workflow_status_to_grpc_status(status),
            output,
            error,
        )
        .await
    }

    async fn enqueue_task(
        &mut self,
        workflow_id: Uuid,
        activity_id: String,
        activity_type: String,
        input: serde_json::Value,
    ) -> Result<Uuid> {
        GrpcDurableStore::enqueue_task(self, workflow_id, activity_id, activity_type, input).await
    }

    async fn start_workflow_with_task(
        &mut self,
        workflow_id: Uuid,
        workflow_type: &str,
        input: serde_json::Value,
        activity_id: String,
        activity_type: String,
    ) -> Result<Uuid> {
        GrpcDurableStore::create_workflow(self, workflow_id, workflow_type, input.clone()).await?;
        let _ = self
            .append_events(workflow_id, 0, vec![WorkflowEvent::started(input.clone())])
            .await?;
        let task_id = GrpcDurableStore::enqueue_task(
            self,
            workflow_id,
            activity_id.clone(),
            activity_type.clone(),
            input.clone(),
        )
        .await?;
        GrpcDurableStore::update_workflow_status(
            self,
            workflow_id,
            workflow_status_to_grpc_status(WorkflowStatus::Running),
            None,
            None,
        )
        .await?;
        let _ = self
            .append_events(
                workflow_id,
                1,
                vec![WorkflowEvent::ActivityScheduled {
                    activity_id,
                    activity_type,
                    input,
                    options: crate::durable::ActivityOptions::default(),
                }],
            )
            .await?;
        Ok(task_id)
    }

    async fn count_active_workflows(&mut self) -> Result<usize> {
        GrpcDurableStore::count_active_workflows(self).await
    }

    async fn cancel_pending_tasks(&mut self, _workflow_id: Uuid) -> Result<u64> {
        Ok(0)
    }

    async fn append_events(
        &mut self,
        _workflow_id: Uuid,
        _expected_sequence: i32,
        _events: Vec<WorkflowEvent>,
    ) -> Result<i32> {
        Ok(0)
    }

    async fn try_claim_workflow_for_new_turn(&mut self, _workflow_id: Uuid) -> Result<bool> {
        Ok(false)
    }

    async fn send_signal(&mut self, workflow_id: Uuid, signal: WorkflowSignal) -> Result<()> {
        GrpcDurableStore::send_signal(self, workflow_id, signal).await
    }

    async fn get_and_consume_signals(&mut self, workflow_id: Uuid) -> Result<Vec<WorkflowSignal>> {
        GrpcDurableStore::get_and_consume_signals(self, workflow_id).await
    }
}

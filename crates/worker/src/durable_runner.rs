// Durable execution engine runner
// Decision: Use custom PostgreSQL-backed durable engine for workflow orchestration
// Decision: AgentRunner interface for clean abstraction
// Decision: Workers communicate with control-plane via gRPC (no direct DB access)
// Decision: Control-plane uses direct database access (PostgresWorkflowEventStore)

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::info;
use uuid::Uuid;

use crate::grpc_durable_store::{GrpcDurableStore, WorkflowStatus};
use crate::runner::AgentRunner;
use everruns_durable::{
    InMemoryWorkflowEventStore, PostgresWorkflowEventStore, WorkflowEventStore,
};

// =============================================================================
// TurnWorkflow Input/Output
// =============================================================================

/// Input for the turn workflow
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DurableTurnInput {
    pub session_id: Uuid,
    pub agent_id: Uuid,
    pub input_message_id: Uuid,
}

/// Output from the turn workflow
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DurableTurnOutput {
    pub session_id: Uuid,
    pub success: bool,
    pub error: Option<String>,
}

// =============================================================================
// DurableStoreBackend Trait
// =============================================================================

/// Backend trait for durable store operations
/// Allows switching between gRPC (for workers) and direct DB (for control-plane)
#[async_trait]
pub trait DurableStoreBackend: Send + Sync {
    async fn get_workflow_status(
        &mut self,
        workflow_id: Uuid,
    ) -> Result<(WorkflowStatus, Option<serde_json::Value>, Option<String>)>;

    async fn create_workflow(
        &mut self,
        workflow_id: Uuid,
        workflow_type: &str,
        input: serde_json::Value,
    ) -> Result<Uuid>;

    async fn update_workflow_status(
        &mut self,
        workflow_id: Uuid,
        status: WorkflowStatus,
        output: Option<serde_json::Value>,
        error: Option<String>,
    ) -> Result<()>;

    async fn enqueue_task(
        &mut self,
        workflow_id: Uuid,
        activity_id: String,
        activity_type: String,
        input: serde_json::Value,
    ) -> Result<Uuid>;

    async fn count_active_workflows(&mut self) -> Result<usize>;
}

// =============================================================================
// GrpcDurableStore Backend Implementation
// =============================================================================

#[async_trait]
impl DurableStoreBackend for GrpcDurableStore {
    async fn get_workflow_status(
        &mut self,
        workflow_id: Uuid,
    ) -> Result<(WorkflowStatus, Option<serde_json::Value>, Option<String>)> {
        GrpcDurableStore::get_workflow_status(self, workflow_id).await
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
        GrpcDurableStore::update_workflow_status(self, workflow_id, status, output, error).await
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

    async fn count_active_workflows(&mut self) -> Result<usize> {
        GrpcDurableStore::count_active_workflows(self).await
    }
}

// =============================================================================
// DirectDurableStore - wraps PostgresWorkflowEventStore for control-plane use
// =============================================================================

/// Direct database store for control-plane use
pub struct DirectDurableStore {
    store: PostgresWorkflowEventStore,
}

impl DirectDurableStore {
    pub fn new(pool: sqlx::PgPool) -> Self {
        Self {
            store: PostgresWorkflowEventStore::new(pool),
        }
    }
}

#[async_trait]
impl DurableStoreBackend for DirectDurableStore {
    async fn get_workflow_status(
        &mut self,
        workflow_id: Uuid,
    ) -> Result<(WorkflowStatus, Option<serde_json::Value>, Option<String>)> {
        let info = self.store.get_workflow_info(workflow_id).await?;
        Ok((
            durable_to_local_status(info.status),
            info.result,
            info.error.map(|e| format!("{:?}", e)),
        ))
    }

    async fn create_workflow(
        &mut self,
        workflow_id: Uuid,
        workflow_type: &str,
        input: serde_json::Value,
    ) -> Result<Uuid> {
        self.store
            .create_workflow(workflow_id, workflow_type, input, None)
            .await?;
        Ok(workflow_id)
    }

    async fn update_workflow_status(
        &mut self,
        workflow_id: Uuid,
        status: WorkflowStatus,
        output: Option<serde_json::Value>,
        error: Option<String>,
    ) -> Result<()> {
        let durable_status = local_to_durable_status(status);
        let durable_error = error.map(everruns_durable::WorkflowError::new);
        self.store
            .update_workflow_status(workflow_id, durable_status, output, durable_error)
            .await?;
        Ok(())
    }

    async fn enqueue_task(
        &mut self,
        workflow_id: Uuid,
        activity_id: String,
        activity_type: String,
        input: serde_json::Value,
    ) -> Result<Uuid> {
        let task = everruns_durable::TaskDefinition {
            workflow_id,
            activity_id,
            activity_type,
            input,
            options: everruns_durable::ActivityOptions::default(),
        };
        self.store.enqueue_task(task).await.map_err(Into::into)
    }

    async fn count_active_workflows(&mut self) -> Result<usize> {
        self.store
            .count_active_workflows()
            .await
            .map(|c| c as usize)
            .map_err(Into::into)
    }
}

// =============================================================================
// InMemoryDurableStore - wraps InMemoryWorkflowEventStore for dev mode
// =============================================================================

/// In-memory store for dev mode (no PostgreSQL required)
pub struct InMemoryDurableStore {
    store: Arc<InMemoryWorkflowEventStore>,
}

impl InMemoryDurableStore {
    pub fn new() -> Self {
        Self {
            store: Arc::new(InMemoryWorkflowEventStore::new()),
        }
    }

    /// Get a reference to the underlying store (for sharing between components)
    pub fn store(&self) -> Arc<InMemoryWorkflowEventStore> {
        Arc::clone(&self.store)
    }
}

impl Default for InMemoryDurableStore {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl DurableStoreBackend for InMemoryDurableStore {
    async fn get_workflow_status(
        &mut self,
        workflow_id: Uuid,
    ) -> Result<(WorkflowStatus, Option<serde_json::Value>, Option<String>)> {
        let info = self.store.get_workflow_info(workflow_id).await?;
        Ok((
            durable_to_local_status(info.status),
            info.result,
            info.error.map(|e| format!("{:?}", e)),
        ))
    }

    async fn create_workflow(
        &mut self,
        workflow_id: Uuid,
        workflow_type: &str,
        input: serde_json::Value,
    ) -> Result<Uuid> {
        self.store
            .create_workflow(workflow_id, workflow_type, input, None)
            .await?;
        Ok(workflow_id)
    }

    async fn update_workflow_status(
        &mut self,
        workflow_id: Uuid,
        status: WorkflowStatus,
        output: Option<serde_json::Value>,
        error: Option<String>,
    ) -> Result<()> {
        let durable_status = local_to_durable_status(status);
        let durable_error = error.map(everruns_durable::WorkflowError::new);
        self.store
            .update_workflow_status(workflow_id, durable_status, output, durable_error)
            .await?;
        Ok(())
    }

    async fn enqueue_task(
        &mut self,
        workflow_id: Uuid,
        activity_id: String,
        activity_type: String,
        input: serde_json::Value,
    ) -> Result<Uuid> {
        let task = everruns_durable::TaskDefinition {
            workflow_id,
            activity_id,
            activity_type,
            input,
            options: everruns_durable::ActivityOptions::default(),
        };
        self.store.enqueue_task(task).await.map_err(Into::into)
    }

    async fn count_active_workflows(&mut self) -> Result<usize> {
        // In-memory store doesn't have count_active_workflows, return workflow count
        Ok(self.store.workflow_count())
    }
}

fn durable_to_local_status(s: everruns_durable::WorkflowStatus) -> WorkflowStatus {
    match s {
        everruns_durable::WorkflowStatus::Pending => WorkflowStatus::Pending,
        everruns_durable::WorkflowStatus::Running => WorkflowStatus::Running,
        everruns_durable::WorkflowStatus::Completed => WorkflowStatus::Completed,
        everruns_durable::WorkflowStatus::Failed => WorkflowStatus::Failed,
        everruns_durable::WorkflowStatus::Cancelled => WorkflowStatus::Cancelled,
    }
}

fn local_to_durable_status(s: WorkflowStatus) -> everruns_durable::WorkflowStatus {
    match s {
        WorkflowStatus::Pending => everruns_durable::WorkflowStatus::Pending,
        WorkflowStatus::Running => everruns_durable::WorkflowStatus::Running,
        WorkflowStatus::Completed => everruns_durable::WorkflowStatus::Completed,
        WorkflowStatus::Failed => everruns_durable::WorkflowStatus::Failed,
        WorkflowStatus::Cancelled => everruns_durable::WorkflowStatus::Cancelled,
    }
}

// =============================================================================
// DurableRunner Implementation
// =============================================================================

/// Durable execution engine based runner
///
/// This runner uses the custom durable engine backed by PostgreSQL
/// for workflow orchestration.
/// - Workers communicate with the control-plane via gRPC
/// - Control-plane uses direct database access
pub struct DurableRunner {
    store: Arc<Mutex<dyn DurableStoreBackend>>,
}

impl DurableRunner {
    /// Create a new durable runner connected to control-plane gRPC
    /// Used by workers that connect to the control-plane
    pub async fn new(grpc_address: &str) -> Result<Self> {
        info!(
            grpc_address = %grpc_address,
            "Initializing Durable execution engine runner (gRPC mode)"
        );

        let store = GrpcDurableStore::connect(grpc_address).await?;

        info!("Durable runner initialized");

        Ok(Self {
            store: Arc::new(Mutex::new(store)),
        })
    }

    /// Create a new durable runner with direct database access
    /// Used by the control-plane which has direct database access
    pub fn new_with_pool(pool: sqlx::PgPool) -> Self {
        info!("Initializing Durable execution engine runner (direct DB mode)");

        let store = DirectDurableStore::new(pool);

        Self {
            store: Arc::new(Mutex::new(store)),
        }
    }

    /// Create a new durable runner with in-memory storage
    /// Used by the control-plane in dev mode (no PostgreSQL required)
    pub fn new_in_memory() -> Self {
        info!("Initializing Durable execution engine runner (in-memory dev mode)");

        let store = InMemoryDurableStore::new();

        Self {
            store: Arc::new(Mutex::new(store)),
        }
    }

    /// Create from GRPC_ADDRESS environment variable (defaults to 127.0.0.1:9001)
    /// Used by workers
    pub async fn from_env() -> Result<Self> {
        let grpc_address =
            std::env::var("GRPC_ADDRESS").unwrap_or_else(|_| "127.0.0.1:9001".to_string());

        Self::new(&grpc_address).await
    }
}

#[async_trait]
impl AgentRunner for DurableRunner {
    /// Start a turn workflow for the given session
    async fn start_run(
        &self,
        session_id: Uuid,
        agent_id: Uuid,
        input_message_id: Uuid,
    ) -> Result<()> {
        info!(
            session_id = %session_id,
            agent_id = %agent_id,
            input_message_id = %input_message_id,
            "Starting durable turn workflow for session"
        );

        // Build workflow input
        let input = DurableTurnInput {
            session_id,
            agent_id,
            input_message_id,
        };

        // Create workflow instance
        // Use session_id as workflow_id for consistency
        let workflow_id = session_id;
        let input_json = serde_json::to_value(&input)?;

        let mut store = self.store.lock().await;

        // Check if workflow already exists (idempotency)
        match store.get_workflow_status(workflow_id).await {
            Ok((status, _, _)) => {
                if !status.is_terminal() {
                    info!(
                        session_id = %session_id,
                        workflow_id = %workflow_id,
                        status = ?status,
                        "Workflow already running, skipping creation"
                    );
                    return Ok(());
                }
            }
            Err(e) => {
                // Check if it's a not found error (expected for new workflows)
                let err_str = e.to_string();
                if !err_str.contains("not found") && !err_str.contains("NOT_FOUND") {
                    return Err(anyhow::anyhow!("Failed to check workflow status: {}", e));
                }
            }
        }

        // Create new workflow
        store
            .create_workflow(workflow_id, "turn_workflow", input_json.clone())
            .await
            .map_err(|e| anyhow::anyhow!("Failed to create workflow: {}", e))?;

        // Enqueue the initial activity (input processing)
        store
            .enqueue_task(
                workflow_id,
                format!("input_{}", Uuid::now_v7()),
                "process_input".to_string(),
                input_json,
            )
            .await
            .map_err(|e| anyhow::anyhow!("Failed to enqueue task: {}", e))?;

        info!(
            session_id = %session_id,
            workflow_id = %workflow_id,
            "Durable workflow created and input task enqueued"
        );

        Ok(())
    }

    async fn cancel_run(&self, session_id: Uuid) -> Result<()> {
        info!(session_id = %session_id, "Cancelling durable workflow");

        let workflow_id = session_id;
        let mut store = self.store.lock().await;

        // Update workflow status to cancelled
        store
            .update_workflow_status(
                workflow_id,
                WorkflowStatus::Cancelled,
                None,
                Some("User requested cancellation".to_string()),
            )
            .await
            .map_err(|e| anyhow::anyhow!("Failed to cancel workflow: {}", e))?;

        info!(
            session_id = %session_id,
            workflow_id = %workflow_id,
            "Workflow cancelled"
        );

        Ok(())
    }

    async fn is_running(&self, session_id: Uuid) -> bool {
        let workflow_id = session_id;
        let mut store = self.store.lock().await;

        match store.get_workflow_status(workflow_id).await {
            Ok((status, _, _)) => !status.is_terminal(),
            Err(_) => false,
        }
    }

    async fn active_count(&self) -> usize {
        let mut store = self.store.lock().await;
        store.count_active_workflows().await.unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_durable_turn_input_serialization() {
        let input = DurableTurnInput {
            session_id: Uuid::now_v7(),
            agent_id: Uuid::now_v7(),
            input_message_id: Uuid::now_v7(),
        };

        let json = serde_json::to_string(&input).unwrap();
        let parsed: DurableTurnInput = serde_json::from_str(&json).unwrap();

        assert_eq!(input.session_id, parsed.session_id);
        assert_eq!(input.agent_id, parsed.agent_id);
        assert_eq!(input.input_message_id, parsed.input_message_id);
    }
}

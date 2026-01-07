// Durable execution engine runner
// Decision: Use the custom durable engine as an alternative to Temporal
// Decision: Same AgentRunner interface for seamless switching
// Decision: Workers communicate with control-plane via gRPC (no direct DB access)

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::info;
use uuid::Uuid;

use crate::grpc_durable_store::{GrpcDurableStore, WorkflowStatus};
use crate::runner::AgentRunner;

// =============================================================================
// TurnWorkflow Input/Output
// =============================================================================

/// Input for the turn workflow (same as Temporal version)
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
// DurableRunner Implementation
// =============================================================================

/// Durable execution engine based runner
///
/// This runner uses the custom durable engine backed by PostgreSQL
/// instead of Temporal for workflow orchestration.
/// Workers communicate with the control-plane via gRPC for all database operations.
pub struct DurableRunner {
    store: Arc<Mutex<GrpcDurableStore>>,
}

impl DurableRunner {
    /// Create a new durable runner connected to control-plane gRPC
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

    /// Create from GRPC_ADDRESS environment variable (defaults to 127.0.0.1:9001)
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
        // Use session_id as workflow_id for consistency with Temporal approach
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

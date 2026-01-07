// Durable execution engine runner
// Decision: Use the custom durable engine as an alternative to Temporal
// Decision: Same AgentRunner interface for seamless switching

use anyhow::Result;
use async_trait::async_trait;
use everruns_durable::{
    ActivityOptions, PostgresWorkflowEventStore, StoreError, TaskDefinition, WorkflowEventStore,
    WorkflowSignal,
};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::sync::Arc;
use tracing::info;
use uuid::Uuid;

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
pub struct DurableRunner {
    store: Arc<PostgresWorkflowEventStore>,
}

impl DurableRunner {
    /// Create a new durable runner connected to PostgreSQL
    pub async fn new(pool: PgPool) -> Result<Self> {
        info!("Initializing Durable execution engine runner");

        let store = PostgresWorkflowEventStore::new(pool);

        info!("Durable runner initialized");

        Ok(Self {
            store: Arc::new(store),
        })
    }

    /// Create from DATABASE_URL environment variable
    pub async fn from_env() -> Result<Self> {
        let database_url = std::env::var("DATABASE_URL")
            .map_err(|_| anyhow::anyhow!("DATABASE_URL environment variable required"))?;

        let pool = PgPool::connect(&database_url).await?;
        Self::new(pool).await
    }

    /// Get the workflow store for external access
    pub fn store(&self) -> Arc<PostgresWorkflowEventStore> {
        self.store.clone()
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

        // Check if workflow already exists (idempotency)
        match self.store.get_workflow_status(workflow_id).await {
            Ok(status) => {
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
            Err(StoreError::WorkflowNotFound(_)) => {
                // Expected for new workflows
            }
            Err(e) => {
                return Err(anyhow::anyhow!("Failed to check workflow status: {}", e));
            }
        }

        // Create new workflow
        self.store
            .create_workflow(workflow_id, "turn_workflow", input_json.clone(), None)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to create workflow: {}", e))?;

        // Enqueue the initial activity (input processing)
        let task_def = TaskDefinition {
            workflow_id,
            activity_id: format!("input_{}", Uuid::now_v7()),
            activity_type: "process_input".to_string(),
            input: input_json,
            options: ActivityOptions::default(),
        };

        self.store
            .enqueue_task(task_def)
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

        // Send cancel signal
        self.store
            .send_signal(
                workflow_id,
                WorkflowSignal::cancel("User requested cancellation"),
            )
            .await
            .map_err(|e| anyhow::anyhow!("Failed to send cancel signal: {}", e))?;

        info!(
            session_id = %session_id,
            workflow_id = %workflow_id,
            "Workflow cancellation signal sent"
        );

        Ok(())
    }

    async fn is_running(&self, session_id: Uuid) -> bool {
        let workflow_id = session_id;

        match self.store.get_workflow_status(workflow_id).await {
            Ok(status) => !status.is_terminal(),
            Err(_) => false,
        }
    }

    async fn active_count(&self) -> usize {
        // Query count of running workflows
        match self.store.count_active_workflows().await {
            Ok(count) => count as usize,
            Err(_) => 0,
        }
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

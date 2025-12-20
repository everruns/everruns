// Agent runner for workflow execution
// Decision: Use trait-based abstraction for workflow execution
// Decision: Use true Temporal workflows for durable, distributed execution
//
// Architecture:
// - API calls `start_run` which queues a workflow to Temporal
// - Worker polls Temporal task queues and executes activities
// - Each activity (load_agent, call_llm, execute_tools) is idempotent
// - Events are persisted to database for SSE streaming to clients
//
// M2 Note: run_id maps to session_id, agent_id maps to harness_id

use anyhow::Result;
use async_trait::async_trait;
use everruns_storage::repositories::Database;
use std::sync::Arc;
use tracing::{error, info};
use uuid::Uuid;

use crate::client::TemporalClient;
use crate::types::SessionWorkflowInput;
use crate::worker::TemporalWorker;

// =============================================================================
// Configuration
// =============================================================================

/// Configuration for the agent runner (Temporal)
#[derive(Debug, Clone, Default)]
pub struct RunnerConfig {
    /// Temporal server address
    pub temporal_address: Option<String>,
    /// Temporal namespace
    pub temporal_namespace: Option<String>,
    /// Temporal task queue
    pub temporal_task_queue: Option<String>,
}

impl RunnerConfig {
    /// Create configuration from environment variables
    pub fn from_env() -> Self {
        Self {
            temporal_address: std::env::var("TEMPORAL_ADDRESS").ok(),
            temporal_namespace: std::env::var("TEMPORAL_NAMESPACE").ok(),
            temporal_task_queue: std::env::var("TEMPORAL_TASK_QUEUE").ok(),
        }
    }

    /// Get Temporal address with default
    pub fn temporal_address(&self) -> String {
        self.temporal_address
            .clone()
            .unwrap_or_else(|| "localhost:7233".to_string())
    }

    /// Get Temporal namespace with default
    pub fn temporal_namespace(&self) -> String {
        self.temporal_namespace
            .clone()
            .unwrap_or_else(|| "default".to_string())
    }

    /// Get Temporal task queue with default
    pub fn temporal_task_queue(&self) -> String {
        self.temporal_task_queue
            .clone()
            .unwrap_or_else(|| "everruns-agent-runs".to_string())
    }
}

// =============================================================================
// AgentRunner Trait
// =============================================================================

/// Trait for agent workflow execution
/// Implementations handle the actual execution of agent runs
///
/// M2 Note: Parameters map to session concepts:
/// - run_id -> session_id
/// - agent_id -> harness_id
/// - thread_id -> session_id (same value, kept for backwards compatibility)
#[async_trait]
pub trait AgentRunner: Send + Sync {
    /// Start a new workflow execution for the given session
    async fn start_run(&self, run_id: Uuid, agent_id: Uuid, thread_id: Uuid) -> Result<()>;

    /// Cancel a running workflow
    async fn cancel_run(&self, run_id: Uuid) -> Result<()>;

    /// Check if a workflow is still running
    async fn is_running(&self, run_id: Uuid) -> bool;

    /// Get count of active workflows (for monitoring)
    async fn active_count(&self) -> usize;
}

// =============================================================================
// TemporalRunner Implementation
// =============================================================================

/// Temporal-based session runner using true Temporal workflows
///
/// This runner connects to a Temporal server and starts workflows.
/// Actual execution happens in the worker process that polls for tasks.
pub struct TemporalRunner {
    /// Temporal client for starting workflows
    client: Arc<TemporalClient>,
    /// Database connection (for recording workflow IDs)
    db: Database,
}

impl TemporalRunner {
    /// Create a new Temporal runner connected to the server
    pub async fn new(config: RunnerConfig, db: Database) -> Result<Self> {
        info!(
            address = %config.temporal_address(),
            namespace = %config.temporal_namespace(),
            task_queue = %config.temporal_task_queue(),
            "Initializing Temporal runner"
        );

        let client = TemporalClient::new(config).await?;

        info!("Temporal runner initialized");

        Ok(Self {
            client: Arc::new(client),
            db,
        })
    }
}

#[async_trait]
impl AgentRunner for TemporalRunner {
    /// Start a session workflow
    /// In M2: run_id = session_id, agent_id = agent_id, thread_id = session_id
    async fn start_run(&self, session_id: Uuid, agent_id: Uuid, _thread_id: Uuid) -> Result<()> {
        info!(
            session_id = %session_id,
            agent_id = %agent_id,
            "Starting Temporal workflow for session"
        );

        // Build workflow input
        let input = SessionWorkflowInput {
            session_id,
            agent_id,
        };

        // Start the workflow on Temporal server
        let response = self.client.start_session_workflow(&input).await?;

        // Workflow ID is derived from session_id (session-{session_id})
        let workflow_id = TemporalClient::workflow_id_for_session(session_id);

        info!(
            session_id = %session_id,
            workflow_id = %workflow_id,
            temporal_run_id = %response.run_id,
            "Temporal workflow started successfully"
        );

        Ok(())
    }

    async fn cancel_run(&self, session_id: Uuid) -> Result<()> {
        info!(session_id = %session_id, "Cancelling Temporal workflow");

        // TODO: Implement workflow cancellation via Temporal API
        // For now, we'll use signal-based cancellation when supported

        let workflow_id = TemporalClient::workflow_id_for_session(session_id);
        info!(
            session_id = %session_id,
            workflow_id = %workflow_id,
            "Workflow cancellation requested (not yet implemented)"
        );

        Ok(())
    }

    async fn is_running(&self, session_id: Uuid) -> bool {
        // Check with database since workflow state is persisted there
        match self.db.get_session(session_id).await {
            Ok(Some(session)) => session.status == "running",
            _ => false,
        }
    }

    async fn active_count(&self) -> usize {
        // Would need to query Temporal for accurate count
        // For now, return 0 as this is informational
        0
    }
}

// =============================================================================
// Factory Functions
// =============================================================================

/// Create the Temporal agent runner
pub async fn create_runner(config: &RunnerConfig, db: Database) -> Result<Arc<dyn AgentRunner>> {
    tracing::info!(
        address = %config.temporal_address(),
        namespace = %config.temporal_namespace(),
        task_queue = %config.temporal_task_queue(),
        "Creating Temporal agent runner"
    );
    let runner = TemporalRunner::new(config.clone(), db).await?;
    Ok(Arc::new(runner))
}

/// Run the Temporal worker
///
/// This function starts the worker that polls Temporal for tasks and executes activities.
/// It should be run in a separate process from the API.
pub async fn run_worker(config: &RunnerConfig, db: Database) -> Result<()> {
    info!(
        address = %config.temporal_address(),
        namespace = %config.temporal_namespace(),
        task_queue = %config.temporal_task_queue(),
        "Starting Temporal worker"
    );

    let worker = TemporalWorker::new(config.clone(), db).await?;

    info!("Temporal worker started, polling for tasks...");

    // Run the worker (blocks until shutdown)
    if let Err(e) = worker.run().await {
        error!(error = %e, "Temporal worker error");
        return Err(e);
    }

    info!("Temporal worker shut down");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_workflow_id_format() {
        let session_id = Uuid::now_v7();
        let workflow_id = TemporalClient::workflow_id_for_session(session_id);
        assert!(workflow_id.starts_with("session-"));
        assert!(workflow_id.contains(&session_id.to_string()));
    }
}

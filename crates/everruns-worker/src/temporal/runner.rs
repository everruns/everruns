// Temporal-based session runner (M2)
// Decision: Use true Temporal workflows for durable, distributed execution
//
// Architecture:
// - API calls `start_run` which queues a workflow to Temporal
// - Worker polls Temporal task queues and executes activities
// - Each activity (load_agent, call_llm, execute_tools) is idempotent
// - Events are persisted to database for SSE streaming to clients

use anyhow::Result;
use async_trait::async_trait;
use everruns_storage::repositories::Database;
use std::sync::Arc;
use tracing::{error, info};
use uuid::Uuid;

use crate::runner::{AgentRunner, RunnerConfig};

use super::client::TemporalClient;
use super::types::SessionWorkflowInput;
use super::worker::TemporalWorker;

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

/// Run the Temporal worker
///
/// This function starts the worker that polls Temporal for tasks and executes activities.
/// It should be run in a separate process from the API.
pub async fn run_temporal_worker(config: &RunnerConfig, db: Database) -> Result<()> {
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

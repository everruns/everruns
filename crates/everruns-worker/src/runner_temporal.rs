// Temporal-based agent runner
// Decision: Use true Temporal workflows for durable, distributed execution
//
// Architecture:
// - API calls `start_run` which queues a workflow to Temporal
// - Worker polls Temporal task queues and executes activities
// - Each activity (load_agent, call_llm, execute_tools) is idempotent
// - Events are persisted to database for SSE streaming to clients
//
// Note: This module is conditionally compiled via #[cfg(feature = "temporal")] in lib.rs

use anyhow::Result;
use async_trait::async_trait;
use everruns_storage::models::UpdateRun;
use everruns_storage::repositories::Database;
use std::sync::Arc;
use tracing::{error, info};
use uuid::Uuid;

use crate::runner::{AgentRunner, RunnerConfig};
use crate::temporal_client::TemporalClient;
use crate::temporal_types::AgentRunWorkflowInput;
use crate::temporal_worker::TemporalWorker;

/// Temporal-based agent runner using true Temporal workflows
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
    async fn start_run(&self, run_id: Uuid, agent_id: Uuid, thread_id: Uuid) -> Result<()> {
        info!(
            run_id = %run_id,
            agent_id = %agent_id,
            thread_id = %thread_id,
            "Starting Temporal workflow"
        );

        // Build workflow input
        let input = AgentRunWorkflowInput {
            run_id,
            agent_id,
            thread_id,
        };

        // Start the workflow on Temporal server
        let response = self.client.start_agent_run_workflow(&input).await?;

        // Record Temporal workflow ID for observability
        let workflow_id = TemporalClient::workflow_id_for_run(run_id);
        let update = UpdateRun {
            status: None,
            temporal_workflow_id: Some(workflow_id.clone()),
            temporal_run_id: Some(response.run_id.clone()),
            started_at: None,
            finished_at: None,
        };
        self.db.update_run(run_id, update).await?;

        info!(
            run_id = %run_id,
            workflow_id = %workflow_id,
            temporal_run_id = %response.run_id,
            "Temporal workflow started successfully"
        );

        Ok(())
    }

    async fn cancel_run(&self, run_id: Uuid) -> Result<()> {
        info!(run_id = %run_id, "Cancelling Temporal workflow");

        // TODO: Implement workflow cancellation via Temporal API
        // For now, we'll use signal-based cancellation when supported

        let workflow_id = TemporalClient::workflow_id_for_run(run_id);
        info!(
            run_id = %run_id,
            workflow_id = %workflow_id,
            "Workflow cancellation requested (not yet implemented)"
        );

        Ok(())
    }

    async fn is_running(&self, run_id: Uuid) -> bool {
        // Check with database since workflow state is persisted there
        match self.db.get_run(run_id).await {
            Ok(Some(run)) => run.status == "running",
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
        let run_id = Uuid::now_v7();
        let workflow_id = TemporalClient::workflow_id_for_run(run_id);
        assert!(workflow_id.starts_with("agent-run-"));
        assert!(workflow_id.contains(&run_id.to_string()));
    }
}

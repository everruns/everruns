// Workflow executor - manages workflow lifecycle
// M4: Simple in-memory execution
// Future: Will integrate with Temporal for durable execution

use anyhow::Result;
use everruns_storage::repositories::Database;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};
use tokio::task::JoinHandle;
use tracing::{info, warn};
use uuid::Uuid;

use crate::workflows::AgentRunWorkflow;

/// Manages workflow execution
pub struct WorkflowExecutor {
    db: Database,
    /// Active workflows (run_id -> task handle)
    active_workflows: Arc<RwLock<HashMap<Uuid, JoinHandle<()>>>>,
    /// Cancellation signals (run_id -> cancel flag)
    cancel_signals: Arc<Mutex<HashMap<Uuid, bool>>>,
}

impl WorkflowExecutor {
    pub fn new(db: Database) -> Self {
        Self {
            db,
            active_workflows: Arc::new(RwLock::new(HashMap::new())),
            cancel_signals: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Start a new workflow execution
    pub async fn start_workflow(
        &self,
        run_id: Uuid,
        agent_id: Uuid,
        thread_id: Uuid,
    ) -> Result<()> {
        info!(
            run_id = %run_id,
            agent_id = %agent_id,
            thread_id = %thread_id,
            "Starting workflow execution"
        );

        let workflow = AgentRunWorkflow::new(run_id, agent_id, thread_id, self.db.clone()).await?;

        let cancel_signals = self.cancel_signals.clone();
        let active_workflows = self.active_workflows.clone();

        // Spawn workflow execution as a background task
        let handle = tokio::spawn(async move {
            // Execute workflow
            let result = workflow.execute().await;

            // Handle errors
            if let Err(e) = result {
                if let Err(err) = workflow.handle_error(&e).await {
                    warn!(run_id = %run_id, error = %err, "Failed to handle workflow error");
                }
            }

            // Cleanup
            cancel_signals.lock().await.remove(&run_id);
            active_workflows.write().await.remove(&run_id);
        });

        // Store the workflow handle
        self.active_workflows.write().await.insert(run_id, handle);

        Ok(())
    }

    /// Cancel a running workflow
    pub async fn cancel_workflow(&self, run_id: Uuid) -> Result<()> {
        info!(run_id = %run_id, "Cancelling workflow");

        // Set cancel signal
        self.cancel_signals.lock().await.insert(run_id, true);

        // Note: In M4, we don't actively cancel the task
        // Future: With real Temporal, we'll send a cancel signal to the workflow

        Ok(())
    }

    /// Check if a workflow is still running
    pub async fn is_running(&self, run_id: Uuid) -> bool {
        let workflows = self.active_workflows.read().await;
        workflows.contains_key(&run_id)
    }

    /// Get count of active workflows
    pub async fn active_count(&self) -> usize {
        self.active_workflows.read().await.len()
    }
}

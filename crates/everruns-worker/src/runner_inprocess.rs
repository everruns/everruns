// In-process agent runner
// This is the default runner that executes workflows using tokio tasks
// Same behavior as M4, but now behind the AgentRunner trait

use anyhow::Result;
use async_trait::async_trait;
use everruns_storage::repositories::Database;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};
use tokio::task::JoinHandle;
use tracing::{info, warn};
use uuid::Uuid;

use crate::runner::AgentRunner;
use crate::workflows::AgentRunWorkflow;

/// In-process agent runner using tokio tasks
pub struct InProcessRunner {
    db: Database,
    /// Active workflows (run_id -> task handle)
    active_workflows: Arc<RwLock<HashMap<Uuid, JoinHandle<()>>>>,
    /// Cancellation signals (run_id -> cancel flag)
    cancel_signals: Arc<Mutex<HashMap<Uuid, bool>>>,
}

impl InProcessRunner {
    pub fn new(db: Database) -> Self {
        Self {
            db,
            active_workflows: Arc::new(RwLock::new(HashMap::new())),
            cancel_signals: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

#[async_trait]
impl AgentRunner for InProcessRunner {
    async fn start_run(&self, run_id: Uuid, agent_id: Uuid, thread_id: Uuid) -> Result<()> {
        info!(
            run_id = %run_id,
            agent_id = %agent_id,
            thread_id = %thread_id,
            "Starting in-process workflow execution"
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

    async fn cancel_run(&self, run_id: Uuid) -> Result<()> {
        info!(run_id = %run_id, "Cancelling in-process workflow");

        // Set cancel signal
        self.cancel_signals.lock().await.insert(run_id, true);

        // Note: We don't actively abort the task to allow graceful cleanup
        // The workflow should check the cancel signal periodically

        Ok(())
    }

    async fn is_running(&self, run_id: Uuid) -> bool {
        let workflows = self.active_workflows.read().await;
        workflows.contains_key(&run_id)
    }

    async fn active_count(&self) -> usize {
        self.active_workflows.read().await.len()
    }
}

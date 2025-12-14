// In-memory workflow runner using Tokio tasks
// This is the default runner - fast but not durable across process restarts.

use anyhow::Result;
use async_trait::async_trait;
use everruns_storage::repositories::Database;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};
use tokio::task::JoinHandle;
use tracing::{info, warn};
use uuid::Uuid;

use super::{WorkflowInput, WorkflowRunner};
use crate::workflows::AgentRunWorkflow;

/// In-memory workflow runner using Tokio tasks
pub struct InMemoryRunner {
    db: Database,
    /// Active workflows (run_id -> task handle)
    active_workflows: Arc<RwLock<HashMap<Uuid, JoinHandle<()>>>>,
    /// Cancellation signals (run_id -> cancel flag)
    cancel_signals: Arc<Mutex<HashMap<Uuid, bool>>>,
}

impl InMemoryRunner {
    pub fn new(db: Database) -> Self {
        Self {
            db,
            active_workflows: Arc::new(RwLock::new(HashMap::new())),
            cancel_signals: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

#[async_trait]
impl WorkflowRunner for InMemoryRunner {
    async fn start_workflow(&self, input: WorkflowInput) -> Result<()> {
        info!(
            run_id = %input.run_id,
            agent_id = %input.agent_id,
            thread_id = %input.thread_id,
            "Starting in-memory workflow execution"
        );

        let workflow = AgentRunWorkflow::new(
            input.run_id,
            input.agent_id,
            input.thread_id,
            self.db.clone(),
        )
        .await?;

        let cancel_signals = self.cancel_signals.clone();
        let active_workflows = self.active_workflows.clone();
        let run_id = input.run_id;

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

    async fn cancel_workflow(&self, run_id: Uuid) -> Result<()> {
        info!(run_id = %run_id, "Cancelling in-memory workflow");

        // Set cancel signal
        self.cancel_signals.lock().await.insert(run_id, true);

        // Note: In-memory runner doesn't actively abort the task
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

    async fn shutdown(&self) -> Result<()> {
        info!("Shutting down in-memory workflow runner");

        // Cancel all running workflows
        let mut workflows = self.active_workflows.write().await;
        for (run_id, handle) in workflows.drain() {
            info!(run_id = %run_id, "Aborting workflow on shutdown");
            handle.abort();
        }

        Ok(())
    }
}

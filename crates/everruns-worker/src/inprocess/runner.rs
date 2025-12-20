// In-process session runner
// Decision: Executes workflows using tokio tasks (non-durable)
// Decision: For durable execution, use temporal/runner.rs instead

use anyhow::Result;
use async_trait::async_trait;
use everruns_storage::repositories::Database;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};
use tokio::task::JoinHandle;
use tracing::{info, warn};
use uuid::Uuid;

use super::workflow::InProcessWorkflow;
use crate::runner::AgentRunner;

/// In-process session runner using tokio tasks
pub struct InProcessRunner {
    db: Database,
    /// Active workflows (session_id -> task handle)
    active_workflows: Arc<RwLock<HashMap<Uuid, JoinHandle<()>>>>,
    /// Cancellation signals (session_id -> cancel flag)
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
    /// Start a session workflow
    /// In M2, run_id maps to session_id, agent_id maps to harness_id, thread_id is same as session_id
    async fn start_run(&self, session_id: Uuid, harness_id: Uuid, _thread_id: Uuid) -> Result<()> {
        info!(
            session_id = %session_id,
            harness_id = %harness_id,
            "Starting in-process session workflow"
        );

        let workflow = InProcessWorkflow::new(session_id, harness_id, self.db.clone()).await?;

        let cancel_signals = self.cancel_signals.clone();
        let active_workflows = self.active_workflows.clone();

        // Spawn workflow execution as a background task
        let handle = tokio::spawn(async move {
            // Execute workflow
            let result = workflow.execute().await;

            // Handle errors
            if let Err(e) = result {
                if let Err(err) = workflow.handle_error(&e).await {
                    warn!(session_id = %session_id, error = %err, "Failed to handle workflow error");
                }
            }

            // Cleanup
            cancel_signals.lock().await.remove(&session_id);
            active_workflows.write().await.remove(&session_id);
        });

        // Store the workflow handle
        self.active_workflows
            .write()
            .await
            .insert(session_id, handle);

        Ok(())
    }

    async fn cancel_run(&self, session_id: Uuid) -> Result<()> {
        info!(session_id = %session_id, "Cancelling in-process session workflow");

        // Set cancel signal
        self.cancel_signals.lock().await.insert(session_id, true);

        // Note: We don't actively abort the task to allow graceful cleanup
        // The workflow should check the cancel signal periodically

        Ok(())
    }

    async fn is_running(&self, session_id: Uuid) -> bool {
        let workflows = self.active_workflows.read().await;
        workflows.contains_key(&session_id)
    }

    async fn active_count(&self) -> usize {
        self.active_workflows.read().await.len()
    }
}

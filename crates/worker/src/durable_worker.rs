// Durable execution engine worker
// Decision: Polls task queue via gRPC instead of direct database access
// Decision: Uses gRPC adapters for control-plane communication

use anyhow::Result;
use everruns_core::atoms::AtomContext;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{watch, Mutex};
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::activities::{input_activity, reason_activity, InputAtomInput, ReasonInput};
use crate::durable_runner::DurableTurnInput;
use crate::grpc_adapters::GrpcClient;
use crate::grpc_durable_store::{ClaimedTask, GrpcDurableStore, WorkflowStatus};

// =============================================================================
// Configuration
// =============================================================================

/// Configuration for the durable worker
#[derive(Debug, Clone)]
pub struct DurableWorkerConfig {
    /// Worker ID (unique identifier for this worker instance)
    pub worker_id: String,
    /// Activity types this worker handles
    pub activity_types: Vec<String>,
    /// Maximum concurrent tasks
    pub max_concurrent_tasks: usize,
    /// Poll interval when no tasks available
    pub poll_interval: Duration,
    /// Heartbeat interval for claimed tasks
    pub heartbeat_interval: Duration,
    /// gRPC address for control-plane communication
    pub grpc_address: String,
}

impl Default for DurableWorkerConfig {
    fn default() -> Self {
        Self {
            worker_id: format!("worker-{}", Uuid::now_v7()),
            activity_types: vec![
                "process_input".to_string(),
                "reason".to_string(),
                "act".to_string(),
            ],
            max_concurrent_tasks: 10,
            poll_interval: Duration::from_secs(1),
            heartbeat_interval: Duration::from_secs(10),
            grpc_address: "127.0.0.1:9001".to_string(),
        }
    }
}

impl DurableWorkerConfig {
    /// Create configuration from environment variables
    pub fn from_env() -> Self {
        let worker_id =
            std::env::var("WORKER_ID").unwrap_or_else(|_| format!("worker-{}", Uuid::now_v7()));

        let grpc_address =
            std::env::var("GRPC_ADDRESS").unwrap_or_else(|_| "127.0.0.1:9001".to_string());

        let max_concurrent = std::env::var("MAX_CONCURRENT_TASKS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(10);

        Self {
            worker_id,
            grpc_address,
            max_concurrent_tasks: max_concurrent,
            ..Default::default()
        }
    }
}

// =============================================================================
// DurableWorker
// =============================================================================

/// Worker that polls and executes tasks from the durable task queue via gRPC
pub struct DurableWorker {
    config: DurableWorkerConfig,
    store: Arc<Mutex<GrpcDurableStore>>,
    grpc_address: String,
    shutdown_tx: watch::Sender<bool>,
    shutdown_rx: watch::Receiver<bool>,
}

impl DurableWorker {
    /// Create a new durable worker
    pub async fn new(config: DurableWorkerConfig) -> Result<Self> {
        info!(
            worker_id = %config.worker_id,
            grpc_address = %config.grpc_address,
            max_concurrent = config.max_concurrent_tasks,
            "Initializing durable worker (gRPC mode)"
        );

        let store = GrpcDurableStore::connect(&config.grpc_address).await?;
        let grpc_address = config.grpc_address.clone();

        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        info!("Durable worker initialized");

        Ok(Self {
            config,
            store: Arc::new(Mutex::new(store)),
            grpc_address,
            shutdown_tx,
            shutdown_rx,
        })
    }

    /// Create from environment variables
    pub async fn from_env() -> Result<Self> {
        let config = DurableWorkerConfig::from_env();
        Self::new(config).await
    }

    /// Run the worker (blocking until shutdown)
    pub async fn run(&mut self) -> Result<()> {
        info!(
            worker_id = %self.config.worker_id,
            "Starting durable worker"
        );

        // Main poll loop
        loop {
            // Check for shutdown
            if *self.shutdown_rx.borrow() {
                info!("Shutdown signal received, stopping worker");
                break;
            }

            // Poll for tasks
            match self.poll_and_execute().await {
                Ok(executed) => {
                    if executed == 0 {
                        // No tasks available, wait before next poll
                        tokio::select! {
                            _ = tokio::time::sleep(self.config.poll_interval) => {}
                            _ = self.shutdown_rx.changed() => {
                                info!("Shutdown during poll wait");
                                break;
                            }
                        }
                    }
                }
                Err(e) => {
                    error!("Error polling tasks: {}", e);
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
            }
        }

        info!("Durable worker stopped");
        Ok(())
    }

    /// Signal the worker to shutdown
    pub fn shutdown(&self) {
        let _ = self.shutdown_tx.send(true);
    }

    /// Poll for tasks and execute them
    async fn poll_and_execute(&self) -> Result<usize> {
        // Claim tasks
        let tasks = {
            let mut store = self.store.lock().await;
            store
                .claim_tasks(
                    &self.config.worker_id,
                    &self.config.activity_types,
                    self.config.max_concurrent_tasks,
                )
                .await
                .map_err(|e| anyhow::anyhow!("Failed to claim tasks: {}", e))?
        };

        if tasks.is_empty() {
            return Ok(0);
        }

        debug!(
            worker_id = %self.config.worker_id,
            task_count = tasks.len(),
            "Claimed tasks"
        );

        // Execute tasks
        for task in &tasks {
            if let Err(e) = self.execute_task(task).await {
                error!(
                    task_id = %task.id,
                    activity_type = %task.activity_type,
                    error = %e,
                    "Task execution failed"
                );

                // Report failure to store
                let mut store = self.store.lock().await;
                let _ = store.fail_task(task.id, &e.to_string()).await;
            }
        }

        Ok(tasks.len())
    }

    /// Execute a single task
    async fn execute_task(&self, task: &ClaimedTask) -> Result<()> {
        info!(
            task_id = %task.id,
            workflow_id = %task.workflow_id,
            activity_type = %task.activity_type,
            attempt = task.attempt,
            "Executing task"
        );

        // Parse workflow input from task input
        let turn_input: DurableTurnInput = serde_json::from_value(task.input.clone())
            .map_err(|e| anyhow::anyhow!("Failed to parse task input: {}", e))?;

        // Create a new gRPC client for this task execution
        let grpc_client = GrpcClient::connect(&self.grpc_address).await?;

        // Spawn heartbeat background task
        let task_id = task.id;
        let worker_id = self.config.worker_id.clone();
        let heartbeat_interval = self.config.heartbeat_interval;
        let store_for_heartbeat = self.store.clone();
        let (cancel_tx, mut cancel_rx) = tokio::sync::oneshot::channel::<()>();

        let heartbeat_handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(heartbeat_interval);
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        let mut store = store_for_heartbeat.lock().await;
                        match store.heartbeat_task(task_id, &worker_id, None).await {
                            Ok(response) => {
                                if response.should_cancel {
                                    warn!(task_id = %task_id, "Task cancellation requested via heartbeat");
                                    break;
                                }
                                debug!(task_id = %task_id, "Heartbeat sent");
                            }
                            Err(e) => {
                                warn!(task_id = %task_id, error = %e, "Failed to send heartbeat");
                            }
                        }
                    }
                    _ = &mut cancel_rx => {
                        debug!(task_id = %task_id, "Heartbeat loop cancelled");
                        break;
                    }
                }
            }
        });

        // Execute based on activity type
        let result = match task.activity_type.as_str() {
            "process_input" => self.execute_input_activity(grpc_client, &turn_input).await,
            "reason" => self.execute_reason_activity(grpc_client, &turn_input).await,
            "act" => {
                // Act activity is handled internally by reason activity for now
                Ok(serde_json::json!({"completed": true}))
            }
            _ => Err(anyhow::anyhow!(
                "Unknown activity type: {}",
                task.activity_type
            )),
        };

        // Stop heartbeat loop
        let _ = cancel_tx.send(());
        let _ = heartbeat_handle.await;

        match result {
            Ok(output) => {
                // Complete the task
                {
                    let mut store = self.store.lock().await;
                    store
                        .complete_task(task.id, output)
                        .await
                        .map_err(|e| anyhow::anyhow!("Failed to complete task: {}", e))?;
                }

                info!(
                    task_id = %task.id,
                    activity_type = %task.activity_type,
                    "Task completed successfully"
                );

                // Check if workflow is complete and schedule next activity
                self.schedule_next_activity(task.workflow_id, &task.activity_type, &turn_input)
                    .await?;
            }
            Err(e) => {
                return Err(e);
            }
        }

        Ok(())
    }

    /// Execute input processing activity
    async fn execute_input_activity(
        &self,
        grpc_client: GrpcClient,
        input: &DurableTurnInput,
    ) -> Result<serde_json::Value> {
        debug!(
            session_id = %input.session_id,
            "Executing input activity"
        );

        // Create AtomContext for this execution
        let context = AtomContext {
            session_id: input.session_id,
            turn_id: Uuid::now_v7(),
            input_message_id: input.input_message_id,
            exec_id: Uuid::now_v7(),
        };

        let atom_input = InputAtomInput { context };

        // Use the existing input_activity function with gRPC adapters
        let result = input_activity(grpc_client, atom_input).await?;

        Ok(serde_json::to_value(&result)?)
    }

    /// Execute reasoning activity (LLM call)
    async fn execute_reason_activity(
        &self,
        grpc_client: GrpcClient,
        input: &DurableTurnInput,
    ) -> Result<serde_json::Value> {
        debug!(
            session_id = %input.session_id,
            "Executing reason activity"
        );

        // Create AtomContext for this execution
        let context = AtomContext {
            session_id: input.session_id,
            turn_id: Uuid::now_v7(),
            input_message_id: input.input_message_id,
            exec_id: Uuid::now_v7(),
        };

        let reason_input = ReasonInput {
            context,
            agent_id: input.agent_id,
        };

        // Use the existing reason_activity function with gRPC adapters
        let result = reason_activity(grpc_client, reason_input).await?;

        Ok(serde_json::to_value(&result)?)
    }

    /// Schedule the next activity based on current activity completion
    async fn schedule_next_activity(
        &self,
        workflow_id: Uuid,
        completed_activity: &str,
        input: &DurableTurnInput,
    ) -> Result<()> {
        let input_json = serde_json::to_value(input)?;
        let mut store = self.store.lock().await;

        match completed_activity {
            "process_input" => {
                // After input processing, schedule reason activity
                store
                    .enqueue_task(
                        workflow_id,
                        format!("reason_{}", Uuid::now_v7()),
                        "reason".to_string(),
                        input_json,
                    )
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to enqueue reason task: {}", e))?;

                debug!(workflow_id = %workflow_id, "Scheduled reason activity");
            }
            "reason" => {
                // After reasoning, check if there are tool calls
                // If no tool calls, mark workflow as complete
                // For now, mark as complete (simplified flow)
                store
                    .update_workflow_status(workflow_id, WorkflowStatus::Completed, None, None)
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to update workflow status: {}", e))?;

                info!(workflow_id = %workflow_id, "Workflow completed");
            }
            "act" => {
                // After action, schedule another reason activity (continue the loop)
                store
                    .enqueue_task(
                        workflow_id,
                        format!("reason_{}", Uuid::now_v7()),
                        "reason".to_string(),
                        input_json,
                    )
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to enqueue reason task: {}", e))?;
            }
            _ => {
                warn!(
                    activity = completed_activity,
                    "Unknown activity type completed"
                );
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_default() {
        let config = DurableWorkerConfig::default();
        assert!(config.worker_id.starts_with("worker-"));
        assert_eq!(config.max_concurrent_tasks, 10);
        assert_eq!(config.grpc_address, "127.0.0.1:9001");
    }
}

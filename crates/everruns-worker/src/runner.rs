// Agent runner abstraction (M2)
// Decision: Use trait-based abstraction to allow switching between in-process and Temporal execution
// This keeps the API layer agnostic to the execution backend
//
// M2 Note: run_id maps to session_id, agent_id maps to harness_id

use anyhow::Result;
use async_trait::async_trait;
use std::sync::Arc;
use uuid::Uuid;

/// Configuration for the agent runner
#[derive(Debug, Clone, Default)]
pub struct RunnerConfig {
    /// Runner mode: "in-process" or "temporal"
    pub mode: RunnerMode,
    /// Temporal server address (only used when mode is Temporal)
    pub temporal_address: Option<String>,
    /// Temporal namespace (only used when mode is Temporal)
    pub temporal_namespace: Option<String>,
    /// Temporal task queue (only used when mode is Temporal)
    pub temporal_task_queue: Option<String>,
}

/// Runner execution mode
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum RunnerMode {
    /// Execute workflows in-process using tokio tasks (default)
    #[default]
    InProcess,
    /// Execute workflows via Temporal for durability (disabled during M2)
    Temporal,
}

impl RunnerConfig {
    /// Create configuration from environment variables
    pub fn from_env() -> Self {
        let mode = match std::env::var("AGENT_RUNNER_MODE")
            .unwrap_or_default()
            .to_lowercase()
            .as_str()
        {
            "temporal" => RunnerMode::Temporal,
            _ => RunnerMode::InProcess,
        };

        Self {
            mode,
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

/// Create an agent runner based on configuration
pub async fn create_runner(
    config: &RunnerConfig,
    db: everruns_storage::repositories::Database,
) -> Result<Arc<dyn AgentRunner>> {
    match config.mode {
        RunnerMode::InProcess => {
            tracing::info!("Creating in-process agent runner");
            let runner = crate::inprocess::InProcessRunner::new(db);
            Ok(Arc::new(runner))
        }
        RunnerMode::Temporal => {
            tracing::info!(
                address = %config.temporal_address(),
                namespace = %config.temporal_namespace(),
                task_queue = %config.temporal_task_queue(),
                "Creating Temporal agent runner"
            );
            let runner = crate::temporal::TemporalRunner::new(config.clone(), db).await?;
            Ok(Arc::new(runner))
        }
    }
}

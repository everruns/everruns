// Decision: WorkflowRunner trait abstracts workflow execution backend.
// This allows switching between in-memory (Tokio tasks) and Temporal-based execution.
// Configuration is via WORKFLOW_RUNNER env var: "inmemory" (default) or "temporal".

pub mod inmemory;
#[cfg(feature = "temporal")]
pub mod temporal;

use anyhow::Result;
use async_trait::async_trait;
use everruns_storage::repositories::Database;
use std::sync::Arc;
use uuid::Uuid;

/// Configuration for the workflow runner
#[derive(Debug, Clone)]
pub struct RunnerConfig {
    /// Type of runner: "inmemory" or "temporal"
    pub runner_type: RunnerType,
    /// Temporal server address (only used for temporal runner)
    pub temporal_address: Option<String>,
    /// Temporal namespace (only used for temporal runner)
    pub temporal_namespace: Option<String>,
    /// Task queue name (only used for temporal runner)
    pub task_queue: Option<String>,
}

/// Type of workflow runner
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum RunnerType {
    /// In-memory execution using Tokio tasks (default)
    #[default]
    InMemory,
    /// Temporal-based durable execution
    Temporal,
}

impl std::str::FromStr for RunnerType {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "inmemory" | "in-memory" | "in_memory" | "" => Ok(RunnerType::InMemory),
            "temporal" => Ok(RunnerType::Temporal),
            _ => anyhow::bail!("Unknown runner type: {}. Use 'inmemory' or 'temporal'", s),
        }
    }
}

impl RunnerConfig {
    /// Create configuration from environment variables
    pub fn from_env() -> Result<Self> {
        let runner_type = std::env::var("WORKFLOW_RUNNER")
            .unwrap_or_default()
            .parse()?;

        let temporal_address = std::env::var("TEMPORAL_ADDRESS").ok();
        let temporal_namespace = std::env::var("TEMPORAL_NAMESPACE").ok();
        let task_queue = std::env::var("TEMPORAL_TASK_QUEUE").ok();

        Ok(Self {
            runner_type,
            temporal_address,
            temporal_namespace,
            task_queue,
        })
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

    /// Get task queue with default
    pub fn task_queue(&self) -> String {
        self.task_queue
            .clone()
            .unwrap_or_else(|| "everruns-agent-runs".to_string())
    }
}

/// Input for starting a workflow
#[derive(Debug, Clone)]
pub struct WorkflowInput {
    pub run_id: Uuid,
    pub agent_id: Uuid,
    pub thread_id: Uuid,
}

/// Trait for workflow execution backends
///
/// Each step in the agent loop (LLM call, tool execution) becomes reliable via this abstraction.
/// Implementations can be:
/// - In-memory using Tokio tasks (fast, but not durable across restarts)
/// - Temporal-based (durable, each step is a Temporal activity)
#[async_trait]
pub trait WorkflowRunner: Send + Sync {
    /// Start a new workflow execution
    async fn start_workflow(&self, input: WorkflowInput) -> Result<()>;

    /// Cancel a running workflow
    async fn cancel_workflow(&self, run_id: Uuid) -> Result<()>;

    /// Check if a workflow is currently running
    async fn is_running(&self, run_id: Uuid) -> bool;

    /// Get count of active workflows
    async fn active_count(&self) -> usize;

    /// Shutdown the runner gracefully
    async fn shutdown(&self) -> Result<()>;
}

/// Create a workflow runner based on configuration
pub async fn create_runner(config: &RunnerConfig, db: Database) -> Result<Arc<dyn WorkflowRunner>> {
    match config.runner_type {
        RunnerType::InMemory => {
            tracing::info!("Using in-memory workflow runner");
            Ok(Arc::new(inmemory::InMemoryRunner::new(db)))
        }
        RunnerType::Temporal => {
            #[cfg(feature = "temporal")]
            {
                tracing::info!(
                    address = %config.temporal_address(),
                    namespace = %config.temporal_namespace(),
                    task_queue = %config.task_queue(),
                    "Using Temporal workflow runner"
                );
                let runner = temporal::TemporalRunner::new(config.clone(), db).await?;
                Ok(Arc::new(runner))
            }
            #[cfg(not(feature = "temporal"))]
            {
                anyhow::bail!(
                    "Temporal runner requested but 'temporal' feature is not enabled. \
                     Compile with --features temporal or use WORKFLOW_RUNNER=inmemory"
                )
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_runner_type_parse_inmemory() {
        assert_eq!(
            "inmemory".parse::<RunnerType>().unwrap(),
            RunnerType::InMemory
        );
        assert_eq!(
            "in-memory".parse::<RunnerType>().unwrap(),
            RunnerType::InMemory
        );
        assert_eq!(
            "in_memory".parse::<RunnerType>().unwrap(),
            RunnerType::InMemory
        );
        assert_eq!(
            "INMEMORY".parse::<RunnerType>().unwrap(),
            RunnerType::InMemory
        );
        assert_eq!("".parse::<RunnerType>().unwrap(), RunnerType::InMemory);
    }

    #[test]
    fn test_runner_type_parse_temporal() {
        assert_eq!(
            "temporal".parse::<RunnerType>().unwrap(),
            RunnerType::Temporal
        );
        assert_eq!(
            "TEMPORAL".parse::<RunnerType>().unwrap(),
            RunnerType::Temporal
        );
        assert_eq!(
            "Temporal".parse::<RunnerType>().unwrap(),
            RunnerType::Temporal
        );
    }

    #[test]
    fn test_runner_type_parse_invalid() {
        assert!("invalid".parse::<RunnerType>().is_err());
        assert!("foo".parse::<RunnerType>().is_err());
    }

    #[test]
    fn test_runner_type_default() {
        assert_eq!(RunnerType::default(), RunnerType::InMemory);
    }

    #[test]
    fn test_runner_config_defaults() {
        let config = RunnerConfig {
            runner_type: RunnerType::InMemory,
            temporal_address: None,
            temporal_namespace: None,
            task_queue: None,
        };

        assert_eq!(config.temporal_address(), "localhost:7233");
        assert_eq!(config.temporal_namespace(), "default");
        assert_eq!(config.task_queue(), "everruns-agent-runs");
    }

    #[test]
    fn test_runner_config_custom_values() {
        let config = RunnerConfig {
            runner_type: RunnerType::Temporal,
            temporal_address: Some("temporal.example.com:7233".to_string()),
            temporal_namespace: Some("production".to_string()),
            task_queue: Some("my-task-queue".to_string()),
        };

        assert_eq!(config.temporal_address(), "temporal.example.com:7233");
        assert_eq!(config.temporal_namespace(), "production");
        assert_eq!(config.task_queue(), "my-task-queue");
    }

    #[test]
    fn test_workflow_input_clone() {
        let input = WorkflowInput {
            run_id: Uuid::now_v7(),
            agent_id: Uuid::now_v7(),
            thread_id: Uuid::now_v7(),
        };

        let cloned = input.clone();
        assert_eq!(input.run_id, cloned.run_id);
        assert_eq!(input.agent_id, cloned.agent_id);
        assert_eq!(input.thread_id, cloned.thread_id);
    }
}

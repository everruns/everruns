// Agent runner for workflow execution
// Decision: Use trait-based abstraction for workflow execution
// Decision: Support both Temporal and custom Durable engine via RunnerMode
// Decision: Workers communicate with control-plane via gRPC (no direct DB access)
//
// Architecture:
// - API calls `start_run` which queues a workflow (Temporal or Durable)
// - Worker polls task queues and executes activities
// - Each activity (input, reason, act) is idempotent
// - ReasonAtom handles agent loading, model resolution, and LLM calls
// - Events are persisted via gRPC to control-plane
//
// Note: OTel instrumentation is handled via the event-listener pattern.
// turn.started/completed events trigger OtelEventListener to create invoke_agent spans.

use anyhow::Result;
use async_trait::async_trait;
use std::sync::Arc;
use tracing::info;
use uuid::Uuid;

use crate::client::TemporalClient;
use crate::durable_runner::DurableRunner;
use crate::turn_workflow::TurnWorkflowInput;

// =============================================================================
// Runner Mode
// =============================================================================

/// Mode for workflow execution
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RunnerMode {
    /// Use Temporal for workflow orchestration (default)
    #[default]
    Temporal,
    /// Use custom durable execution engine (PostgreSQL-backed)
    Durable,
}

impl RunnerMode {
    /// Parse from string (case-insensitive)
    pub fn parse(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "durable" => Self::Durable,
            _ => Self::Temporal,
        }
    }

    /// Parse from environment variable RUNNER_MODE
    pub fn from_env() -> Self {
        std::env::var("RUNNER_MODE")
            .map(|s| Self::parse(&s))
            .unwrap_or_default()
    }
}

impl std::fmt::Display for RunnerMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Temporal => write!(f, "temporal"),
            Self::Durable => write!(f, "durable"),
        }
    }
}

// =============================================================================
// Configuration
// =============================================================================

/// Configuration for the agent runner
#[derive(Debug, Clone, Default)]
pub struct RunnerConfig {
    /// Runner mode (temporal or durable)
    pub mode: RunnerMode,
    /// Temporal server address (for Temporal mode)
    pub temporal_address: Option<String>,
    /// Temporal namespace (for Temporal mode)
    pub temporal_namespace: Option<String>,
    /// Temporal task queue (for Temporal mode)
    pub temporal_task_queue: Option<String>,
    /// Database URL (for Durable mode, falls back to DATABASE_URL env)
    pub database_url: Option<String>,
}

impl RunnerConfig {
    /// Create configuration from environment variables
    pub fn from_env() -> Self {
        Self {
            mode: RunnerMode::from_env(),
            temporal_address: std::env::var("TEMPORAL_ADDRESS").ok(),
            temporal_namespace: std::env::var("TEMPORAL_NAMESPACE").ok(),
            temporal_task_queue: std::env::var("TEMPORAL_TASK_QUEUE").ok(),
            database_url: std::env::var("DATABASE_URL").ok(),
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

    /// Get database URL (required for durable mode)
    pub fn database_url(&self) -> Option<String> {
        self.database_url
            .clone()
            .or_else(|| std::env::var("DATABASE_URL").ok())
    }
}

// =============================================================================
// AgentRunner Trait
// =============================================================================

/// Trait for agent workflow execution
/// Implementations handle the actual execution of agent runs
///
/// M2 Note: Parameters map to session concepts:
/// - session_id: The session/conversation
/// - agent_id: The agent configuration
/// - input_message_id: The user message that triggered this turn
#[async_trait]
pub trait AgentRunner: Send + Sync {
    /// Start a new turn workflow for the given session
    async fn start_run(
        &self,
        session_id: Uuid,
        agent_id: Uuid,
        input_message_id: Uuid,
    ) -> Result<()>;

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
}

impl TemporalRunner {
    /// Create a new Temporal runner connected to the server
    pub async fn new(config: RunnerConfig) -> Result<Self> {
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
        })
    }
}

#[async_trait]
impl AgentRunner for TemporalRunner {
    /// Start a turn workflow for the given session
    ///
    /// Note: OTel instrumentation is handled via event listeners.
    /// turn.started/completed events trigger OtelEventListener to create invoke_agent spans.
    async fn start_run(
        &self,
        session_id: Uuid,
        agent_id: Uuid,
        input_message_id: Uuid,
    ) -> Result<()> {
        info!(
            session_id = %session_id,
            agent_id = %agent_id,
            input_message_id = %input_message_id,
            "Starting Temporal turn workflow for session"
        );

        // Build workflow input
        let input = TurnWorkflowInput {
            session_id,
            agent_id,
            input_message_id,
        };

        // Start the workflow on Temporal server
        let response = self.client.start_turn_workflow(&input).await?;

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

    async fn is_running(&self, _session_id: Uuid) -> bool {
        // TODO: Query Temporal directly for workflow status
        // For now, return false - callers should check session status from their own DB
        false
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

/// Create an agent runner based on configuration
///
/// This is used by the control-plane API to start workflows.
/// The runner mode is determined by RUNNER_MODE environment variable:
/// - "temporal" (default): Use Temporal for workflow orchestration
/// - "durable": Use custom PostgreSQL-backed durable execution engine
pub async fn create_runner(config: &RunnerConfig) -> Result<Arc<dyn AgentRunner>> {
    match config.mode {
        RunnerMode::Temporal => {
            tracing::info!(
                mode = %config.mode,
                address = %config.temporal_address(),
                namespace = %config.temporal_namespace(),
                task_queue = %config.temporal_task_queue(),
                "Creating Temporal agent runner"
            );
            let runner = TemporalRunner::new(config.clone()).await?;
            Ok(Arc::new(runner))
        }
        RunnerMode::Durable => {
            tracing::info!(
                mode = %config.mode,
                "Creating Durable execution engine runner"
            );
            let runner = DurableRunner::from_env().await?;
            Ok(Arc::new(runner))
        }
    }
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

    #[test]
    fn test_runner_mode_parse() {
        assert_eq!(RunnerMode::parse("temporal"), RunnerMode::Temporal);
        assert_eq!(RunnerMode::parse("TEMPORAL"), RunnerMode::Temporal);
        assert_eq!(RunnerMode::parse("durable"), RunnerMode::Durable);
        assert_eq!(RunnerMode::parse("DURABLE"), RunnerMode::Durable);
        assert_eq!(RunnerMode::parse("unknown"), RunnerMode::Temporal);
    }

    #[test]
    fn test_runner_mode_display() {
        assert_eq!(RunnerMode::Temporal.to_string(), "temporal");
        assert_eq!(RunnerMode::Durable.to_string(), "durable");
    }
}

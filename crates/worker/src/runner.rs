// Agent runner for workflow execution
// Decision: Use trait-based abstraction for workflow execution
// Decision: Use true Temporal workflows for durable, distributed execution
// Decision: Workers communicate with control-plane via gRPC (no direct DB access)
//
// Architecture:
// - API calls `start_run` which queues a TurnWorkflow to Temporal
// - Worker polls Temporal task queues and executes activities
// - Each activity (input, reason, act) is idempotent
// - ReasonAtom handles agent loading, model resolution, and LLM calls
// - Events are persisted via gRPC to control-plane

use anyhow::Result;
use async_trait::async_trait;
use everruns_core::telemetry::gen_ai;
use std::sync::Arc;
use tracing::{info, Instrument};
use uuid::Uuid;

use crate::client::TemporalClient;
use crate::turn_workflow::TurnWorkflowInput;

// =============================================================================
// Configuration
// =============================================================================

/// Configuration for the agent runner (Temporal)
#[derive(Debug, Clone, Default)]
pub struct RunnerConfig {
    /// Temporal server address
    pub temporal_address: Option<String>,
    /// Temporal namespace
    pub temporal_namespace: Option<String>,
    /// Temporal task queue
    pub temporal_task_queue: Option<String>,
}

impl RunnerConfig {
    /// Create configuration from environment variables
    pub fn from_env() -> Self {
        Self {
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

impl TemporalRunner {
    /// Inner implementation for start_run with instrumentation
    async fn start_run_inner(
        &self,
        session_id: Uuid,
        agent_id: Uuid,
        input_message_id: Uuid,
    ) -> Result<()> {
        let span = tracing::Span::current();

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

        // Record workflow IDs on span
        span.record("workflow.id", workflow_id.as_str());
        span.record("workflow.run_id", response.run_id.as_str());

        info!(
            session_id = %session_id,
            workflow_id = %workflow_id,
            temporal_run_id = %response.run_id,
            "Temporal workflow started successfully"
        );

        Ok(())
    }
}

#[async_trait]
impl AgentRunner for TemporalRunner {
    /// Start a turn workflow for the given session
    async fn start_run(
        &self,
        session_id: Uuid,
        agent_id: Uuid,
        input_message_id: Uuid,
    ) -> Result<()> {
        // Create span with gen-ai semantic conventions for agent invocation
        // Span name format: "invoke_agent {agent_id}" per OTel spec
        let span_name = format!("invoke_agent {}", agent_id);
        let span = tracing::info_span!(
            "gen_ai.invoke_agent",
            "otel.name" = %span_name,
            "otel.kind" = "client",
            // Operation
            "gen_ai.operation.name" = gen_ai::operation::INVOKE_AGENT,
            // Agent attributes
            "gen_ai.agent.id" = %agent_id,
            // Conversation context
            "gen_ai.conversation.id" = %session_id,
            // Workflow attributes (filled after start)
            "workflow.id" = tracing::field::Empty,
            "workflow.run_id" = tracing::field::Empty,
        );

        self.start_run_inner(session_id, agent_id, input_message_id)
            .instrument(span)
            .await
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

/// Create the Temporal agent runner
///
/// This is used by the control-plane API to start workflows on Temporal.
/// Note: The worker process uses TemporalWorker directly, not this function.
pub async fn create_runner(config: &RunnerConfig) -> Result<Arc<dyn AgentRunner>> {
    tracing::info!(
        address = %config.temporal_address(),
        namespace = %config.temporal_namespace(),
        task_queue = %config.temporal_task_queue(),
        "Creating Temporal agent runner"
    );
    let runner = TemporalRunner::new(config.clone()).await?;
    Ok(Arc::new(runner))
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

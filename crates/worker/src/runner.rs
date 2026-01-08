// Agent runner for workflow execution
// Decision: Use trait-based abstraction for workflow execution
// Decision: Use PostgreSQL-backed durable execution engine
// Decision: Workers communicate with control-plane via gRPC (no direct DB access)
//
// Architecture:
// - API calls `start_run` which queues a workflow
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
use uuid::Uuid;

use crate::durable_runner::DurableRunner;

// =============================================================================
// AgentRunner Trait
// =============================================================================

/// Trait for agent workflow execution
/// Implementations handle the actual execution of agent runs
///
/// Parameters map to session concepts:
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
// Factory Functions
// =============================================================================

/// Create an agent runner
///
/// This is used by the control-plane API to start workflows.
/// Pass a database pool for direct access (control-plane) or None for gRPC (workers).
pub async fn create_runner(db_pool: Option<sqlx::PgPool>) -> Result<Arc<dyn AgentRunner>> {
    if let Some(pool) = db_pool {
        tracing::info!("Creating Durable execution engine runner (direct DB mode)");
        let runner = DurableRunner::new_with_pool(pool);
        Ok(Arc::new(runner))
    } else {
        tracing::info!("Creating Durable execution engine runner (gRPC mode)");
        let runner = DurableRunner::from_env().await?;
        Ok(Arc::new(runner))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_runner_trait_object_size() {
        // Ensure trait object can be created
        fn _assert_send_sync<T: Send + Sync>() {}
        _assert_send_sync::<Arc<dyn AgentRunner>>();
    }
}

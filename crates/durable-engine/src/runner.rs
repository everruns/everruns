// Agent runner for workflow execution
// Decision: Use trait-based abstraction for workflow execution
// Decision: Use PostgreSQL-backed durable execution engine
// Decision: Workers communicate with control-plane via gRPC (no direct DB access);
// that transport and its runner constructors live in the worker crate.
//
// Architecture:
// - API calls `start_run` which queues a workflow
// - Worker receives push notifications when tasks are enqueued (via gRPC streaming)
// - Each activity (input, reason, act) is idempotent
// - ReasonAtom handles agent loading, model resolution, and LLM calls
// - Events are persisted via gRPC to control-plane
//
// Note: OTel instrumentation is handled via the event-listener pattern.
// turn.started/completed events trigger OtelEventListener to create invoke_agent spans.

use anyhow::Result;
use async_trait::async_trait;
use everruns_contracts::error::AgentLoopError;
use everruns_contracts::typed_id::{AgentId, HarnessId, MessageId, SessionId, TurnId};
use everruns_core::host::{PersistedTurn, TurnBackend, TurnInput, TurnRequest};
use everruns_durable::InMemoryWorkflowEventStore;
use std::sync::Arc;
use uuid::Uuid;

use crate::durable_runner::{DurableRunner, DurableTaskNotifier};

// =============================================================================
// AgentRunner Trait
// =============================================================================

/// Trait for agent workflow execution
/// Implementations handle the actual execution of agent runs
///
/// Parameters map to session concepts:
/// - org_id: The organization ID for resource validation
/// - session_id: The session/conversation
/// - agent_id: The agent configuration
/// - input_message_id: The user message that triggered this turn
#[async_trait]
pub trait AgentRunner: Send + Sync {
    /// Start a new turn workflow for the given session
    async fn start_run(
        &self,
        org_id: i64,
        session_id: SessionId,
        harness_id: HarnessId,
        agent_id: Option<AgentId>,
        input_message_id: MessageId,
        request_id: Option<String>,
    ) -> Result<()>;

    /// Resume a workflow after client-side tool results (e.g. connection_required).
    ///
    /// Unlike `start_run`, this skips `InputAtom` (there is no new user message)
    /// and enqueues a `reason` activity directly using the turn context saved
    /// when the workflow paused for tool results.
    async fn resume_after_tool_results(
        &self,
        session_id: SessionId,
        resolution_id: Uuid,
    ) -> Result<()>;

    /// Cancel a running workflow
    async fn cancel_run(&self, run_id: SessionId) -> Result<()>;

    /// Check if a workflow is still running
    async fn is_running(&self, run_id: SessionId) -> bool;

    /// Get count of active workflows (for monitoring)
    async fn active_count(&self) -> usize;
}

// =============================================================================
// DurableRunner shim
// =============================================================================

// Decision: `DurableRunner`'s turn logic lives in its `TurnBackend`
// implementation (`crate::turn_backend`). These methods only translate the
// server's call shape onto it and drop the ticket: the workflow already
// started, and server callers never awaited a turn. The shim goes once the
// server calls `TurnBackend` directly.
#[async_trait]
impl AgentRunner for DurableRunner {
    async fn start_run(
        &self,
        org_id: i64,
        session_id: SessionId,
        harness_id: HarnessId,
        agent_id: Option<AgentId>,
        input_message_id: MessageId,
        request_id: Option<String>,
    ) -> Result<()> {
        let turn = PersistedTurn::Message {
            org_id,
            harness_id,
            agent_id,
            input_message_id,
            request_id,
        };
        start_persisted(self, session_id, turn).await
    }

    async fn resume_after_tool_results(
        &self,
        session_id: SessionId,
        resolution_id: Uuid,
    ) -> Result<()> {
        let turn = PersistedTurn::ToolResolution { resolution_id };
        start_persisted(self, session_id, turn).await
    }

    async fn cancel_run(&self, session_id: SessionId) -> Result<()> {
        TurnBackend::cancel(self, session_id)
            .await
            .map(drop)
            .map_err(runner_error)
    }

    async fn is_running(&self, session_id: SessionId) -> bool {
        TurnBackend::is_running(self, session_id).await
    }

    async fn active_count(&self) -> usize {
        TurnBackend::active_count(self).await
    }
}

async fn start_persisted(
    runner: &DurableRunner,
    session_id: SessionId,
    turn: PersistedTurn,
) -> Result<()> {
    // The server's turn id is assigned by the input step, so the request's
    // id only labels the dropped ticket.
    let request = TurnRequest::new(
        session_id,
        TurnId::new(),
        TurnInput::Persisted(Box::new(turn)),
    );
    runner
        .start_turn(request)
        .await
        .map(drop)
        .map_err(runner_error)
}

/// Restore the message a store failure carried, which is what the server
/// logged before the seam existed.
fn runner_error(error: AgentLoopError) -> anyhow::Error {
    match error {
        AgentLoopError::MessageStore(message) => anyhow::anyhow!(message),
        other => anyhow::Error::new(other),
    }
}

// =============================================================================
// Runner Backend Configuration
// =============================================================================

/// Configuration for creating an agent runner
pub enum RunnerBackend {
    /// Use PostgreSQL for workflow persistence (production)
    Postgres(everruns_durable::PostgresPool),
    /// Use PostgreSQL and publish task availability through the control-plane notifier.
    PostgresWithNotifier {
        pool: everruns_durable::PostgresPool,
        task_notifier: Arc<dyn DurableTaskNotifier>,
    },
    /// Use in-memory storage (dev mode, no database required)
    InMemory,
    /// Use shared in-memory storage (dev mode with in-process worker)
    SharedInMemory(Arc<InMemoryWorkflowEventStore>),
    // The gRPC control-plane backend is worker-owned: see
    // `everruns_worker::grpc_durable_runner`. This crate carries no transport.
}

// =============================================================================
// Factory Functions
// =============================================================================

/// Create an agent runner with explicit backend configuration
///
/// This allows choosing between PostgreSQL and in-memory backends.
pub async fn create_runner_with_backend(backend: RunnerBackend) -> Result<Arc<dyn AgentRunner>> {
    match backend {
        RunnerBackend::Postgres(pool) => {
            tracing::info!("Creating Durable execution engine runner (PostgreSQL mode)");
            let runner = DurableRunner::new_with_pool(pool);
            Ok(Arc::new(runner))
        }
        RunnerBackend::PostgresWithNotifier {
            pool,
            task_notifier,
        } => {
            tracing::info!(
                "Creating Durable execution engine runner (PostgreSQL mode with task notifier)"
            );
            let runner = DurableRunner::new_with_pool_and_task_notifier(pool, task_notifier);
            Ok(Arc::new(runner))
        }
        RunnerBackend::InMemory => {
            tracing::info!("Creating Durable execution engine runner (in-memory dev mode)");
            let runner = DurableRunner::new_in_memory();
            Ok(Arc::new(runner))
        }
        RunnerBackend::SharedInMemory(store) => {
            tracing::info!("Creating Durable execution engine runner (shared in-memory dev mode)");
            let runner = DurableRunner::new_with_shared_store(store);
            Ok(Arc::new(runner))
        }
    }
}

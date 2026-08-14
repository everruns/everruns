//! Store-backed loading for the neutral core execution snapshot.

use everruns_core::error::{AgentLoopError, Result};
use everruns_core::execution_loading::{AgentStore, HarnessStore, SessionStore};
use everruns_core::{ExecutionSession, ResolvedExecutionSnapshot, SessionId};

/// Load a session and project its executable records into a neutral snapshot.
pub async fn load_execution_snapshot(
    harness_store: &dyn HarnessStore,
    agent_store: &dyn AgentStore,
    session_store: &dyn SessionStore,
    session_id: SessionId,
) -> Result<ResolvedExecutionSnapshot> {
    let session = session_store
        .get_session(session_id)
        .await?
        .ok_or_else(|| AgentLoopError::session_not_found(session_id))?;
    load_execution_snapshot_for_session(harness_store, agent_store, &session).await
}

/// Project a snapshot after a host has applied session-scoped attachments.
pub async fn load_execution_snapshot_for_session(
    harness_store: &dyn HarnessStore,
    agent_store: &dyn AgentStore,
    session: &ExecutionSession,
) -> Result<ResolvedExecutionSnapshot> {
    let harness = harness_store
        .get_harness(session.harness_id)
        .await?
        .ok_or_else(|| AgentLoopError::harness_not_found(session.harness_id))?;
    let agent = match session.agent_id {
        Some(agent_id) => agent_store.get_agent(agent_id).await?,
        None => None,
    };
    ResolvedExecutionSnapshot::project(&harness, agent.as_ref(), session)
}

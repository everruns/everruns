//! Seed the selected workspace from the pinned definitions.
use super::*;
pub(super) async fn seed_runtime_initial_files(
    harness_store: &dyn RuntimeHarnessStore,
    agent_store: &dyn RuntimeAgentStore,
    file_store: &dyn SessionFileSystem,
    session: &ExecutionSession,
) -> Result<()> {
    let harness = harness_store
        .get_harness(session.harness_id)
        .await?
        .ok_or_else(|| {
            AgentLoopError::store(format!(
                "harness not found while seeding files: {}",
                session.harness_id
            ))
        })?;
    let agent = match session.agent_id {
        Some(agent_id) => Some(
            agent_store
                .get_agent(agent_id)
                .await?
                .ok_or_else(|| AgentLoopError::store(format!("agent not found: {agent_id}")))?,
        ),
        None => None,
    };
    let overlay = effective_overlay(&harness, agent.as_ref(), session);
    // Seed into the session's workspace (a shared workspace differs from the
    // session id; the default 1:1 case is equal).
    let seed_key = SessionId::from_uuid(session.workspace_id.uuid());
    for file in &overlay.initial_files {
        file_store.seed_initial_file(seed_key, file).await?;
    }
    Ok(())
}

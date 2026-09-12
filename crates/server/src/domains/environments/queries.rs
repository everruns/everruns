// Effective capability resolution for a session.
//
// Both the environment view and the managed-sandbox service need the same
// answer: what capabilities is this session actually running with, after the
// harness -> agent -> session layering. One implementation, called from both.

use std::sync::Arc;

use anyhow::Context;
use everruns_core::merge_capabilities;
use everruns_provider::typed_id::{AgentId, SessionId};

use crate::domains::harnesses::queries::resolve_effective as resolve_effective_harness;
use crate::org_init;
use crate::storage::StorageBackend;

/// Resolve the capabilities a session runs with, in merge order.
///
/// `None` means the session or its harness no longer exists, which callers
/// report as "not found" rather than as an empty environment.
pub async fn effective_session_capabilities(
    db: &Arc<StorageBackend>,
    session_id: SessionId,
) -> anyhow::Result<Option<Vec<everruns_capability::CapabilityRef>>> {
    let Some(org_id) = db
        .get_session_organization_id(session_id)
        .await
        .context("failed to resolve session org")?
    else {
        return Ok(None);
    };
    let Some(session_row) = db
        .get_session(org_id, session_id)
        .await
        .context("failed to load session")?
    else {
        return Ok(None);
    };

    let harness_id = match session_row.harness_id {
        Some(id) => id,
        None => org_init::base_harness_id(db, org_id)
            .await
            .context("failed to resolve base harness for session without harness_id")?,
    };
    let Some(harness) = resolve_effective_harness(db.as_ref(), org_id, harness_id)
        .await
        .context("failed to resolve effective harness")?
    else {
        return Ok(None);
    };

    let agent_capabilities = match session_row.agent_id {
        Some(agent_id) => agent_capabilities(db, agent_id).await?,
        None => Vec::new(),
    };
    let session_capabilities: Vec<everruns_capability::CapabilityRef> =
        serde_json::from_value(session_row.capabilities)
            .context("failed to parse session capabilities")?;

    let merged = merge_capabilities(&harness.capabilities, &agent_capabilities);
    Ok(Some(merge_capabilities(&merged, &session_capabilities)))
}

async fn agent_capabilities(
    db: &Arc<StorageBackend>,
    agent_id: AgentId,
) -> anyhow::Result<Vec<everruns_capability::CapabilityRef>> {
    db.get_agent_capabilities(agent_id.uuid())
        .await
        .context("failed to load agent capabilities")
        .map(|rows| {
            rows.into_iter()
                .map(|row| {
                    everruns_capability::CapabilityRef::with_config(row.capability_id, row.config)
                })
                .collect()
        })
}

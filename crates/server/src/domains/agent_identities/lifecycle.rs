// Agent-identity lifecycle shared by the lazy and eager creation paths.
//
// An agent's identity appears lazily on its first unattended action (EVE-758)
// and eagerly when an admin authorizes a service MCP grant for it (EVE-1030).
// Both routes must end at the same identity with the same principal parent, so
// the guarded write lives here once rather than in each caller.

use crate::kernel_imports::Caller;
use crate::services::PrincipalService;
use crate::storage::StorageBackend;
use crate::storage::models::{AgentRow, CreateAgentIdentityRow, PrincipalRow};
use everruns_provider::typed_id::AgentIdentityId;
use std::sync::Arc;

/// Resolve the agent-identity principal that owns this agent's unattended work,
/// lazily creating the identity on the agent's first fire (EVE-758).
///
/// - If the agent is already linked to an identity (explicitly or from a prior
///   fire), that identity's principal is ensured and returned — it is NEVER
///   overridden (`ensure_agent_identity_principal` also preserves the existing
///   parent at the principal layer).
/// - Otherwise a fresh `agent_identities` row is created, its principal is
///   ensured (parented to the internal caller's system-owner, so the effective
///   human/system owner is unchanged), and the agent is linked with a guarded
///   set that only writes when the link is still NULL.
///
/// Returns the identity id and its principal row (the durable session owner).
///
/// Shared rather than trigger-local because a service MCP grant needs an owner
/// at authorize time (EVE-1030). Both the lazy first-fire path and the eager
/// authorize path call this, so they converge on one identity with one parent:
/// the guarded write below is what makes that safe under concurrency.
pub(crate) async fn ensure_identity_for_agent(
    db: &Arc<StorageBackend>,
    org_id: i64,
    agent: &AgentRow,
) -> anyhow::Result<(AgentIdentityId, PrincipalRow)> {
    let principals = PrincipalService::new(db.clone());
    let caller = Caller::internal(org_id);

    // Already linked: ensure and return without ever creating a new identity.
    if let Some(identity_id) = agent.agent_identity_id {
        let identity = db
            .get_agent_identity(org_id, identity_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Agent identity not found"))?;
        if identity.status != "active" {
            anyhow::bail!(
                "Agent identity {} is not active and cannot own new trigger sessions",
                identity_id
            );
        }
        let principal = principals
            .default_owner_principal(&caller, Some(identity_id))
            .await?;
        return Ok((identity_id, principal));
    }

    // First unattended action: create the agent's own identity.
    let new_id = AgentIdentityId::new();
    db.create_agent_identity(CreateAgentIdentityRow {
        org_id,
        id: new_id,
        name: agent.name.clone(),
        description: Some(format!("Identity for agent {}", agent.public_id)),
        avatar_url: None,
        locale: None,
        timezone: None,
    })
    .await?;
    let principal = principals
        .default_owner_principal(&caller, Some(new_id))
        .await?;

    // Guarded link: only claims the agent when it is still unlinked. When a
    // concurrent first-fire won the race this returns false; adopt the winner's
    // identity and soft-archive our just-created orphan (best-effort).
    if db.set_agent_identity_id(org_id, agent.id, new_id).await? {
        return Ok((new_id, principal));
    }

    let winner = db
        .get_agent(org_id, agent.id)
        .await?
        .and_then(|a| a.agent_identity_id)
        .ok_or_else(|| anyhow::anyhow!("agent identity link missing after concurrent set"))?;
    // Best-effort cleanup of the orphaned identity + its principal. Failure here
    // is harmless: the orphan is an unreferenced, archivable row on a rare race.
    let _ = db.delete_agent_identity(org_id, new_id).await;
    let _ = principals
        .sync_agent_identity_status(org_id, new_id, everruns_platform::PrincipalStatus::Archived)
        .await;
    let winner_principal = principals
        .default_owner_principal(&caller, Some(winner))
        .await?;
    Ok((winner, winner_principal))
}

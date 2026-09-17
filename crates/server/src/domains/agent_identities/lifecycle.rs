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

#[cfg(test)]
mod tests {
    //! The lazy first-fire path (EVE-758) is covered in
    //! `domains::agent_triggers::tests`. These cover the property that made
    //! this function shared: eager authorize-time creation (EVE-1030) races
    //! the lazy path and every other authorize, and all of them must land on
    //! one identity.

    use super::*;
    use crate::kernel_imports::DEFAULT_ORG_ID;
    use crate::storage::models::CreateAgentRow;

    async fn seed_unlinked_agent(db: &Arc<StorageBackend>) -> AgentRow {
        let created = db
            .create_agent(CreateAgentRow {
                org_id: DEFAULT_ORG_ID,
                name: "Service MCP Agent".to_string(),
                ..Default::default()
            })
            .await
            .expect("seed agent");
        assert!(
            created.agent_identity_id.is_none(),
            "fixture must start unlinked or the race proves nothing"
        );
        created
    }

    #[tokio::test]
    async fn concurrent_first_authorize_resolves_to_a_single_identity() {
        let db = Arc::new(StorageBackend::in_memory());
        let agent = seed_unlinked_agent(&db).await;

        // Both callers observe the same unlinked agent row, which is exactly
        // the interleaving the guarded write exists for: two admins authorizing
        // the same attachment, or an authorize racing a first trigger fire.
        let (left, right) = tokio::join!(
            ensure_identity_for_agent(&db, DEFAULT_ORG_ID, &agent),
            ensure_identity_for_agent(&db, DEFAULT_ORG_ID, &agent),
        );
        let (left_id, _) = left.expect("first caller");
        let (right_id, _) = right.expect("second caller");

        assert_eq!(
            left_id, right_id,
            "a race must not leave the agent with two identities"
        );

        // The winner is what the agent is actually linked to — not merely what
        // the two calls agreed to return.
        let linked = db
            .get_agent(DEFAULT_ORG_ID, agent.id)
            .await
            .unwrap()
            .expect("agent still exists");
        assert_eq!(linked.agent_identity_id, Some(left_id));
    }

    #[tokio::test]
    async fn eager_creation_reuses_the_identity_a_prior_run_created() {
        // Authorizing a second service attachment on the same agent must reuse
        // the identity, or the two grants would land on different owners and
        // only one would ever resolve.
        let db = Arc::new(StorageBackend::in_memory());
        let agent = seed_unlinked_agent(&db).await;

        let (first, _) = ensure_identity_for_agent(&db, DEFAULT_ORG_ID, &agent)
            .await
            .expect("first authorize");
        let relinked = db
            .get_agent(DEFAULT_ORG_ID, agent.id)
            .await
            .unwrap()
            .unwrap();
        let (second, _) = ensure_identity_for_agent(&db, DEFAULT_ORG_ID, &relinked)
            .await
            .expect("second authorize");

        assert_eq!(first, second);
    }

    #[tokio::test]
    async fn an_archived_identity_cannot_own_a_new_grant() {
        // Authorizing against an archived identity must fail loudly rather than
        // write a grant nothing will ever resolve.
        let db = Arc::new(StorageBackend::in_memory());
        let agent = seed_unlinked_agent(&db).await;
        let (identity_id, _) = ensure_identity_for_agent(&db, DEFAULT_ORG_ID, &agent)
            .await
            .expect("create identity");
        db.delete_agent_identity(DEFAULT_ORG_ID, identity_id)
            .await
            .expect("archive identity");

        let relinked = db
            .get_agent(DEFAULT_ORG_ID, agent.id)
            .await
            .unwrap()
            .unwrap();
        let result = ensure_identity_for_agent(&db, DEFAULT_ORG_ID, &relinked).await;

        assert!(
            result.is_err(),
            "an archived identity must not silently own a new service grant"
        );
    }
}

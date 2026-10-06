// Agent version selection for exposures (endpoints and triggers).
//
// An exposure stores `(agent_version_policy, agent_version_id)` and every
// ingress path hands that pair to session creation. This module is the single
// write-side validator, so an endpoint and a trigger cannot disagree on what a
// valid pin is (EVE-1139).

use crate::domains::common::{CommandError, Ctx};
use crate::records::AgentVersionPolicy;
use everruns_contracts::typed_id::{AgentId, AgentVersionId};

/// The stored version selection of an exposure.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct VersionSelection {
    pub policy: AgentVersionPolicy,
    pub version_id: Option<AgentVersionId>,
}

impl VersionSelection {
    pub(crate) fn policy_str(&self) -> String {
        self.policy.to_string()
    }
}

/// Resolve a requested version-policy change against the exposure's current
/// selection. Returns `None` when the request does not touch the selection.
///
/// Rules:
/// - `pinned` needs a version: the requested one, or the one already pinned.
/// - A newly named pin must be a published version of this agent. Automatic
///   draft snapshots are history, not deployment targets.
/// - `default` / `latest` carry no version; switching to them clears the pin.
/// - Moving off `default` needs the `agent_versions` feature. Unpinning is
///   always allowed so an org can leave a pinned state the flag no longer shows.
pub(crate) async fn resolve_version_selection(
    ctx: &Ctx,
    agent_id: AgentId,
    current: Option<&VersionSelection>,
    requested_policy: Option<AgentVersionPolicy>,
    requested_version_id: Option<AgentVersionId>,
) -> Result<Option<VersionSelection>, CommandError> {
    if requested_policy.is_none() && requested_version_id.is_none() {
        return Ok(None);
    }
    let policy = requested_policy
        .or_else(|| current.map(|selection| selection.policy.clone()))
        .unwrap_or_default();
    if policy != AgentVersionPolicy::Default && !ctx.feature_flags.agent_versions {
        return Err(CommandError::feature_not_enabled("agent_versions"));
    }

    if policy != AgentVersionPolicy::Pinned {
        if requested_version_id.is_some() {
            return Err(CommandError::bad_request(
                "agent_version_id is only valid with agent_version_policy 'pinned'",
            ));
        }
        return Ok(Some(VersionSelection {
            policy,
            version_id: None,
        }));
    }

    let Some(version_id) = requested_version_id else {
        let kept = current
            .filter(|selection| selection.policy == AgentVersionPolicy::Pinned)
            .and_then(|selection| selection.version_id);
        return match kept {
            Some(version_id) => Ok(Some(VersionSelection {
                policy,
                version_id: Some(version_id),
            })),
            None => Err(CommandError::bad_request(
                "agent_version_policy 'pinned' requires agent_version_id",
            )),
        };
    };

    // THREAT[TM-AUTHZ-020]: a pin names a row by id. Scope the lookup to the
    // caller's org and the exposure's agent so a pin cannot run another
    // agent's (or org's) configuration snapshot.
    let version = ctx
        .db
        .get_agent_version(ctx.org_id(), version_id)
        .await?
        .filter(|version| version.agent_id == agent_id);
    let Some(version) = version else {
        return Err(CommandError::bad_request(
            "agent_version_id does not name a version of this agent",
        ));
    };
    if !version.is_published {
        return Err(CommandError::bad_request(
            "Only saved (published) agent versions can be pinned; automatic draft snapshots cannot",
        ));
    }
    Ok(Some(VersionSelection {
        policy,
        version_id: Some(version.id),
    }))
}

/// Select the immutable version used when a new exposure session starts.
/// Ingress decisions must judge the same purpose that session creation will run.
pub(crate) async fn resolve_exposure_version(
    db: &crate::storage::StorageBackend,
    org_id: i64,
    agent: &crate::storage::models::AgentRow,
    policy: AgentVersionPolicy,
    pinned_id: Option<AgentVersionId>,
    versioning_enabled: bool,
) -> anyhow::Result<Option<crate::storage::models::AgentVersionRow>> {
    if !versioning_enabled {
        return Ok(None);
    }
    match policy {
        AgentVersionPolicy::Pinned => match pinned_id {
            Some(id) => db.get_agent_version(org_id, id).await,
            None => Ok(None),
        },
        AgentVersionPolicy::Latest => db.get_latest_agent_version(org_id, agent.id).await,
        AgentVersionPolicy::Default => match agent.default_version_id {
            Some(id) => db.get_agent_version(org_id, id).await,
            None => Ok(None),
        },
    }
}

//! A v2 org's agent catalog on its data plane (data plane v2 parity,
//! gap 9).
//!
//! Tools that hand work to another agent (coordination, handoff, teams)
//! name it by the platform's agent id, from their configuration. On the
//! platform's workers they ask the control plane for it; on a shard the
//! answer is pushed ahead instead, as the design's rule says ("a value
//! that can be known before the call is pushed, never asked").
//!
//! Decisions:
//! - Every agent of the org has its own key published: the agent on its
//!   own harness, the run a tool starts when it names the agent. That
//!   key's revision carries the agent's card (`dp::AgentCard`): the
//!   platform's ids for it and its harness, and the definitions the tools
//!   read. A session runs the key's package, never the card.
//! - Cards redact MCP credentials, as packages do.
//! - The console's own Platform Chat agent is in it too: its sessions run
//!   on the data plane like any agent's (parity, Phase B), and its
//!   coordination hands work to the same agent.
//! - A deleted agent's keys leave the data plane's config, so no tool
//!   starts it; its running sessions keep the revision they run.

use everruns_contracts::dataplane as dp;
use everruns_contracts::typed_id::{AgentId, HarnessId};

use super::ControlPlane;

/// Most agents read per page while syncing an org's catalog.
const PAGE: u32 = 500;

/// `agent_key`'s card: set when the key is an agent on its own harness.
pub async fn card(
    cp: &ControlPlane,
    org_id: i64,
    agent_key: &str,
) -> anyhow::Result<Option<dp::AgentCard>> {
    let Some(target) = cp.db.engine_agent_key_target(agent_key).await? else {
        return Ok(None);
    };
    let Some(agent_id) = target.agent_id.map(AgentId::from_uuid) else {
        return Ok(None);
    };
    let Some(row) = cp.db.get_agent(org_id, agent_id).await? else {
        return Ok(None);
    };
    let harness_id = HarnessId::from_uuid(target.harness_id);
    if row.harness_id != harness_id {
        return Ok(None);
    }
    let layers =
        crate::domains::agent_definitions::resolve(&cp.db, org_id, harness_id, Some(agent_id))
            .await?;
    let Some(mut agent) = layers.agent else {
        return Ok(None);
    };
    agent.mcp_servers = everruns_core::agent_package::redact_mcp(&agent.mcp_servers);
    let mut harness = layers.harness;
    harness.mcp_servers = everruns_core::agent_package::redact_mcp(&harness.mcp_servers);
    Ok(Some(dp::AgentCard {
        agent_id: agent_id.to_string(),
        harness_id: harness_id.to_string(),
        agent: serde_json::to_value(&agent)?,
        harness: serde_json::to_value(&harness)?,
    }))
}

/// Publish every agent of `org_id` on its own harness, so the catalog
/// holds each one. Keys already published are left as they are.
pub async fn sync(cp: &ControlPlane, org_id: i64) -> anyhow::Result<()> {
    if cp.db.dp_org(org_id).await?.is_none() {
        return Ok(());
    }
    let mut offset = 0;
    loop {
        let (agents, _) = cp
            .db
            .list_agents(
                org_id,
                None,
                true,
                crate::common_dto::Pagination::new(offset, PAGE),
            )
            .await?;
        for agent in &agents {
            if let Err(error) = agent_edited(cp, org_id, agent.id).await {
                tracing::warn!(org_id, agent_id = %agent.id, %error, "agent not in the data plane's catalog");
            }
        }
        if agents.len() < PAGE as usize {
            return Ok(());
        }
        offset += PAGE;
    }
}

/// `agent_id` of `org_id` changed: publish its own key when it has none
/// yet (the publisher republishes keys that exist), or take its keys off
/// the data plane when it was deleted. A v1 org is left alone.
pub async fn agent_edited(cp: &ControlPlane, org_id: i64, agent_id: AgentId) -> anyhow::Result<()> {
    if cp.db.dp_org(org_id).await?.is_none() {
        return Ok(());
    }
    match cp.db.get_agent(org_id, agent_id).await? {
        Some(row) if row.status != "deleted" => {
            let (key, revision) = crate::domains::agent_definitions::key_for(
                &cp.db,
                org_id,
                row.harness_id,
                Some(agent_id),
            )
            .await?;
            // A key published before the catalog has no card yet: deliver
            // its current revision again with one (the data plane takes a
            // card at the same revision).
            let carded = cp
                .db
                .dp_item(org_id, "agent", &key)
                .await?
                .and_then(|item| item.body)
                .is_some_and(|body| body.get("card").is_some_and(|card| !card.is_null()));
            if carded {
                return Ok(());
            }
            super::agent_published(cp, &key, org_id, &revision).await
        }
        _ => {
            for key in cp
                .db
                .engine_agent_keys_for_edit(org_id, Some(agent_id.uuid()))
                .await?
            {
                super::remove(&cp.db, org_id, "agent", &key.agent_key).await?;
            }
            Ok(())
        }
    }
}

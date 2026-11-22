//! Subagents a data plane starts: each gets a session record here, as a
//! v1 subagent has, from its shard's feed (`Record::Subagent`).
//!
//! Decisions:
//! - A shard starts a subagent itself (`crates/agentd/src/subagents.rs`),
//!   under an id it mints, and reports it on the feed; the record here is
//!   made with that id, so the calls that read a session's record (Memory
//!   mounts, a connection acting as the turn's person, the console's lists)
//!   serve the child as they serve its parent. The shard never waits for it:
//!   a call the child makes before its record lands (ship lag, about a
//!   second) finds no session.
//! - The child belongs to its parent: same org, same owner and identity,
//!   source `subagent`, `parent_session_id` set. The parent must be one of
//!   the org's sessions on its data plane, so a shard can record children
//!   only for sessions it runs for that org; any other record is skipped and
//!   logged.
//! - The record is marked as the data plane's (`dp_session_summaries`), so
//!   nothing sends its messages to the platform's workers.
//! - A replayed record finds the child there and changes nothing.

use everruns_contracts::dataplane::SubagentOrigin;
use everruns_contracts::typed_id::{AgentId, HarnessId, SessionId};

use super::ControlPlane;
use crate::storage::{CreateSessionRow, DpSessionSummary};

/// Give the child `session_id` a session record under its id.
pub async fn record(
    cp: &ControlPlane,
    org_id: i64,
    session_id: &str,
    origin: &SubagentOrigin,
) -> anyhow::Result<()> {
    let child: SessionId = session_id.parse()?;
    let parent: SessionId = origin.parent_session_id.parse()?;
    let harness_id: HarnessId = origin.harness_id.parse()?;
    let agent_id: Option<AgentId> = origin.agent_id.as_deref().map(str::parse).transpose()?;
    if cp.db.dp_session_org(&parent.to_string()).await? != Some(org_id)
        || !super::sessions::runs_on_data_plane(cp, org_id, parent).await?
    {
        tracing::warn!(org_id, session_id, "a subagent of a session not the org's");
        return Ok(());
    }
    if cp.db.get_session(org_id, child).await?.is_some() {
        return Ok(());
    }
    let Some(parent_row) = cp.db.get_session(org_id, parent).await? else {
        tracing::warn!(org_id, session_id, "a subagent of a session with no record");
        return Ok(());
    };
    let plane = cp
        .db
        .dp_org(org_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("org {org_id} is not on a data plane"))?;
    let (key, _) =
        crate::domains::agent_definitions::key_for(&cp.db, org_id, harness_id, agent_id).await?;
    // Marked first, so nothing sends the child's messages to a worker.
    let now = chrono::Utc::now();
    cp.db
        .dp_upsert_session_summary(DpSessionSummary {
            session_id,
            org_id,
            data_plane_id: &plane.data_plane_id,
            agent_id: &key,
            session_version: 0,
            title: "",
            status: "started",
            turns: 0,
            cost_micros: 0,
            created_at: now,
            last_activity_at: now,
        })
        .await?;
    cp.db
        .create_session_with_id(
            child.uuid(),
            CreateSessionRow {
                org_id,
                source: crate::domains::sessions::record::SessionSource::Subagent,
                harness_id: Some(harness_id),
                agent_id,
                virtual_user_id: parent_row.virtual_user_id,
                playground_user_id: parent_row.playground_user_id,
                owner_principal_id: parent_row.owner_principal_id,
                resolved_owner_user_id: parent_row.resolved_owner_user_id,
                title: origin.title.clone(),
                locale: origin.locale.clone(),
                parent_session_id: Some(parent),
                ..CreateSessionRow::default()
            },
        )
        .await?;
    Ok(())
}

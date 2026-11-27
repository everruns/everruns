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
//! - A child names its agent by the id it has here, except a child of its
//!   parent's own agent: the shard names that agent and its harness by the
//!   Engine's session-local ids, and the record takes the parent's instead.
//! - A parent not recorded yet holds the record back rather than drop it: a
//!   grandchild started on one shard can reach the feed before its parent,
//!   a child of another shard, does (codex review, 2026-11-24). The record
//!   fails, so the feed stops there and the shard sends it again
//!   (`feed.rs`). Only for [`PARENT_WAIT`] after the record was made: a
//!   parent that never arrives must not stall its shard's feed forever, so
//!   after that the record is skipped and logged. A parent of another org
//!   is skipped at once.

use everruns_contracts::dataplane::SubagentOrigin;
use everruns_contracts::typed_id::{AgentId, HarnessId, SessionId};

use super::ControlPlane;
use crate::storage::{CreateSessionRow, DpSessionSummary};

/// How long after it was made a record waits for its parent's (module notes).
pub const PARENT_WAIT_MS: i64 = 10 * 60 * 1000;

/// Give the child `session_id` a session record under its id. `at_unix_ms`:
/// when the shard made the feed record.
pub async fn record(
    cp: &ControlPlane,
    org_id: i64,
    session_id: &str,
    origin: &SubagentOrigin,
    at_unix_ms: i64,
) -> anyhow::Result<()> {
    let child: SessionId = session_id.parse()?;
    let parent: SessionId = origin.parent_session_id.parse()?;
    let named_harness: HarnessId = origin.harness_id.parse()?;
    let named: Option<AgentId> = origin.agent_id.as_deref().map(str::parse).transpose()?;
    if cp.db.get_session(org_id, child).await?.is_some() {
        return Ok(());
    }
    let parent_org = cp.db.dp_session_org(&parent.to_string()).await?;
    if parent_org.is_some_and(|org| org != org_id) {
        tracing::warn!(org_id, session_id, "a subagent of a session not the org's");
        return Ok(());
    }
    let parent_row = match parent_org {
        Some(_) if super::sessions::runs_on_data_plane(cp, org_id, parent).await? => {
            cp.db.get_session(org_id, parent).await?
        }
        _ => None,
    };
    let Some(parent_row) = parent_row else {
        if chrono::Utc::now().timestamp_millis() - at_unix_ms < PARENT_WAIT_MS {
            anyhow::bail!(
                "subagent {session_id}: parent {parent} has no record yet; the feed sends it again"
            );
        }
        tracing::warn!(org_id, session_id, %parent, "a subagent whose parent never got a record; skipped");
        return Ok(());
    };
    // A child of its parent's own agent names the Engine's session-local ids
    // for that agent and its harness, which nothing here has (the v2
    // workflows suite's threads row stalled the feed on each): it runs its
    // parent's agent on its parent's harness.
    let agent_id = match named {
        Some(id) if cp.db.get_agent_config(org_id, id).await?.is_some() => Some(id),
        _ => parent_row.agent_id,
    };
    let harness_id = match parent_row.harness_id {
        Some(parent_harness)
            if crate::domains::harnesses::queries::resolve_effective(
                &cp.db,
                org_id,
                named_harness,
            )
            .await?
            .is_none() =>
        {
            parent_harness
        }
        _ => named_harness,
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

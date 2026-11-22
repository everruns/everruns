//! Subagents a data plane starts (data plane v2 parity, review fix): the
//! `subagent_record` host call gives each one a session record here, as a
//! v1 subagent has.
//!
//! Decisions:
//! - A cell starts a subagent itself (`crates/agentd/src/subagents.rs`), but
//!   asks here first: the record is made with the id the child then runs
//!   under, so the calls that read a session's record (Memory mounts, a
//!   connection acting as the turn's person, the console's lists) serve the
//!   child as they serve its parent. The answer carries the child's id and
//!   its Memory mounts, which the cell creates it with.
//! - The child belongs to its parent: same org, same owner and identity,
//!   source `subagent`, `parent_session_id` set. The parent must be one of
//!   the org's sessions on its data plane, so a cell can record children
//!   only for sessions it runs for that org.
//! - The record is marked as the data plane's (`dp_session_summaries`)
//!   before the cell creates the child, so nothing sends its messages to
//!   the platform's workers.
//! - A cell with no control plane (standalone) starts children without a
//!   record, as before.

use everruns_contracts::typed_id::{AgentId, HarnessId, SessionId};
use serde_json::{Value, json};

use super::ControlPlane;
use crate::domains::common::Ctx;
use crate::storage::{CreateSessionRow, DpSessionSummary};

/// Answer `subagent_record`: `{"session_id", "memory"}` for the new child.
pub async fn record(cp: &ControlPlane, ctx: &Ctx, params: &Value) -> anyhow::Result<Value> {
    let org_id = ctx.org_id();
    let parent: SessionId = params["parent_session_id"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("parent_session_id is required"))?
        .parse()?;
    let harness_id: HarnessId = params["harness_id"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("harness_id is required"))?
        .parse()?;
    let agent_id: Option<AgentId> = params["agent_id"].as_str().map(str::parse).transpose()?;
    anyhow::ensure!(
        super::sessions::runs_on_data_plane(cp, org_id, parent).await?
            && cp.db.dp_session_org(&parent.to_string()).await? == Some(org_id),
        "session {parent} is not one of the org's sessions on its data plane"
    );
    let parent_row = cp
        .db
        .get_session(org_id, parent)
        .await?
        .ok_or_else(|| anyhow::anyhow!("session {parent} has no record"))?;
    let child = cp
        .db
        .create_session(CreateSessionRow {
            org_id,
            source: crate::domains::sessions::record::SessionSource::Subagent,
            harness_id: Some(harness_id),
            agent_id,
            virtual_user_id: parent_row.virtual_user_id,
            playground_user_id: parent_row.playground_user_id,
            owner_principal_id: parent_row.owner_principal_id,
            resolved_owner_user_id: parent_row.resolved_owner_user_id,
            title: params["title"].as_str().map(str::to_owned),
            locale: params["locale"].as_str().map(str::to_owned),
            parent_session_id: Some(parent),
            ..CreateSessionRow::default()
        })
        .await?;
    let (key, _) =
        crate::domains::agent_definitions::key_for(&cp.db, org_id, harness_id, agent_id).await?;
    let plane = cp
        .db
        .dp_org(org_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("org {org_id} is not on a data plane"))?;
    let now = chrono::Utc::now();
    cp.db
        .dp_upsert_session_summary(DpSessionSummary {
            session_id: &child.id.to_string(),
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
    let memory = super::memory::mounts(cp, child.id).await?;
    Ok(json!({
        "session_id": child.id,
        "memory": memory,
        "servers": crate::injected::hosted::controller_servers(),
    }))
}

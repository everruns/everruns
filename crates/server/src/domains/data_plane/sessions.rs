//! A v2 org's sessions (data plane v2, step 8): the control plane keeps
//! the session's record and the data plane runs it.
//!
//! Decisions:
//! - `POST /v1/sessions` stays the control plane's: it writes the record
//!   (owner, title, tags, budgets) as for any org, then creates the session
//!   on the org's data plane under the same id, so the record and the
//!   running session are one session and the edge finds it by id.
//! - The session runs its agent's Engine key at the current revision. The
//!   key's revision is put in the org's config and delivered before the
//!   create, so the data plane never misses the agent it is asked to run.
//! - A create the data plane refuses fails the request: a record with no
//!   running session would take messages nothing answers. The data plane's
//!   create is idempotent, so a retry is safe.
//! - A session with overrides the Engine cannot express (its own prompt,
//!   tools, MCP servers, files, network access, capabilities) is refused
//!   for a v2 org rather than silently run without them (stage 1 is chat
//!   only).

use everruns_contracts::typed_id::{AgentId, HarnessId, SessionId};

use super::{ControlPlane, agent_published, for_org, org_public, outbox};
use crate::domains::agent_definitions::key_for;
use crate::domains::agent_definitions::sessions::Overrides;
use crate::storage::{CreateSessionRow, DpSessionSummary};

/// Tries at the data plane before the create fails.
const TRIES: u32 = 3;

/// Whether `org_id` runs on a data plane.
pub async fn on_data_plane(cp: &ControlPlane, org_id: i64) -> anyhow::Result<bool> {
    Ok(cp.db.dp_org(org_id).await?.is_some())
}

/// Why a session about to be created cannot run on a data plane yet, if
/// it cannot: a session-level setting the data plane would drop.
pub fn unsupported(row: &CreateSessionRow, own_capabilities: bool) -> Option<&'static str> {
    if own_capabilities {
        return Some("capabilities");
    }
    Overrides {
        system_prompt: row.system_prompt.as_deref(),
        tools: &row.tools,
        mcp_servers: &row.mcp_servers,
        initial_files: &row.initial_files,
        network_access: row.network_access.as_ref(),
        max_iterations: row.max_iterations.is_some(),
        parallel_tool_calls: row.parallel_tool_calls.is_some(),
    }
    .first()
}

/// Create the record `session`'s running session on `org_id`'s data plane.
/// A v1 org is left alone.
pub async fn created(
    cp: &ControlPlane,
    org_id: i64,
    session: SessionId,
    harness_id: HarnessId,
    agent_id: Option<AgentId>,
) -> anyhow::Result<()> {
    if !on_data_plane(cp, org_id).await? {
        return Ok(());
    }
    let (key, revision) = key_for(&cp.db, org_id, harness_id, agent_id).await?;
    agent_published(&cp.db, &key, org_id, &revision).await?;
    let plane = for_org(cp, org_id).await?;
    let public = org_public(org_id);
    let mut last = None;
    for attempt in 0..TRIES {
        if attempt > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(200 * u64::from(attempt))).await;
        }
        // Up to date first: a no-op when the dispatcher already delivered.
        let org = cp
            .db
            .dp_org(org_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("org {org_id} left its data plane"))?;
        if org.delivered_version < org.version || !org.enabled {
            let epoch = cp.db.dp_epoch().await?;
            if let Err(error) = outbox::deliver(cp, &org, epoch).await {
                last = Some(error);
                continue;
            }
        }
        match plane.create_session(&public, session, &key).await {
            Ok(()) => {
                // Marks the session as the data plane's: its messages go
                // there, never to the platform's workers. The feed's
                // summaries (version 1 and up) update it.
                let now = chrono::Utc::now();
                let data_plane = plane.id().unwrap_or_default().to_owned();
                cp.db
                    .dp_upsert_session_summary(DpSessionSummary {
                        session_id: &session.to_string(),
                        org_id,
                        data_plane_id: &data_plane,
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
                return Ok(());
            }
            Err(error) => last = Some(error),
        }
    }
    Err(last
        .unwrap_or_else(|| anyhow::anyhow!("no attempt made"))
        .context("the data plane did not create the session"))
}

/// Whether `session` of `org_id` runs on a data plane (created there by
/// [`created`]). Decision: a message send is a hot path, so whether the
/// org has a data plane at all is cached for a few seconds; only a v2
/// org's sends pay for the session lookup. An org leaving its data plane
/// keeps its sessions there, which the session lookup still answers.
pub async fn runs_on_data_plane(
    cp: &ControlPlane,
    org_id: i64,
    session: SessionId,
) -> anyhow::Result<bool> {
    static ORGS: std::sync::OnceLock<moka::future::Cache<i64, bool>> = std::sync::OnceLock::new();
    let orgs = ORGS.get_or_init(|| {
        moka::future::Cache::builder()
            .max_capacity(100_000)
            .time_to_live(std::time::Duration::from_secs(5))
            .build()
    });
    let v2 = match orgs.get(&org_id).await {
        Some(v2) => v2,
        None => {
            let v2 = cp.db.dp_org_ever(org_id).await?;
            orgs.insert(org_id, v2).await;
            v2
        }
    };
    if !v2 {
        return Ok(false);
    }
    cp.db.dp_session_on_data_plane(&session.to_string()).await
}

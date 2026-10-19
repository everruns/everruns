//! Organization health issue for the active-turn limit (`ORG_MAX_ACTIVE_TURNS`).
//!
//! Opened when a message is refused at the limit, resolved once the
//! organization's executing turns drop below it, so members see in Settings ->
//! Health why their messages are refused. Operators see the same refusals as a
//! warning log, which SaaS forwards to Sentry.
//!
//! Decisions:
//! - Recording is best effort and leaves the refused request alone: a
//!   detached task writes the issue, because the refusal is an error that rolls
//!   back any transaction the caller holds, and the write must not add latency
//!   to the refusal. A still-open issue is refreshed at most once a minute
//!   (see `observe_org_health_issue`), so a burst of refusals writes no rows.
//! - Resolution is a recount, not a hook: sessions leave `active` in many
//!   places. A sweep recounts the organizations with an open issue every
//!   minute, and "Check again" recounts one on demand.

use crate::domains::sessions::limits::OrgCaps;
use crate::storage::StorageBackend;
use chrono::Utc;
use std::sync::Arc;
use std::time::Duration;

/// Detector code of the organization-level active-turn limit issue.
pub const ACTIVE_TURN_LIMIT: &str = "org.active_turn_limit";

const SWEEP_INTERVAL: Duration = Duration::from_secs(60);

/// Title and body of the issue in `status`.
pub fn copy(status: &str) -> (String, String) {
    if status == "resolved" {
        return (
            "Active turn limit no longer reached".into(),
            "This organization is back under its limit of concurrently running turns.".into(),
        );
    }
    let limit = OrgCaps::from_env().max_active_turns;
    (
        "Active turn limit reached".into(),
        format!(
            "New messages are refused while this organization has {limit} turns running at once. They are accepted again as running turns finish."
        ),
    )
}

/// Record that a message was refused at the limit. Returns at once; see the
/// module notes for why the write is detached.
pub fn record_limit_reached(db: Arc<StorageBackend>, org_id: i64) {
    tokio::spawn(async move {
        if let Err(error) = observe(&db, org_id, "open").await {
            tracing::warn!(%error, org_id, "failed to record the active-turn limit health issue");
        }
    });
}

/// Recount the organization's executing turns: still at the limit keeps the
/// issue open with fresh evidence, under it resolves the issue.
pub async fn recheck(db: &StorageBackend, org_id: i64, caps: OrgCaps) -> anyhow::Result<()> {
    let active = db.count_org_active_turns(org_id).await?;
    let status = if active >= caps.max_active_turns as i64 {
        "open"
    } else {
        "resolved"
    };
    observe(db, org_id, status).await
}

async fn observe(db: &StorageBackend, org_id: i64, status: &str) -> anyhow::Result<()> {
    let (title, body) = copy("resolved");
    db.observe_org_health_issue(
        org_id,
        ACTIVE_TURN_LIMIT,
        status,
        Utc::now(),
        (&title, &body),
    )
    .await
}

/// Resolve open issues whose organization is back under the limit.
pub fn spawn_sweep(db: Arc<StorageBackend>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            match db.orgs_with_open_org_health_issue(ACTIVE_TURN_LIMIT).await {
                Ok(orgs) => {
                    let caps = OrgCaps::from_env();
                    for org_id in orgs {
                        if let Err(error) = recheck(&db, org_id, caps).await {
                            tracing::warn!(%error, org_id, "active-turn limit recheck failed");
                        }
                    }
                }
                Err(error) => tracing::warn!(%error, "active-turn limit sweep failed"),
            }
            tokio::time::sleep(SWEEP_INTERVAL).await;
        }
    })
}

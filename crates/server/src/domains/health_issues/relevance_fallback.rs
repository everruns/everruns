//! Organization health issue: relevance checks have no decision model and fall
//! back to the agent's own chat model (`crate::services::relevance`).
//!
//! Slack channels set to "Relevant messages" and room members set to
//! `relevant` still work without one, but each check spends the agent's model
//! tokens and answers with a label rather than a measured probability. The
//! issue tells the org's people to set up a decision model.
//!
//! Decisions:
//! - Opened by the check itself and recorded off the request path, like the
//!   active-turn limit. A process reports an org at most every ten minutes;
//!   the storage layer also refreshes an open issue at most once a minute.
//! - Resolved by the first check a decision model answers for the org, once
//!   per process unless a fallback reopened it, so a healthy org costs one
//!   read per process. "Check again" resolves it when the org has a decision
//!   model selected, or leaves the choice to a configured deployment.

use crate::storage::{StorageBackend, SystemDecisions};
use chrono::Utc;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Detector code of the organization-level fallback issue.
pub const CODE: &str = "org.relevance_check_fallback";

const REPORT_EVERY: Duration = Duration::from_secs(10 * 60);

/// What one server process has reported, so a busy org is not written on
/// every check. Shared by every relevance check of the process.
#[derive(Default)]
pub struct FallbackReports {
    /// Orgs reported, with when.
    reported: Mutex<HashMap<i64, Instant>>,
    /// Orgs answered by a decision model since their last report, so their
    /// issue is already resolved.
    clear: Mutex<HashSet<i64>>,
}

/// Title and body of the issue in `status`.
pub fn copy(status: &str) -> (String, String) {
    if status == "resolved" {
        return (
            "Relevance checks use a decision model again".into(),
            "Agents that answer relevant messages are checked by a decision model.".into(),
        );
    }
    (
        "Relevance checks fall back to the agent's model".into(),
        "No decision model is set up, so Slack channels set to Relevant messages and room agents \
         set to wake on relevant messages ask their own chat model, with less context. Those \
         answers are less reliable and spend the agent's model tokens. Pick a Default decision \
         model on the Models page and set System decisions to This organization's model."
            .into(),
    )
}

impl FallbackReports {
    /// Record that a check fell back. Returns at once.
    pub fn report(&self, db: Arc<StorageBackend>, org_id: i64, reason: &'static str) {
        self.clear.lock().unwrap().remove(&org_id);
        {
            let mut reported = self.reported.lock().unwrap();
            if reported
                .get(&org_id)
                .is_some_and(|at| at.elapsed() < REPORT_EVERY)
            {
                return;
            }
            reported.insert(org_id, Instant::now());
        }
        tracing::warn!(
            org_id,
            reason,
            "relevance check fell back to the agent's own model"
        );
        tokio::spawn(async move {
            if let Err(error) = observe(&db, org_id, "open").await {
                tracing::warn!(%error, org_id, "failed to record the relevance fallback health issue");
            }
        });
    }

    /// Record that a decision model answered a check. Returns at once.
    pub fn clear(&self, db: Arc<StorageBackend>, org_id: i64) {
        if !self.clear.lock().unwrap().insert(org_id) {
            return;
        }
        self.reported.lock().unwrap().remove(&org_id);
        tokio::spawn(async move {
            if let Err(error) = resolve_if_open(&db, org_id).await {
                tracing::warn!(%error, org_id, "failed to resolve the relevance fallback health issue");
            }
        });
    }
}

async fn resolve_if_open(db: &StorageBackend, org_id: i64) -> anyhow::Result<()> {
    if db
        .orgs_with_open_org_health_issue(CODE)
        .await?
        .contains(&org_id)
    {
        observe(db, org_id, "resolved").await?;
    }
    Ok(())
}

/// "Check again": resolved when a decision model would answer the org's
/// checks now, still open otherwise.
pub async fn recheck(
    db: &StorageBackend,
    org_id: i64,
    deployment_configured: bool,
) -> anyhow::Result<()> {
    let settings = db.get_organization_settings(org_id).await?;
    let organization = settings.as_ref().is_some_and(|settings| {
        SystemDecisions::from_db(&settings.system_decisions) == SystemDecisions::Organization
    });
    let ready = if organization {
        db.get_decision_default(org_id).await?.is_some()
    } else {
        deployment_configured
    };
    observe(db, org_id, if ready { "resolved" } else { "open" }).await
}

async fn observe(db: &StorageBackend, org_id: i64, status: &str) -> anyhow::Result<()> {
    let (title, body) = copy("resolved");
    db.observe_org_health_issue(org_id, CODE, status, Utc::now(), (&title, &body))
        .await
}

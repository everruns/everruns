//! A daily check that Slack still accepts the manifests Add to Slack builds.
//!
//! Decisions:
//! - The server asks Slack itself, through `apps.manifest.validate`, because
//!   Slack enforces limits it does not document (a 140-byte description was
//!   refused). Only Slack can answer, and this server already keeps every
//!   connected workspace's configuration token fresh, so no CI credential has
//!   to rotate.
//! - Any one connected workspace answers for the whole install: validation is
//!   read-only and Slack's rules are the same everywhere.
//! - The result is kept in memory on each server and reported by the system
//!   doctor. A day between checks is plenty for limits that change rarely,
//!   and a few calls per replica per day stays far below Slack's rate limits.

use std::sync::Mutex;
use std::time::Duration;

use chrono::{DateTime, Utc};

use super::SlackApiProvisioner;
use crate::channels::slack::events::worst_case_manifest_yamls;
use crate::domains::agent_channels::record::slack_provisioning::SlackProvisioningError;

pub(super) const CHECK_EVERY: Duration = Duration::from_secs(24 * 60 * 60);
/// Long enough after start for token rotation to have run once.
pub(super) const FIRST_CHECK_AFTER: Duration = Duration::from_secs(2 * 60);

/// What the last check found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SlackManifestOutcome {
    /// Slack accepted every worst-case manifest.
    Accepted { cases: usize },
    /// Slack refused at least one; each entry names the case and Slack's
    /// field errors (`pointer: message`).
    Refused {
        refusals: Vec<(String, Vec<String>)>,
    },
    /// No workspace is connected, so there is nothing to ask Slack with.
    NoWorkspace,
    /// The check could not get an answer from Slack.
    Failed(String),
}

#[derive(Clone, Debug)]
pub struct SlackManifestHealth {
    pub checked_at: DateTime<Utc>,
    pub outcome: SlackManifestOutcome,
}

static LAST: Mutex<Option<SlackManifestHealth>> = Mutex::new(None);

/// The last check on this server, if one has run.
pub fn last_manifest_health() -> Option<SlackManifestHealth> {
    LAST.lock().ok().and_then(|last| last.clone())
}

/// Keep `outcome` as this server's latest answer. Only the daily loop records,
/// so a test asking Slack through its own provisioner never leaks a result
/// into another test's doctor report.
pub(super) fn record(outcome: SlackManifestOutcome) {
    if let Ok(mut last) = LAST.lock() {
        *last = Some(SlackManifestHealth {
            checked_at: Utc::now(),
            outcome,
        });
    }
}

impl SlackApiProvisioner {
    /// Ask Slack about every worst-case manifest.
    pub(super) async fn check_manifests(&self) -> SlackManifestOutcome {
        let outcome = self.validate_worst_case_manifests().await;
        if !matches!(outcome, SlackManifestOutcome::Accepted { .. }) {
            tracing::warn!(?outcome, "Slack manifest check did not pass");
        }
        outcome
    }

    async fn validate_worst_case_manifests(&self) -> SlackManifestOutcome {
        // Every connected workspace, whatever its expiry: the due-for-rotation
        // query with a horizon no token reaches.
        let connected = match self
            .db
            .list_due_org_slack_connections(Utc::now() + chrono::Duration::days(36_500))
            .await
        {
            Ok(rows) => rows,
            Err(error) => return SlackManifestOutcome::Failed(format!("storage: {error}")),
        };
        let Some(row) = connected.into_iter().find(|row| row.team_id.is_some()) else {
            return SlackManifestOutcome::NoWorkspace;
        };
        let cases = worst_case_manifest_yamls();
        let total = cases.len();
        let mut refusals = Vec::new();
        for (case, yaml) in cases {
            // Slack's manifest API takes the manifest as a JSON-encoded
            // string, as `create_app` sends it.
            let manifest = match serde_yaml::from_str::<serde_json::Value>(&yaml)
                .ok()
                .and_then(|value| serde_json::to_string(&value).ok())
            {
                Some(json) => json,
                None => {
                    refusals.push((case.to_string(), vec!["not valid YAML".to_string()]));
                    continue;
                }
            };
            match self
                .manifest_call(
                    row.clone(),
                    "/apps.manifest.validate",
                    serde_json::json!({ "manifest": manifest }),
                )
                .await
            {
                Ok(_) => {}
                Err(SlackProvisioningError::RejectedWithProblems { problems, .. }) => {
                    refusals.push((case.to_string(), problems));
                }
                Err(SlackProvisioningError::Rejected(code)) if code == "invalid_manifest" => {
                    refusals.push((case.to_string(), Vec::new()));
                }
                Err(error) => return SlackManifestOutcome::Failed(error.to_string()),
            }
        }
        if refusals.is_empty() {
            SlackManifestOutcome::Accepted { cases: total }
        } else {
            SlackManifestOutcome::Refused { refusals }
        }
    }
}

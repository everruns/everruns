//! Health issues for agents whose MCP server sign-in stopped working, or will
//! stop soon (knowledge/integrations/mcp-oauth-grant-health.md).
//!
//! An agent acting as its own service account signs in to an OAuth MCP server
//! once. When that grant can no longer produce a token, the server's tools
//! drop out of every turn; before this, nobody was told (the Kids Duties
//! agent lost stend and visti for 11 days). The sweep reads each grant's
//! [`GrantHealth`] and keeps one issue per agent and server.
//!
//! Decisions:
//! - One issue per agent and server (`subject_id` = `<agent>:<provider>`),
//!   opened while the grant expires within `EXPIRY_WARNING` with nothing to
//!   renew it, and kept open, with fresh evidence, once it needs sign-in. The
//!   move from "expires soon" to "needs sign-in" tells people again.
//! - It resolves on its own once someone signs in again (the grant is usable
//!   or no longer watched) or the server leaves the catalog.
//! - The fix is a person signing in on the agent's MCP tab: Everruns cannot do
//!   it for them, so the issue offers that tab as its one step.
//! - The sweep reads only: it never refreshes a grant. A v2 org's grant is
//!   refreshed on its data plane, and a refresh from here would spend the
//!   refresh token the data plane holds.
use super::background_work::{agent_recipients, record, resolve};
use crate::storage::background_work::SubjectHealthIssue;
use crate::storage::connection_resolver::grant_health::{EXPIRY_WARNING, reason};
use crate::storage::encryption::EncryptionService;
use crate::storage::{DbConnectionResolver, GrantHealth, HealthIssueRow, StorageBackend};
use chrono::Utc;
use everruns_contracts::typed_id::VirtualUserId;
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

/// An agent's sign-in to an MCP server needs renewing.
pub const MCP_SIGN_IN: &str = "mcp.sign_in_required";

const SWEEP_INTERVAL: Duration = Duration::from_secs(15 * 60);

/// The agent's MCP tab, where its service account signs in.
pub fn sign_in_href(agent_public_id: &str) -> String {
    format!("/agents/{agent_public_id}?tab=mcp")
}

pub fn reason_text(reason: &str) -> &'static str {
    match reason {
        reason::UNBOUND => {
            "it was signed in before Everruns tied sign-ins to their server, so it cannot renew itself"
        }
        reason::SETTINGS_CHANGED => "the server's sign-in settings changed since it was signed in",
        reason::NO_REFRESH_TOKEN => "the server gave no way to renew it",
        reason::REJECTED => "the server keeps rejecting its token",
        _ => "the server refused to renew it",
    }
}

/// Title and body of an issue in `status`.
pub fn copy(row: &HealthIssueRow) -> (String, String) {
    let evidence = row.evidence.clone().unwrap_or_default();
    let server = evidence["server"].as_str().unwrap_or("An MCP server");
    let agent = row.agent_name.as_deref().unwrap_or("The agent");
    if row.status == "resolved" {
        return (
            format!("{server} sign-in renewed"),
            "Nothing needs doing.".into(),
        );
    }
    let why = reason_text(evidence["reason"].as_str().unwrap_or_default());
    if evidence["state"].as_str() == Some("expires_soon") {
        let at = evidence["expires_at"]
            .as_str()
            .and_then(|at| chrono::DateTime::parse_from_rfc3339(at).ok())
            .map(|at| at.format("%Y-%m-%d %H:%M UTC").to_string())
            .unwrap_or_else(|| "soon".into());
        return (
            format!("{server} sign-in expires soon"),
            format!(
                "{agent}'s sign-in to {server} stops working on {at}: {why}. Sign in again on the agent's MCP tab to keep its tools."
            ),
        );
    }
    (
        format!("{server} needs sign-in again"),
        format!(
            "{agent} can no longer use {server}: {why}. Its tools are missing from every turn until someone signs in again on the agent's MCP tab."
        ),
    )
}

/// One pass over every watched grant.
pub async fn sweep(db: &StorageBackend, encryption: &EncryptionService) -> anyhow::Result<()> {
    let resolver = DbConnectionResolver::new(
        db.clone(),
        encryption.clone(),
        None,
        // Never used: the sweep only reads grants.
        Arc::new(everruns_core::host::DirectEgressService::for_runtime_traffic_from_env()),
    );
    let open = db.open_subject_health_issues(MCP_SIGN_IN).await?;
    let mut seen = Vec::new();
    for watched in db
        .service_mcp_grants_to_watch(Utc::now() + EXPIRY_WARNING)
        .await?
    {
        let Some(server_id) = watched
            .provider
            .strip_prefix("mcp_oauth_")
            .and_then(|id| id.parse::<Uuid>().ok())
        else {
            continue;
        };
        let Some(server) = db
            .get_mcp_server(watched.org_id, server_id)
            .await?
            .filter(|s| s.status == "active")
        else {
            continue;
        };
        let Some(row) = db
            .get_virtual_user_connection(
                VirtualUserId::from_uuid(watched.virtual_user_id),
                &watched.provider,
            )
            .await?
        else {
            continue;
        };
        let health = match resolver
            .mcp_grant_health(watched.org_id, server_id, &row)
            .await
        {
            Ok(health) => health,
            Err(error) => {
                tracing::warn!(%error, connection_id = %row.id, "Could not read an MCP grant's health");
                continue;
            }
        };
        let (state, reason, at) = match health {
            GrantHealth::Usable => continue,
            GrantHealth::ExpiresSoon { at, .. } if at > Utc::now() + EXPIRY_WARNING => continue,
            GrantHealth::ExpiresSoon { at, reason } => ("expires_soon", reason, Some(at)),
            GrantHealth::NeedsSignIn { reason, since } => ("needs_sign_in", reason, since),
        };
        let Some((public_id, name)) = db.agent_label(watched.agent_id).await? else {
            continue;
        };
        let subject = format!("{public_id}:{}", watched.provider);
        // Worse than when people last heard: tell them again.
        let resurface = state == "needs_sign_in"
            && open.iter().any(|issue| {
                issue.subject_id.as_deref() == Some(subject.as_str())
                    && issue
                        .evidence
                        .as_ref()
                        .is_some_and(|e| e["state"] == "expires_soon")
            });
        let issue = SubjectHealthIssue {
            org_id: watched.org_id,
            code: MCP_SIGN_IN,
            subject_id: &subject,
            agent_id: Some(watched.agent_id),
            session_id: None,
            status: "open",
            evidence: serde_json::json!({
                "server": server.name,
                "provider": watched.provider,
                "state": state,
                "reason": reason,
                "expires_at": at.map(|at| at.to_rfc3339()),
            }),
            checked_at: Utc::now(),
        };
        let recipients = agent_recipients(db, watched.org_id, Some(watched.agent_id)).await;
        record(db, issue, recipients, Some((public_id, name)), resurface).await;
        seen.push((watched.org_id, subject));
    }
    for row in open {
        if !seen
            .iter()
            .any(|(org, id)| *org == row.org_id && Some(id.as_str()) == row.subject_id.as_deref())
        {
            resolve(db, &row).await?;
        }
    }
    Ok(())
}

/// Run [`sweep`] every 15 minutes.
pub fn spawn_sweep(
    db: Arc<StorageBackend>,
    encryption: Arc<EncryptionService>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            if let Err(error) = sweep(&db, &encryption).await {
                tracing::warn!(%error, "MCP sign-in health sweep failed");
            }
            let jitter = Uuid::new_v4().as_u128() % 61;
            tokio::time::sleep(SWEEP_INTERVAL + Duration::from_secs(jitter as u64)).await;
        }
    })
}

/// The one step that fixes it, shown on the issue: sign in on the MCP tab.
pub fn fixer(row: &HealthIssueRow) -> Option<super::fixers::IssueFixer> {
    let agent = row.agent_public_id.as_deref()?;
    let server = row
        .evidence
        .as_ref()
        .and_then(|e| e["server"].as_str())
        .unwrap_or("the server");
    Some(super::fixers::IssueFixer {
        kind: "mcp.sign_in".into(),
        label: "Sign in again".into(),
        steps: vec![
            "Open the agent's MCP tab.".into(),
            format!("Sign in to {server} again as the agent."),
            "Everruns checks the sign-in again within 15 minutes, or when you press Check again."
                .into(),
        ],
        effect: "consent".into(),
        undoable: false,
        needs_provider_admin: false,
        // Only the person can sign in; the card links the step instead of
        // running anything here.
        unavailable_reason: Some(format!("Sign in to {server} on the agent's MCP tab.")),
        unblock: Some(super::fixers::FixerUnblock {
            label: "Open MCP tab".into(),
            href: sign_in_href(agent),
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(status: &str, evidence: serde_json::Value) -> HealthIssueRow {
        HealthIssueRow {
            id: Uuid::nil(),
            org_id: 1,
            channel_id: None,
            channel_public_id: None,
            agent_public_id: Some("agent_1".into()),
            agent_name: Some("Kids Duties".into()),
            code: MCP_SIGN_IN.into(),
            episode_id: Uuid::nil(),
            status: status.into(),
            missing_scopes: Vec::new(),
            error_code: None,
            channel_revision: None,
            first_detected_at: Utc::now(),
            last_checked_at: Utc::now(),
            subject_id: Some("agent_1:mcp_oauth_x".into()),
            session_id: None,
            evidence: Some(evidence),
        }
    }

    #[test]
    fn copy_names_the_agent_server_and_why() {
        let (title, body) = copy(&row(
            "open",
            serde_json::json!({"server": "stend", "state": "needs_sign_in", "reason": "unbound"}),
        ));
        assert_eq!(title, "stend needs sign-in again");
        assert!(
            body.starts_with("Kids Duties can no longer use stend"),
            "{body}"
        );
        let (title, body) = copy(&row(
            "open",
            serde_json::json!({
                "server": "visti", "state": "expires_soon", "reason": "unbound",
                "expires_at": "2026-11-10T23:16:00+00:00",
            }),
        ));
        assert_eq!(title, "visti sign-in expires soon");
        assert!(body.contains("2026-11-10 23:16 UTC"), "{body}");
        assert_eq!(
            copy(&row("resolved", serde_json::json!({"server": "stend"}))).0,
            "stend sign-in renewed"
        );
    }

    #[test]
    fn the_fixer_links_the_agents_mcp_tab() {
        let fixer = fixer(&row("open", serde_json::json!({"server": "stend"}))).unwrap();
        assert_eq!(fixer.unblock.unwrap().href, "/agents/agent_1?tab=mcp");
        assert!(fixer.unavailable_reason.is_some());
    }
}

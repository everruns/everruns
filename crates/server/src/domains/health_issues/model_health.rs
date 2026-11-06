//! Health issues about the model an agent or the organization is set to use
//! (knowledge/operations/health-issues.md, "Model detectors"): the model was
//! turned off, the provider no longer lists it, it is not a chat model, or
//! its provider is turned off. Chats, channels (Slack included) and schedules
//! that use it can then fail to answer, and the model pickers flag the setting.
//!
//! Decisions:
//! - One rule for "usable", the one the model pickers apply: enabled, still
//!   listed by its provider, a chat model, on an active provider. An issue
//!   here and the picker's "The configured model is unavailable" agree.
//! - Agents are judged on the model they name. An agent without one uses the
//!   organization default, which is its own issue, so a broken default is one
//!   issue rather than one per agent.
//! - The sweep and "Check again" judge every configured model; a provider sync
//!   or a model change rechecks its organization at once, so fixing the cause
//!   resolves the issue without waiting for the next sweep.

use super::background_work::{agent_recipients, record, resolve};
use crate::domains::models::ModelService;
use crate::storage::StorageBackend;
use crate::storage::background_work::{ConfiguredModelRow, SubjectHealthIssue};
use chrono::Utc;

/// The model an agent names cannot be used.
pub const AGENT_MODEL_UNAVAILABLE: &str = "agent.model_unavailable";
/// The organization's default model cannot be used.
pub const ORG_DEFAULT_MODEL_UNAVAILABLE: &str = "org.default_model_unavailable";

/// Every code this detector opens.
pub const CODES: [&str; 2] = [AGENT_MODEL_UNAVAILABLE, ORG_DEFAULT_MODEL_UNAVAILABLE];

/// Whether `code` is one of these detectors.
pub fn owns(code: &str) -> bool {
    CODES.contains(&code)
}

/// Why a configured model cannot be used, as an evidence `reason`.
pub fn problem(row: &ConfiguredModelRow) -> Option<&'static str> {
    if row.provider_status != "active" {
        Some("provider_inactive")
    } else if !row.enabled {
        Some("disabled")
    } else if !ModelService::row_is_chat_model(&row.capabilities, row.provider_metadata.as_ref()) {
        Some("not_chat")
    } else if row.no_longer_listed {
        Some("no_longer_listed")
    } else {
        None
    }
}

/// Title and body of one of these issues in `status`.
pub fn copy_for(
    code: &str,
    status: &str,
    evidence: &serde_json::Value,
    agent: Option<&str>,
) -> (String, String) {
    let org_default = code == ORG_DEFAULT_MODEL_UNAVAILABLE;
    if status == "resolved" {
        let title = if org_default {
            "Default model is usable again"
        } else {
            "Agent's model is usable again"
        };
        return (title.into(), "Nothing needs doing.".into());
    }
    let model = evidence["model"].as_str().unwrap_or("its model");
    let provider = evidence["provider"].as_str().unwrap_or("its provider");
    let reason = evidence["reason"].as_str();
    if !matches!(reason, Some("provider_inactive" | "disabled" | "not_chat")) {
        return listing_copy(org_default, agent, model, provider, evidence);
    }
    let (why, fix) = match reason {
        Some("provider_inactive") => (
            format!("is on {provider}, which is turned off"),
            format!("turn {provider} back on from the Models page"),
        ),
        Some("disabled") => (
            "is turned off on the Models page".to_string(),
            "turn it back on".to_string(),
        ),
        _ => (
            "is not a chat model".to_string(),
            "choose a chat model".to_string(),
        ),
    };
    if org_default {
        (
            "Default model is unavailable".into(),
            format!(
                "The organization's default model, {model} ({provider}), {why}. Agents without a model of their own use it, so their chats and channels can fail to answer. Choose another default model, or {fix}."
            ),
        )
    } else {
        let agent = agent.unwrap_or("An agent");
        (
            "Agent's model is unavailable".into(),
            format!(
                "{agent} is set to use {model} ({provider}), which {why}. Its chats and channels, Slack included, can fail to answer. Choose another model in the agent's settings, or {fix}."
            ),
        )
    }
}

/// Copy for a model missing from the provider's last model list.
///
/// Decision: say what was observed, not a conclusion. The list is read only
/// when the provider's models are synced, which a deployment may never do on
/// its own, so it can be days old, and missing one spelling is not proof the
/// provider dropped the model. The copy names the id and the date of the list
/// so a person can tell an old list from a real removal, and the issue offers
/// the sync that settles it (`fixers::MODELS_SYNC`).
fn listing_copy(
    org_default: bool,
    agent: Option<&str>,
    model: &str,
    provider: &str,
    evidence: &serde_json::Value,
) -> (String, String) {
    let id = evidence["model_id"]
        .as_str()
        .map(|id| format!(" {id}"))
        .unwrap_or_else(|| " it".into());
    let list = evidence["synced_at"]
        .as_str()
        .and_then(|at| chrono::DateTime::parse_from_rfc3339(at).ok())
        .map(|at| format!("{provider}'s model list from {}", at.format("%b %-d")))
        .unwrap_or_else(|| format!("{provider}'s last model list"));
    let (title, subject, other) = if org_default {
        (
            "Default model not found at its provider",
            format!("The organization's default model, {model} ({provider})"),
            "choose another default model",
        )
    } else {
        (
            "Agent's model not found at its provider",
            format!("{} uses {model} ({provider})", agent.unwrap_or("An agent")),
            "choose another model in the agent's settings",
        )
    };
    (
        title.into(),
        format!(
            "{subject}, but {list} does not include{id}. Sync {provider}'s models to check again: if {provider} still offers it, this clears. If not, {other}, since its chats and channels can fail to answer."
        ),
    )
}

/// One pass over every organization.
pub async fn sweep(db: &StorageBackend) -> anyhow::Result<()> {
    check(db, None).await
}

/// Recheck one organization now, after something its models depend on
/// changed. Best effort: a failure waits for the next sweep.
pub async fn check_org(db: &StorageBackend, org_id: i64) {
    if let Err(error) = check(db, Some(org_id)).await {
        tracing::warn!(%error, org_id, "Model health check failed");
    }
}

async fn check(db: &StorageBackend, org_id: Option<i64>) -> anyhow::Result<()> {
    let rows = db.configured_models(org_id).await?;
    let mut seen: Vec<(&'static str, i64, String)> = Vec::new();
    for row in &rows {
        let Some(reason) = problem(row) else {
            continue;
        };
        let evidence = serde_json::json!({
            "reason": reason,
            "model": row.model_name,
            "model_id": row.model_id,
            "provider": row.provider_name,
            "provider_id": row.provider_id,
            "synced_at": row.last_synced_at.map(|at| at.to_rfc3339()),
        });
        let (code, subject, agent) = match row.agent_id {
            Some(agent_id) => {
                let Some(label) = db.agent_label(agent_id).await? else {
                    continue;
                };
                (AGENT_MODEL_UNAVAILABLE, label.0.clone(), Some(label))
            }
            None => (
                ORG_DEFAULT_MODEL_UNAVAILABLE,
                everruns_core::org_public_id_from_internal(row.org_id),
                None,
            ),
        };
        // A person dismissed this finding: leave it closed.
        if db.finding_dismissed(row.org_id, code, &subject).await? {
            seen.push((code, row.org_id, subject));
            continue;
        }
        let issue = SubjectHealthIssue {
            org_id: row.org_id,
            code,
            subject_id: &subject,
            agent_id: row.agent_id,
            session_id: None,
            status: "open",
            evidence,
            checked_at: Utc::now(),
        };
        let recipients = agent_recipients(db, row.org_id, row.agent_id).await;
        record(db, issue, recipients, agent, false).await;
        seen.push((code, row.org_id, subject));
    }
    for code in CODES {
        for open in db.open_subject_health_issues(code).await? {
            if org_id.is_some_and(|org| org != open.org_id) {
                continue;
            }
            let still = seen.iter().any(|(c, org, subject)| {
                *c == code
                    && *org == open.org_id
                    && Some(subject.as_str()) == open.subject_id.as_deref()
            });
            if !still {
                resolve(db, &open).await?;
            }
        }
    }
    Ok(())
}

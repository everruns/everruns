//! GitHub triggers: GitHub events delivered through the agent's own GitHub App.
//!
//! Design decisions:
//! - **No per-trigger secret.** A GitHub trigger listens on the webhook of the
//!   GitHub App its agent's identity created ("Connect GitHub",
//!   `crate::github_apps`). GitHub signs every delivery with that App's
//!   webhook secret, so the ingress authenticates the App once and fans the
//!   event out to the identity's GitHub triggers.
//! - **Same credential for events and actions.** The session runs as the
//!   agent's identity, whose `github` connection is that App's installation,
//!   so the agent answers with the identity that received the event.
//! - **Unsubscribed events are not recorded.** An App receives every event
//!   type it subscribes to; recording each one a trigger ignores would flood
//!   the delivery log. Repository and custom-filter misses are recorded as
//!   `filtered`, since those are the ones a user debugs.
//! - **The App never wakes itself.** Events sent by the App's own bot are
//!   dropped, so a comment the agent posts cannot trigger it again.
//! - Everything after normalization (dedupe on `X-GitHub-Delivery`, per pull
//!   request sessions, filters, delivery log) is the shared event pipeline in
//!   [`super::events`].

use super::events::{self, TriggerEvent, TriggerEventOutcome, TriggerEventRoute};
use super::queries as q;
use super::types::{CreateAgentTriggerRequest, UpdateAgentTriggerRequest};
use crate::domains::common::{CommandError, Ctx, classify_anyhow};
use crate::domains::messages::MessageService;
use crate::domains::sessions::SessionService;
use crate::records::{
    AgentTrigger, AgentTriggerType, GitHubTriggerConfig, TriggerEventFilter, TriggerFilterCondition,
};
use crate::storage::StorageBackend;
use crate::storage::github_app_rows::GitHubAppRow;
use crate::storage::models::AgentRow;
use everruns_contracts::typed_id::{AgentId, TriggerId};
use serde_json::{Value, json};
use std::sync::Arc;

/// Events a GitHub trigger subscribes to when the request names none: the
/// moments a pull request summary is worth (re)writing.
pub const DEFAULT_EVENTS: &[&str] = &[
    "pull_request.opened",
    "pull_request.reopened",
    "pull_request.synchronize",
    "pull_request.ready_for_review",
];

const MAX_LIST_LEN: usize = 50;

/// A signed delivery, as received from GitHub.
#[derive(Debug, Clone)]
pub struct GitHubDelivery {
    /// `X-GitHub-Event`.
    pub event: String,
    /// `X-GitHub-Delivery`; the dedupe key.
    pub delivery_id: String,
    /// Parsed JSON body.
    pub payload: Value,
}

/// What one delivery did to one trigger.
#[derive(Debug)]
pub struct GitHubDispatch {
    pub trigger_id: TriggerId,
    pub outcome: Result<TriggerEventOutcome, CommandError>,
}

// ============================================================================
// Configuration
// ============================================================================

/// Build the stored config for a new GitHub trigger.
///
/// The agent's identity must already hold a GitHub App: without one, nothing
/// would ever deliver events to this trigger.
pub(super) async fn create_config(
    ctx: &Ctx,
    agent: &AgentRow,
    req: &CreateAgentTriggerRequest,
) -> Result<Value, CommandError> {
    let identity_id = agent.virtual_user_id.ok_or_else(connect_first)?;
    ctx.db
        .get_github_app_for_identity(ctx.org_id(), identity_id)
        .await
        .map_err(classify_anyhow)?
        .ok_or_else(connect_first)?;
    let config = GitHubTriggerConfig {
        events: normalize_events(req.github_events.clone())?,
        repositories: normalize_repositories(req.repositories.clone())?,
        session_mode: req.session_mode,
        message: require_message(&req.message)?,
        filter: events::optional_filter(req.filter.clone())?,
    };
    to_value(&config)
}

/// Apply a partial update to a stored GitHub trigger config.
pub(super) fn update_config(
    trigger: &AgentTrigger,
    req: &UpdateAgentTriggerRequest,
) -> Result<Value, CommandError> {
    let mut config = trigger
        .github_config()
        .map_err(|_| CommandError::bad_request("Invalid stored GitHub trigger configuration"))?;
    if let Some(list) = &req.github_events {
        config.events = normalize_events(Some(list.clone()))?;
    }
    if let Some(list) = &req.repositories {
        config.repositories = normalize_repositories(Some(list.clone()))?;
    }
    if let Some(mode) = req.session_mode {
        events::validate_trigger_binding(mode, true)?;
        config.session_mode = mode;
    }
    if let Some(message) = &req.message {
        config.message = require_message(message)?;
    }
    if let Some(filter) = &req.filter {
        config.filter = events::optional_filter(Some(filter.clone()))?;
    }
    to_value(&config)
}

fn connect_first() -> CommandError {
    CommandError::bad_request(
        "A GitHub trigger needs GitHub connected on the agent's identity first",
    )
}

fn to_value(config: &GitHubTriggerConfig) -> Result<Value, CommandError> {
    serde_json::to_value(config).map_err(|e| CommandError::internal(e.into()))
}

fn require_message(message: &str) -> Result<String, CommandError> {
    if message.trim().is_empty() {
        return Err(CommandError::bad_request(
            "GitHub trigger requires a non-empty message",
        ));
    }
    Ok(message.to_string())
}

fn is_event_token(token: &str) -> bool {
    !token.is_empty()
        && token
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// `name` or `name.action`, lowercase; defaults when none are given.
fn normalize_events(events: Option<Vec<String>>) -> Result<Vec<String>, CommandError> {
    let mut events: Vec<String> = events
        .unwrap_or_default()
        .into_iter()
        .map(|event| event.trim().to_ascii_lowercase())
        .filter(|event| !event.is_empty())
        .collect();
    if events.is_empty() {
        return Ok(DEFAULT_EVENTS
            .iter()
            .map(|event| event.to_string())
            .collect());
    }
    if events.len() > MAX_LIST_LEN {
        return Err(CommandError::bad_request(format!(
            "A GitHub trigger may subscribe to at most {MAX_LIST_LEN} events"
        )));
    }
    for event in &events {
        let mut parts = event.splitn(2, '.');
        let name_ok = parts.next().is_some_and(is_event_token);
        let action_ok = parts.next().is_none_or(is_event_token);
        if !name_ok || !action_ok {
            return Err(CommandError::bad_request(format!(
                "Invalid GitHub event '{event}': use an event name such as pull_request, \
                 optionally with an action such as pull_request.opened"
            )));
        }
    }
    events.sort();
    events.dedup();
    Ok(events)
}

/// `owner/name`, compared case-insensitively as GitHub does.
fn normalize_repositories(repositories: Option<Vec<String>>) -> Result<Vec<String>, CommandError> {
    let mut repositories: Vec<String> = repositories
        .unwrap_or_default()
        .into_iter()
        .map(|repo| repo.trim().to_ascii_lowercase())
        .filter(|repo| !repo.is_empty())
        .collect();
    if repositories.len() > MAX_LIST_LEN {
        return Err(CommandError::bad_request(format!(
            "A GitHub trigger may name at most {MAX_LIST_LEN} repositories"
        )));
    }
    let segment_ok = |segment: &str| {
        !segment.is_empty()
            && segment != "."
            && segment != ".."
            && segment
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    };
    for repo in &repositories {
        let valid = repo
            .split_once('/')
            .is_some_and(|(owner, name)| segment_ok(owner) && segment_ok(name));
        if !valid {
            return Err(CommandError::bad_request(format!(
                "Invalid repository '{repo}': use owner/name"
            )));
        }
    }
    repositories.sort();
    repositories.dedup();
    Ok(repositories)
}

// ============================================================================
// Delivery
// ============================================================================

/// The normalized GitHub facts templates and filters read as `github.*`.
fn github_context(delivery: &GitHubDelivery) -> Value {
    let payload = &delivery.payload;
    let action = payload.get("action").and_then(Value::as_str);
    let item = payload
        .get("pull_request")
        .or_else(|| payload.get("issue"))
        .filter(|item| item.is_object());
    let repository = payload
        .pointer("/repository/full_name")
        .and_then(Value::as_str);
    json!({
        "event": delivery.event,
        "action": action,
        "event_action": event_key(&delivery.event, action),
        "delivery": delivery.delivery_id,
        "repository": repository,
        "number": item.and_then(|item| item.get("number")).or_else(|| payload.get("number")),
        "title": item.and_then(|item| item.get("title")),
        "url": item.and_then(|item| item.get("html_url")),
        "is_pull_request": payload.get("pull_request").is_some()
            || payload.pointer("/issue/pull_request").is_some(),
        "sender": payload.pointer("/sender/login"),
    })
}

fn event_key(event: &str, action: Option<&str>) -> String {
    match action {
        Some(action) => format!("{event}.{action}"),
        None => event.to_string(),
    }
}

/// `owner/repo#12` for pull request and issue events, `owner/repo` otherwise.
fn subject(github: &Value) -> Option<String> {
    let repository = github.get("repository").and_then(Value::as_str)?;
    Some(match github.get("number").and_then(Value::as_i64) {
        Some(number) => format!("{repository}#{number}"),
        None => repository.to_string(),
    })
}

fn subscribes(config: &GitHubTriggerConfig, event: &str, action: Option<&str>) -> bool {
    let with_action = event_key(event, action);
    config
        .events
        .iter()
        .any(|subscribed| subscribed == event || *subscribed == with_action)
}

/// Whether the App's own bot sent this event.
fn sent_by_app(app: &GitHubAppRow, payload: &Value) -> bool {
    let bot_login = format!("{}[bot]", app.slug);
    payload
        .pointer("/sender/login")
        .and_then(Value::as_str)
        .is_some_and(|login| login.eq_ignore_ascii_case(&bot_login))
}

/// Hand one verified delivery to every GitHub trigger of the App's identity.
pub async fn dispatch_github_delivery(
    db: &Arc<StorageBackend>,
    session_service: &SessionService,
    message_service: &MessageService,
    app: &GitHubAppRow,
    delivery: &GitHubDelivery,
    request_id: Option<String>,
) -> Result<Vec<GitHubDispatch>, CommandError> {
    if sent_by_app(app, &delivery.payload) {
        return Ok(Vec::new());
    }
    let action = delivery.payload.get("action").and_then(Value::as_str);
    let github = github_context(delivery);
    let subject = subject(&github);

    let triggers = db
        .list_agent_triggers(app.org_id, None, false)
        .await
        .map_err(classify_anyhow)?
        .into_iter()
        .filter(|row| {
            row.enabled
                && row.status == "active"
                && row.trigger_type == AgentTriggerType::GitHub.to_string()
        });

    let mut results = Vec::new();
    for row in triggers {
        let Some(agent) = db
            .get_agent(row.org_id, row.agent_id)
            .await
            .map_err(classify_anyhow)?
            .filter(|agent| agent.status == "active" && !agent.exposures_suspended)
            .filter(|agent| agent.virtual_user_id == Some(app.virtual_user_id))
        else {
            continue;
        };
        let Ok(agent_public) = agent.public_id.parse::<AgentId>() else {
            continue;
        };
        let trigger = q::row_to_trigger(row.clone(), agent_public, None);
        let Ok(config) = trigger.github_config() else {
            tracing::warn!(trigger_id = %row.id, "invalid GitHub trigger configuration");
            continue;
        };
        if !subscribes(&config, &delivery.event, action) {
            continue;
        }

        // Repository scope is a filter condition so a miss is recorded.
        let mut filter = config.filter.clone().unwrap_or_default();
        if !config.repositories.is_empty() {
            filter.conditions.push(TriggerFilterCondition {
                path: "github.repository_key".to_string(),
                any_of: config
                    .repositories
                    .iter()
                    .map(|repo| Value::String(repo.clone()))
                    .collect(),
            });
        }
        let filter: Option<TriggerEventFilter> = (!filter.conditions.is_empty()).then_some(filter);

        let mut github = github.clone();
        github["repository_key"] = github
            .get("repository")
            .and_then(Value::as_str)
            .map(|repo| Value::String(repo.to_ascii_lowercase()))
            .unwrap_or(Value::Null);
        let context = json!({
            "agent": { "id": agent.public_id, "name": agent.name },
            "trigger": { "id": row.id.to_string(), "type": "github" },
            "invocation": {
                "source": "github",
                "triggered_at": chrono::Utc::now().to_rfc3339(),
            },
            "github": github,
            "payload": delivery.payload,
        });
        let outcome = events::dispatch_trigger_event(
            db,
            session_service,
            message_service,
            TriggerEventRoute {
                trigger: &row,
                agent: &agent,
                message_template: &config.message,
                // per_thread without a subject (App-level events) falls back to
                // one session per event in the pipeline.
                session_mode: config.session_mode,
                filter: filter.as_ref(),
                session_source: crate::records::SessionSource::Webhook,
                webhook_compat: None,
            },
            TriggerEvent {
                source: "github",
                event_id: Some(delivery.delivery_id.clone()),
                event_type: Some(event_key(&delivery.event, action)),
                subject: subject.clone(),
                context,
            },
            request_id.clone(),
        )
        .await;
        if let Err(err) = &outcome {
            tracing::warn!(trigger_id = %row.id, error = %err, "GitHub trigger dispatch failed");
        }
        results.push(GitHubDispatch {
            trigger_id: row.id,
            outcome,
        });
    }
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_core::channel::SessionBinding;

    #[test]
    fn events_default_normalize_and_reject_garbage() {
        assert_eq!(normalize_events(None).unwrap(), DEFAULT_EVENTS);
        assert_eq!(
            normalize_events(Some(vec![
                " Pull_Request.Opened ".into(),
                "issue_comment".into(),
                "issue_comment".into()
            ]))
            .unwrap(),
            vec!["issue_comment", "pull_request.opened"]
        );
        for bad in [
            "pull request",
            "pull_request.",
            ".opened",
            "a.b.c",
            "pr;drop",
        ] {
            assert!(normalize_events(Some(vec![bad.into()])).is_err(), "{bad}");
        }
    }

    #[test]
    fn repositories_must_be_owner_slash_name() {
        assert_eq!(
            normalize_repositories(Some(vec!["Acme/API".into(), "acme/api".into()])).unwrap(),
            vec!["acme/api"]
        );
        for bad in ["acme", "acme/", "/api", "acme/../x", "acme/a b", "a/b/c"] {
            assert!(
                normalize_repositories(Some(vec![bad.into()])).is_err(),
                "{bad}"
            );
        }
    }

    fn delivery(event: &str, payload: Value) -> GitHubDelivery {
        GitHubDelivery {
            event: event.to_string(),
            delivery_id: "d-1".to_string(),
            payload,
        }
    }

    #[test]
    fn context_and_subject_for_pull_requests_issues_and_repo_events() {
        let pr = github_context(&delivery(
            "pull_request",
            json!({
                "action": "opened",
                "number": 12,
                "pull_request": {"number": 12, "title": "Fix", "html_url": "https://x/pull/12"},
                "repository": {"full_name": "acme/api"},
                "sender": {"login": "octo"}
            }),
        ));
        assert_eq!(pr["event_action"], "pull_request.opened");
        assert_eq!(pr["is_pull_request"], true);
        assert_eq!(subject(&pr).as_deref(), Some("acme/api#12"));

        let comment = github_context(&delivery(
            "issue_comment",
            json!({
                "action": "created",
                "issue": {"number": 7, "title": "Bug", "pull_request": {}},
                "repository": {"full_name": "acme/api"}
            }),
        ));
        assert_eq!(comment["is_pull_request"], true);
        assert_eq!(subject(&comment).as_deref(), Some("acme/api#7"));

        let push = github_context(&delivery(
            "push",
            json!({"repository": {"full_name": "acme/api"}}),
        ));
        assert_eq!(push["event_action"], "push");
        assert_eq!(subject(&push).as_deref(), Some("acme/api"));
        assert_eq!(subject(&github_context(&delivery("ping", json!({})))), None);
    }

    #[test]
    fn subscription_matches_event_or_event_and_action() {
        let config = GitHubTriggerConfig {
            events: vec!["issue_comment".into(), "pull_request.opened".into()],
            repositories: vec![],
            session_mode: SessionBinding::Thread,
            message: "m".into(),
            filter: None,
        };
        assert!(subscribes(&config, "pull_request", Some("opened")));
        assert!(!subscribes(&config, "pull_request", Some("closed")));
        assert!(subscribes(&config, "issue_comment", Some("edited")));
        assert!(!subscribes(&config, "push", None));
    }
}

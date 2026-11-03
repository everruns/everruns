//! Manual fire with an event: run a trigger now on an event you name.
//!
//! Design decisions (knowledge/runtime-resources/agent-triggers.md, "Manual fire"):
//! - **One path.** The trigger's own source normalizes the event (webhook
//!   request, GitHub event, MCP event, schedule payload) and hands it to the
//!   shared pipeline in [`super::events`], so a manual event meets the same
//!   filter, message, per subject session and delivery log a real one does.
//!   The differences are recorded, not hidden: the delivery source and
//!   `invocation.source` are `manual`, and the event id is fresh unless the
//!   caller names one.
//! - **Source authentication is skipped, not faked.** The caller already holds
//!   `agent.manage` on the agent, the right that edits the trigger's message
//!   and filter. GitHub's own-bot suppression does not apply either: the
//!   person asked for this event.
//! - **Explicit over silent.** An event the GitHub trigger does not subscribe
//!   to is refused with a reason, where a real delivery would be dropped
//!   quietly. A filter miss comes back as `filtered` with its reason.
//! - **Two ways to name an event.** Raw, as the source sends it (`payload`,
//!   plus `name` and `headers` where the source has them), or a reference the
//!   trigger type resolves. GitHub resolves `{repository, pull_request}` by
//!   reading the pull request through the agent's own GitHub connection and
//!   building the `pull_request` event GitHub would send, so a manual review
//!   lands in that pull request's own `per_thread` session.
//! - Delivery rows do not keep payloads (events carry third-party content),
//!   so replaying a past delivery means sending its event again.

use super::events::TriggerEventOutcome;
use super::github::{self, GitHubDelivery, GitHubTarget};
use super::mcp_event::{self, McpEventBody, McpEventTarget};
use super::queries as q;
use super::schedule_invocation;
use super::webhook_invocation::{self, WebhookTriggerInvocationRequest};
use crate::domains::agent_triggers::record::AgentTriggerType;
use crate::domains::common::{CommandError, Ctx};
use crate::storage::{AgentRow, AgentTriggerRow};
use everruns_contracts::typed_id::{AgentId, SessionId};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use utoipa::ToSchema;

/// Delivery source and `invocation.source` of a manual fire.
pub const SOURCE: &str = "manual";

/// Largest event a manual fire accepts, serialized.
const MAX_EVENT_BYTES: usize = 1024 * 1024;

/// Pull request actions a reference fire picks from, in order, when the
/// caller names none: the first one the trigger listens to.
const PULL_REQUEST_ACTIONS: &[&str] = &["opened", "synchronize", "reopened", "ready_for_review"];

/// An event to fire a trigger with, instead of a plain run.
///
/// Name it raw, as its source sends it (`payload`, with `name` for GitHub and
/// `headers` for webhooks), or for a GitHub trigger by reference
/// (`repository` and `pull_request`).
#[derive(Debug, Default, Clone, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ManualTriggerEvent {
    /// The event as its source sends it: a webhook request body, a GitHub
    /// event payload, an MCP event's `data`, or extra context a schedule
    /// trigger's message reads as `{{payload.*}}`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Object)]
    pub payload: Option<Value>,
    /// Event name: the GitHub event (`pull_request`, `issue_comment`, the
    /// `X-GitHub-Event` header). For an MCP event trigger it defaults to the
    /// trigger's event and must match it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(example = "pull_request")]
    pub name: Option<String>,
    /// Request headers a webhook trigger's templates see as `webhook.headers`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headers: Option<HashMap<String, String>>,
    /// Idempotency key. Defaults to a fresh id, so every manual fire runs;
    /// repeat an id the trigger already saw to test deduplication.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// GitHub reference: the pull request's repository, `owner/name`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(example = "acme/api")]
    pub repository: Option<String>,
    /// GitHub reference: the pull request number. The server reads it through
    /// the agent's GitHub connection and builds a `pull_request` event.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(example = 12)]
    pub pull_request: Option<u64>,
    /// GitHub reference: the action the built event carries. Defaults to the
    /// first of opened, synchronize, reopened, ready_for_review the trigger
    /// listens to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(example = "synchronize")]
    pub action: Option<String>,
}

/// What a manual fire did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ManualFireStatus {
    /// The event started (or continued) a run.
    Dispatched,
    /// The event missed the trigger's filter; nothing ran.
    Filtered,
    /// The trigger already handled an event with this id; nothing ran.
    Duplicate,
}

/// How a manual fire presents itself to a source's normalization.
#[derive(Debug, Clone)]
pub struct ManualFire {
    /// The id the pipeline dedupes on.
    pub event_id: String,
    /// The caller named the id, rather than taking a fresh one.
    pub explicit_id: bool,
}

impl ManualFire {
    fn new(id: Option<String>) -> Self {
        match id
            .map(|id| id.trim().to_string())
            .filter(|id| !id.is_empty())
        {
            Some(event_id) => Self {
                event_id,
                explicit_id: true,
            },
            None => Self {
                event_id: format!("manual-{}", uuid::Uuid::now_v7()),
                explicit_id: false,
            },
        }
    }
}

/// The delivery source: `manual` for a manual fire, the source's otherwise.
pub fn source_name(manual: Option<&ManualFire>, source: &'static str) -> &'static str {
    if manual.is_some() { SOURCE } else { source }
}

/// The event id: the manual fire's, or the source's.
pub fn event_id(manual: Option<&ManualFire>, source_id: Option<String>) -> Option<String> {
    match manual {
        Some(fire) => Some(fire.event_id.clone()),
        None => source_id,
    }
}

/// The outcome of a manual fire, as the fire command reports it.
pub struct ManualFireResult {
    pub status: ManualFireStatus,
    pub session_id: Option<SessionId>,
    pub created_session: bool,
    pub reason: Option<String>,
}

impl From<TriggerEventOutcome> for ManualFireResult {
    fn from(outcome: TriggerEventOutcome) -> Self {
        match outcome {
            TriggerEventOutcome::Dispatched(result) => Self {
                status: ManualFireStatus::Dispatched,
                session_id: Some(result.session_id),
                created_session: result.created_session,
                reason: None,
            },
            TriggerEventOutcome::Filtered { reason } => Self {
                status: ManualFireStatus::Filtered,
                session_id: None,
                created_session: false,
                reason: Some(reason),
            },
            TriggerEventOutcome::Duplicate => Self {
                status: ManualFireStatus::Duplicate,
                session_id: None,
                created_session: false,
                reason: Some("The trigger already handled an event with this id".to_string()),
            },
        }
    }
}

/// Fire `trigger` once with `event`, through its source's normalization.
pub(super) async fn fire(
    ctx: &Ctx,
    agent: &AgentRow,
    trigger: &AgentTriggerRow,
    event: ManualTriggerEvent,
) -> Result<ManualFireResult, CommandError> {
    if trigger.status != "active" {
        return Err(CommandError::not_found("Agent trigger"));
    }
    if !trigger.enabled {
        return Err(CommandError::forbidden("Agent trigger is disabled"));
    }
    let size = serde_json::to_vec(&event)
        .map(|bytes| bytes.len())
        .unwrap_or(0);
    if size > MAX_EVENT_BYTES {
        return Err(CommandError::bad_request(format!(
            "The event is {size} bytes; a manual fire accepts at most {MAX_EVENT_BYTES}"
        )));
    }
    let session_service = ctx
        .session_service
        .as_ref()
        .ok_or_else(|| CommandError::unavailable("Session service is not available"))?;
    let message_service = ctx
        .message_service
        .as_ref()
        .ok_or_else(|| CommandError::unavailable("Message service is not available"))?;
    let trigger_type = AgentTriggerType::from(trigger.trigger_type.as_str());
    let shape = EventShape::of(&event);
    let fire = ManualFire::new(event.id.clone());

    let outcome = match trigger_type {
        AgentTriggerType::Schedule => {
            shape.only(&[Field::Payload], "a schedule trigger")?;
            schedule_invocation::dispatch_schedule(
                &ctx.db,
                session_service,
                message_service,
                ctx.channels.as_deref(),
                ctx.encryption.as_ref(),
                trigger,
                agent,
                Some((&fire, event.payload.unwrap_or(Value::Null))),
            )
            .await?
        }
        AgentTriggerType::Webhook => {
            shape.only(&[Field::Payload, Field::Headers], "a webhook trigger")?;
            webhook_invocation::dispatch_webhook(
                &ctx.db,
                ctx.encryption.as_ref(),
                session_service,
                message_service,
                trigger,
                agent,
                webhook_request(trigger.ingress_id.as_deref(), event),
                Some(&fire),
                None,
            )
            .await?
        }
        AgentTriggerType::GitHub => {
            let config = q::row_to_trigger(trigger.clone(), agent_id(agent)?, None)
                .github_config()
                .map_err(|_| CommandError::bad_request("Invalid GitHub trigger configuration"))?;
            let delivery = if shape.reference {
                shape.only(&[Field::Reference], "a pull request reference")?;
                pull_request_event(ctx, agent, &config, &event, &fire).await?
            } else {
                shape.only(&[Field::Payload, Field::Name], "a GitHub trigger")?;
                raw_github_event(event, &fire)?
            };
            let action = delivery.payload.get("action").and_then(Value::as_str);
            if !github::subscribes(&config, &delivery.event, action) {
                let event_action = match action {
                    Some(action) => format!("{}.{action}", delivery.event),
                    None => delivery.event.clone(),
                };
                return Err(CommandError::bad_request(format!(
                    "This trigger does not listen to {event_action}; it listens to {}",
                    config.events.join(", ")
                )));
            }
            github::dispatch_to_trigger(
                &ctx.db,
                session_service,
                message_service,
                GitHubTarget {
                    row: trigger,
                    agent,
                    config: &config,
                },
                &delivery,
                Some(&fire),
                None,
            )
            .await?
        }
        AgentTriggerType::McpEvent => {
            shape.only(&[Field::Payload, Field::Name], "an MCP event trigger")?;
            let config = q::row_to_trigger(trigger.clone(), agent_id(agent)?, None)
                .mcp_event_config()
                .map_err(|_| {
                    CommandError::bad_request("Invalid MCP event trigger configuration")
                })?;
            if let Some(name) = event.name.as_deref()
                && name != config.event
            {
                return Err(CommandError::bad_request(format!(
                    "This trigger listens to the MCP event {}, not {name}",
                    config.event
                )));
            }
            let body = json!({
                "name": config.event,
                "eventId": fire.event_id,
                "timestamp": chrono::Utc::now().to_rfc3339(),
                "data": event.payload.unwrap_or(Value::Null),
            });
            mcp_event::dispatch_event(
                &ctx.db,
                session_service,
                message_service,
                McpEventTarget {
                    trigger,
                    agent,
                    config: &config,
                },
                McpEventBody {
                    body: &body,
                    delivery_id: &fire.event_id,
                    subscription_id: None,
                },
                Some(&fire),
                None,
            )
            .await?
        }
    };
    Ok(outcome.into())
}

fn agent_id(agent: &AgentRow) -> Result<AgentId, CommandError> {
    super::commands::parse_agent_id(&agent.public_id)
}

/// A webhook request as the ingress would have built it from `event`.
fn webhook_request(
    ingress_id: Option<&str>,
    event: ManualTriggerEvent,
) -> WebhookTriggerInvocationRequest {
    let (body, json_payload) = match event.payload {
        // A string is the raw body; it is JSON when it parses as JSON.
        Some(Value::String(body)) => {
            let json = serde_json::from_str(&body).ok();
            (body, json)
        }
        Some(payload) => (payload.to_string(), Some(payload)),
        None => (String::new(), None),
    };
    WebhookTriggerInvocationRequest {
        ingress_id: ingress_id.unwrap_or_default().to_string(),
        body,
        json_payload,
        // The ingress hands templates lowercase header names.
        headers: event
            .headers
            .unwrap_or_default()
            .into_iter()
            .map(|(name, value)| (name.to_ascii_lowercase(), value))
            .collect(),
    }
}

fn raw_github_event(
    event: ManualTriggerEvent,
    fire: &ManualFire,
) -> Result<GitHubDelivery, CommandError> {
    let name = event
        .name
        .map(|name| name.trim().to_ascii_lowercase())
        .filter(|name| !name.is_empty())
        .ok_or_else(|| {
            CommandError::bad_request(
                "Name the GitHub event (name, e.g. pull_request), or pass repository and pull_request",
            )
        })?;
    let payload = event
        .payload
        .filter(Value::is_object)
        .ok_or_else(|| CommandError::bad_request("A GitHub event payload must be a JSON object"))?;
    Ok(GitHubDelivery {
        event: name,
        delivery_id: fire.event_id.clone(),
        payload,
    })
}

/// Build the `pull_request` event GitHub would send for an existing pull
/// request, read through the agent's own GitHub connection: the identity the
/// run will act as must be able to see it.
async fn pull_request_event(
    ctx: &Ctx,
    agent: &AgentRow,
    config: &crate::domains::agent_triggers::record::GitHubTriggerConfig,
    event: &ManualTriggerEvent,
    fire: &ManualFire,
) -> Result<GitHubDelivery, CommandError> {
    let (Some(repository), Some(number)) = (event.repository.as_deref(), event.pull_request) else {
        return Err(CommandError::bad_request(
            "A pull request reference needs both repository and pull_request",
        ));
    };
    let repository = github::normalize_repositories(Some(vec![repository.to_string()]))?
        .pop()
        .ok_or_else(|| CommandError::bad_request("Invalid repository: use owner/name"))?;
    let action = match event.action.as_deref().map(str::trim) {
        Some(action) if !action.is_empty() => action.to_ascii_lowercase(),
        _ => PULL_REQUEST_ACTIONS
            .iter()
            .find(|action| github::subscribes(config, "pull_request", Some(action)))
            .map(|action| (*action).to_string())
            .ok_or_else(|| {
                CommandError::bad_request(
                    "This trigger listens to no pull request events; pass action to choose one",
                )
            })?,
    };
    let identity = agent.virtual_user_id.ok_or_else(|| {
        CommandError::bad_request(
            "The agent has no GitHub connection. Connect GitHub on the agent first",
        )
    })?;
    let resolver = ctx
        .connection_resolver
        .as_ref()
        .ok_or_else(|| CommandError::unavailable("GitHub connections are not available here"))?;
    let access = everruns_contracts::runtime::GitHubAccess::new(
        everruns_core::McpServerActsAs::Service,
        &[("pull_requests", "read")],
    )
    .for_repository(repository.clone());
    let token = resolver
        .get_github_token_for_owner(ctx.org_id(), identity.uuid(), &access)
        .await
        .ok()
        .flatten()
        .ok_or_else(|| {
            CommandError::bad_request(format!(
                "The agent's GitHub connection cannot read pull requests in {repository}. Install its GitHub App there"
            ))
        })?;
    let pull_request = ctx
        .github_apps
        .api
        .read_source_json(&format!("/repos/{repository}/pulls/{number}"), &token)
        .await
        .map_err(|_| {
            CommandError::bad_request(format!(
                "GitHub did not return {repository}#{number} to the agent's GitHub connection: it does not exist, or the connection cannot see it"
            ))
        })?;
    Ok(pull_request_delivery(pull_request, &action, fire))
}

/// The `pull_request` webhook payload GitHub sends, from the REST pull request.
fn pull_request_delivery(pull_request: Value, action: &str, fire: &ManualFire) -> GitHubDelivery {
    let payload = json!({
        "action": action,
        "number": pull_request.get("number"),
        "repository": pull_request.pointer("/base/repo"),
        "sender": pull_request.get("user"),
        "pull_request": pull_request,
    });
    GitHubDelivery {
        event: "pull_request".to_string(),
        delivery_id: fire.event_id.clone(),
        payload,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Field {
    Payload,
    Name,
    Headers,
    Reference,
}

/// Which parts of a manual event the caller filled in.
struct EventShape {
    payload: bool,
    name: bool,
    headers: bool,
    reference: bool,
}

impl EventShape {
    fn of(event: &ManualTriggerEvent) -> Self {
        Self {
            payload: event.payload.is_some(),
            name: event.name.is_some(),
            headers: event.headers.is_some(),
            reference: event.repository.is_some()
                || event.pull_request.is_some()
                || event.action.is_some(),
        }
    }

    /// Refuse the fields `allowed` leaves out, naming what they do not apply to.
    fn only(&self, allowed: &[Field], what: &str) -> Result<(), CommandError> {
        let present = [
            (Field::Payload, self.payload, "payload"),
            (Field::Name, self.name, "name"),
            (Field::Headers, self.headers, "headers"),
            (
                Field::Reference,
                self.reference,
                "repository, pull_request and action",
            ),
        ];
        let extra: Vec<&str> = present
            .iter()
            .filter(|(field, set, _)| *set && !allowed.contains(field))
            .map(|(_, _, label)| *label)
            .collect();
        if extra.is_empty() {
            Ok(())
        } else {
            Err(CommandError::bad_request(format!(
                "{} do not apply to {what}",
                extra.join(", ")
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fire() -> ManualFire {
        ManualFire::new(None)
    }

    #[test]
    fn a_fresh_id_is_unique_and_an_explicit_one_is_kept() {
        let first = ManualFire::new(None);
        let second = ManualFire::new(Some("  ".to_string()));
        assert!(first.event_id.starts_with("manual-"));
        assert!(!first.explicit_id && !second.explicit_id);
        assert_ne!(first.event_id, second.event_id);
        let named = ManualFire::new(Some(" d-1 ".to_string()));
        assert_eq!(named.event_id, "d-1");
        assert!(named.explicit_id);
        assert_eq!(source_name(Some(&named), "github"), "manual");
        assert_eq!(source_name(None, "github"), "github");
        assert_eq!(event_id(None, Some("x".into())).as_deref(), Some("x"));
    }

    #[test]
    fn pull_request_reference_builds_the_webhook_shape() {
        let delivery = pull_request_delivery(
            json!({
                "number": 12,
                "title": "Fix",
                "html_url": "https://github.com/acme/api/pull/12",
                "draft": false,
                "user": {"login": "octo"},
                "base": {"repo": {"full_name": "acme/api"}},
            }),
            "opened",
            &fire(),
        );
        assert_eq!(delivery.event, "pull_request");
        assert_eq!(delivery.payload["action"], "opened");
        assert_eq!(delivery.payload["number"], 12);
        assert_eq!(delivery.payload["repository"]["full_name"], "acme/api");
        assert_eq!(delivery.payload["pull_request"]["draft"], false);
        assert_eq!(delivery.payload["sender"]["login"], "octo");
    }

    #[test]
    fn webhook_payloads_become_the_request_body() {
        let object = webhook_request(
            Some("whk_1"),
            ManualTriggerEvent {
                payload: Some(json!({"ticket": 7})),
                headers: Some(HashMap::from([(
                    "X-Source".to_string(),
                    "test".to_string(),
                )])),
                ..Default::default()
            },
        );
        assert_eq!(object.json_payload, Some(json!({"ticket": 7})));
        assert_eq!(object.body, r#"{"ticket":7}"#);
        assert_eq!(object.headers["x-source"], "test");
        let text = webhook_request(
            Some("whk_1"),
            ManualTriggerEvent {
                payload: Some(json!("plain")),
                ..Default::default()
            },
        );
        assert_eq!(text.body, "plain");
        assert_eq!(text.json_payload, None);
    }

    #[test]
    fn raw_github_events_need_a_name_and_an_object() {
        assert!(raw_github_event(ManualTriggerEvent::default(), &fire()).is_err());
        assert!(
            raw_github_event(
                ManualTriggerEvent {
                    name: Some("pull_request".into()),
                    payload: Some(json!([1])),
                    ..Default::default()
                },
                &fire()
            )
            .is_err()
        );
        let delivery = raw_github_event(
            ManualTriggerEvent {
                name: Some(" Issue_Comment ".into()),
                payload: Some(json!({"action": "created"})),
                ..Default::default()
            },
            &fire(),
        )
        .unwrap();
        assert_eq!(delivery.event, "issue_comment");
    }

    #[test]
    fn fields_that_do_not_apply_are_named() {
        let shape = EventShape::of(&ManualTriggerEvent {
            headers: Some(HashMap::new()),
            repository: Some("acme/api".into()),
            ..Default::default()
        });
        let error = shape
            .only(&[Field::Payload], "a schedule trigger")
            .unwrap_err()
            .to_string();
        assert!(error.contains("headers"), "{error}");
        assert!(error.contains("repository"), "{error}");
        assert!(shape.only(&[Field::Headers, Field::Reference], "x").is_ok());
    }
}

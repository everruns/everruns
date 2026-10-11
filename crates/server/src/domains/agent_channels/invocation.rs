// Agent channel invocation runtime.
//
// Turns an inbound schedule call into a session message (webhook and A2A
// run on the channel host, `channels::host`), and owns the schedule and
// template utilities shared by agent triggers. Tag, metadata and audit strings
// keep their App-era values: they are persisted and matched by existing
// sessions.

use crate::auth::audit;
use crate::domains::agent_channels::record::SessionBinding;
use crate::domains::audit_logs::record::{AgentAction, AuditEvent};
use crate::domains::common::CommandError;
use crate::domains::messages::types::{CreateMessageRequest, InputMessage, MessageRole};
use crate::domains::messages::{CreateMessageContext, MessageService};
use crate::domains::sessions::SessionService;
use crate::domains::sessions::types::CreateSessionRequest;
use crate::execution_metadata;
use chrono::{DateTime, Duration, Utc};
use everruns_contracts::typed_id::SessionId;
use everruns_core::InputContentPart;
use regex::Regex;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::{Arc, LazyLock};
use uuid::Uuid;

static TEMPLATE_EXPR_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\{\{\s*([a-zA-Z0-9_.-]+)\s*\}\}").expect("template regex is valid")
});

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelInvocationSource {
    Schedule,
    Webhook,
    A2a,
}

impl ChannelInvocationSource {
    fn as_str(self) -> &'static str {
        match self {
            Self::Schedule => "schedule",
            Self::Webhook => "webhook",
            Self::A2a => "a2a",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ChannelInvocationResult {
    pub session_id: SessionId,
    pub created_session: bool,
}

/// Message metadata key carrying the caller's A2A `messageId`.
pub const A2A_MESSAGE_ID_METADATA: &str = "a2a_message_id";

pub(crate) fn normalize_cron_expression(cron_expression: &str) -> Result<String, CommandError> {
    let fields = cron_expression.split_whitespace().collect::<Vec<_>>();
    let normalized = match fields.len() {
        5 => format!("0 {} *", fields.join(" ")),
        7 => fields.join(" "),
        _ => {
            return Err(CommandError::bad_request(
                "Cron expression must be either 5 fields (min hour day month weekday) or 7 fields (sec min hour day month weekday year)",
            ));
        }
    };
    cron::Schedule::from_str(&normalized).map_err(|e| {
        CommandError::bad_request(format!(
            "Invalid schedule cron expression '{cron_expression}': {e}"
        ))
    })?;
    Ok(normalized)
}

/// Returns the minimum interval in seconds between consecutive triggers.
///
/// Cron expressions without a year constraint are periodic over a full
/// leap-year-sized horizon, so scan that bounded window to catch non-uniform
/// bursts. If the horizon has too few occurrences, keep sampling upcoming
/// triggers without the horizon so future-dated bursts cannot bypass the limit.
pub(crate) fn cron_min_interval_seconds(
    schedule: &cron::Schedule,
    reject_below_seconds: i64,
) -> Option<i64> {
    const FALLBACK_OCCURRENCE_LIMIT: usize = 3;

    let start = Utc::now();
    let end = start + Duration::days(366);
    let mut previous: Option<DateTime<Utc>> = None;
    let mut min_interval: Option<i64> = None;
    let mut occurrences = 0;

    for next in schedule.after(&start).take_while(|next| *next <= end) {
        occurrences += 1;
        if let Some(previous) = previous {
            let interval = (next - previous).num_seconds();
            min_interval =
                Some(min_interval.map_or(interval, |current: i64| current.min(interval)));
            if interval < reject_below_seconds {
                return min_interval;
            }
        }
        previous = Some(next);
    }

    if occurrences >= 2 {
        return min_interval;
    }

    // Horizon had fewer than two occurrences (a future-dated or very sparse
    // schedule). Reset `previous` so the fallback measures intervals purely
    // among the sampled occurrences: otherwise the single horizon occurrence
    // would be compared against itself (the fallback restarts from `start`),
    // yielding a spurious 0-second interval that rejects a valid schedule.
    previous = None;
    for next in schedule.after(&start).take(FALLBACK_OCCURRENCE_LIMIT) {
        if let Some(previous) = previous {
            let interval = (next - previous).num_seconds();
            min_interval =
                Some(min_interval.map_or(interval, |current: i64| current.min(interval)));
            if interval < reject_below_seconds {
                break;
            }
        }
        previous = Some(next);
    }

    min_interval
}

pub(crate) fn calculate_schedule_next_trigger(
    cron_expression: &str,
) -> Result<Option<DateTime<Utc>>, CommandError> {
    let normalized = normalize_cron_expression(cron_expression)?;
    let schedule = cron::Schedule::from_str(&normalized).map_err(|e| {
        CommandError::bad_request(format!("Invalid cron expression '{cron_expression}': {e}"))
    })?;
    Ok(schedule.upcoming(Utc).next())
}

/// Hash a plaintext A2A execution key using SHA-256.
pub fn hash_a2a_api_key(plaintext: &str) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(plaintext.as_bytes()))
}

pub(crate) fn template_lookup<'a>(context: &'a Value, path: &str) -> Option<&'a Value> {
    let mut current = context;
    for segment in path.split('.') {
        current = match current {
            Value::Object(map) => map.get(segment)?,
            Value::Array(items) => items.get(segment.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(current)
}

fn template_value_to_string(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

pub(crate) fn render_message_template(template: &str, context: &Value) -> String {
    TEMPLATE_EXPR_RE
        .replace_all(template, |captures: &regex::Captures<'_>| {
            let path = captures.get(1).map(|m| m.as_str()).unwrap_or_default();
            template_lookup(context, path)
                .map(template_value_to_string)
                .unwrap_or_default()
        })
        .into_owned()
}

fn channel_session_tags(
    ingress: &crate::domains::agent_channels::ingress::IngressContext,
    channel: &crate::domains::agent_channels::ingress::IngressChannel,
) -> Vec<String> {
    vec![
        format!("app:{}", ingress.public_id),
        format!("app_channel:{}", channel.public_id),
        format!("app_channel_type:{}", channel.channel_type),
        "__internal:app_invocation".to_string(),
    ]
}

pub(crate) fn channel_invocation_message_metadata(
    ingress: &crate::domains::agent_channels::ingress::IngressContext,
    channel: &crate::domains::agent_channels::ingress::IngressChannel,
    source: ChannelInvocationSource,
) -> HashMap<String, Value> {
    [
        (
            "source".to_string(),
            Value::String(format!("app_{}", source.as_str())),
        ),
        (
            "_app_id".to_string(),
            Value::String(ingress.public_id.to_string()),
        ),
        (
            "app_channel_id".to_string(),
            Value::String(channel.public_id.to_string()),
        ),
        (
            "app_channel_type".to_string(),
            Value::String(channel.channel_type.to_string()),
        ),
    ]
    .into_iter()
    .collect()
}

pub(crate) fn emit_channel_invocation_audit_event(
    db: Arc<crate::storage::StorageBackend>,
    ingress: &crate::domains::agent_channels::ingress::IngressContext,
    channel: &crate::domains::agent_channels::ingress::IngressChannel,
    session_id: SessionId,
    source: ChannelInvocationSource,
    created_session: bool,
) {
    let mut event = AuditEvent::agent(AgentAction::AppInvocationStarted, ingress.org_id, None)
        .target("app_channel", channel.public_id.to_string())
        .detail("source", format!("app_{}", source.as_str()))
        .detail("app_id", ingress.public_id.to_string())
        .detail("app_channel_id", channel.public_id.to_string())
        .detail("app_channel_type", channel.channel_type.to_string())
        .detail("session_id", session_id.to_string())
        .detail("created_session", created_session)
        .detail(
            "app_owner_principal_id",
            ingress.owner_principal_id.to_string(),
        );
    if let Some(virtual_user_id) = ingress.virtual_user_id {
        event = event.detail("virtual_user_id", virtual_user_id.to_string());
    }
    audit::emit_event(db, event.build());
}
fn shared_session_title(
    ingress: &crate::domains::agent_channels::ingress::IngressContext,
    source: ChannelInvocationSource,
) -> String {
    format!("{} {}", ingress.name, source.as_str())
}

fn invocation_session_title(
    ingress: &crate::domains::agent_channels::ingress::IngressContext,
    source: ChannelInvocationSource,
) -> String {
    format!(
        "{} {} {}",
        ingress.name,
        source.as_str(),
        Utc::now().to_rfc3339()
    )
}

async fn find_or_create_invocation_session(
    db: &Arc<crate::storage::StorageBackend>,
    session_service: &SessionService,
    ingress: &crate::domains::agent_channels::ingress::IngressContext,
    channel: &crate::domains::agent_channels::ingress::IngressChannel,
    session_mode: SessionBinding,
    source: ChannelInvocationSource,
) -> Result<(SessionId, bool), CommandError> {
    let shared_tags = channel_session_tags(ingress, channel);
    if session_mode == SessionBinding::Shared
        && let Some(existing) = match ingress.historical_app_id {
            Some(app_id) => {
                db.find_app_session_by_tags_and_owner(
                    ingress.org_id,
                    app_id,
                    ingress.owner_principal_id,
                    &shared_tags,
                )
                .await
            }
            None => {
                db.find_channel_session_by_tags_and_owner(
                    ingress.org_id,
                    channel.internal_id,
                    ingress.owner_principal_id,
                    &shared_tags,
                )
                .await
            }
        }?
    {
        return Ok((existing.id, false));
    }

    let mut tags = shared_tags;
    if session_mode == SessionBinding::Ephemeral {
        tags.push(format!("app_invocation:{}", Uuid::now_v7()));
    }

    let title = if session_mode == SessionBinding::Shared {
        shared_session_title(ingress, source)
    } else {
        invocation_session_title(ingress, source)
    };

    let session = session_service
        .create_from_app(
            &everruns_core::Caller::internal(ingress.org_id),
            ingress.harness_id.uuid(),
            Some(ingress.agent_internal_id),
            ingress.agent_id,
            ingress.historical_app_id,
            Some(channel.internal_id),
            // Channel ingress, not a trigger.
            None,
            // Pass the App's owner so the resulting session matches the
            // owner-keyed lookup in `find_app_session_by_tags_and_owner` —
            // shared-session reuse depends on this. See `create_from_app` doc.
            ingress.owner_principal_id,
            ingress.resolved_owner_user_id,
            // The invocation channel *is* the session's origin.
            match source {
                ChannelInvocationSource::Schedule => {
                    crate::domains::sessions::record::SessionSource::Schedule
                }
                ChannelInvocationSource::Webhook => {
                    crate::domains::sessions::record::SessionSource::Webhook
                }
                ChannelInvocationSource::A2a => {
                    crate::domains::sessions::record::SessionSource::A2a
                }
            },
            CreateSessionRequest {
                playground_user_id: None,
                source: None,
                workspace_id: None,
                harness_id: Some(ingress.harness_id),
                harness_name: None,
                agent_id: ingress.agent_id,
                agent_name: None,
                virtual_user_id: ingress.virtual_user_id,
                title: Some(title),
                goal: None,
                locale: None,
                tags,
                model_id: None,
                capabilities: vec![],
                sandbox: None,
                tools: vec![],
                mcp_servers: Default::default(),
                system_prompt: None,
                initial_files: vec![],
                hints: None,
                network_access: None,
                max_iterations: None,
                parallel_tool_calls: None,
                parent_session_id: None,
                forked_from_session_id: None,
                budget_root_session_id: None,
                seed: everruns_core::SessionSeedMode::Fresh,
                fork_up_to_sequence: None,
            },
        )
        .await?;

    Ok((session.id, true))
}

#[allow(clippy::too_many_arguments)]
async fn dispatch_invocation_message(
    message_service: &MessageService,
    ingress: &crate::domains::agent_channels::ingress::IngressContext,
    channel: &crate::domains::agent_channels::ingress::IngressChannel,
    session_id: SessionId,
    source: ChannelInvocationSource,
    request_id: Option<String>,
    rendered_message: String,
) -> Result<(), CommandError> {
    let metadata = Some(channel_invocation_message_metadata(
        ingress, channel, source,
    ));

    message_service
        .create(
            CreateMessageContext {
                runtime_subject_principal_id: None,
                org_id: ingress.org_id,
                user_id: None,
                harness_id: ingress.harness_id.uuid(),
                agent_id: Some(ingress.agent_internal_id),
                session_id: session_id.uuid(),
                event_metadata: Some(execution_metadata::channel_message_metadata(
                    ingress.public_id,
                    ingress.owner_principal_id,
                    ingress.virtual_user_id,
                )),
                request_id,
            },
            CreateMessageRequest {
                message: InputMessage {
                    role: MessageRole::User,
                    content: vec![InputContentPart::text(rendered_message)],
                },
                addressed_participant_id: None,
                controls: None,
                metadata,
                tags: None,
                external_actor: None,
                client_message_id: None,
            },
        )
        .await?;

    Ok(())
}

struct InvocationServices<'a> {
    db: &'a Arc<crate::storage::StorageBackend>,
    session_service: &'a SessionService,
    message_service: &'a MessageService,
}

struct InvocationRequest {
    ingress: crate::domains::agent_channels::ingress::IngressContext,
    channel: crate::domains::agent_channels::ingress::IngressChannel,
    session_mode: SessionBinding,
    source: ChannelInvocationSource,
    template_context: Value,
    request_id: Option<String>,
}

async fn invoke_channel_inner(
    services: InvocationServices<'_>,
    request: InvocationRequest,
) -> Result<ChannelInvocationResult, CommandError> {
    let InvocationRequest {
        ingress,
        channel,
        session_mode,
        source,
        template_context,
        request_id,
    } = request;

    if !channel.status.is_live() {
        return Err(CommandError::forbidden("App is not published".to_string()));
    }
    // Webhook and A2A channels run on the channel host. What remains here is a
    // schedule's template.
    let message_template = match source {
        ChannelInvocationSource::Schedule => {
            channel
                .schedule_config()
                .ok_or_else(|| CommandError::bad_request("Invalid schedule channel configuration"))?
                .message
        }
        ChannelInvocationSource::Webhook | ChannelInvocationSource::A2a => {
            return Err(CommandError::bad_request(format!(
                "{} channels do not take scheduled invocations",
                source.as_str()
            )));
        }
    };
    let rendered_message = render_message_template(&message_template, &template_context);
    if rendered_message.trim().is_empty() {
        return Err(CommandError::bad_request(
            "Rendered invocation message is empty",
        ));
    }

    let (session_id, created_session) = find_or_create_invocation_session(
        services.db,
        services.session_service,
        &ingress,
        &channel,
        session_mode,
        source,
    )
    .await?;

    dispatch_invocation_message(
        services.message_service,
        &ingress,
        &channel,
        session_id,
        source,
        request_id,
        rendered_message,
    )
    .await?;

    emit_channel_invocation_audit_event(
        Arc::clone(services.db),
        &ingress,
        &channel,
        session_id,
        source,
        created_session,
    );

    Ok(ChannelInvocationResult {
        session_id,
        created_session,
    })
}

pub async fn invoke_scheduled_legacy_alias_channel(
    db: &Arc<crate::storage::StorageBackend>,
    encryption: Option<&Arc<crate::storage::encryption::EncryptionService>>,
    session_service: &SessionService,
    message_service: &MessageService,
    org_id: i64,
    legacy_app_id: &str,
    channel_id: &str,
) -> Result<ChannelInvocationResult, CommandError> {
    let (ingress, channel) =
        crate::domains::agent_channels::ingress::resolve_channel(db, encryption, channel_id)
            .await?
            .filter(|(context, _)| context.org_id == org_id)
            .ok_or_else(|| CommandError::not_found("Channel"))?;
    if !ingress.matches_legacy_app_id(legacy_app_id) {
        return Err(CommandError::not_found("Channel"));
    }
    invoke_scheduled_channel_inner(db, session_service, message_service, ingress, channel).await
}

pub async fn invoke_scheduled_agent_channel(
    db: &Arc<crate::storage::StorageBackend>,
    encryption: Option<&Arc<crate::storage::encryption::EncryptionService>>,
    session_service: &SessionService,
    message_service: &MessageService,
    org_id: i64,
    channel_id: &str,
) -> Result<ChannelInvocationResult, CommandError> {
    let (ingress, channel) =
        crate::domains::agent_channels::ingress::resolve_channel(db, encryption, channel_id)
            .await?
            .filter(|(context, _)| context.org_id == org_id)
            .ok_or_else(|| CommandError::not_found("Channel"))?;
    invoke_scheduled_channel_inner(db, session_service, message_service, ingress, channel).await
}

async fn invoke_scheduled_channel_inner(
    db: &Arc<crate::storage::StorageBackend>,
    session_service: &SessionService,
    message_service: &MessageService,
    ingress: crate::domains::agent_channels::ingress::IngressContext,
    channel: crate::domains::agent_channels::ingress::IngressChannel,
) -> Result<ChannelInvocationResult, CommandError> {
    let config = channel
        .schedule_config()
        .ok_or_else(|| CommandError::bad_request("Invalid schedule channel configuration"))?;
    let template_context = json!({
        "app": {
            "id": ingress.public_id.to_string(),
            "name": ingress.name.clone(),
        },
        "channel": {
            "id": channel.public_id.to_string(),
            "type": channel.channel_type.to_string(),
        },
        "invocation": {
            "source": "schedule",
            "triggered_at": Utc::now().to_rfc3339(),
        },
    });

    invoke_channel_inner(
        InvocationServices {
            db,
            session_service,
            message_service,
        },
        InvocationRequest {
            ingress,
            channel,
            session_mode: config.session_mode,
            source: ChannelInvocationSource::Schedule,
            template_context,
            request_id: None,
        },
    )
    .await
}

// Agent channel invocation runtime.
//
// Turns an inbound channel call (webhook, A2A, api_endpoint, schedule) into a
// session message, and owns the schedule and template utilities shared by
// agent triggers. Tag, metadata and audit strings keep their App-era values:
// they are persisted and matched by existing sessions.

use crate::api::messages::{CreateMessageRequest, InputContentPart, InputMessage, MessageRole};
use crate::api::sessions::CreateSessionRequest;
use crate::auth::audit;
use crate::domains::common::CommandError;
use crate::domains::messages::{CreateMessageContext, MessageService};
use crate::domains::sessions::SessionService;
use crate::execution_metadata;
use crate::records::agent_channel::SessionBinding;
use crate::records::{AgentAction, AuditEvent, ChannelType};
use chrono::{DateTime, Duration, Utc};
use everruns_contracts::typed_id::PrincipalId;
use everruns_contracts::typed_id::SessionId;
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
    ApiEndpoint,
}

impl ChannelInvocationSource {
    fn as_str(self) -> &'static str {
        match self {
            Self::Schedule => "schedule",
            Self::Webhook => "webhook",
            Self::A2a => "a2a",
            Self::ApiEndpoint => "api_endpoint",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ChannelInvocationResult {
    pub session_id: SessionId,
    pub created_session: bool,
}

#[derive(Debug, Clone)]
pub struct WebhookInvocationRequest {
    pub legacy_app_id: String,
    pub channel_id: String,
    pub body: String,
    pub json_payload: Option<Value>,
    pub headers: HashMap<String, String>,
}

/// Message metadata key carrying the caller's A2A `messageId`.
pub const A2A_MESSAGE_ID_METADATA: &str = "a2a_message_id";

#[derive(Debug, Clone)]
pub struct A2aInvocationRequest {
    pub legacy_app_id: String,
    pub channel_id: String,
    /// Verbatim A2A `params` object from the JSON-RPC envelope.
    pub params: Value,
    /// Concatenated text parts from `params.message.parts` (newline-joined).
    pub text: String,
    pub message_id: Option<String>,
    pub task_id: String,
    pub context_id: Option<String>,
    pub role: Option<String>,
    /// A session the caller already proved is bound to this channel (an A2A
    /// `contextId` / `taskId` from an earlier response). When set, the message
    /// continues that session instead of resolving one by `session_mode`.
    pub continue_session: Option<SessionId>,
    /// Routing tag naming one authenticated caller (a PACT personal agent's
    /// user). When set, a message that continues no session starts its own,
    /// tagged with it, whatever the channel's `session_mode`: one caller's
    /// conversation never lands in another's session.
    pub caller_tag: Option<String>,
    /// Principal of the end user the turn runs as (a PACT delegation's
    /// company user), so tools that act as the user resolve that user's
    /// grants. `None` runs as the channel, as for every other caller.
    pub runtime_subject: Option<PrincipalId>,
}

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

/// Hash a plaintext API channel execution key using SHA-256.
pub fn hash_channel_api_key(plaintext: &str) -> String {
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
    ingress: &crate::api::channel_ingress::IngressContext,
    channel: &crate::api::channel_ingress::IngressChannel,
) -> Vec<String> {
    vec![
        format!("app:{}", ingress.public_id),
        format!("app_channel:{}", channel.public_id),
        format!("app_channel_type:{}", channel.channel_type),
        "__internal:app_invocation".to_string(),
    ]
}

fn channel_invocation_message_metadata(
    ingress: &crate::api::channel_ingress::IngressContext,
    channel: &crate::api::channel_ingress::IngressChannel,
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

fn emit_channel_invocation_audit_event(
    db: Arc<crate::storage::StorageBackend>,
    ingress: &crate::api::channel_ingress::IngressContext,
    channel: &crate::api::channel_ingress::IngressChannel,
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
    ingress: &crate::api::channel_ingress::IngressContext,
    source: ChannelInvocationSource,
) -> String {
    format!("{} {}", ingress.name, source.as_str())
}

fn invocation_session_title(
    ingress: &crate::api::channel_ingress::IngressContext,
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
    ingress: &crate::api::channel_ingress::IngressContext,
    channel: &crate::api::channel_ingress::IngressChannel,
    session_mode: SessionBinding,
    source: ChannelInvocationSource,
    caller_tag: Option<String>,
) -> Result<(SessionId, bool), CommandError> {
    let shared_tags = channel_session_tags(ingress, channel);
    // A caller-bound conversation is never shared (see
    // `A2aInvocationRequest::caller_tag`).
    let session_mode = if caller_tag.is_some() {
        SessionBinding::Ephemeral
    } else {
        session_mode
    };
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
    tags.extend(caller_tag);

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
            // The invocation channel *is* the session's origin. `api_endpoint`
            // collapses into `webhook`: both are an inbound HTTP call into the app.
            match source {
                ChannelInvocationSource::Schedule => crate::records::SessionSource::Schedule,
                ChannelInvocationSource::Webhook | ChannelInvocationSource::ApiEndpoint => {
                    crate::records::SessionSource::Webhook
                }
                ChannelInvocationSource::A2a => crate::records::SessionSource::A2a,
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
            },
        )
        .await?;

    Ok((session.id, true))
}

#[allow(clippy::too_many_arguments)]
async fn dispatch_invocation_message(
    message_service: &MessageService,
    ingress: &crate::api::channel_ingress::IngressContext,
    channel: &crate::api::channel_ingress::IngressChannel,
    session_id: SessionId,
    source: ChannelInvocationSource,
    request_id: Option<String>,
    rendered_message: String,
    extra_metadata: HashMap<String, Value>,
    runtime_subject: Option<PrincipalId>,
) -> Result<(), CommandError> {
    let mut metadata = channel_invocation_message_metadata(ingress, channel, source);
    metadata.extend(extra_metadata);
    let metadata = Some(metadata);

    message_service
        .create(
            CreateMessageContext {
                runtime_subject_principal_id: runtime_subject,
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
    ingress: crate::api::channel_ingress::IngressContext,
    channel: crate::api::channel_ingress::IngressChannel,
    session_mode: SessionBinding,
    source: ChannelInvocationSource,
    template_context: Value,
    request_id: Option<String>,
    continue_session: Option<SessionId>,
    caller_tag: Option<String>,
    message_metadata: HashMap<String, Value>,
    runtime_subject: Option<PrincipalId>,
}

async fn invoke_channel_inner(
    services: InvocationServices<'_>,
    request: InvocationRequest,
) -> Result<ChannelInvocationResult, CommandError> {
    invoke_channel_inner_with_hook(services, request, |_session_id| async { Ok(()) }).await
}

/// Variant of [`invoke_channel_inner`] that runs a caller-supplied async
/// hook between session resolution and message dispatch. The hook gives
/// streaming callers a deterministic point to start an event subscription so
/// they cannot miss workflow events that the dispatched turn emits.
async fn invoke_channel_inner_with_hook<F, Fut>(
    services: InvocationServices<'_>,
    request: InvocationRequest,
    after_session_resolved: F,
) -> Result<ChannelInvocationResult, CommandError>
where
    F: FnOnce(SessionId) -> Fut,
    Fut: std::future::Future<Output = Result<(), CommandError>>,
{
    let InvocationRequest {
        ingress,
        channel,
        session_mode,
        source,
        template_context,
        request_id,
        continue_session,
        caller_tag,
        message_metadata,
        runtime_subject,
    } = request;

    if !channel.status.is_live() {
        return Err(CommandError::forbidden("App is not published".to_string()));
    }
    let message_template = match source {
        ChannelInvocationSource::Schedule => {
            channel
                .schedule_config()
                .ok_or_else(|| CommandError::bad_request("Invalid schedule channel configuration"))?
                .message
        }
        ChannelInvocationSource::Webhook => {
            channel
                .webhook_config()
                .ok_or_else(|| CommandError::bad_request("Invalid webhook channel configuration"))?
                .message
        }
        ChannelInvocationSource::A2a => {
            channel
                .a2a_config()
                .ok_or_else(|| CommandError::bad_request("Invalid A2A channel configuration"))?
                .message
        }
        // api_endpoint channels carry no config-side message template — the
        // caller supplies the message directly, so they use the dedicated
        // `invoke_channel_api` path instead of this template renderer.
        ChannelInvocationSource::ApiEndpoint => {
            return Err(CommandError::bad_request(
                "api_endpoint channels do not use message templates",
            ));
        }
    };
    let rendered_message = render_message_template(&message_template, &template_context);
    if rendered_message.trim().is_empty() {
        return Err(CommandError::bad_request(
            "Rendered invocation message is empty",
        ));
    }

    let (session_id, created_session) = match continue_session {
        Some(session_id) => (session_id, false),
        None => {
            find_or_create_invocation_session(
                services.db,
                services.session_service,
                &ingress,
                &channel,
                session_mode,
                source,
                caller_tag,
            )
            .await?
        }
    };

    // Subscribe-before-dispatch hook: streaming callers register here so the
    // workflow events emitted by `dispatch_invocation_message` cannot race
    // ahead of the SSE subscription.
    after_session_resolved(session_id).await?;

    dispatch_invocation_message(
        services.message_service,
        &ingress,
        &channel,
        session_id,
        source,
        request_id,
        rendered_message,
        message_metadata,
        runtime_subject,
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
        crate::api::channel_ingress::resolve_channel(db, encryption, channel_id)
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
        crate::api::channel_ingress::resolve_channel(db, encryption, channel_id)
            .await?
            .filter(|(context, _)| context.org_id == org_id)
            .ok_or_else(|| CommandError::not_found("Channel"))?;
    invoke_scheduled_channel_inner(db, session_service, message_service, ingress, channel).await
}

async fn invoke_scheduled_channel_inner(
    db: &Arc<crate::storage::StorageBackend>,
    session_service: &SessionService,
    message_service: &MessageService,
    ingress: crate::api::channel_ingress::IngressContext,
    channel: crate::api::channel_ingress::IngressChannel,
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
            continue_session: None,
            caller_tag: None,
            message_metadata: HashMap::new(),
            runtime_subject: None,
        },
    )
    .await
}

/// Invoke an A2A channel, running a caller-supplied async hook between
/// session resolution and message dispatch. Streaming callers
/// use the hook to subscribe to session events at the safe point — before
/// the durable workflow that the dispatched message triggers can emit any
/// translatable events.
pub async fn invoke_channel_a2a_with_hook<F, Fut>(
    db: &Arc<crate::storage::StorageBackend>,
    encryption: Option<&Arc<crate::storage::encryption::EncryptionService>>,
    session_service: &SessionService,
    message_service: &MessageService,
    req: A2aInvocationRequest,
    request_id: Option<String>,
    after_session_resolved: F,
) -> Result<ChannelInvocationResult, CommandError>
where
    F: FnOnce(SessionId) -> Fut,
    Fut: std::future::Future<Output = Result<(), CommandError>>,
{
    let (ingress, channel) =
        crate::api::channel_ingress::resolve_channel(db, encryption, &req.channel_id)
            .await?
            .ok_or_else(|| CommandError::not_found("Channel"))?;
    if !ingress.matches_legacy_app_id(&req.legacy_app_id) {
        return Err(CommandError::not_found("Channel"));
    }
    let config = channel
        .a2a_config()
        .ok_or_else(|| CommandError::bad_request("Invalid A2A channel configuration"))?;
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
            "source": "a2a",
            "triggered_at": Utc::now().to_rfc3339(),
        },
        "payload": req.params.clone(),
        "a2a": {
            "text": req.text,
            "message_id": req.message_id,
            "task_id": req.task_id,
            "context_id": req.context_id,
            "role": req.role,
        },
    });
    // The caller's A2A `messageId` rides on the stored message, so a retried
    // message can be matched to the reply it already got (PACT §4.3).
    let message_metadata = req
        .message_id
        .map(|id| HashMap::from([(A2A_MESSAGE_ID_METADATA.to_string(), Value::String(id))]))
        .unwrap_or_default();

    invoke_channel_inner_with_hook(
        InvocationServices {
            db,
            session_service,
            message_service,
        },
        InvocationRequest {
            ingress,
            channel,
            session_mode: config.session_mode,
            source: ChannelInvocationSource::A2a,
            template_context,
            request_id,
            continue_session: req.continue_session,
            caller_tag: req.caller_tag,
            message_metadata,
            runtime_subject: req.runtime_subject,
        },
        after_session_resolved,
    )
    .await
}

/// Request to start an api_endpoint execution-key invocation.
#[derive(Debug, Clone)]
pub struct ApiInvocationRequest {
    pub legacy_app_id: String,
    pub channel_id: String,
    /// Caller-supplied message dispatched into the channel session.
    pub message: String,
}

/// Resolve the live ingress context + enabled api_endpoint channel for an
/// execution-key request. Shared by the create-session and post-message paths
/// **and** by the HTTP auth layer (`api::channel_api::authenticate_request`) so the
/// published / enabled / channel-type gate lives in exactly one place and
/// cannot drift between the two.
pub async fn resolve_channel_api(
    db: &Arc<crate::storage::StorageBackend>,
    encryption: Option<&Arc<crate::storage::encryption::EncryptionService>>,
    legacy_app_id: &str,
    channel_id: &str,
) -> Result<
    (
        crate::api::channel_ingress::IngressContext,
        crate::api::channel_ingress::IngressChannel,
    ),
    CommandError,
> {
    let (ingress, channel) =
        crate::api::channel_ingress::resolve_channel(db, encryption, channel_id)
            .await?
            .ok_or_else(|| CommandError::not_found("Channel"))?;
    if !ingress.matches_legacy_app_id(legacy_app_id) {
        return Err(CommandError::not_found("Channel"));
    }
    if channel.channel_type != ChannelType::ApiEndpoint {
        return Err(CommandError::not_found("Channel"));
    }
    // EVE-1007: the channel's own status, folded with the agent-level terms, is
    // the authority. The rejection stays the same forbidden shape a draft App
    // produced before, so a key holder cannot tell the reasons apart.
    if let Err(reason) = crate::api::channel_ingress::channel_liveness(&ingress, &channel) {
        tracing::debug!(
            app_id = %ingress.public_id,
            channel_id = %channel.public_id,
            reason = reason.as_str(),
            "api_endpoint request rejected: channel not live"
        );
        return Err(CommandError::forbidden("App is not published".to_string()));
    }
    Ok((ingress, channel))
}

/// Whether a session's routing tags bind it to the given ingress context + channel.
/// Confinement check for api_endpoint execution keys (mirrors the A2A
/// `session_belongs_to_a2a_channel` guard). THREAT[TM-APIKEY-002].
pub fn session_has_channel_tags(
    tags: &[String],
    app_public_id: &str,
    channel_public_id: &str,
) -> bool {
    let app_tag = format!("app:{app_public_id}");
    let channel_tag = format!("app_channel:{channel_public_id}");
    tags.iter().any(|t| t == &app_tag) && tags.iter().any(|t| t == &channel_tag)
}

/// Create (or resolve, for shared-session mode) the channel session for an
/// api_endpoint channel and dispatch the caller-supplied message, triggering a
/// turn. Mirrors `invoke_channel_a2a_with_hook`, but the message is supplied by the
/// caller rather than rendered from a config-side template.
pub async fn invoke_channel_api(
    db: &Arc<crate::storage::StorageBackend>,
    encryption: Option<&Arc<crate::storage::encryption::EncryptionService>>,
    session_service: &SessionService,
    message_service: &MessageService,
    req: ApiInvocationRequest,
    request_id: Option<String>,
) -> Result<ChannelInvocationResult, CommandError> {
    if req.message.trim().is_empty() {
        return Err(CommandError::bad_request("message must not be empty"));
    }
    let (ingress, channel) =
        resolve_channel_api(db, encryption, &req.legacy_app_id, &req.channel_id).await?;
    let config = channel
        .api_channel_config()
        .ok_or_else(|| CommandError::bad_request("Invalid api_endpoint channel configuration"))?;

    let (session_id, created_session) = find_or_create_invocation_session(
        db,
        session_service,
        &ingress,
        &channel,
        config.session_mode,
        ChannelInvocationSource::ApiEndpoint,
        None,
    )
    .await?;

    dispatch_invocation_message(
        message_service,
        &ingress,
        &channel,
        session_id,
        ChannelInvocationSource::ApiEndpoint,
        request_id,
        req.message,
        HashMap::new(),
        None,
    )
    .await?;

    emit_channel_invocation_audit_event(
        Arc::clone(db),
        &ingress,
        &channel,
        session_id,
        ChannelInvocationSource::ApiEndpoint,
        created_session,
    );

    Ok(ChannelInvocationResult {
        session_id,
        created_session,
    })
}

/// Dispatch a follow-up message into an existing session that belongs to the
/// api_endpoint channel. The session must carry the channel's routing tags
/// (confinement) or the call fails with not-found, so one channel's key cannot
/// drive another channel's sessions. THREAT[TM-APIKEY-002].
#[allow(clippy::too_many_arguments)]
pub async fn post_channel_api_message(
    db: &Arc<crate::storage::StorageBackend>,
    encryption: Option<&Arc<crate::storage::encryption::EncryptionService>>,
    message_service: &MessageService,
    legacy_app_id: &str,
    channel_id: &str,
    session_id: SessionId,
    message: String,
    request_id: Option<String>,
) -> Result<ChannelInvocationResult, CommandError> {
    if message.trim().is_empty() {
        return Err(CommandError::bad_request("message must not be empty"));
    }
    let (ingress, channel) = resolve_channel_api(db, encryption, legacy_app_id, channel_id).await?;

    let session = db
        .get_session(ingress.org_id, session_id)
        .await?
        .ok_or_else(|| CommandError::not_found("Session"))?;
    if !session_has_channel_tags(
        &session.tags,
        &ingress.public_id.to_string(),
        &channel.public_id.to_string(),
    ) {
        return Err(CommandError::not_found("Session"));
    }

    dispatch_invocation_message(
        message_service,
        &ingress,
        &channel,
        session_id,
        ChannelInvocationSource::ApiEndpoint,
        request_id,
        message,
        HashMap::new(),
        None,
    )
    .await?;

    emit_channel_invocation_audit_event(
        Arc::clone(db),
        &ingress,
        &channel,
        session_id,
        ChannelInvocationSource::ApiEndpoint,
        false,
    );

    Ok(ChannelInvocationResult {
        session_id,
        created_session: false,
    })
}

pub async fn invoke_channel_webhook(
    db: &Arc<crate::storage::StorageBackend>,
    encryption: Option<&Arc<crate::storage::encryption::EncryptionService>>,
    session_service: &SessionService,
    message_service: &MessageService,
    req: WebhookInvocationRequest,
    request_id: Option<String>,
) -> Result<ChannelInvocationResult, CommandError> {
    let (ingress, channel) =
        crate::api::channel_ingress::resolve_channel(db, encryption, &req.channel_id)
            .await?
            .ok_or_else(|| CommandError::not_found("Channel"))?;
    if !ingress.matches_legacy_app_id(&req.legacy_app_id) {
        return Err(CommandError::not_found("Channel"));
    }
    let config = channel
        .webhook_config()
        .ok_or_else(|| CommandError::bad_request("Invalid webhook channel configuration"))?;
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
            "source": "webhook",
            "triggered_at": Utc::now().to_rfc3339(),
        },
        "payload": req
            .json_payload
            .clone()
            .unwrap_or_else(|| Value::String(req.body.clone())),
        "webhook": {
            "body": req.body,
            "json": req.json_payload,
            "headers": req.headers,
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
            source: ChannelInvocationSource::Webhook,
            template_context,
            request_id,
            continue_session: None,
            caller_tag: None,
            message_metadata: HashMap::new(),
            runtime_subject: None,
        },
    )
    .await
}

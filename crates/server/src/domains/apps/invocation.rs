// Frozen App compatibility runtime.
//
// App management is retired. This module only preserves endpoint invocation,
// legacy session attribution, and schedule utilities shared by agent triggers.

use crate::api::messages::{CreateMessageRequest, InputContentPart, InputMessage, MessageRole};
use crate::api::sessions::CreateSessionRequest;
use crate::auth::audit;
use crate::domains::common::{CommandError, classify_anyhow};
use crate::domains::messages::{CreateMessageContext, MessageService};
use crate::domains::sessions::SessionService;
use crate::execution_metadata;
use chrono::{DateTime, Duration, Utc};
use everruns_platform::app::SessionBinding;
use everruns_platform::{AgentAction, AuditEvent, ChannelType};
use everruns_provider::typed_id::SessionId;
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
pub enum AppInvocationSource {
    Schedule,
    Webhook,
    A2a,
    ApiEndpoint,
}

impl AppInvocationSource {
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
pub struct AppInvocationResult {
    pub session_id: SessionId,
    pub created_session: bool,
}

#[derive(Debug, Clone)]
pub struct WebhookInvocationRequest {
    pub app_id: String,
    pub channel_id: String,
    pub body: String,
    pub json_payload: Option<Value>,
    pub headers: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct A2aInvocationRequest {
    pub app_id: String,
    pub channel_id: String,
    /// Verbatim A2A `params` object from the JSON-RPC envelope.
    pub params: Value,
    /// Concatenated text parts from `params.message.parts` (newline-joined).
    pub text: String,
    pub message_id: Option<String>,
    pub task_id: String,
    pub context_id: Option<String>,
    pub role: Option<String>,
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

/// Hash a plaintext API endpoint execution key using SHA-256.
pub fn hash_app_api_key(plaintext: &str) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(plaintext.as_bytes()))
}

fn template_lookup<'a>(context: &'a Value, path: &str) -> Option<&'a Value> {
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

fn app_session_tags(
    app: &crate::api::app_ingress::IngressContext,
    channel: &crate::api::app_ingress::IngressEndpoint,
) -> Vec<String> {
    vec![
        format!("app:{}", app.public_id),
        format!("app_channel:{}", channel.public_id),
        format!("app_channel_type:{}", channel.channel_type),
        "__internal:app_invocation".to_string(),
    ]
}

fn app_invocation_message_metadata(
    app: &crate::api::app_ingress::IngressContext,
    channel: &crate::api::app_ingress::IngressEndpoint,
    source: AppInvocationSource,
) -> HashMap<String, Value> {
    [
        (
            "source".to_string(),
            Value::String(format!("app_{}", source.as_str())),
        ),
        (
            "_app_id".to_string(),
            Value::String(app.public_id.to_string()),
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

fn emit_app_invocation_audit_event(
    db: Arc<crate::storage::StorageBackend>,
    app: &crate::api::app_ingress::IngressContext,
    channel: &crate::api::app_ingress::IngressEndpoint,
    session_id: SessionId,
    source: AppInvocationSource,
    created_session: bool,
) {
    let mut event = AuditEvent::agent(AgentAction::AppInvocationStarted, app.org_id, None)
        .target("app_channel", channel.public_id.to_string())
        .detail("source", format!("app_{}", source.as_str()))
        .detail("app_id", app.public_id.to_string())
        .detail("app_channel_id", channel.public_id.to_string())
        .detail("app_channel_type", channel.channel_type.to_string())
        .detail("session_id", session_id.to_string())
        .detail("created_session", created_session)
        .detail("app_owner_principal_id", app.owner_principal_id.to_string());
    if let Some(agent_identity_id) = app.agent_identity_id {
        event = event.detail("agent_identity_id", agent_identity_id.to_string());
    }
    audit::emit_event(db, event.build());
}
fn shared_session_title(
    app: &crate::api::app_ingress::IngressContext,
    source: AppInvocationSource,
) -> String {
    format!("{} {}", app.name, source.as_str())
}

fn invocation_session_title(
    app: &crate::api::app_ingress::IngressContext,
    source: AppInvocationSource,
) -> String {
    format!(
        "{} {} {}",
        app.name,
        source.as_str(),
        Utc::now().to_rfc3339()
    )
}

async fn find_or_create_invocation_session(
    db: &Arc<crate::storage::StorageBackend>,
    session_service: &SessionService,
    app: &crate::api::app_ingress::IngressContext,
    channel: &crate::api::app_ingress::IngressEndpoint,
    session_mode: SessionBinding,
    source: AppInvocationSource,
) -> Result<(SessionId, bool), CommandError> {
    let shared_tags = app_session_tags(app, channel);
    if session_mode == SessionBinding::Endpoint
        && let Some(existing) = match app.historical_app_id {
            Some(app_id) => {
                db.find_app_session_by_tags_and_owner(
                    app.org_id,
                    app_id,
                    app.owner_principal_id,
                    &shared_tags,
                )
                .await
            }
            None => {
                db.find_endpoint_session_by_tags_and_owner(
                    app.org_id,
                    channel.internal_id,
                    app.owner_principal_id,
                    &shared_tags,
                )
                .await
            }
        }
        .map_err(classify_anyhow)?
    {
        return Ok((existing.id, false));
    }

    let mut tags = shared_tags;
    if session_mode == SessionBinding::Ephemeral {
        tags.push(format!("app_invocation:{}", Uuid::now_v7()));
    }

    let title = if session_mode == SessionBinding::Endpoint {
        shared_session_title(app, source)
    } else {
        invocation_session_title(app, source)
    };

    let session = session_service
        .create_from_app(
            &everruns_core::Caller::internal(app.org_id),
            app.harness_id.uuid(),
            Some(app.agent_internal_id),
            app.agent_id,
            app.historical_app_id,
            app.agent_version_policy.clone(),
            app.agent_version_id,
            Some(channel.internal_id),
            // Pass the App's owner so the resulting session matches the
            // owner-keyed lookup in `find_app_session_by_tags_and_owner` —
            // shared-session reuse depends on this. See `create_from_app` doc.
            app.owner_principal_id,
            app.resolved_owner_user_id,
            // The invocation channel *is* the session's origin. `api_endpoint`
            // collapses into `webhook`: both are an inbound HTTP call into the app.
            match source {
                AppInvocationSource::Schedule => everruns_platform::SessionSource::Schedule,
                AppInvocationSource::Webhook | AppInvocationSource::ApiEndpoint => {
                    everruns_platform::SessionSource::Webhook
                }
                AppInvocationSource::A2a => everruns_platform::SessionSource::A2a,
            },
            CreateSessionRequest {
                source: None,
                workspace_id: None,
                harness_id: Some(app.harness_id),
                harness_name: None,
                agent_id: app.agent_id,
                agent_name: None,
                agent_identity_id: app.agent_identity_id,
                title: Some(title),
                goal: None,
                locale: None,
                tags,
                model_id: None,
                capabilities: vec![],
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
        .await
        .map_err(classify_anyhow)?;

    Ok((session.id, true))
}

async fn dispatch_invocation_message(
    message_service: &MessageService,
    app: &crate::api::app_ingress::IngressContext,
    channel: &crate::api::app_ingress::IngressEndpoint,
    session_id: SessionId,
    source: AppInvocationSource,
    request_id: Option<String>,
    rendered_message: String,
) -> Result<(), CommandError> {
    let metadata = Some(app_invocation_message_metadata(app, channel, source));

    message_service
        .create(
            CreateMessageContext {
                org_id: app.org_id,
                user_id: None,
                harness_id: app.harness_id.uuid(),
                agent_id: Some(app.agent_internal_id),
                session_id: session_id.uuid(),
                event_metadata: Some(execution_metadata::app_message_metadata(
                    app.public_id,
                    app.owner_principal_id,
                    app.agent_identity_id,
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
        .await
        .map_err(classify_anyhow)?;

    Ok(())
}

struct InvocationServices<'a> {
    db: &'a Arc<crate::storage::StorageBackend>,
    session_service: &'a SessionService,
    message_service: &'a MessageService,
}

struct InvocationRequest {
    app: crate::api::app_ingress::IngressContext,
    channel: crate::api::app_ingress::IngressEndpoint,
    session_mode: SessionBinding,
    source: AppInvocationSource,
    template_context: Value,
    request_id: Option<String>,
}

async fn invoke_app_channel_inner(
    services: InvocationServices<'_>,
    request: InvocationRequest,
) -> Result<AppInvocationResult, CommandError> {
    invoke_app_channel_inner_with_hook(services, request, |_session_id| async { Ok(()) }).await
}

/// Variant of [`invoke_app_channel_inner`] that runs a caller-supplied async
/// hook between session resolution and message dispatch. The hook gives
/// streaming callers a deterministic point to start an event subscription so
/// they cannot miss workflow events that the dispatched turn emits.
async fn invoke_app_channel_inner_with_hook<F, Fut>(
    services: InvocationServices<'_>,
    request: InvocationRequest,
    after_session_resolved: F,
) -> Result<AppInvocationResult, CommandError>
where
    F: FnOnce(SessionId) -> Fut,
    Fut: std::future::Future<Output = Result<(), CommandError>>,
{
    let InvocationRequest {
        app,
        channel,
        session_mode,
        source,
        template_context,
        request_id,
    } = request;

    if !channel.status.is_live() {
        return Err(CommandError::forbidden("App is not published".to_string()));
    }
    let message_template = match source {
        AppInvocationSource::Schedule => {
            channel
                .schedule_config()
                .ok_or_else(|| CommandError::bad_request("Invalid schedule channel configuration"))?
                .message
        }
        AppInvocationSource::Webhook => {
            channel
                .webhook_config()
                .ok_or_else(|| CommandError::bad_request("Invalid webhook channel configuration"))?
                .message
        }
        AppInvocationSource::A2a => {
            channel
                .a2a_config()
                .ok_or_else(|| CommandError::bad_request("Invalid A2A channel configuration"))?
                .message
        }
        // api_endpoint channels carry no config-side message template — the
        // caller supplies the message directly, so they use the dedicated
        // `invoke_api_app_channel` path instead of this template renderer.
        AppInvocationSource::ApiEndpoint => {
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

    let (session_id, created_session) = find_or_create_invocation_session(
        services.db,
        services.session_service,
        &app,
        &channel,
        session_mode,
        source,
    )
    .await?;

    // Subscribe-before-dispatch hook: streaming callers register here so the
    // workflow events emitted by `dispatch_invocation_message` cannot race
    // ahead of the SSE subscription.
    after_session_resolved(session_id).await?;

    dispatch_invocation_message(
        services.message_service,
        &app,
        &channel,
        session_id,
        source,
        request_id,
        rendered_message,
    )
    .await?;

    emit_app_invocation_audit_event(
        Arc::clone(services.db),
        &app,
        &channel,
        session_id,
        source,
        created_session,
    );

    Ok(AppInvocationResult {
        session_id,
        created_session,
    })
}

pub async fn invoke_scheduled_app_channel(
    db: &Arc<crate::storage::StorageBackend>,
    encryption: Option<&Arc<crate::storage::encryption::EncryptionService>>,
    session_service: &SessionService,
    message_service: &MessageService,
    org_id: i64,
    app_id: &str,
    channel_id: &str,
) -> Result<AppInvocationResult, CommandError> {
    let (app, channel) = crate::api::app_ingress::resolve_endpoint(db, encryption, channel_id)
        .await
        .map_err(classify_anyhow)?
        .filter(|(context, _)| context.org_id == org_id)
        .ok_or_else(|| CommandError::not_found("Endpoint"))?;
    if !app.matches_legacy_app_id(app_id) {
        return Err(CommandError::not_found("Endpoint"));
    }
    invoke_scheduled_endpoint_inner(db, session_service, message_service, app, channel).await
}

pub async fn invoke_scheduled_agent_endpoint(
    db: &Arc<crate::storage::StorageBackend>,
    encryption: Option<&Arc<crate::storage::encryption::EncryptionService>>,
    session_service: &SessionService,
    message_service: &MessageService,
    org_id: i64,
    channel_id: &str,
) -> Result<AppInvocationResult, CommandError> {
    let (app, channel) = crate::api::app_ingress::resolve_endpoint(db, encryption, channel_id)
        .await
        .map_err(classify_anyhow)?
        .filter(|(context, _)| context.org_id == org_id)
        .ok_or_else(|| CommandError::not_found("Endpoint"))?;
    invoke_scheduled_endpoint_inner(db, session_service, message_service, app, channel).await
}

async fn invoke_scheduled_endpoint_inner(
    db: &Arc<crate::storage::StorageBackend>,
    session_service: &SessionService,
    message_service: &MessageService,
    app: crate::api::app_ingress::IngressContext,
    channel: crate::api::app_ingress::IngressEndpoint,
) -> Result<AppInvocationResult, CommandError> {
    let config = channel
        .schedule_config()
        .ok_or_else(|| CommandError::bad_request("Invalid schedule channel configuration"))?;
    let template_context = json!({
        "app": {
            "id": app.public_id.to_string(),
            "name": app.name.clone(),
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

    invoke_app_channel_inner(
        InvocationServices {
            db,
            session_service,
            message_service,
        },
        InvocationRequest {
            app,
            channel,
            session_mode: config.session_mode,
            source: AppInvocationSource::Schedule,
            template_context,
            request_id: None,
        },
    )
    .await
}

pub async fn invoke_a2a_app_channel(
    db: &Arc<crate::storage::StorageBackend>,
    encryption: Option<&Arc<crate::storage::encryption::EncryptionService>>,
    session_service: &SessionService,
    message_service: &MessageService,
    req: A2aInvocationRequest,
    request_id: Option<String>,
) -> Result<AppInvocationResult, CommandError> {
    invoke_a2a_app_channel_with_hook(
        db,
        encryption,
        session_service,
        message_service,
        req,
        request_id,
        |_session_id| async { Ok(()) },
    )
    .await
}

/// Variant of [`invoke_a2a_app_channel`] that runs a caller-supplied async
/// hook between session resolution and message dispatch. Streaming callers
/// use the hook to subscribe to session events at the safe point — before
/// the durable workflow that the dispatched message triggers can emit any
/// translatable events.
pub async fn invoke_a2a_app_channel_with_hook<F, Fut>(
    db: &Arc<crate::storage::StorageBackend>,
    encryption: Option<&Arc<crate::storage::encryption::EncryptionService>>,
    session_service: &SessionService,
    message_service: &MessageService,
    req: A2aInvocationRequest,
    request_id: Option<String>,
    after_session_resolved: F,
) -> Result<AppInvocationResult, CommandError>
where
    F: FnOnce(SessionId) -> Fut,
    Fut: std::future::Future<Output = Result<(), CommandError>>,
{
    let (app, channel) = crate::api::app_ingress::resolve_endpoint(db, encryption, &req.channel_id)
        .await
        .map_err(classify_anyhow)?
        .ok_or_else(|| CommandError::not_found("Endpoint"))?;
    if !app.matches_legacy_app_id(&req.app_id) {
        return Err(CommandError::not_found("Endpoint"));
    }
    let config = channel
        .a2a_config()
        .ok_or_else(|| CommandError::bad_request("Invalid A2A channel configuration"))?;
    let template_context = json!({
        "app": {
            "id": app.public_id.to_string(),
            "name": app.name.clone(),
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

    invoke_app_channel_inner_with_hook(
        InvocationServices {
            db,
            session_service,
            message_service,
        },
        InvocationRequest {
            app,
            channel,
            session_mode: config.session_mode,
            source: AppInvocationSource::A2a,
            template_context,
            request_id,
        },
        after_session_resolved,
    )
    .await
}

/// Request to start an api_endpoint execution-key invocation.
#[derive(Debug, Clone)]
pub struct ApiInvocationRequest {
    pub app_id: String,
    pub channel_id: String,
    /// Caller-supplied message dispatched into the app session.
    pub message: String,
}

/// Resolve the published app + enabled api_endpoint channel for an
/// execution-key request. Shared by the create-session and post-message paths
/// **and** by the HTTP auth layer (`api::app_api::authenticate_request`) so the
/// published / enabled / channel-type gate lives in exactly one place and
/// cannot drift between the two.
pub async fn resolve_api_app_channel(
    db: &Arc<crate::storage::StorageBackend>,
    encryption: Option<&Arc<crate::storage::encryption::EncryptionService>>,
    app_id: &str,
    channel_id: &str,
) -> Result<
    (
        crate::api::app_ingress::IngressContext,
        crate::api::app_ingress::IngressEndpoint,
    ),
    CommandError,
> {
    let (app, channel) = crate::api::app_ingress::resolve_endpoint(db, encryption, channel_id)
        .await
        .map_err(classify_anyhow)?
        .ok_or_else(|| CommandError::not_found("Endpoint"))?;
    if !app.matches_legacy_app_id(app_id) {
        return Err(CommandError::not_found("Endpoint"));
    }
    if channel.channel_type != ChannelType::ApiEndpoint {
        return Err(CommandError::not_found("Channel"));
    }
    // EVE-1007: the endpoint's own status, folded with the agent-level terms, is
    // the authority. The rejection stays the same forbidden shape a draft App
    // produced before, so a key holder cannot tell the reasons apart.
    if let Err(reason) = crate::api::app_ingress::endpoint_liveness(&app, &channel) {
        tracing::debug!(
            app_id = %app.public_id,
            endpoint_id = %channel.public_id,
            reason = reason.as_str(),
            "api_endpoint request rejected: endpoint not live"
        );
        return Err(CommandError::forbidden("App is not published".to_string()));
    }
    Ok((app, channel))
}

/// Whether a session's routing tags bind it to the given app + channel.
/// Confinement check for api_endpoint execution keys (mirrors the A2A
/// `session_belongs_to_a2a_channel` guard). THREAT[TM-APIKEY-002].
pub fn session_has_app_channel_tags(
    tags: &[String],
    app_public_id: &str,
    channel_public_id: &str,
) -> bool {
    let app_tag = format!("app:{app_public_id}");
    let channel_tag = format!("app_channel:{channel_public_id}");
    tags.iter().any(|t| t == &app_tag) && tags.iter().any(|t| t == &channel_tag)
}

/// Create (or resolve, for shared-session mode) the app-owned session for an
/// api_endpoint channel and dispatch the caller-supplied message, triggering a
/// turn. Mirrors `invoke_a2a_app_channel`, but the message is supplied by the
/// caller rather than rendered from a config-side template.
pub async fn invoke_api_app_channel(
    db: &Arc<crate::storage::StorageBackend>,
    encryption: Option<&Arc<crate::storage::encryption::EncryptionService>>,
    session_service: &SessionService,
    message_service: &MessageService,
    req: ApiInvocationRequest,
    request_id: Option<String>,
) -> Result<AppInvocationResult, CommandError> {
    if req.message.trim().is_empty() {
        return Err(CommandError::bad_request("message must not be empty"));
    }
    let (app, channel) =
        resolve_api_app_channel(db, encryption, &req.app_id, &req.channel_id).await?;
    let config = channel
        .api_endpoint_config()
        .ok_or_else(|| CommandError::bad_request("Invalid api_endpoint channel configuration"))?;

    let (session_id, created_session) = find_or_create_invocation_session(
        db,
        session_service,
        &app,
        &channel,
        config.session_mode,
        AppInvocationSource::ApiEndpoint,
    )
    .await?;

    dispatch_invocation_message(
        message_service,
        &app,
        &channel,
        session_id,
        AppInvocationSource::ApiEndpoint,
        request_id,
        req.message,
    )
    .await?;

    emit_app_invocation_audit_event(
        Arc::clone(db),
        &app,
        &channel,
        session_id,
        AppInvocationSource::ApiEndpoint,
        created_session,
    );

    Ok(AppInvocationResult {
        session_id,
        created_session,
    })
}

/// Dispatch a follow-up message into an existing session that belongs to the
/// api_endpoint channel. The session must carry the channel's routing tags
/// (confinement) or the call fails with not-found, so one app's key cannot
/// drive another app's sessions. THREAT[TM-APIKEY-002].
#[allow(clippy::too_many_arguments)]
pub async fn post_api_app_channel_message(
    db: &Arc<crate::storage::StorageBackend>,
    encryption: Option<&Arc<crate::storage::encryption::EncryptionService>>,
    message_service: &MessageService,
    app_id: &str,
    channel_id: &str,
    session_id: SessionId,
    message: String,
    request_id: Option<String>,
) -> Result<AppInvocationResult, CommandError> {
    if message.trim().is_empty() {
        return Err(CommandError::bad_request("message must not be empty"));
    }
    let (app, channel) = resolve_api_app_channel(db, encryption, app_id, channel_id).await?;

    let session = db
        .get_session(app.org_id, session_id)
        .await
        .map_err(classify_anyhow)?
        .ok_or_else(|| CommandError::not_found("Session"))?;
    if !session_has_app_channel_tags(
        &session.tags,
        &app.public_id.to_string(),
        &channel.public_id.to_string(),
    ) {
        return Err(CommandError::not_found("Session"));
    }

    dispatch_invocation_message(
        message_service,
        &app,
        &channel,
        session_id,
        AppInvocationSource::ApiEndpoint,
        request_id,
        message,
    )
    .await?;

    emit_app_invocation_audit_event(
        Arc::clone(db),
        &app,
        &channel,
        session_id,
        AppInvocationSource::ApiEndpoint,
        false,
    );

    Ok(AppInvocationResult {
        session_id,
        created_session: false,
    })
}

pub async fn invoke_webhook_app_channel(
    db: &Arc<crate::storage::StorageBackend>,
    encryption: Option<&Arc<crate::storage::encryption::EncryptionService>>,
    session_service: &SessionService,
    message_service: &MessageService,
    req: WebhookInvocationRequest,
    request_id: Option<String>,
) -> Result<AppInvocationResult, CommandError> {
    let (app, channel) = crate::api::app_ingress::resolve_endpoint(db, encryption, &req.channel_id)
        .await
        .map_err(classify_anyhow)?
        .ok_or_else(|| CommandError::not_found("Endpoint"))?;
    if !app.matches_legacy_app_id(&req.app_id) {
        return Err(CommandError::not_found("Endpoint"));
    }
    let config = channel
        .webhook_config()
        .ok_or_else(|| CommandError::bad_request("Invalid webhook channel configuration"))?;
    let template_context = json!({
        "app": {
            "id": app.public_id.to_string(),
            "name": app.name.clone(),
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

    invoke_app_channel_inner(
        InvocationServices {
            db,
            session_service,
            message_service,
        },
        InvocationRequest {
            app,
            channel,
            session_mode: config.session_mode,
            source: AppInvocationSource::Webhook,
            template_context,
            request_id,
        },
    )
    .await
}

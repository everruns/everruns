// App commands — user-facing operations.
//
// Each struct is the request type, catalog entry, and execution logic.
// inventory::submit! auto-registers for MCP catalog.

use super::queries as q;
use super::types::{
    AddChannelRequest, CreateAppChannelRow, CreateAppRequest, CreateAppRow, UpdateApp,
    UpdateAppChannel, UpdateAppRequest, UpdateChannelRequest,
};
use super::{APP_DANGEROUS, APP_MANAGE, APP_VIEW};
use crate::api::messages::{CreateMessageRequest, InputContentPart, InputMessage, MessageRole};
use crate::api::sessions::CreateSessionRequest;
use crate::auth::audit;
use crate::domains::common::*;
use crate::domains::messages::{CreateMessageContext, MessageService};
use crate::domains::sessions::SessionService;
use crate::errors::ResourceNotFoundError;
use crate::execution_metadata;
use crate::services::PrincipalService;
use chrono::{DateTime, Utc};
use everruns_core::app::{InvocationSessionMode, ScheduleChannelConfig, WebhookChannelConfig};
use everruns_core::typed_id::{AgentId, AppChannelId, AppId, HarnessId, SessionId};
use everruns_core::{
    A2aChannelConfig, AgUiChannelConfig, AgUiToolVisibility, AgentAction, App, AppChannel,
    AppStatus, AuditEvent, ChannelType, Policy, SlackChannelConfig,
};
use everruns_durable::{
    CreateScheduleRow, ScheduleTargetType, StoreError, UpdateField, UpdateSchedule,
    WorkflowEventStore,
};
use regex::Regex;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::{Arc, LazyLock};
use utoipa::ToSchema;
use uuid::Uuid;

// ============================================================================
// Validation helpers
// ============================================================================

async fn validate_harness(ctx: &Ctx, harness_id: HarnessId) -> Result<Uuid, CommandError> {
    let row = ctx
        .db
        .get_harness(ctx.org_id(), harness_id)
        .await
        .map_err(classify_anyhow)?
        .ok_or_else(|| classify_anyhow(ResourceNotFoundError::new("Harness").into()))?;
    if row.status != "active" {
        return Err(classify_anyhow(anyhow::anyhow!(
            "Archived or deleted harnesses cannot be assigned"
        )));
    }
    Ok(row.id.uuid())
}

async fn validate_agent(ctx: &Ctx, agent_id: &AgentId) -> Result<Uuid, CommandError> {
    let row = ctx
        .db
        .get_agent_by_public_id(ctx.org_id(), &agent_id.to_string())
        .await
        .map_err(classify_anyhow)?
        .ok_or_else(|| classify_anyhow(ResourceNotFoundError::new("Agent").into()))?;
    if row.status != "active" {
        return Err(classify_anyhow(anyhow::anyhow!(
            "Archived or deleted agents cannot be assigned"
        )));
    }
    Ok(row.id.uuid())
}

async fn validate_agent_identity(
    ctx: &Ctx,
    identity_id: everruns_core::typed_id::AgentIdentityId,
) -> Result<Uuid, CommandError> {
    let identity = ctx
        .db
        .get_agent_identity(ctx.org_id(), identity_id)
        .await
        .map_err(classify_anyhow)?
        .ok_or_else(|| CommandError::not_found("Agent identity"))?;
    if identity.status != "active" {
        return Err(classify_anyhow(anyhow::anyhow!(
            "Archived or deleted agent identities cannot be assigned"
        )));
    }
    Ok(identity.id.uuid())
}

const SCHEDULE_CHANNEL_ACTIVITY: &str = "invoke_scheduled_app_channel";

static TEMPLATE_EXPR_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\{\{\s*([a-zA-Z0-9_.-]+)\s*\}\}").expect("template regex is valid")
});

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppInvocationSource {
    Schedule,
    Webhook,
    A2a,
}

impl AppInvocationSource {
    fn as_str(self) -> &'static str {
        match self {
            Self::Schedule => "schedule",
            Self::Webhook => "webhook",
            Self::A2a => "a2a",
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

fn durable_store(ctx: &Ctx) -> Result<&Arc<dyn WorkflowEventStore + Send + Sync>, CommandError> {
    ctx.workflow_store.as_ref().ok_or_else(|| {
        CommandError::bad_request("App schedule channels require durable execution to be enabled")
    })
}

fn normalize_cron_expression(cron_expression: &str) -> String {
    let fields: Vec<&str> = cron_expression.split_whitespace().collect();
    if fields.len() == 5 {
        format!("0 {} *", fields.join(" "))
    } else {
        cron_expression.trim().to_string()
    }
}

fn normalize_and_validate_channel_config(
    channel_type: ChannelType,
    channel_config: Value,
) -> Result<Value, CommandError> {
    match channel_type {
        ChannelType::Slack => {
            let config: SlackChannelConfig = serde_json::from_value(channel_config.clone())
                .map_err(|e| {
                    CommandError::bad_request(format!("Invalid Slack channel config: {e}"))
                })?;
            if config.signing_secret.trim().is_empty() || config.bot_token.trim().is_empty() {
                return Err(CommandError::bad_request(
                    "Slack channel config requires non-empty signing_secret and bot_token",
                ));
            }
        }
        ChannelType::AgUi => {
            let config: AgUiChannelConfig = serde_json::from_value(channel_config.clone())
                .map_err(|e| {
                    CommandError::bad_request(format!("Invalid AG-UI channel config: {e}"))
                })?;
            // Reject obviously broken caps (e.g. > 1M req/min) so a typo can't
            // silently disable the per-app limit by overflowing reasonable
            // expectations. `0` is allowed and means "no per-app cap".
            if let Some(limit) = config.rate_limit_per_minute
                && limit > 1_000_000
            {
                return Err(CommandError::bad_request(
                    "AG-UI rate_limit_per_minute must be at most 1,000,000",
                ));
            }
            if let Some(token) = config.token
                && token.trim().is_empty()
            {
                return Err(CommandError::bad_request(
                    "AG-UI token must be non-empty when configured",
                ));
            }
            if config.generic_tool_text.chars().count() > 120 {
                return Err(CommandError::bad_request(
                    "AG-UI generic_tool_text must be at most 120 characters",
                ));
            }
            // Narrated also emits the configured generic_tool_text on public streams (the
            // raw narration is intentionally not forwarded), so non-empty text is required
            // for both Generic and Narrated.
            if matches!(
                config.tool_visibility,
                AgUiToolVisibility::Generic | AgUiToolVisibility::Narrated
            ) && config.generic_tool_text.trim().is_empty()
            {
                return Err(CommandError::bad_request(
                    "AG-UI generic_tool_text cannot be empty when tool_visibility is generic or narrated",
                ));
            }
        }
        ChannelType::Schedule => {
            let config: ScheduleChannelConfig = serde_json::from_value(channel_config.clone())
                .map_err(|e| {
                    CommandError::bad_request(format!("Invalid schedule channel config: {e}"))
                })?;
            if config.message.trim().is_empty() {
                return Err(CommandError::bad_request(
                    "Schedule channel config requires a non-empty message",
                ));
            }
            let cron_expression = normalize_cron_expression(&config.cron_expression);
            cron::Schedule::from_str(&cron_expression).map_err(|e| {
                CommandError::bad_request(format!(
                    "Invalid schedule cron expression '{}': {e}",
                    cron_expression
                ))
            })?;
            let mut channel_config = channel_config;
            if let Some(object) = channel_config.as_object_mut() {
                object.insert(
                    "cron_expression".to_string(),
                    Value::String(cron_expression),
                );
            }
            return Ok(channel_config);
        }
        ChannelType::Webhook => {
            let config: WebhookChannelConfig = serde_json::from_value(channel_config.clone())
                .map_err(|e| {
                    CommandError::bad_request(format!("Invalid webhook channel config: {e}"))
                })?;
            if config.token.trim().is_empty() {
                return Err(CommandError::bad_request(
                    "Webhook channel config requires a non-empty token",
                ));
            }
            if config.message.trim().is_empty() {
                return Err(CommandError::bad_request(
                    "Webhook channel config requires a non-empty message",
                ));
            }
        }
        ChannelType::A2a => {
            let config: A2aChannelConfig =
                serde_json::from_value(channel_config.clone()).map_err(|e| {
                    CommandError::bad_request(format!("Invalid A2A channel config: {e}"))
                })?;
            if config.api_key_hash.trim().is_empty() {
                return Err(CommandError::bad_request(
                    "A2A channel config requires a non-empty api_key_hash",
                ));
            }
            if config.api_key_prefix.trim().is_empty() {
                return Err(CommandError::bad_request(
                    "A2A channel config requires a non-empty api_key_prefix",
                ));
            }
            if config.message.trim().is_empty() {
                return Err(CommandError::bad_request(
                    "A2A channel config requires a non-empty message",
                ));
            }
        }
    }

    Ok(channel_config)
}

fn calculate_schedule_next_trigger(
    cron_expression: &str,
) -> Result<Option<DateTime<Utc>>, CommandError> {
    let schedule = cron::Schedule::from_str(cron_expression).map_err(|e| {
        CommandError::bad_request(format!("Invalid cron expression '{cron_expression}': {e}"))
    })?;
    Ok(schedule.upcoming(Utc).next())
}

fn schedule_binding_name(channel_id: AppChannelId) -> String {
    format!("app-channel-{channel_id}")
}

fn app_session_tags(app: &App, channel: &AppChannel) -> Vec<String> {
    vec![
        format!("app:{}", app.public_id),
        format!("app_channel:{}", channel.public_id),
        format!("app_channel_type:{}", channel.channel_type),
        "__internal:app_invocation".to_string(),
    ]
}

fn build_schedule_target_input(app: &App, channel: &AppChannel) -> Value {
    json!({
        "org_id": app.org_id,
        "app_id": app.public_id.to_string(),
        "channel_id": channel.public_id.to_string(),
    })
}

fn app_invocation_message_metadata(
    app: &App,
    channel: &AppChannel,
    source: AppInvocationSource,
) -> HashMap<String, Value> {
    [
        (
            "source".to_string(),
            Value::String(format!("app_{}", source.as_str())),
        ),
        (
            "app_id".to_string(),
            Value::String(app.public_id.to_string()),
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
    app: &App,
    channel: &AppChannel,
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
        .detail("created_session", created_session);
    event = event.detail("app_owner_principal_id", app.owner_principal_id.to_string());
    if let Some(agent_identity_id) = app.agent_identity_id {
        event = event.detail("agent_identity_id", agent_identity_id.to_string());
    }
    audit::emit_event(db, event.build());
}

async fn set_channel_durable_schedule_id(
    ctx: &Ctx,
    channel_internal_id: Uuid,
    durable_schedule_id: UpdateField<Uuid>,
) -> Result<(), CommandError> {
    ctx.db
        .update_app_channel(
            channel_internal_id,
            UpdateAppChannel {
                durable_schedule_id,
                ..Default::default()
            },
        )
        .await
        .map_err(classify_anyhow)?;
    Ok(())
}

async fn delete_durable_schedule_if_present(
    store: &Arc<dyn WorkflowEventStore + Send + Sync>,
    durable_schedule_id: Option<Uuid>,
) -> Result<(), CommandError> {
    let Some(schedule_id) = durable_schedule_id else {
        return Ok(());
    };

    match store.delete_schedule(schedule_id).await {
        Ok(()) | Err(StoreError::ScheduleNotFound(_)) => Ok(()),
        Err(err) => Err(classify_anyhow(err.into())),
    }
}

async fn sync_schedule_binding_for_channel(
    ctx: &Ctx,
    app: &App,
    channel_row: &crate::storage::models::AppChannelRow,
) -> Result<(), CommandError> {
    let encryption = ctx.encryption.as_ref();
    let channel = q::channel_row_to_channel(encryption, channel_row.clone());

    if channel.channel_type != ChannelType::Schedule {
        if ctx.workflow_store.is_some() {
            delete_durable_schedule_if_present(
                durable_store(ctx)?,
                channel_row.durable_schedule_id,
            )
            .await?;
            if channel_row.durable_schedule_id.is_some() {
                set_channel_durable_schedule_id(ctx, channel_row.id, UpdateField::Clear).await?;
            }
        }
        return Ok(());
    }

    let store = durable_store(ctx)?;
    let config = channel
        .schedule_config()
        .ok_or_else(|| CommandError::bad_request("Invalid schedule channel configuration"))?;
    let enabled = app.status == AppStatus::Published && channel.enabled;
    let next_trigger_at = if enabled {
        UpdateField::from_option(calculate_schedule_next_trigger(&config.cron_expression)?)
    } else {
        UpdateField::Clear
    };

    let mut created_schedule_id = None;
    let schedule_id = match channel_row.durable_schedule_id {
        Some(schedule_id) => {
            let update = UpdateSchedule {
                name: Some(schedule_binding_name(channel.public_id)),
                description: UpdateField::Set(format!(
                    "App schedule channel {} for {}",
                    channel.public_id, app.name
                )),
                cron_expression: Some(config.cron_expression.clone()),
                timezone: Some(config.timezone.clone()),
                target_type: Some(ScheduleTargetType::Activity),
                target_name: Some(SCHEDULE_CHANNEL_ACTIVITY.to_string()),
                target_input: Some(build_schedule_target_input(app, &channel)),
                enabled: Some(enabled),
                max_concurrent: UpdateField::Set(1),
                catch_up_missed: Some(false),
                max_catch_up: UpdateField::Set(1),
                retry_policy: UpdateField::Clear,
                next_trigger_at,
            };

            match store.update_schedule(schedule_id, update).await {
                Ok(()) => schedule_id,
                Err(StoreError::ScheduleNotFound(_)) => {
                    let created = store
                        .create_schedule(CreateScheduleRow {
                            name: schedule_binding_name(channel.public_id),
                            description: Some(format!(
                                "App schedule channel {} for {}",
                                channel.public_id, app.name
                            )),
                            cron_expression: config.cron_expression.clone(),
                            timezone: config.timezone.clone(),
                            target_type: ScheduleTargetType::Activity,
                            target_name: SCHEDULE_CHANNEL_ACTIVITY.to_string(),
                            target_input: build_schedule_target_input(app, &channel),
                            enabled,
                            max_concurrent: Some(1),
                            catch_up_missed: false,
                            max_catch_up: Some(1),
                            retry_policy: None,
                            next_trigger_at: if enabled {
                                calculate_schedule_next_trigger(&config.cron_expression)?
                            } else {
                                None
                            },
                        })
                        .await
                        .map_err(|err| classify_anyhow(err.into()))?;
                    created_schedule_id = Some(created);
                    created
                }
                Err(err) => return Err(classify_anyhow(err.into())),
            }
        }
        None => {
            let created = store
                .create_schedule(CreateScheduleRow {
                    name: schedule_binding_name(channel.public_id),
                    description: Some(format!(
                        "App schedule channel {} for {}",
                        channel.public_id, app.name
                    )),
                    cron_expression: config.cron_expression.clone(),
                    timezone: config.timezone.clone(),
                    target_type: ScheduleTargetType::Activity,
                    target_name: SCHEDULE_CHANNEL_ACTIVITY.to_string(),
                    target_input: build_schedule_target_input(app, &channel),
                    enabled,
                    max_concurrent: Some(1),
                    catch_up_missed: false,
                    max_catch_up: Some(1),
                    retry_policy: None,
                    next_trigger_at: if enabled {
                        calculate_schedule_next_trigger(&config.cron_expression)?
                    } else {
                        None
                    },
                })
                .await
                .map_err(|err| classify_anyhow(err.into()))?;
            created_schedule_id = Some(created);
            created
        }
    };

    if channel_row.durable_schedule_id != Some(schedule_id)
        && let Err(err) =
            set_channel_durable_schedule_id(ctx, channel_row.id, UpdateField::Set(schedule_id))
                .await
    {
        if let Some(created_id) = created_schedule_id {
            let _ = store.delete_schedule(created_id).await;
        }
        return Err(err);
    }

    Ok(())
}

async fn sync_all_schedule_bindings(ctx: &Ctx, app: &App) -> Result<(), CommandError> {
    for channel_row in ctx
        .db
        .list_app_channels(app.internal_id)
        .await
        .map_err(classify_anyhow)?
    {
        sync_schedule_binding_for_channel(ctx, app, &channel_row).await?;
    }
    Ok(())
}

async fn require_app_channel_before_publish(
    ctx: &Ctx,
    app_internal_id: Uuid,
) -> Result<(), CommandError> {
    let has_channels = ctx
        .db
        .app_has_channels(app_internal_id)
        .await
        .map_err(classify_anyhow)?;
    if !has_channels {
        return Err(CommandError::bad_request(
            "App must have at least one channel before publishing",
        ));
    }
    Ok(())
}

async fn remove_schedule_binding_for_channel(
    ctx: &Ctx,
    channel_row: &crate::storage::models::AppChannelRow,
) -> Result<(), CommandError> {
    let Some(store) = ctx.workflow_store.as_ref() else {
        return Ok(());
    };
    delete_durable_schedule_if_present(store, channel_row.durable_schedule_id).await?;
    if channel_row.durable_schedule_id.is_some() {
        set_channel_durable_schedule_id(ctx, channel_row.id, UpdateField::Clear).await?;
    }
    Ok(())
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

fn render_message_template(template: &str, context: &Value) -> String {
    TEMPLATE_EXPR_RE
        .replace_all(template, |captures: &regex::Captures<'_>| {
            let path = captures.get(1).map(|m| m.as_str()).unwrap_or_default();
            template_lookup(context, path)
                .map(template_value_to_string)
                .unwrap_or_default()
        })
        .into_owned()
}

fn shared_session_title(app: &App, source: AppInvocationSource) -> String {
    format!("{} {}", app.name, source.as_str())
}

fn invocation_session_title(app: &App, source: AppInvocationSource) -> String {
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
    app: &App,
    channel: &AppChannel,
    session_mode: InvocationSessionMode,
    source: AppInvocationSource,
) -> Result<(SessionId, bool), CommandError> {
    let shared_tags = app_session_tags(app, channel);
    if session_mode == InvocationSessionMode::SharedSession
        && let Some(existing) = db
            .find_app_session_by_tags_and_owner(
                app.org_id,
                app.internal_id,
                app.owner_principal_id,
                &shared_tags,
            )
            .await
            .map_err(classify_anyhow)?
    {
        return Ok((existing.id, false));
    }

    let mut tags = shared_tags;
    if session_mode == InvocationSessionMode::SessionPerInvocation {
        tags.push(format!("app_invocation:{}", Uuid::now_v7()));
    }

    let title = if session_mode == InvocationSessionMode::SharedSession {
        shared_session_title(app, source)
    } else {
        invocation_session_title(app, source)
    };

    let session = session_service
        .create_from_app(
            &everruns_core::Caller::internal(app.org_id),
            app.harness_id.uuid(),
            app.agent_id.map(|agent_id| agent_id.uuid()),
            app.agent_id,
            app.internal_id,
            // Pass the App's owner so the resulting session matches the
            // owner-keyed lookup in `find_app_session_by_tags_and_owner` —
            // shared-session reuse depends on this. See `create_from_app` doc.
            app.owner_principal_id,
            app.resolved_owner_user_id,
            CreateSessionRequest {
                harness_id: Some(app.harness_id),
                harness_name: None,
                agent_id: app.agent_id,
                agent_identity_id: app.agent_identity_id,
                title: Some(title),
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
            },
        )
        .await
        .map_err(classify_anyhow)?;

    Ok((session.id, true))
}

async fn dispatch_invocation_message(
    message_service: &MessageService,
    app: &App,
    channel: &AppChannel,
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
                agent_id: app.agent_id.map(|agent_id| agent_id.uuid()),
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
    app: App,
    channel: AppChannel,
    session_mode: InvocationSessionMode,
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

    if app.status != AppStatus::Published {
        return Err(CommandError::Forbidden("App is not published".to_string()));
    }
    if !channel.enabled {
        return Err(CommandError::Forbidden(
            "App channel is disabled".to_string(),
        ));
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
    let app = q::get_by_public_id(db, encryption, org_id, app_id)
        .await
        .map_err(classify_anyhow)?
        .ok_or_else(|| CommandError::not_found("App"))?;
    let channel_public_id: AppChannelId = channel_id
        .parse()
        .map_err(|e| CommandError::bad_request(format!("Invalid channel ID: {e}")))?;
    let channel = app
        .channel_by_id(&channel_public_id)
        .cloned()
        .ok_or_else(|| CommandError::not_found("Channel"))?;
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
    let app = q::get_by_public_id_unscoped(db, encryption, &req.app_id)
        .await
        .map_err(classify_anyhow)?
        .ok_or_else(|| CommandError::not_found("App"))?;
    let channel_public_id: AppChannelId = req
        .channel_id
        .parse()
        .map_err(|e| CommandError::bad_request(format!("Invalid channel ID: {e}")))?;
    let channel = app
        .channel_by_id(&channel_public_id)
        .cloned()
        .ok_or_else(|| CommandError::not_found("Channel"))?;
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

pub async fn invoke_webhook_app_channel(
    db: &Arc<crate::storage::StorageBackend>,
    encryption: Option<&Arc<crate::storage::encryption::EncryptionService>>,
    session_service: &SessionService,
    message_service: &MessageService,
    req: WebhookInvocationRequest,
    request_id: Option<String>,
) -> Result<AppInvocationResult, CommandError> {
    let app = q::get_by_public_id_unscoped(db, encryption, &req.app_id)
        .await
        .map_err(classify_anyhow)?
        .ok_or_else(|| CommandError::not_found("App"))?;
    let channel_public_id: AppChannelId = req
        .channel_id
        .parse()
        .map_err(|e| CommandError::bad_request(format!("Invalid channel ID: {e}")))?;
    let channel = app
        .channel_by_id(&channel_public_id)
        .cloned()
        .ok_or_else(|| CommandError::not_found("Channel"))?;
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

// ============================================================================
// CreateApp
// ============================================================================

/// Create a new app with a harness and optional agent/channel.
#[derive(Debug, Deserialize)]
pub struct CreateApp(pub CreateAppRequest);

impl CommandSchema for CreateApp {
    fn param_schema() -> serde_json::Value {
        delegated_param_schema::<CreateAppRequest>()
    }
}

impl Command for CreateApp {
    type Output = App;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "create_app",
            category: "apps",
            description: "Create a new app with a harness and optional agent/channel.",
            method: "POST",
            path: "/v1/apps",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&APP_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<App, CommandError> {
        let CreateAppRequest {
            name,
            description,
            harness_id,
            agent_id,
            agent_version_policy,
            agent_version_id,
            agent_identity_id,
            channel_type,
            channel_config,
        } = self.0;
        let encryption = ctx.encryption.as_ref();

        if channel_type.is_none() && channel_config.is_some() {
            return Err(CommandError::bad_request(
                "channel_config requires channel_type",
            ));
        }

        let mut channel_config = channel_config.unwrap_or_default();

        if let Some(channel_type) = channel_type.clone() {
            if channel_type == ChannelType::Schedule {
                let _ = durable_store(ctx)?;
            }
            channel_config = normalize_and_validate_channel_config(channel_type, channel_config)?;
        }

        // Validate references
        let harness_uuid = validate_harness(ctx, harness_id).await?;
        let agent_uuid = match agent_id {
            Some(agent_id) => Some(validate_agent(ctx, &agent_id).await?),
            None => None,
        };
        let agent_version_uuid = if let Some(version_id) = agent_version_id {
            let version = ctx
                .db
                .get_agent_version(ctx.org_id(), version_id)
                .await
                .map_err(classify_anyhow)?
                .ok_or_else(|| CommandError::not_found("Agent version"))?;
            if Some(version.agent_id.uuid()) != agent_uuid {
                return Err(CommandError::bad_request(
                    "Agent version must belong to the selected agent",
                ));
            }
            Some(version.id.uuid())
        } else {
            None
        };
        let agent_identity_uuid = if let Some(identity_id) = agent_identity_id {
            Some(validate_agent_identity(ctx, identity_id).await?)
        } else {
            None
        };
        let owner_principal = PrincipalService::new(ctx.db.clone())
            .default_owner_principal(&ctx.caller, agent_identity_id)
            .await
            .map_err(classify_anyhow)?;

        // Prepare channel config
        let (stored_plaintext, channel_config_encrypted) =
            q::prepare_channel_config(encryption, &channel_config).map_err(classify_anyhow)?;

        // Persist app
        let internal_uuid = Uuid::now_v7();
        let public_id = AppId::from_uuid(internal_uuid);
        let input = CreateAppRow {
            public_id: public_id.to_string(),
            name,
            description,
            harness_id: harness_uuid,
            agent_id: agent_uuid,
            agent_version_policy: agent_version_policy.to_string(),
            agent_version_id: agent_version_uuid,
            agent_identity_id: agent_identity_uuid,
            owner_principal_id: owner_principal.id,
            resolved_owner_user_id: owner_principal.resolved_user_id,
            channel_type: channel_type.as_ref().map(ToString::to_string),
            channel_config: stored_plaintext.clone(),
            channel_config_encrypted: channel_config_encrypted.clone(),
        };
        let row = ctx
            .db
            .create_app(ctx.org_id(), input)
            .await
            .map_err(classify_anyhow)?;

        let mut created_channel = None;
        if let Some(channel_type) = channel_type {
            let channel_uuid = Uuid::now_v7();
            let channel_public_id = AppChannelId::from_uuid(channel_uuid);
            let channel_input = CreateAppChannelRow {
                public_id: channel_public_id.to_string(),
                channel_type: channel_type.to_string(),
                channel_config: stored_plaintext,
                channel_config_encrypted,
                durable_schedule_id: None,
                enabled: true,
            };
            let channel_row = ctx
                .db
                .create_app_channel(row.id, channel_input)
                .await
                .map_err(classify_anyhow)?;
            created_channel = Some(channel_row);
        }

        let app = q::row_to_app(&ctx.db, encryption, row, ctx.org_id()).await;
        if let Some(channel_row) = created_channel.as_ref() {
            sync_schedule_binding_for_channel(ctx, &app, channel_row).await?;
        }

        Ok(app)
    }
}

inventory::submit! { CommandDescriptor::of::<CreateApp>() }

// ============================================================================
// ListApps
// ============================================================================

/// List apps. Supports search and include_archived.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ListApps {
    pub search: Option<String>,
    #[serde(default, deserialize_with = "deserialize_bool_lenient")]
    pub include_archived: bool,
}

impl Command for ListApps {
    type Output = Vec<App>;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "list_apps",
            category: "apps",
            description: "List all apps. Use search for name search, include_archived=true to include archived.",
            method: "GET",
            path: "/v1/apps",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&APP_VIEW)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Vec<App>, CommandError> {
        let encryption = ctx.encryption.as_ref();
        let rows = ctx
            .db
            .list_apps(ctx.org_id(), self.search.as_deref(), self.include_archived)
            .await
            .map_err(classify_anyhow)?;
        q::load_apps_list(&ctx.db, encryption, rows, ctx.org_id())
            .await
            .map_err(classify_anyhow)
    }
}

inventory::submit! { CommandDescriptor::of::<ListApps>() }

// ============================================================================
// GetApp
// ============================================================================

/// Get a single app by ID.
#[derive(Debug, Deserialize, ToSchema)]
pub struct GetApp {
    pub id: String,
}

impl Command for GetApp {
    type Output = App;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "get_app",
            category: "apps",
            description: "Get a single app by ID.",
            method: "GET",
            path: "/v1/apps/{id}",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&APP_VIEW)
    }

    fn positional_arg() -> Option<&'static str> {
        Some("id")
    }

    async fn execute(self, ctx: &Ctx) -> Result<App, CommandError> {
        let app_id: AppId = self
            .id
            .parse()
            .map_err(|e| CommandError::bad_request(format!("Invalid app ID: {e}")))?;

        let encryption = ctx.encryption.as_ref();
        q::get_by_public_id(&ctx.db, encryption, ctx.org_id(), &app_id.to_string())
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("App"))
    }
}

inventory::submit! { CommandDescriptor::of::<GetApp>() }

// ============================================================================
// UpdateApp
// ============================================================================

/// Update an app. Only provided fields are changed.
#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateAppCmd {
    pub id: String,
    #[serde(flatten)]
    pub req: UpdateAppRequest,
}

impl Command for UpdateAppCmd {
    type Output = App;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "update_app",
            category: "apps",
            description: "Update an app. Only provided fields are changed.",
            method: "PATCH",
            path: "/v1/apps/{id}",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&APP_MANAGE)
    }

    fn positional_arg() -> Option<&'static str> {
        Some("id")
    }

    async fn execute(self, ctx: &Ctx) -> Result<App, CommandError> {
        let app_id: AppId = self
            .id
            .parse()
            .map_err(|e| CommandError::bad_request(format!("Invalid app ID: {e}")))?;

        let req = self.req;
        if matches!(req.status, Some(AppStatus::Deleted)) {
            return Err(CommandError::Forbidden(
                "Setting status=deleted requires dangerous delete permission".to_string(),
            ));
        }

        let existing = ctx
            .db
            .get_app_by_public_id(ctx.org_id(), &app_id.to_string())
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("App"))?;

        if !matches!(existing.status.as_str(), "draft" | "published") {
            return Err(CommandError::bad_request(
                "Archived or deleted apps cannot be edited",
            ));
        }
        if matches!(req.status, Some(AppStatus::Published | AppStatus::Draft)) {
            return Err(CommandError::bad_request(
                "Use publish/unpublish endpoints to change app publish status",
            ));
        }

        // Validate optional references
        let harness_id = if let Some(hid) = req.harness_id {
            Some(validate_harness(ctx, hid).await?)
        } else {
            None
        };

        let agent_id = if let Some(aid) = req.agent_id {
            Some(validate_agent(ctx, &aid).await?)
        } else {
            None
        };
        let effective_agent_id = agent_id.or(existing.agent_id);
        let agent_version_id = match req.agent_version_id {
            UpdateField::Set(version_id) => {
                let version = ctx
                    .db
                    .get_agent_version(ctx.org_id(), version_id)
                    .await
                    .map_err(classify_anyhow)?
                    .ok_or_else(|| CommandError::not_found("Agent version"))?;
                if Some(version.agent_id.uuid()) != effective_agent_id {
                    return Err(CommandError::bad_request(
                        "Agent version must belong to the selected agent",
                    ));
                }
                UpdateField::Set(version.id.uuid())
            }
            UpdateField::Clear => UpdateField::Clear,
            UpdateField::Unchanged => UpdateField::Unchanged,
        };

        let agent_identity_id = match req.agent_identity_id {
            UpdateField::Set(identity_id) => {
                let uuid = validate_agent_identity(ctx, identity_id).await?;
                UpdateField::Set(uuid)
            }
            UpdateField::Clear => UpdateField::Clear,
            UpdateField::Unchanged => UpdateField::Unchanged,
        };
        let (owner_principal_id, resolved_owner_user_id) = match agent_identity_id {
            UpdateField::Set(identity_uuid) => {
                let owner = PrincipalService::new(ctx.db.clone())
                    .owner_for_entity(
                        ctx.org_id(),
                        existing.owner_principal_id,
                        existing.resolved_owner_user_id,
                        Some(everruns_core::typed_id::AgentIdentityId::from_uuid(
                            identity_uuid,
                        )),
                    )
                    .await
                    .map_err(classify_anyhow)?;
                (
                    Some(owner.id),
                    UpdateField::from_option(owner.resolved_user_id),
                )
            }
            UpdateField::Clear => {
                let owner = PrincipalService::new(ctx.db.clone())
                    .owner_for_entity(
                        ctx.org_id(),
                        existing.owner_principal_id,
                        existing.resolved_owner_user_id,
                        None,
                    )
                    .await
                    .map_err(classify_anyhow)?;
                (
                    Some(owner.id),
                    UpdateField::from_option(owner.resolved_user_id),
                )
            }
            UpdateField::Unchanged => (None, UpdateField::Unchanged),
        };

        let encryption = ctx.encryption.as_ref();
        let mut channel_config = None;
        let mut channel_config_encrypted = None;
        if existing.channel_config_encrypted.is_none() && encryption.is_some() {
            let (stored, encrypted) =
                q::prepare_channel_config(encryption, &existing.channel_config)
                    .map_err(classify_anyhow)?;
            channel_config = Some(stored);
            channel_config_encrypted = encrypted;
        }

        let input = UpdateApp {
            name: req.name,
            description: req.description,
            harness_id,
            agent_id,
            agent_version_policy: req.agent_version_policy.map(|policy| policy.to_string()),
            agent_version_id,
            agent_identity_id,
            owner_principal_id,
            resolved_owner_user_id,
            channel_type: None,
            channel_config,
            channel_config_encrypted,
            status: req.status.clone().map(|s| s.to_string()),
            published_at: UpdateField::Unchanged,
        };

        let row = ctx
            .db
            .update_app(ctx.org_id(), existing.id, input)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("App"))?;

        if encryption.is_some() {
            let channel_rows = ctx
                .db
                .list_app_channels(existing.id)
                .await
                .map_err(classify_anyhow)?;
            for channel_row in channel_rows
                .into_iter()
                .filter(|channel_row| channel_row.channel_config_encrypted.is_none())
            {
                let (stored, encrypted) =
                    q::prepare_channel_config(encryption, &channel_row.channel_config)
                        .map_err(classify_anyhow)?;
                let input = UpdateAppChannel {
                    channel_config: Some(stored),
                    channel_config_encrypted: encrypted,
                    ..Default::default()
                };
                ctx.db
                    .update_app_channel(channel_row.id, input)
                    .await
                    .map_err(classify_anyhow)?;
            }
        }

        let app = q::row_to_app(&ctx.db, encryption, row, ctx.org_id()).await;
        if req.status.is_some() {
            sync_all_schedule_bindings(ctx, &app).await?;
        }
        Ok(app)
    }
}

inventory::submit! { CommandDescriptor::of::<UpdateAppCmd>() }

// ============================================================================
// DeleteApp
// ============================================================================

/// Archive an app (soft delete).
#[derive(Debug, Deserialize, ToSchema)]
pub struct DeleteApp {
    pub id: String,
}

impl Command for DeleteApp {
    type Output = serde_json::Value;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "delete_app",
            category: "apps",
            description: "Archive an app (soft delete). Can be restored.",
            method: "DELETE",
            path: "/v1/apps/{id}",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&APP_DANGEROUS)
    }

    fn positional_arg() -> Option<&'static str> {
        Some("id")
    }

    async fn execute(self, ctx: &Ctx) -> Result<serde_json::Value, CommandError> {
        let app_id: AppId = self
            .id
            .parse()
            .map_err(|e| CommandError::bad_request(format!("Invalid app ID: {e}")))?;

        let existing = ctx
            .db
            .get_app_by_public_id(ctx.org_id(), &app_id.to_string())
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("App"))?;

        ctx.db
            .delete_app(ctx.org_id(), existing.id)
            .await
            .map_err(classify_anyhow)?;

        if let Some(row) = ctx
            .db
            .get_app_by_public_id(ctx.org_id(), &app_id.to_string())
            .await
            .map_err(classify_anyhow)?
        {
            let app = q::row_to_app(&ctx.db, ctx.encryption.as_ref(), row, ctx.org_id()).await;
            sync_all_schedule_bindings(ctx, &app).await?;
        }

        Ok(serde_json::json!({"deleted": true}))
    }
}

inventory::submit! { CommandDescriptor::of::<DeleteApp>() }

// ============================================================================
// DestroyApp (hard delete)
// ============================================================================

/// Permanently delete an archived app.
#[derive(Debug, Deserialize, ToSchema)]
pub struct DestroyApp {
    pub id: String,
}

impl Command for DestroyApp {
    type Output = serde_json::Value;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "destroy_app",
            category: "apps",
            description: "Permanently delete an archived app.",
            method: "POST",
            path: "/v1/apps/{id}/delete",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&APP_DANGEROUS)
    }

    fn positional_arg() -> Option<&'static str> {
        Some("id")
    }

    async fn execute(self, ctx: &Ctx) -> Result<serde_json::Value, CommandError> {
        let app_id: AppId = self
            .id
            .parse()
            .map_err(|e| CommandError::bad_request(format!("Invalid app ID: {e}")))?;

        let existing = ctx
            .db
            .get_app_by_public_id(ctx.org_id(), &app_id.to_string())
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("App"))?;

        if existing.status != "archived" {
            return Err(CommandError::bad_request(
                "App must be archived before deletion",
            ));
        }

        for channel_row in ctx
            .db
            .list_app_channels(existing.id)
            .await
            .map_err(classify_anyhow)?
        {
            remove_schedule_binding_for_channel(ctx, &channel_row).await?;
        }

        ctx.db
            .destroy_app(ctx.org_id(), existing.id)
            .await
            .map_err(classify_anyhow)?;

        Ok(serde_json::json!({"destroyed": true}))
    }
}

inventory::submit! { CommandDescriptor::of::<DestroyApp>() }

// ============================================================================
// PublishApp
// ============================================================================

/// Publish an app (start accepting requests).
#[derive(Debug, Deserialize, ToSchema)]
pub struct PublishApp {
    pub id: String,
}

impl Command for PublishApp {
    type Output = App;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "publish_app",
            category: "apps",
            description: "Publish an app (start accepting requests).",
            method: "POST",
            path: "/v1/apps/{id}/publish",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&APP_DANGEROUS)
    }

    fn positional_arg() -> Option<&'static str> {
        Some("id")
    }

    async fn execute(self, ctx: &Ctx) -> Result<App, CommandError> {
        let app_id: AppId = self
            .id
            .parse()
            .map_err(|e| CommandError::bad_request(format!("Invalid app ID: {e}")))?;

        let existing = ctx
            .db
            .get_app_by_public_id(ctx.org_id(), &app_id.to_string())
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("App"))?;
        if existing.status != "draft" {
            return Err(CommandError::bad_request(
                "App must be draft before publishing",
            ));
        }
        require_app_channel_before_publish(ctx, existing.id).await?;

        let input = UpdateApp {
            status: Some("published".to_string()),
            published_at: UpdateField::Set(Utc::now()),
            ..Default::default()
        };

        let encryption = ctx.encryption.as_ref();
        let row = ctx
            .db
            .update_app(ctx.org_id(), existing.id, input)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("App"))?;
        let app = q::row_to_app(&ctx.db, encryption, row, ctx.org_id()).await;
        sync_all_schedule_bindings(ctx, &app).await?;
        Ok(app)
    }
}

inventory::submit! { CommandDescriptor::of::<PublishApp>() }

// ============================================================================
// UnpublishApp
// ============================================================================

/// Unpublish an app (stop accepting requests).
#[derive(Debug, Deserialize, ToSchema)]
pub struct UnpublishApp {
    pub id: String,
}

impl Command for UnpublishApp {
    type Output = App;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "unpublish_app",
            category: "apps",
            description: "Unpublish an app (stop accepting requests).",
            method: "POST",
            path: "/v1/apps/{id}/unpublish",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&APP_DANGEROUS)
    }

    fn positional_arg() -> Option<&'static str> {
        Some("id")
    }

    async fn execute(self, ctx: &Ctx) -> Result<App, CommandError> {
        let app_id: AppId = self
            .id
            .parse()
            .map_err(|e| CommandError::bad_request(format!("Invalid app ID: {e}")))?;

        let existing = ctx
            .db
            .get_app_by_public_id(ctx.org_id(), &app_id.to_string())
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("App"))?;

        let input = UpdateApp {
            status: Some("draft".to_string()),
            ..Default::default()
        };

        let encryption = ctx.encryption.as_ref();
        let row = ctx
            .db
            .update_app(ctx.org_id(), existing.id, input)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("App"))?;
        let app = q::row_to_app(&ctx.db, encryption, row, ctx.org_id()).await;
        sync_all_schedule_bindings(ctx, &app).await?;
        Ok(app)
    }
}

inventory::submit! { CommandDescriptor::of::<UnpublishApp>() }

// ============================================================================
// AddChannel
// ============================================================================

/// Add a channel to an app.
#[derive(Debug, Deserialize, ToSchema)]
pub struct AddChannel {
    pub app_id: String,
    #[serde(flatten)]
    pub req: AddChannelRequest,
}

impl Command for AddChannel {
    type Output = AppChannel;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "add_app_channel",
            category: "apps",
            description: "Add a channel to an app.",
            method: "POST",
            path: "/v1/apps/{id}/channels",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&APP_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<AppChannel, CommandError> {
        let app_id: AppId = self
            .app_id
            .parse()
            .map_err(|e| CommandError::bad_request(format!("Invalid app ID: {e}")))?;

        let encryption = ctx.encryption.as_ref();
        let app = ctx
            .db
            .get_app_by_public_id(ctx.org_id(), &app_id.to_string())
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("App"))?;

        if !matches!(app.status.as_str(), "draft" | "published") {
            return Err(CommandError::bad_request(
                "Archived or deleted apps cannot be edited",
            ));
        }

        if self.req.channel_type == ChannelType::Schedule {
            let _ = durable_store(ctx)?;
        }

        let channel_config = normalize_and_validate_channel_config(
            self.req.channel_type.clone(),
            self.req.channel_config.unwrap_or_default(),
        )?;
        let (stored_plaintext, encrypted) =
            q::prepare_channel_config(encryption, &channel_config).map_err(classify_anyhow)?;

        let channel_uuid = Uuid::now_v7();
        let channel_public_id = AppChannelId::from_uuid(channel_uuid);
        let input = CreateAppChannelRow {
            public_id: channel_public_id.to_string(),
            channel_type: self.req.channel_type.to_string(),
            channel_config: stored_plaintext,
            channel_config_encrypted: encrypted,
            durable_schedule_id: None,
            enabled: self.req.enabled.unwrap_or(true),
        };

        let row = ctx
            .db
            .create_app_channel(app.id, input)
            .await
            .map_err(classify_anyhow)?;

        let app = q::row_to_app(&ctx.db, encryption, app, ctx.org_id()).await;
        sync_schedule_binding_for_channel(ctx, &app, &row).await?;

        Ok(q::channel_row_to_channel(encryption, row))
    }
}

inventory::submit! { CommandDescriptor::of::<AddChannel>() }

// ============================================================================
// AddScheduleChannel
// ============================================================================

/// Add a schedule invocation channel to an app using flat args suitable for bash mode.
#[derive(Debug, Deserialize, ToSchema)]
pub struct AddScheduleChannelCmd {
    pub app_id: String,
    pub cron_expression: String,
    pub timezone: Option<String>,
    #[serde(default)]
    pub session_mode: InvocationSessionMode,
    pub message: String,
    // Bashkit's MCP flag parser forwards bools as JSON strings ("true"/"false"),
    // so the lenient deserializer is required to accept `--enabled true`.
    #[serde(default, deserialize_with = "deserialize_opt_bool_lenient")]
    pub enabled: Option<bool>,
}

impl Command for AddScheduleChannelCmd {
    type Output = AppChannel;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "add_schedule_app_channel",
            category: "apps",
            description: "Add a schedule invocation channel to an app using flat args.",
            method: "POST",
            path: "/v1/apps/{id}/channels",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&APP_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<AppChannel, CommandError> {
        AddChannel {
            app_id: self.app_id,
            req: AddChannelRequest {
                channel_type: ChannelType::Schedule,
                channel_config: Some(json!({
                    "cron_expression": self.cron_expression,
                    "timezone": self.timezone.unwrap_or_else(|| "UTC".to_string()),
                    "session_mode": self.session_mode,
                    "message": self.message,
                })),
                enabled: self.enabled,
            },
        }
        .execute(ctx)
        .await
    }
}

inventory::submit! { CommandDescriptor::of::<AddScheduleChannelCmd>() }

// ============================================================================
// AddWebhookChannel
// ============================================================================

/// Add a webhook invocation channel to an app using flat args suitable for bash mode.
#[derive(Debug, Deserialize, ToSchema)]
pub struct AddWebhookChannelCmd {
    pub app_id: String,
    pub token: String,
    #[serde(default)]
    pub session_mode: InvocationSessionMode,
    pub message: String,
    // Bashkit's MCP flag parser forwards bools as JSON strings ("true"/"false"),
    // so the lenient deserializer is required to accept `--enabled true`.
    #[serde(default, deserialize_with = "deserialize_opt_bool_lenient")]
    pub enabled: Option<bool>,
}

impl Command for AddWebhookChannelCmd {
    type Output = AppChannel;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "add_webhook_app_channel",
            category: "apps",
            description: "Add a webhook invocation channel to an app using flat args.",
            method: "POST",
            path: "/v1/apps/{id}/channels",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&APP_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<AppChannel, CommandError> {
        AddChannel {
            app_id: self.app_id,
            req: AddChannelRequest {
                channel_type: ChannelType::Webhook,
                channel_config: Some(json!({
                    "token": self.token,
                    "session_mode": self.session_mode,
                    "message": self.message,
                })),
                enabled: self.enabled,
            },
        }
        .execute(ctx)
        .await
    }
}

inventory::submit! { CommandDescriptor::of::<AddWebhookChannelCmd>() }

// ============================================================================
// AddA2aChannel
// ============================================================================

/// Generate a fresh A2A API key, return (plaintext, sha256_hash, display_prefix).
///
/// Plaintext format: `evra2a_<64 hex chars>` — 32 random bytes (256-bit
/// entropy). The hash is SHA-256 hex, matching `auth/api_key.rs`. The prefix
/// is the first 8 hex chars after `evra2a_`, suffixed with `...`, for
/// non-secret UI display.
pub fn generate_a2a_api_key() -> (String, String, String) {
    use rand::RngCore;

    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    let hex = hex::encode(bytes);
    let plaintext = format!("evra2a_{hex}");
    let hash = hash_a2a_api_key(&plaintext);
    let prefix = format!("evra2a_{}...", &hex[..8]);
    (plaintext, hash, prefix)
}

/// Hash a plaintext A2A API key using SHA-256, returning hex.
pub fn hash_a2a_api_key(plaintext: &str) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(plaintext.as_bytes()))
}

/// Output of [`AddA2aChannelCmd`] — includes the plaintext API key (returned
/// **once**, never persisted) plus the resulting [`AppChannel`].
#[derive(Debug, serde::Serialize, ToSchema)]
pub struct AddA2aChannelOutput {
    /// Plaintext API key. Persist this — it cannot be recovered later.
    pub api_key: String,
    /// The created A2A channel.
    pub channel: AppChannel,
}

/// Add an A2A invocation channel to an app. Generates the API key server-side
/// and returns the plaintext exactly once.
#[derive(Debug, Deserialize, ToSchema)]
pub struct AddA2aChannelCmd {
    pub app_id: String,
    #[serde(default)]
    pub session_mode: InvocationSessionMode,
    pub message: String,
    pub agent_card_name: Option<String>,
    pub agent_card_description: Option<String>,
    #[serde(default, deserialize_with = "deserialize_opt_bool_lenient")]
    pub enabled: Option<bool>,
}

impl Command for AddA2aChannelCmd {
    type Output = AddA2aChannelOutput;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "add_a2a_app_channel",
            category: "apps",
            description: "Add an A2A (Agent2Agent) invocation channel to an app. Returns the plaintext API key exactly once.",
            method: "POST",
            path: "/v1/apps/{id}/a2a-channels",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&APP_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<AddA2aChannelOutput, CommandError> {
        let (plaintext, hash, prefix) = generate_a2a_api_key();
        let mut config = json!({
            "api_key_hash": hash,
            "api_key_prefix": prefix,
            "session_mode": self.session_mode,
            "message": self.message,
        });
        if let Some(name) = self.agent_card_name {
            config["agent_card_name"] = Value::String(name);
        }
        if let Some(desc) = self.agent_card_description {
            config["agent_card_description"] = Value::String(desc);
        }
        let channel = AddChannel {
            app_id: self.app_id,
            req: AddChannelRequest {
                channel_type: ChannelType::A2a,
                channel_config: Some(config),
                enabled: self.enabled,
            },
        }
        .execute(ctx)
        .await?;
        Ok(AddA2aChannelOutput {
            api_key: plaintext,
            channel,
        })
    }
}

inventory::submit! { CommandDescriptor::of::<AddA2aChannelCmd>() }

// ============================================================================
// RegenerateA2aApiKey
// ============================================================================

/// Regenerate the API key for an A2A channel. Returns the new plaintext key
/// exactly once and invalidates the previous key.
#[derive(Debug, Deserialize, ToSchema)]
pub struct RegenerateA2aApiKeyCmd {
    pub app_id: String,
    pub channel_id: String,
}

/// Output of [`RegenerateA2aApiKeyCmd`] — includes the newly generated
/// plaintext API key (returned **once**, never persisted) plus the updated
/// [`AppChannel`].
#[derive(Debug, serde::Serialize, ToSchema)]
pub struct RegenerateA2aApiKeyOutput {
    /// New plaintext API key. Persist this — it cannot be recovered later.
    /// The previous key is invalidated immediately.
    pub api_key: String,
    /// The updated A2A channel.
    pub channel: AppChannel,
}

impl Command for RegenerateA2aApiKeyCmd {
    type Output = RegenerateA2aApiKeyOutput;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "regenerate_a2a_app_channel_key",
            category: "apps",
            description: "Regenerate the A2A channel API key. Returns the new plaintext exactly once and invalidates the previous key.",
            method: "POST",
            path: "/v1/apps/{id}/a2a-channels/{channel_id}/regenerate-key",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&APP_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<RegenerateA2aApiKeyOutput, CommandError> {
        let app_id: AppId = self
            .app_id
            .parse()
            .map_err(|e| CommandError::bad_request(format!("Invalid app ID: {e}")))?;
        let app = ctx
            .db
            .get_app_by_public_id(ctx.org_id(), &app_id.to_string())
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("App"))?;
        if !matches!(app.status.as_str(), "draft" | "published") {
            return Err(CommandError::bad_request(
                "Archived or deleted apps cannot be edited",
            ));
        }

        let channel_row = ctx
            .db
            .get_app_channel_by_public_id(&self.channel_id)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Channel"))?;
        if channel_row.app_id != app.id {
            return Err(CommandError::bad_request(
                "Channel does not belong to this app",
            ));
        }
        if channel_row.channel_type != ChannelType::A2a.to_string() {
            return Err(CommandError::bad_request("Channel is not an A2A channel"));
        }

        let encryption = ctx.encryption.as_ref();
        let mut existing_config: Value = q::decrypt_channel_config(
            encryption,
            channel_row.channel_config_encrypted.as_deref(),
            &channel_row.channel_config,
        );
        let (plaintext, hash, prefix) = generate_a2a_api_key();
        if let Some(map) = existing_config.as_object_mut() {
            map.insert("api_key_hash".to_string(), Value::String(hash));
            map.insert("api_key_prefix".to_string(), Value::String(prefix));
        } else {
            return Err(CommandError::bad_request(
                "Existing A2A channel configuration is not a JSON object",
            ));
        }
        let existing_config =
            normalize_and_validate_channel_config(ChannelType::A2a, existing_config)?;

        let (stored, encrypted) =
            q::prepare_channel_config(encryption, &existing_config).map_err(classify_anyhow)?;
        let row = ctx
            .db
            .update_app_channel(
                channel_row.id,
                UpdateAppChannel {
                    channel_config: Some(stored),
                    channel_config_encrypted: encrypted,
                    ..Default::default()
                },
            )
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Channel"))?;

        Ok(RegenerateA2aApiKeyOutput {
            api_key: plaintext,
            channel: q::channel_row_to_channel(encryption, row),
        })
    }
}

inventory::submit! { CommandDescriptor::of::<RegenerateA2aApiKeyCmd>() }

// ============================================================================
// UpdateChannel
// ============================================================================

/// Update a channel on an app.
#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateChannelCmd {
    pub app_id: String,
    pub channel_id: String,
    #[serde(flatten)]
    pub req: UpdateChannelRequest,
}

impl Command for UpdateChannelCmd {
    type Output = AppChannel;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "update_app_channel",
            category: "apps",
            description: "Update a channel on an app.",
            method: "PATCH",
            path: "/v1/apps/{id}/channels/{channel_id}",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&APP_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<AppChannel, CommandError> {
        let app_id: AppId = self
            .app_id
            .parse()
            .map_err(|e| CommandError::bad_request(format!("Invalid app ID: {e}")))?;

        let encryption = ctx.encryption.as_ref();
        let app = ctx
            .db
            .get_app_by_public_id(ctx.org_id(), &app_id.to_string())
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("App"))?;

        if !matches!(app.status.as_str(), "draft" | "published") {
            return Err(CommandError::bad_request(
                "Archived or deleted apps cannot be edited",
            ));
        }

        let channel_row = ctx
            .db
            .get_app_channel_by_public_id(&self.channel_id)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Channel"))?;

        if channel_row.app_id != app.id {
            return Err(CommandError::bad_request(
                "Channel does not belong to this app",
            ));
        }

        let current_channel_type = ChannelType::from_str_opt(&channel_row.channel_type)
            .ok_or_else(|| CommandError::bad_request("Unknown existing channel type"))?;
        let final_channel_type = self
            .req
            .channel_type
            .clone()
            .unwrap_or(current_channel_type);
        let existing_decrypted = q::decrypt_channel_config(
            encryption,
            channel_row.channel_config_encrypted.as_deref(),
            &channel_row.channel_config,
        );
        let mut final_channel_config = self
            .req
            .channel_config
            .clone()
            .unwrap_or_else(|| existing_decrypted.clone());
        // A2A: server preserves the secret key material across PATCH. Clients
        // updating non-secret fields (message, session_mode, agent card)
        // should not need to round-trip the api_key_hash and api_key_prefix —
        // and must not be able to overwrite them with bogus values. Use the
        // dedicated regenerate-key endpoint to rotate the key.
        if final_channel_type == ChannelType::A2a
            && let (Some(out), Some(existing)) = (
                final_channel_config.as_object_mut(),
                existing_decrypted.as_object(),
            )
        {
            for key in ["api_key_hash", "api_key_prefix"] {
                if let Some(existing_value) = existing.get(key) {
                    out.insert(key.to_string(), existing_value.clone());
                }
            }
        }
        if final_channel_type == ChannelType::Schedule {
            let _ = durable_store(ctx)?;
        }
        let normalized_channel_config =
            normalize_and_validate_channel_config(final_channel_type, final_channel_config)?;

        let config_changed = self.req.channel_config.is_some();
        let (channel_config, channel_config_encrypted) = if config_changed {
            // Persist the merged config (so A2A preserves the existing
            // api_key_hash / prefix when the client omits them).
            let (stored, encrypted) =
                q::prepare_channel_config(encryption, &normalized_channel_config)
                    .map_err(classify_anyhow)?;
            (Some(stored), encrypted)
        } else {
            (None, None)
        };

        let input = UpdateAppChannel {
            channel_type: self.req.channel_type.map(|ct| ct.to_string()),
            channel_config,
            channel_config_encrypted,
            durable_schedule_id: UpdateField::Unchanged,
            enabled: self.req.enabled,
        };

        let row = ctx
            .db
            .update_app_channel(channel_row.id, input)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Channel"))?;

        let app = q::row_to_app(&ctx.db, encryption, app, ctx.org_id()).await;
        sync_schedule_binding_for_channel(ctx, &app, &row).await?;

        Ok(q::channel_row_to_channel(encryption, row))
    }
}

inventory::submit! { CommandDescriptor::of::<UpdateChannelCmd>() }

// ============================================================================
// DeleteChannel
// ============================================================================

/// Remove a channel from an app.
#[derive(Debug, Deserialize, ToSchema)]
pub struct DeleteChannel {
    pub app_id: String,
    pub channel_id: String,
}

impl Command for DeleteChannel {
    type Output = serde_json::Value;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "delete_app_channel",
            category: "apps",
            description: "Remove a channel from an app.",
            method: "DELETE",
            path: "/v1/apps/{id}/channels/{channel_id}",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&APP_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<serde_json::Value, CommandError> {
        let app_id: AppId = self
            .app_id
            .parse()
            .map_err(|e| CommandError::bad_request(format!("Invalid app ID: {e}")))?;

        let app = ctx
            .db
            .get_app_by_public_id(ctx.org_id(), &app_id.to_string())
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("App"))?;

        if !matches!(app.status.as_str(), "draft" | "published") {
            return Err(CommandError::bad_request(
                "Archived or deleted apps cannot be edited",
            ));
        }

        let channel_row = ctx
            .db
            .get_app_channel_by_public_id(&self.channel_id)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Channel"))?;

        if channel_row.app_id != app.id {
            return Err(CommandError::bad_request(
                "Channel does not belong to this app",
            ));
        }

        remove_schedule_binding_for_channel(ctx, &channel_row).await?;

        ctx.db
            .delete_app_channel(channel_row.id)
            .await
            .map_err(classify_anyhow)?;

        Ok(serde_json::json!({"deleted": true}))
    }
}

inventory::submit! { CommandDescriptor::of::<DeleteChannel>() }

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_core::typed_id::{AppChannelId, HarnessId, PrincipalId};

    fn test_app_and_channel() -> (App, AppChannel) {
        let now = Utc::now();
        let channel = AppChannel {
            public_id: AppChannelId::from_seed(2),
            internal_id: Uuid::nil(),
            channel_type: ChannelType::Webhook,
            channel_config: json!({}),
            enabled: true,
            created_at: now,
            updated_at: now,
        };
        let app = App {
            public_id: AppId::from_seed(1),
            internal_id: Uuid::nil(),
            org_id: 1,
            name: "Test App".to_string(),
            description: None,
            harness_id: HarnessId::from_seed(3),
            agent_id: None,
            agent_version_policy: everruns_core::AgentVersionPolicy::Default,
            agent_version_id: None,
            agent_identity_id: None,
            owner_principal_id: PrincipalId::from_seed(4),
            resolved_owner_user_id: None,
            owner: None,
            effective_owner: None,
            channels: vec![channel.clone()],
            status: AppStatus::Published,
            published_at: None,
            created_at: now,
            updated_at: now,
            archived_at: None,
            deleted_at: None,
        };

        (app, channel)
    }

    #[test]
    fn app_invocation_message_metadata_includes_system_app_id() {
        let (app, channel) = test_app_and_channel();

        let metadata =
            app_invocation_message_metadata(&app, &channel, AppInvocationSource::Webhook);

        assert_eq!(
            metadata.get("_app_id"),
            Some(&Value::String(app.public_id.to_string()))
        );
        assert_eq!(
            metadata.get("app_channel_id"),
            Some(&Value::String(channel.public_id.to_string()))
        );
    }
}

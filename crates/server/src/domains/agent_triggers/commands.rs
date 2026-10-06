// Agent-triggers commands — user-facing operations.
//
// An agent trigger is an agent-owned schedule or webhook that wakes the agent
// on its own harness. Schedule behavior mirrors the former App schedule channel
// path. Webhook trigger secrets use the same encrypted configuration mechanism
// as App channel secrets.

use super::events;
use super::github;
use super::mcp_event;
use super::queries as q;
use super::types::{AgentTriggerRun, CreateAgentTriggerRequest, UpdateAgentTriggerRequest};
use super::webhook;
use crate::api::messages::{CreateMessageRequest, InputContentPart, InputMessage, MessageRole};
use crate::api::sessions::CreateSessionRequest;
use crate::auth::audit;
use crate::domains::agent_channels::invocation::{
    calculate_schedule_next_trigger, cron_min_interval_seconds, normalize_cron_expression,
};
use crate::domains::agents::version_policy::{VersionSelection, resolve_version_selection};
use crate::domains::agents::{AGENT_MANAGE, AGENT_VIEW};
use crate::domains::common::*;
use crate::domains::messages::{CreateMessageContext, MessageService};
use crate::domains::sessions::SessionService;
use crate::domains::virtual_users::lifecycle::ensure_identity_for_agent;
use crate::execution_metadata;
use crate::kernel_imports::{Caller, Policy};
use crate::records::AgentChannelId;
use crate::records::{AgentAction, AuditEvent};
use crate::records::{AgentTrigger, AgentTriggerType, ScheduleTriggerConfig, WebhookTriggerConfig};
use crate::storage::StorageBackend;
use crate::storage::models::{
    AgentRow, AgentTriggerRow, CreateAgentTriggerRow, UpdateAgentTrigger,
};
use chrono::Utc;
use everruns_contracts::typed_id::{AgentId, SessionId, TriggerId};
use everruns_core::channel::SessionBinding;
use everruns_db::UpdateField;
use everruns_durable::{
    CreateScheduleRow, Pagination as DurablePagination, ScheduleExecutionFilter,
    ScheduleTargetType, StoreError, UpdateSchedule, WorkflowEventStore,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::str::FromStr;
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

/// Durable activity name that fires an agent schedule trigger. Must match
/// `everruns_worker::activities::activity_types::INVOKE_AGENT_TRIGGER`.
const INVOKE_AGENT_TRIGGER: &str = "invoke_agent_trigger";

// ============================================================================
// Limits — dedicated env knobs so agent triggers tune independently of App
// schedule channels (which use SCHEDULE_CHANNEL_*).
// ============================================================================

/// Minimum seconds between consecutive agent-trigger fires.
/// Configurable via `AGENT_TRIGGER_MIN_INTERVAL_SECONDS`; default 300 (5 min).
fn agent_trigger_min_interval_seconds() -> i64 {
    const DEFAULT: i64 = 300;
    std::env::var("AGENT_TRIGGER_MIN_INTERVAL_SECONDS")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .filter(|&v| v > 0)
        .unwrap_or(DEFAULT)
}

/// Maximum number of enabled triggers per org.
/// Configurable via `AGENT_TRIGGER_MAX_PER_ORG`; default 10.
fn agent_trigger_max_per_org() -> i64 {
    const DEFAULT: i64 = 10;
    std::env::var("AGENT_TRIGGER_MAX_PER_ORG")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .filter(|&v| v > 0)
        .unwrap_or(DEFAULT)
}

// ============================================================================
// Config validation / normalization
// ============================================================================

fn validate_schedule_config(cron_expression: &str, message: &str) -> Result<String, CommandError> {
    if message.trim().is_empty() {
        return Err(CommandError::bad_request(
            "Agent trigger requires a non-empty message",
        ));
    }
    let normalized = normalize_cron_expression(cron_expression)?;
    let schedule = cron::Schedule::from_str(&normalized).expect("already validated");
    let min_limit = agent_trigger_min_interval_seconds();
    if let Some(interval) = cron_min_interval_seconds(&schedule, min_limit)
        && interval < min_limit
    {
        return Err(CommandError::bad_request(format!(
            "Agent trigger cron must fire no more than once every {min_limit} seconds (≥ {} min); expression fires every {interval} seconds",
            min_limit / 60
        )));
    }
    Ok(normalized)
}

/// Serialize a [`ScheduleTriggerConfig`] with a normalized cron into JSONB.
fn build_schedule_config_value(config: &ScheduleTriggerConfig) -> Result<Value, CommandError> {
    serde_json::to_value(config).map_err(|e| CommandError::internal(e.into()))
}

fn validate_webhook_config(
    token: &str,
    message: &str,
    auth: Option<&Value>,
) -> Result<(), CommandError> {
    if auth.is_some() {
        return Err(CommandError::bad_request(
            "Webhook triggers do not support auth config. Use the token field.",
        ));
    }
    if token.trim().is_empty() {
        return Err(CommandError::bad_request(
            "Webhook trigger requires a non-empty token",
        ));
    }
    if message.trim().is_empty() {
        return Err(CommandError::bad_request(
            "Webhook trigger requires a non-empty message",
        ));
    }
    Ok(())
}

fn prepare_trigger_config(
    ctx: &Ctx,
    config: &impl serde::Serialize,
) -> Result<(Value, Option<Vec<u8>>), CommandError> {
    let config = serde_json::to_value(config).map_err(|e| CommandError::internal(e.into()))?;
    crate::domains::agent_channels::queries::prepare_channel_config(
        ctx.encryption.as_ref(),
        &config,
    )
    .map_err(classify_anyhow)
}

/// Count currently-enabled triggers in an org (for the per-org cap). Uses the
/// list path rather than a bespoke aggregate — the cap is small.
async fn count_enabled_triggers(ctx: &Ctx) -> Result<i64, CommandError> {
    let rows = ctx
        .db
        .list_agent_triggers(ctx.org_id(), None, false)
        .await
        .map_err(classify_anyhow)?;
    Ok(rows.iter().filter(|row| row.enabled).count() as i64)
}

// ============================================================================
// Durable binding — mirrors apps::sync_schedule_binding_for_channel
// ============================================================================

fn durable_store(ctx: &Ctx) -> Result<&Arc<dyn WorkflowEventStore + Send + Sync>, CommandError> {
    ctx.workflow_store.as_ref().ok_or_else(|| {
        CommandError::bad_request("Agent triggers require durable execution to be enabled")
    })
}

fn trigger_binding_name(trigger_id: TriggerId) -> String {
    format!("agent-trigger-{trigger_id}")
}

fn build_trigger_target_input(org_id: i64, agent_public_id: &str, trigger_id: TriggerId) -> Value {
    json!({
        "org_id": org_id,
        "agent_id": agent_public_id,
        "trigger_id": trigger_id.to_string(),
    })
}

async fn set_trigger_durable_schedule_id(
    ctx: &Ctx,
    trigger_id: TriggerId,
    durable_schedule_id: Option<Uuid>,
) -> Result<(), CommandError> {
    ctx.db
        .set_agent_trigger_durable_schedule_id(ctx.org_id(), trigger_id, durable_schedule_id)
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

/// Create/update the durable schedule backing a trigger. `enabled` gates on the
/// trigger's `enabled` flag alone (agents have no publish gate).
pub(crate) async fn sync_agent_trigger_binding(
    ctx: &Ctx,
    trigger_row: &AgentTriggerRow,
) -> Result<(), CommandError> {
    let agent = ctx
        .db
        .get_agent(ctx.org_id(), trigger_row.agent_id)
        .await
        .map_err(classify_anyhow)?
        .ok_or_else(|| CommandError::not_found("Agent"))?;
    let agent_public_id = parse_agent_id(&agent.public_id)?;
    let trigger = q::row_to_trigger(
        trigger_row.clone(),
        agent_public_id,
        ctx.encryption.as_ref(),
    );
    if trigger.trigger_type != AgentTriggerType::Schedule {
        return Ok(());
    }
    let store = durable_store(ctx)?;
    let config = trigger
        .schedule_config()
        .map_err(|_| CommandError::bad_request("Invalid schedule trigger configuration"))?;
    let cron_expression = normalize_cron_expression(&config.cron_expression)?;
    let enabled = trigger_row.enabled && trigger_row.status == "active";

    // The durable target must name the agent by its user-facing public id so the
    // invocation activity can resolve it.
    let target_input = build_trigger_target_input(ctx.org_id(), &agent.public_id, trigger_row.id);
    let description = format!(
        "Agent schedule trigger {} for {}",
        trigger_row.id, agent.name
    );
    let next_trigger_at = if enabled {
        UpdateField::from_option(calculate_schedule_next_trigger(&cron_expression)?)
    } else {
        UpdateField::Clear
    };

    let mut created_schedule_id = None;
    let schedule_id = match trigger_row.durable_schedule_id {
        Some(schedule_id) => {
            let update = UpdateSchedule {
                name: Some(trigger_binding_name(trigger_row.id)),
                description: UpdateField::Set(description.clone()),
                cron_expression: Some(cron_expression.clone()),
                timezone: Some(config.timezone.clone()),
                target_type: Some(ScheduleTargetType::Activity),
                target_name: Some(INVOKE_AGENT_TRIGGER.to_string()),
                target_input: Some(target_input.clone()),
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
                        .create_schedule(new_schedule_row(
                            trigger_row.id,
                            &description,
                            &cron_expression,
                            &config,
                            &target_input,
                            enabled,
                        )?)
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
                .create_schedule(new_schedule_row(
                    trigger_row.id,
                    &description,
                    &cron_expression,
                    &config,
                    &target_input,
                    enabled,
                )?)
                .await
                .map_err(|err| classify_anyhow(err.into()))?;
            created_schedule_id = Some(created);
            created
        }
    };

    if trigger_row.durable_schedule_id != Some(schedule_id)
        && let Err(err) =
            set_trigger_durable_schedule_id(ctx, trigger_row.id, Some(schedule_id)).await
    {
        if let Some(created_id) = created_schedule_id {
            let _ = store.delete_schedule(created_id).await;
        }
        return Err(err);
    }
    Ok(())
}

fn new_schedule_row(
    trigger_id: TriggerId,
    description: &str,
    cron_expression: &str,
    config: &ScheduleTriggerConfig,
    target_input: &Value,
    enabled: bool,
) -> Result<CreateScheduleRow, CommandError> {
    Ok(CreateScheduleRow {
        name: trigger_binding_name(trigger_id),
        description: Some(description.to_string()),
        cron_expression: cron_expression.to_string(),
        timezone: config.timezone.clone(),
        target_type: ScheduleTargetType::Activity,
        target_name: INVOKE_AGENT_TRIGGER.to_string(),
        target_input: target_input.clone(),
        enabled,
        max_concurrent: Some(1),
        catch_up_missed: false,
        max_catch_up: Some(1),
        retry_policy: None,
        next_trigger_at: if enabled {
            calculate_schedule_next_trigger(cron_expression)?
        } else {
            None
        },
    })
}

/// Tear down the durable schedule backing a trigger (disable/delete).
pub(crate) async fn remove_agent_trigger_binding(
    ctx: &Ctx,
    trigger_row: &AgentTriggerRow,
) -> Result<(), CommandError> {
    let Some(store) = ctx.workflow_store.as_ref() else {
        return Ok(());
    };
    delete_durable_schedule_if_present(store, trigger_row.durable_schedule_id).await?;
    if trigger_row.durable_schedule_id.is_some() {
        set_trigger_durable_schedule_id(ctx, trigger_row.id, None).await?;
    }
    Ok(())
}

// ============================================================================
// Shared command helpers
// ============================================================================

fn parse_trigger_id(raw: &str) -> Result<TriggerId, CommandError> {
    raw.parse()
        .map_err(|e| CommandError::bad_request(format!("Invalid trigger ID: {e}")))
}

pub(super) fn parse_agent_id(raw: &str) -> Result<AgentId, CommandError> {
    raw.parse()
        .map_err(|e| CommandError::bad_request(format!("Invalid agent ID: {e}")))
}

// ============================================================================
// CreateAgentTrigger
// ============================================================================

/// Create a trigger on an agent.
#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct CreateAgentTrigger {
    /// Owning agent's prefixed public identifier.
    pub agent_id: String,
    #[serde(flatten)]
    pub req: CreateAgentTriggerRequest,
}

impl Command for CreateAgentTrigger {
    type Output = AgentTrigger;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "create_agent_trigger",
            category: "agent_triggers",
            description: "Create a schedule or webhook trigger for an agent.",
            method: "POST",
            path: "/v1/agents/{agent_id}/triggers",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&AGENT_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<AgentTrigger, CommandError> {
        let agent_public = parse_agent_id(&self.agent_id)?;
        let agent = q::require_active_agent(&ctx.db, ctx.org_id(), &agent_public).await?;

        let req = self.req;
        let version = resolve_version_selection(
            ctx,
            agent.id,
            None,
            req.agent_version_policy.clone(),
            req.agent_version_id,
        )
        .await?;

        // GitHub events always carry a subject (repository, pull request).
        let has_subject = req.trigger_type == AgentTriggerType::GitHub
            || matches!(
                req.trigger_type,
                AgentTriggerType::Webhook | AgentTriggerType::McpEvent
            ) && req
                .subject_template
                .as_deref()
                .is_some_and(|template| !template.trim().is_empty());
        events::validate_trigger_binding(req.session_mode, has_subject)?;
        webhook::require_publication_permission(ctx, req.trigger_type, req.enabled)?;
        let trigger_id = TriggerId::new();
        let (ingress_id, config, config_encrypted) = match req.trigger_type {
            AgentTriggerType::Schedule => {
                let cron_expression = req.cron_expression.as_deref().ok_or_else(|| {
                    CommandError::bad_request("Schedule trigger requires cron_expression")
                })?;
                let normalized_cron = validate_schedule_config(cron_expression, &req.message)?;
                if req.enabled {
                    let count = count_enabled_triggers(ctx).await?;
                    let max = agent_trigger_max_per_org();
                    if count >= max {
                        return Err(CommandError::bad_request(format!(
                            "Organization may have at most {max} enabled agent trigger(s); currently has {count}"
                        )));
                    }
                }
                let config = ScheduleTriggerConfig {
                    cron_expression: normalized_cron,
                    timezone: req.timezone,
                    session_mode: req.session_mode,
                    message: req.message,
                };
                (None, build_schedule_config_value(&config)?, None)
            }
            AgentTriggerType::Webhook => {
                let token = req.token.ok_or_else(|| {
                    CommandError::bad_request("Webhook trigger requires a non-empty token")
                })?;
                validate_webhook_config(&token, &req.message, req.auth.as_ref())?;
                let config = WebhookTriggerConfig {
                    token,
                    session_mode: req.session_mode,
                    message: req.message,
                    rate_limit_per_minute: req.rate_limit_per_minute,
                    event_id_template: events::optional_template(req.event_id_template),
                    subject_template: events::optional_template(req.subject_template),
                    filter: events::optional_filter(req.filter)?,
                };
                let (config, encrypted) = prepare_trigger_config(ctx, &config)?;
                (
                    Some(AgentChannelId::from_uuid(trigger_id.uuid()).to_string()),
                    config,
                    encrypted,
                )
            }
            AgentTriggerType::GitHub => {
                (None, github::create_config(ctx, &agent, &req).await?, None)
            }
            // The ingress id names the MCP server's callback for this trigger.
            AgentTriggerType::McpEvent => (
                Some(AgentChannelId::from_uuid(trigger_id.uuid()).to_string()),
                mcp_event::create_config(ctx, &agent, &req).await?,
                None,
            ),
        };
        let row = ctx
            .db
            .create_agent_trigger(CreateAgentTriggerRow {
                org_id: ctx.org_id(),
                id: trigger_id,
                agent_id: agent.id,
                trigger_type: req.trigger_type.to_string(),
                ingress_id,
                config,
                config_encrypted,
                enabled: req.enabled,
                durable_schedule_id: None,
                execution_harness_id: None,
                execution_owner_principal_id: None,
                execution_resolved_owner_user_id: None,
                execution_virtual_user_id: None,
                execution_app_id: None,
                legacy_alias_id: None,
                legacy_alias_name: None,
                agent_version_policy: version.as_ref().map(VersionSelection::policy_str),
                agent_version_id: version.and_then(|version| version.version_id),
            })
            .await
            .map_err(classify_anyhow)?;

        sync_agent_trigger_binding(ctx, &row).await?;
        if req.trigger_type == AgentTriggerType::McpEvent {
            mcp_event::after_create(ctx, &row).await?;
        }
        // Re-read so the response carries the persisted durable_schedule_id.
        let row = q::get_by_id(&ctx.db, ctx.org_id(), row.id)
            .await?
            .unwrap_or(row);
        Ok(webhook::redact_for_response(q::row_to_trigger(
            row,
            agent_public,
            ctx.encryption.as_ref(),
        )))
    }
}

inventory::submit! { CommandDescriptor::of::<CreateAgentTrigger>() }

// ============================================================================
// ListAgentTriggers
// ============================================================================

/// List an agent's triggers.
#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct ListAgentTriggers {
    pub agent_id: String,
    #[serde(default, deserialize_with = "deserialize_bool_lenient")]
    pub include_archived: bool,
}

impl Command for ListAgentTriggers {
    type Output = Vec<AgentTrigger>;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "list_agent_triggers",
            category: "agent_triggers",
            description: "List an agent's triggers. include_archived=true also returns archived.",
            method: "GET",
            path: "/v1/agents/{agent_id}/triggers",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&AGENT_VIEW)
    }

    fn positional_arg() -> Option<&'static str> {
        Some("agent_id")
    }

    async fn execute(self, ctx: &Ctx) -> Result<Vec<AgentTrigger>, CommandError> {
        let agent_public = parse_agent_id(&self.agent_id)?;
        let agent = q::require_active_agent(&ctx.db, ctx.org_id(), &agent_public).await?;
        let rows = ctx
            .db
            .list_agent_triggers(ctx.org_id(), Some(agent.id), self.include_archived)
            .await
            .map_err(classify_anyhow)?;
        Ok(rows
            .into_iter()
            .map(|row| {
                webhook::redact_for_response(q::row_to_trigger(
                    row,
                    agent_public,
                    ctx.encryption.as_ref(),
                ))
            })
            .collect())
    }
}

inventory::submit! { CommandDescriptor::of::<ListAgentTriggers>() }

// ============================================================================
// GetAgentTrigger
// ============================================================================

/// Get a single trigger by id.
#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct GetAgentTrigger {
    pub agent_id: String,
    pub trigger_id: String,
}

impl Command for GetAgentTrigger {
    type Output = AgentTrigger;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "get_agent_trigger",
            category: "agent_triggers",
            description: "Get a single agent trigger by id.",
            method: "GET",
            path: "/v1/agents/{agent_id}/triggers/{trigger_id}",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&AGENT_VIEW)
    }

    async fn execute(self, ctx: &Ctx) -> Result<AgentTrigger, CommandError> {
        let (agent, trigger) =
            resolve_trigger_for_agent(ctx, &self.agent_id, &self.trigger_id).await?;
        let agent_public = parse_agent_id(&agent.public_id)?;
        Ok(webhook::redact_for_response(q::row_to_trigger(
            trigger,
            agent_public,
            ctx.encryption.as_ref(),
        )))
    }
}

inventory::submit! { CommandDescriptor::of::<GetAgentTrigger>() }

// ============================================================================
// ListAgentTriggerRuns
// ============================================================================

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct ListAgentTriggerRuns {
    pub agent_id: String,
    pub trigger_id: String,
}

impl Command for ListAgentTriggerRuns {
    type Output = Vec<AgentTriggerRun>;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "list_agent_trigger_runs",
            category: "agent_triggers",
            description: "List recent execution outcomes for an agent trigger.",
            method: "GET",
            path: "/v1/agents/{agent_id}/triggers/{trigger_id}/runs",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&AGENT_VIEW)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Vec<AgentTriggerRun>, CommandError> {
        let (_, trigger) = resolve_trigger_for_agent(ctx, &self.agent_id, &self.trigger_id).await?;
        let Some(schedule_id) = trigger.durable_schedule_id else {
            return Ok(Vec::new());
        };
        let executions = durable_store(ctx)?
            .list_schedule_executions(
                ScheduleExecutionFilter {
                    schedule_id: Some(schedule_id),
                    status: None,
                },
                DurablePagination {
                    offset: 0,
                    limit: 10,
                },
            )
            .await
            .map_err(|err| classify_anyhow(err.into()))?;
        Ok(executions
            .into_iter()
            .map(|execution| AgentTriggerRun {
                id: execution.id.to_string(),
                status: execution.status.to_string(),
                scheduled_at: execution.scheduled_at,
                completed_at: execution.completed_at,
                error: execution.error,
            })
            .collect())
    }
}

inventory::submit! { CommandDescriptor::of::<ListAgentTriggerRuns>() }

// ============================================================================
// UpdateAgentTrigger
// ============================================================================

/// Update a trigger. Only provided fields change; the rest are preserved.
#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct UpdateAgentTriggerCmd {
    pub agent_id: String,
    pub trigger_id: String,
    #[serde(flatten)]
    pub req: UpdateAgentTriggerRequest,
}

impl Command for UpdateAgentTriggerCmd {
    type Output = AgentTrigger;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "update_agent_trigger",
            category: "agent_triggers",
            description: "Update an agent trigger. Only provided fields change.",
            method: "PATCH",
            path: "/v1/agents/{agent_id}/triggers/{trigger_id}",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&AGENT_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<AgentTrigger, CommandError> {
        let (agent, existing) =
            resolve_trigger_for_agent(ctx, &self.agent_id, &self.trigger_id).await?;
        let req = self.req;
        let mut resubscribe = false;
        let version = resolve_version_selection(
            ctx,
            agent.id,
            Some(&stored_version_selection(&existing)),
            req.agent_version_policy.clone(),
            req.agent_version_id,
        )
        .await?;

        let trigger = q::row_to_trigger(
            existing.clone(),
            parse_agent_id(&self.agent_id)?,
            ctx.encryption.as_ref(),
        );

        let new_enabled = req.enabled.unwrap_or(existing.enabled);
        webhook::require_publication_permission(
            ctx,
            trigger.trigger_type,
            new_enabled && !existing.enabled,
        )?;
        if trigger.trigger_type == AgentTriggerType::Schedule && new_enabled && !existing.enabled {
            let count = count_enabled_triggers(ctx).await?;
            let max = agent_trigger_max_per_org();
            if count >= max {
                return Err(CommandError::bad_request(format!(
                    "Organization may have at most {max} enabled agent trigger(s); currently has {count}"
                )));
            }
        }
        let (config, config_encrypted) = match trigger.trigger_type {
            AgentTriggerType::Schedule => {
                let mut config = trigger.schedule_config().map_err(|_| {
                    CommandError::bad_request("Invalid stored schedule configuration")
                })?;
                if let Some(cron) = req.cron_expression {
                    config.cron_expression = cron;
                }
                if let Some(tz) = req.timezone {
                    config.timezone = tz;
                }
                if let Some(mode) = req.session_mode {
                    events::validate_trigger_binding(mode, false)?;
                    config.session_mode = mode;
                }
                if let Some(message) = req.message {
                    config.message = message;
                }
                config.cron_expression =
                    validate_schedule_config(&config.cron_expression, &config.message)?;
                (build_schedule_config_value(&config)?, None)
            }
            AgentTriggerType::Webhook => {
                let mut config = trigger.webhook_config().map_err(|_| {
                    CommandError::bad_request("Invalid stored webhook configuration")
                })?;
                if let Some(token) = req.token {
                    config.token = token;
                }
                if let Some(mode) = req.session_mode {
                    config.session_mode = mode;
                }
                if let Some(message) = req.message {
                    config.message = message;
                }
                if let Some(limit) = req.rate_limit_per_minute {
                    config.rate_limit_per_minute = Some(limit);
                }
                if let Some(template) = req.event_id_template {
                    config.event_id_template = events::optional_template(Some(template));
                }
                if let Some(template) = req.subject_template {
                    config.subject_template = events::optional_template(Some(template));
                }
                if let Some(filter) = req.filter {
                    config.filter = events::optional_filter(Some(filter))?;
                }
                events::validate_trigger_binding(
                    config.session_mode,
                    config.subject_template.is_some(),
                )?;
                validate_webhook_config(&config.token, &config.message, req.auth.as_ref())?;
                prepare_trigger_config(ctx, &config)?
            }
            AgentTriggerType::GitHub => (github::update_config(&trigger, &req)?, None),
            AgentTriggerType::McpEvent => {
                let (config, changed) =
                    mcp_event::update_config(ctx, &agent, &trigger, &req).await?;
                resubscribe = changed;
                (config, None)
            }
        };

        let row = ctx
            .db
            .update_agent_trigger(
                ctx.org_id(),
                existing.id,
                UpdateAgentTrigger {
                    config: Some(config),
                    config_encrypted,
                    enabled: Some(new_enabled),
                    agent_version_policy: version.as_ref().map(VersionSelection::policy_str),
                    agent_version_id: version
                        .map_or(everruns_db::UpdateField::Unchanged, |version| {
                            everruns_db::UpdateField::from_option(version.version_id)
                        }),
                    ..Default::default()
                },
            )
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Agent trigger"))?;

        if trigger.trigger_type == AgentTriggerType::Schedule {
            if new_enabled {
                sync_agent_trigger_binding(ctx, &row).await?;
            } else {
                remove_agent_trigger_binding(ctx, &row).await?;
            }
        } else if trigger.trigger_type == AgentTriggerType::McpEvent {
            mcp_event::after_update(ctx, &existing, &row, resubscribe).await?;
        }
        let row = q::get_by_id(&ctx.db, ctx.org_id(), row.id)
            .await?
            .unwrap_or(row);
        Ok(webhook::redact_for_response(q::row_to_trigger(
            row,
            parse_agent_id(&self.agent_id)?,
            ctx.encryption.as_ref(),
        )))
    }
}

inventory::submit! { CommandDescriptor::of::<UpdateAgentTriggerCmd>() }

// ============================================================================
// DeleteAgentTrigger
// ============================================================================

/// Archive a trigger (soft delete) and tear down its durable schedule.
#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct DeleteAgentTrigger {
    pub agent_id: String,
    pub trigger_id: String,
}

impl Command for DeleteAgentTrigger {
    type Output = serde_json::Value;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "delete_agent_trigger",
            category: "agent_triggers",
            description: "Archive an agent trigger and remove its durable schedule.",
            method: "DELETE",
            path: "/v1/agents/{agent_id}/triggers/{trigger_id}",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&AGENT_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<serde_json::Value, CommandError> {
        let (_, trigger) = resolve_trigger_for_agent(ctx, &self.agent_id, &self.trigger_id).await?;
        // Tear down the binding first so a fire cannot race the archive.
        remove_agent_trigger_binding(ctx, &trigger).await?;
        if trigger.trigger_type == AgentTriggerType::McpEvent.to_string() {
            mcp_event::deactivate(ctx, &trigger).await;
        }
        let deleted = ctx
            .db
            .delete_agent_trigger(ctx.org_id(), trigger.id)
            .await
            .map_err(classify_anyhow)?;
        if deleted {
            Ok(json!({ "deleted": true }))
        } else {
            Err(CommandError::not_found("Agent trigger"))
        }
    }
}

inventory::submit! { CommandDescriptor::of::<DeleteAgentTrigger>() }

// ============================================================================
// TriggerAgentTriggerNow (manual fire)
// ============================================================================

#[derive(Debug, serde::Serialize, ToSchema)]
pub struct TriggerAgentTriggerOutput {
    /// Session's prefixed public identifier.
    pub session_id: SessionId,
    pub created_session: bool,
}

/// Manually fire a trigger once (does not expose the durable schedule id).
#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct TriggerAgentTriggerNow {
    pub agent_id: String,
    pub trigger_id: String,
}

impl Command for TriggerAgentTriggerNow {
    type Output = TriggerAgentTriggerOutput;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "trigger_agent_trigger",
            category: "agent_triggers",
            description: "Manually fire an agent trigger once.",
            method: "POST",
            path: "/v1/agents/{agent_id}/triggers/{trigger_id}/trigger",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&AGENT_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<TriggerAgentTriggerOutput, CommandError> {
        let (agent, trigger) =
            resolve_trigger_for_agent(ctx, &self.agent_id, &self.trigger_id).await?;
        let session_service = ctx.session_service.as_ref().ok_or_else(|| {
            CommandError::internal(anyhow::anyhow!("Session service not available"))
        })?;
        let message_service = ctx.message_service.as_ref().ok_or_else(|| {
            CommandError::internal(anyhow::anyhow!("Message service not available"))
        })?;
        let trigger_id = parse_trigger_id(&self.trigger_id)?;
        let execution_id = if let Some(schedule_id) = trigger.durable_schedule_id {
            Some(
                durable_store(ctx)?
                    .create_schedule_execution(schedule_id, Utc::now())
                    .await
                    .map_err(|err| classify_anyhow(err.into()))?,
            )
        } else {
            None
        };
        let result = invoke_agent_trigger(
            &ctx.db,
            session_service,
            message_service,
            ctx.org_id(),
            &agent.public_id,
            &trigger_id.to_string(),
        )
        .await;
        let result = match result {
            Ok(result) => {
                if let Some(execution_id) = execution_id {
                    durable_store(ctx)?
                        .complete_schedule_execution(execution_id, result.session_id.uuid(), false)
                        .await
                        .map_err(|err| classify_anyhow(err.into()))?;
                }
                result
            }
            Err(err) => {
                if let Some(execution_id) = execution_id {
                    durable_store(ctx)?
                        .fail_schedule_execution(execution_id, &err.to_string())
                        .await
                        .map_err(|store_err| classify_anyhow(store_err.into()))?;
                }
                return Err(err);
            }
        };
        Ok(TriggerAgentTriggerOutput {
            session_id: result.session_id,
            created_session: result.created_session,
        })
    }
}

inventory::submit! { CommandDescriptor::of::<TriggerAgentTriggerNow>() }

// ============================================================================
// invoke_agent_trigger — execution activity (mirrors invoke_scheduled_legacy_alias_channel)
// ============================================================================

#[derive(Debug, Clone)]
pub struct AgentTriggerInvocationResult {
    pub session_id: SessionId,
    pub created_session: bool,
}

/// Resolve a trigger + its agent, render the schedule message, create-or-reuse
/// the agent's session, and dispatch. The trigger is the source of truth; the
/// `agent_id` argument scopes/validates the relationship.
pub async fn invoke_agent_trigger(
    db: &Arc<StorageBackend>,
    session_service: &SessionService,
    message_service: &MessageService,
    org_id: i64,
    agent_id: &str,
    trigger_id: &str,
) -> Result<AgentTriggerInvocationResult, CommandError> {
    let trigger_id = parse_trigger_id(trigger_id)?;
    let trigger_row = db
        .get_agent_trigger(org_id, trigger_id)
        .await
        .map_err(classify_anyhow)?
        .filter(|row| row.status == "active")
        .ok_or_else(|| CommandError::not_found("Agent trigger"))?;
    if !trigger_row.enabled {
        return Err(CommandError::forbidden(
            "Agent trigger is disabled".to_string(),
        ));
    }

    let agent = db
        .get_agent(org_id, trigger_row.agent_id)
        .await
        .map_err(classify_anyhow)?
        .ok_or_else(|| CommandError::not_found("Agent"))?;
    if agent.status != "active" {
        return Err(CommandError::forbidden("Agent is not active".to_string()));
    }
    // Defense-in-depth: the requested agent (from the route/durable target) must
    // match the trigger's owner.
    if let Ok(requested) = agent_id.parse::<AgentId>()
        && requested.to_string() != agent.public_id
    {
        return Err(CommandError::not_found("Agent trigger"));
    }

    let trigger = q::row_to_trigger(trigger_row.clone(), parse_agent_id(&agent.public_id)?, None);
    let config = trigger
        .schedule_config()
        .map_err(|_| CommandError::bad_request("Invalid schedule trigger configuration"))?;

    let context = json!({
        "agent": {
            "id": agent.public_id,
            "name": agent.name.clone(),
        },
        "trigger": {
            "id": trigger_id.to_string(),
            "type": trigger.trigger_type.to_string(),
        },
        "invocation": {
            "source": "agent_trigger",
            "triggered_at": Utc::now().to_rfc3339(),
        },
    });
    let outcome = events::dispatch_trigger_event(
        db,
        session_service,
        message_service,
        events::TriggerEventRoute {
            trigger: &trigger_row,
            agent: &agent,
            message_template: &config.message,
            session_mode: config.session_mode,
            filter: None,
            session_source: crate::records::SessionSource::Schedule,
            webhook_compat: None,
        },
        events::TriggerEvent {
            source: "schedule",
            event_id: None,
            event_type: None,
            subject: None,
            context,
        },
        None,
    )
    .await?;
    match outcome {
        events::TriggerEventOutcome::Dispatched(result) => Ok(result),
        // A schedule event has no filter and no event id, so neither can occur.
        events::TriggerEventOutcome::Filtered | events::TriggerEventOutcome::Duplicate => Err(
            CommandError::internal(anyhow::anyhow!("schedule event was not dispatched")),
        ),
    }
}

#[derive(Debug, Clone)]
pub(super) struct TriggerExecutionContext {
    pub(super) harness_id: everruns_contracts::typed_id::HarnessId,
    pub(super) owner_principal_id: everruns_contracts::typed_id::PrincipalId,
    resolved_owner_user_id: Option<Uuid>,
    virtual_user_id: Option<everruns_contracts::typed_id::VirtualUserId>,
    app_id: Option<Uuid>,
    agent_version_policy: crate::records::AgentVersionPolicy,
    agent_version_id: Option<everruns_contracts::typed_id::AgentVersionId>,
}

/// Legacy App attribution for webhook triggers migrated from App channels.
#[derive(Debug, Clone)]
pub struct WebhookCompatibilityContext {
    pub(super) app_public_id: String,
    pub(super) app_name: String,
    pub(super) ingress_id: String,
}

/// Resolve the execution context (harness, owner principal, identity, app) that
/// a trigger fire runs under.
///
/// - Migrated App schedules persist their original App execution context on the
///   trigger row (`execution_*`). Preserving it avoids silently changing harness
///   capabilities, private memory scope, or identity-backed providers after the
///   106 migration retargeted App schedules to Agent triggers.
/// - Native Agent triggers have no persisted context: the session is owned by
///   the agent's own identity principal, lazily created on first fire (EVE-758),
///   so the agent acts as itself and the shared-session reuse key stays stable.
pub(super) async fn resolve_trigger_execution_context(
    db: &Arc<StorageBackend>,
    org_id: i64,
    agent: &AgentRow,
    trigger: &AgentTriggerRow,
) -> Result<TriggerExecutionContext, CommandError> {
    if let (Some(harness_id), Some(owner_principal_id)) = (
        trigger.execution_harness_id,
        trigger.execution_owner_principal_id,
    ) {
        return Ok(TriggerExecutionContext {
            harness_id,
            owner_principal_id,
            resolved_owner_user_id: trigger.execution_resolved_owner_user_id,
            virtual_user_id: trigger.execution_virtual_user_id,
            app_id: trigger.execution_app_id,
            agent_version_policy: stored_version_selection(trigger).policy,
            agent_version_id: trigger.agent_version_id,
        });
    }

    let (virtual_user_id, owner) = ensure_identity_for_agent(db, org_id, agent)
        .await
        .map_err(classify_anyhow)?;
    let harness_id = if agent.harness_source == "organization_default" {
        db.get_organization_settings(org_id)
            .await
            .map_err(classify_anyhow)?
            .and_then(|settings| settings.default_harness_id)
            .unwrap_or(agent.harness_id)
    } else {
        agent.harness_id
    };

    Ok(TriggerExecutionContext {
        harness_id,
        owner_principal_id: owner.id,
        resolved_owner_user_id: owner.resolved_user_id,
        virtual_user_id: Some(virtual_user_id),
        app_id: None,
        // Native triggers carry their own version selection (EVE-1139); a NULL
        // policy on an older row means the agent's default version.
        agent_version_policy: stored_version_selection(trigger).policy,
        agent_version_id: trigger.agent_version_id,
    })
}

/// The trigger's stored version selection. NULL policy predates per-trigger
/// pinning and means `default`.
fn stored_version_selection(trigger: &AgentTriggerRow) -> VersionSelection {
    VersionSelection {
        policy: trigger
            .agent_version_policy
            .as_deref()
            .map(crate::records::AgentVersionPolicy::from)
            .unwrap_or_default(),
        version_id: trigger.agent_version_id,
    }
}

fn trigger_session_tags(
    trigger_id: TriggerId,
    webhook: Option<&WebhookCompatibilityContext>,
) -> Vec<String> {
    if let Some(webhook) = webhook {
        return vec![
            format!("app:{}", webhook.app_public_id),
            format!("app_channel:{}", webhook.ingress_id),
            "app_channel_type:webhook".to_string(),
            "__internal:app_invocation".to_string(),
        ];
    }
    vec![
        format!("agent_trigger:{trigger_id}"),
        "__internal:agent_trigger".to_string(),
    ]
}

pub(super) async fn find_or_create_trigger_session(
    db: &Arc<StorageBackend>,
    session_service: &SessionService,
    org_id: i64,
    agent: &AgentRow,
    execution_context: &TriggerExecutionContext,
    route: events::TriggerSessionRoute<'_>,
) -> Result<(SessionId, bool), CommandError> {
    let events::TriggerSessionRoute {
        trigger_id,
        session_mode,
        subject,
        source,
        webhook,
    } = route;
    let mut shared_tags = trigger_session_tags(trigger_id, webhook);
    // `per_thread` on a trigger means one session per event subject. An event
    // without a subject cannot be keyed and gets a fresh session instead.
    let session_mode = match (session_mode, subject) {
        (SessionBinding::Thread, Some(subject)) => {
            shared_tags.push(events::subject_session_tag(subject));
            SessionBinding::Shared
        }
        (SessionBinding::Thread, None) => SessionBinding::Ephemeral,
        (mode, _) => mode,
    };
    if session_mode == SessionBinding::Shared {
        let existing = if let Some(app_id) = execution_context.app_id {
            db.find_app_session_by_tags_and_owner(
                org_id,
                app_id,
                execution_context.owner_principal_id,
                &shared_tags,
            )
            .await
            .map_err(classify_anyhow)?
        } else {
            db.find_session_by_tags_and_owner(
                org_id,
                execution_context.owner_principal_id,
                &shared_tags,
            )
            .await
            .map_err(classify_anyhow)?
        };
        if let Some(existing) = existing {
            return Ok((existing.id, false));
        }
    }

    let mut tags = shared_tags;
    if session_mode == SessionBinding::Ephemeral {
        let prefix = if webhook.is_some() {
            "app_invocation"
        } else {
            "agent_invocation"
        };
        tags.push(format!("{prefix}:{}", Uuid::now_v7()));
    }

    let title_subject = webhook.map_or(agent.name.as_str(), |value| value.app_name.as_str());
    let title_source = if webhook.is_some() {
        "webhook"
    } else {
        "agent_trigger"
    };
    let title = if let Some(subject) = subject.filter(|_| session_mode == SessionBinding::Shared) {
        format!("{title_subject} {subject}")
    } else if session_mode == SessionBinding::Shared {
        format!("{title_subject} {title_source}")
    } else {
        format!("{title_subject} {title_source} {}", Utc::now().to_rfc3339())
    };

    let req = CreateSessionRequest {
        playground_user_id: None,
        source: None,
        workspace_id: None,
        harness_id: Some(execution_context.harness_id),
        harness_name: None,
        agent_id: Some(agent.id),
        agent_name: None,
        virtual_user_id: execution_context.virtual_user_id,
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
    };

    let session = if let Some(app_id) = execution_context.app_id {
        session_service
            .create_from_app(
                &Caller::internal(org_id),
                execution_context.harness_id.uuid(),
                Some(agent.id.uuid()),
                Some(agent.id),
                Some(app_id),
                execution_context.agent_version_policy.clone(),
                execution_context.agent_version_id,
                // Migrated App schedules (migration 106) kept
                // `execution_app_id` but never an endpoint pointer, so there
                // is nothing structural to record here.
                None,
                Some(trigger_id.uuid()),
                execution_context.owner_principal_id,
                execution_context.resolved_owner_user_id,
                source,
                req,
            )
            .await
    } else {
        session_service
            .create_from_agent_trigger(
                &Caller::internal(org_id),
                execution_context.harness_id.uuid(),
                agent.id.uuid(),
                agent.id,
                Some(trigger_id.uuid()),
                execution_context.agent_version_policy.clone(),
                execution_context.agent_version_id,
                execution_context.owner_principal_id,
                execution_context.resolved_owner_user_id,
                source,
                req,
            )
            .await
    }
    .map_err(classify_anyhow)?;

    Ok((session.id, true))
}

// Trigger dispatch threads the resolved execution context (org, agent, harness,
// principal) plus the rendered message into one durable send; grouping these
// into a struct would not improve clarity over the explicit parameter list.
#[allow(clippy::too_many_arguments)]
pub(super) async fn dispatch_trigger_message(
    message_service: &MessageService,
    org_id: i64,
    agent: &AgentRow,
    trigger_id: TriggerId,
    session_id: SessionId,
    harness_id: everruns_contracts::typed_id::HarnessId,
    owner_principal_id: everruns_contracts::typed_id::PrincipalId,
    rendered_message: String,
    request_id: Option<String>,
) -> Result<(), CommandError> {
    let metadata = Some(
        [
            (
                "source".to_string(),
                Value::String("agent_trigger".to_string()),
            ),
            (
                "_agent_id".to_string(),
                Value::String(agent.public_id.clone()),
            ),
            (
                "agent_trigger_id".to_string(),
                Value::String(trigger_id.to_string()),
            ),
        ]
        .into_iter()
        .collect(),
    );

    message_service
        .create(
            CreateMessageContext {
                runtime_subject_principal_id: None,
                org_id,
                user_id: None,
                harness_id: harness_id.uuid(),
                agent_id: Some(agent.id.uuid()),
                session_id: session_id.uuid(),
                event_metadata: Some(execution_metadata::agent_trigger_message_metadata(
                    trigger_id,
                    owner_principal_id,
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

#[allow(clippy::too_many_arguments)]
pub(super) fn emit_agent_trigger_audit_event(
    db: Arc<StorageBackend>,
    org_id: i64,
    agent: &AgentRow,
    trigger_id: TriggerId,
    session_id: SessionId,
    owner_principal_id: everruns_contracts::typed_id::PrincipalId,
    created_session: bool,
) {
    // Reuse the AppInvocationStarted action — the audit token space has no
    // agent-trigger-specific variant, and the `source` detail disambiguates.
    let event = AuditEvent::agent(AgentAction::AppInvocationStarted, org_id, None)
        .target("agent_trigger", trigger_id.to_string())
        .detail("source", "agent_trigger")
        .detail("agent_id", agent.public_id.clone())
        .detail("agent_trigger_id", trigger_id.to_string())
        .detail("session_id", session_id.to_string())
        .detail("created_session", created_session)
        .detail("owner_principal_id", owner_principal_id.to_string());
    audit::emit_event(db, event.build());
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

mod resolve_trigger;
pub(super) use resolve_trigger::resolve_trigger_for_agent;

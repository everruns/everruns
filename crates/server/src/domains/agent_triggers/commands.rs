// Agent-triggers commands — user-facing operations.
//
// An agent trigger is an agent-owned, cron-driven durable schedule that wakes
// the agent on its own harness. This mirrors the App **schedule channel** path
// (`crate::domains::apps::commands`) almost exactly, re-homed on the agent:
// there is no App row, no publish gate, and no config encryption — just the
// trigger's `enabled` flag and its `schedule` config.

use super::queries as q;
use super::types::{CreateAgentTriggerRequest, UpdateAgentTriggerRequest};
use crate::api::messages::{CreateMessageRequest, InputContentPart, InputMessage, MessageRole};
use crate::api::sessions::CreateSessionRequest;
use crate::auth::audit;
use crate::domains::agents::{AGENT_MANAGE, AGENT_VIEW};
use crate::domains::apps::commands::{
    calculate_schedule_next_trigger, cron_min_interval_seconds, normalize_cron_expression,
    render_message_template,
};
use crate::domains::common::*;
use crate::domains::messages::{CreateMessageContext, MessageService};
use crate::domains::sessions::SessionService;
use crate::execution_metadata;
use crate::services::PrincipalService;
use crate::storage::StorageBackend;
use crate::storage::models::{
    AgentRow, AgentTriggerRow, CreateAgentTriggerRow, UpdateAgentTrigger,
};
use chrono::Utc;
use everruns_core::typed_id::{AgentId, SessionId, TriggerId};
use everruns_core::{
    AgentAction, AgentTrigger, AgentTriggerType, AuditEvent, Caller, InvocationSessionMode, Policy,
    ScheduleTriggerConfig,
};
use everruns_durable::{
    CreateScheduleRow, ScheduleTargetType, StoreError, UpdateField, UpdateSchedule,
    WorkflowEventStore,
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

/// Validate a schedule config's message + cron, returning the normalized cron.
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
    let trigger = q::row_to_trigger(trigger_row.clone(), agent_public_id);
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

fn parse_agent_id(raw: &str) -> Result<AgentId, CommandError> {
    raw.parse()
        .map_err(|e| CommandError::bad_request(format!("Invalid agent ID: {e}")))
}

/// Resolve a trigger and confirm it belongs to the named agent (org-scoped).
async fn resolve_trigger_for_agent(
    ctx: &Ctx,
    agent_id: &str,
    trigger_id: &str,
) -> Result<(AgentRow, AgentTriggerRow), CommandError> {
    let agent_public = parse_agent_id(agent_id)?;
    let agent = q::require_active_agent(&ctx.db, ctx.org_id(), &agent_public).await?;
    let trigger_id = parse_trigger_id(trigger_id)?;
    let trigger = q::get_by_id(&ctx.db, ctx.org_id(), trigger_id)
        .await?
        .filter(|row| row.status != "deleted")
        .ok_or_else(|| CommandError::not_found("Agent trigger"))?;
    if trigger.agent_id != agent.id {
        return Err(CommandError::not_found("Agent trigger"));
    }
    Ok((agent, trigger))
}

// ============================================================================
// CreateAgentTrigger
// ============================================================================

/// Create a schedule trigger on an agent.
#[derive(Debug, Deserialize, ToSchema)]
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
            description: "Create a schedule trigger that wakes an agent on a cron.",
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
        let normalized_cron = validate_schedule_config(&req.cron_expression, &req.message)?;

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
        let row = ctx
            .db
            .create_agent_trigger(CreateAgentTriggerRow {
                org_id: ctx.org_id(),
                id: TriggerId::new(),
                agent_id: agent.id,
                trigger_type: AgentTriggerType::Schedule.to_string(),
                config: build_schedule_config_value(&config)?,
                enabled: req.enabled,
                durable_schedule_id: None,
            })
            .await
            .map_err(classify_anyhow)?;

        sync_agent_trigger_binding(ctx, &row).await?;
        // Re-read so the response carries the persisted durable_schedule_id.
        let row = q::get_by_id(&ctx.db, ctx.org_id(), row.id)
            .await?
            .unwrap_or(row);
        Ok(q::row_to_trigger(row, agent_public))
    }
}

inventory::submit! { CommandDescriptor::of::<CreateAgentTrigger>() }

// ============================================================================
// ListAgentTriggers
// ============================================================================

/// List an agent's triggers.
#[derive(Debug, Deserialize, ToSchema)]
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
            .map(|row| q::row_to_trigger(row, agent_public))
            .collect())
    }
}

inventory::submit! { CommandDescriptor::of::<ListAgentTriggers>() }

// ============================================================================
// GetAgentTrigger
// ============================================================================

/// Get a single trigger by id.
#[derive(Debug, Deserialize, ToSchema)]
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
        Ok(q::row_to_trigger(trigger, agent_public))
    }
}

inventory::submit! { CommandDescriptor::of::<GetAgentTrigger>() }

// ============================================================================
// UpdateAgentTrigger
// ============================================================================

/// Update a trigger. Only provided fields change; the rest are preserved.
#[derive(Debug, Deserialize, ToSchema)]
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
        let (_, existing) =
            resolve_trigger_for_agent(ctx, &self.agent_id, &self.trigger_id).await?;
        let req = self.req;

        // Merge onto the stored schedule config.
        let mut config = q::row_to_trigger(existing.clone(), parse_agent_id(&self.agent_id)?)
            .schedule_config()
            .map_err(|_| CommandError::bad_request("Invalid stored schedule configuration"))?;
        if let Some(cron) = req.cron_expression {
            config.cron_expression = cron;
        }
        if let Some(tz) = req.timezone {
            config.timezone = tz;
        }
        if let Some(mode) = req.session_mode {
            config.session_mode = mode;
        }
        if let Some(message) = req.message {
            config.message = message;
        }
        let normalized_cron = validate_schedule_config(&config.cron_expression, &config.message)?;
        config.cron_expression = normalized_cron;

        let new_enabled = req.enabled.unwrap_or(existing.enabled);
        // Enforce the per-org enabled cap when flipping a trigger on.
        if new_enabled && !existing.enabled {
            let count = count_enabled_triggers(ctx).await?;
            let max = agent_trigger_max_per_org();
            if count >= max {
                return Err(CommandError::bad_request(format!(
                    "Organization may have at most {max} enabled agent trigger(s); currently has {count}"
                )));
            }
        }

        let row = ctx
            .db
            .update_agent_trigger(
                ctx.org_id(),
                existing.id,
                UpdateAgentTrigger {
                    config: Some(build_schedule_config_value(&config)?),
                    enabled: Some(new_enabled),
                    ..Default::default()
                },
            )
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Agent trigger"))?;

        if new_enabled {
            sync_agent_trigger_binding(ctx, &row).await?;
        } else {
            remove_agent_trigger_binding(ctx, &row).await?;
        }
        let row = q::get_by_id(&ctx.db, ctx.org_id(), row.id)
            .await?
            .unwrap_or(row);
        Ok(q::row_to_trigger(row, parse_agent_id(&self.agent_id)?))
    }
}

inventory::submit! { CommandDescriptor::of::<UpdateAgentTriggerCmd>() }

// ============================================================================
// DeleteAgentTrigger
// ============================================================================

/// Archive a trigger (soft delete) and tear down its durable schedule.
#[derive(Debug, Deserialize, ToSchema)]
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
#[derive(Debug, Deserialize, ToSchema)]
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
        let (agent, _) = resolve_trigger_for_agent(ctx, &self.agent_id, &self.trigger_id).await?;
        let session_service = ctx.session_service.as_ref().ok_or_else(|| {
            CommandError::internal(anyhow::anyhow!("Session service not available"))
        })?;
        let message_service = ctx.message_service.as_ref().ok_or_else(|| {
            CommandError::internal(anyhow::anyhow!("Message service not available"))
        })?;
        let trigger_id = parse_trigger_id(&self.trigger_id)?;
        let result = invoke_agent_trigger(
            &ctx.db,
            session_service,
            message_service,
            ctx.org_id(),
            &agent.public_id,
            &trigger_id.to_string(),
        )
        .await?;
        Ok(TriggerAgentTriggerOutput {
            session_id: result.session_id,
            created_session: result.created_session,
        })
    }
}

inventory::submit! { CommandDescriptor::of::<TriggerAgentTriggerNow>() }

// ============================================================================
// invoke_agent_trigger — execution activity (mirrors invoke_scheduled_app_channel)
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

    let trigger = q::row_to_trigger(trigger_row.clone(), parse_agent_id(&agent.public_id)?);
    let config = trigger
        .schedule_config()
        .map_err(|_| CommandError::bad_request("Invalid schedule trigger configuration"))?;

    let template_context = json!({
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
    let rendered_message = render_message_template(&config.message, &template_context);
    if rendered_message.trim().is_empty() {
        return Err(CommandError::bad_request(
            "Rendered invocation message is empty",
        ));
    }

    // Ownership: agents have no agent-identity layer, so the session is owned by
    // the org's default owner principal (internal caller → system/human owner).
    // Resolving it up front lets the shared-session reuse lookup key on it.
    let owner = PrincipalService::new(db.clone())
        .default_owner_principal(&Caller::internal(org_id), None)
        .await
        .map_err(classify_anyhow)?;

    let (session_id, created_session) = find_or_create_trigger_session(
        db,
        session_service,
        org_id,
        &agent,
        trigger_id,
        config.session_mode,
        owner.id,
        owner.resolved_user_id,
    )
    .await?;

    dispatch_trigger_message(
        message_service,
        org_id,
        &agent,
        trigger_id,
        session_id,
        owner.id,
        rendered_message,
    )
    .await?;

    emit_agent_trigger_audit_event(
        Arc::clone(db),
        org_id,
        &agent,
        trigger_id,
        session_id,
        owner.id,
        created_session,
    );

    Ok(AgentTriggerInvocationResult {
        session_id,
        created_session,
    })
}

fn trigger_session_tags(trigger_id: TriggerId) -> Vec<String> {
    vec![
        format!("agent_trigger:{trigger_id}"),
        "__internal:agent_trigger".to_string(),
    ]
}

#[allow(clippy::too_many_arguments)]
async fn find_or_create_trigger_session(
    db: &Arc<StorageBackend>,
    session_service: &SessionService,
    org_id: i64,
    agent: &AgentRow,
    trigger_id: TriggerId,
    session_mode: InvocationSessionMode,
    owner_principal_id: everruns_core::PrincipalId,
    resolved_owner_user_id: Option<Uuid>,
) -> Result<(SessionId, bool), CommandError> {
    let shared_tags = trigger_session_tags(trigger_id);
    if session_mode == InvocationSessionMode::SharedSession
        && let Some(existing) = db
            .find_session_by_tags_and_owner(org_id, owner_principal_id, &shared_tags)
            .await
            .map_err(classify_anyhow)?
    {
        return Ok((existing.id, false));
    }

    let mut tags = shared_tags;
    if session_mode == InvocationSessionMode::SessionPerInvocation {
        tags.push(format!("agent_invocation:{}", Uuid::now_v7()));
    }

    let title = if session_mode == InvocationSessionMode::SharedSession {
        format!("{} agent_trigger", agent.name)
    } else {
        format!("{} agent_trigger {}", agent.name, Utc::now().to_rfc3339())
    };

    let session = session_service
        .create_from_agent_trigger(
            &Caller::internal(org_id),
            agent.harness_id.uuid(),
            agent.id.uuid(),
            agent.id,
            owner_principal_id,
            resolved_owner_user_id,
            CreateSessionRequest {
                workspace_id: None,
                harness_id: Some(agent.harness_id),
                harness_name: None,
                agent_id: Some(agent.id),
                agent_name: None,
                agent_identity_id: None,
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

async fn dispatch_trigger_message(
    message_service: &MessageService,
    org_id: i64,
    agent: &AgentRow,
    trigger_id: TriggerId,
    session_id: SessionId,
    owner_principal_id: everruns_core::PrincipalId,
    rendered_message: String,
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
                org_id,
                user_id: None,
                harness_id: agent.harness_id.uuid(),
                agent_id: Some(agent.id.uuid()),
                session_id: session_id.uuid(),
                event_metadata: Some(execution_metadata::agent_trigger_message_metadata(
                    trigger_id,
                    owner_principal_id,
                )),
                request_id: None,
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
fn emit_agent_trigger_audit_event(
    db: Arc<StorageBackend>,
    org_id: i64,
    agent: &AgentRow,
    trigger_id: TriggerId,
    session_id: SessionId,
    owner_principal_id: everruns_core::PrincipalId,
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

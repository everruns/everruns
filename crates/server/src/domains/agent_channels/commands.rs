use super::redact_channel_for_response;
use super::types::{CreateAgentChannelRequest, UpdateAgentChannelRequest};
use super::validation::{merge_preserved_secret_fields, normalize_and_validate_channel_config};
use crate::api::channel_ingress::{channel_liveness, row_to_ingress};
use crate::domains::agents::{AGENT_DANGEROUS, AGENT_MANAGE, AGENT_VIEW};
use crate::domains::common::*;
use crate::domains::virtual_users::lifecycle::ensure_identity_for_agent;
use crate::records::{AgentChannel, AgentChannelId, ChannelType};
use crate::storage::UpdateField;
use crate::storage::{CreateAgentChannelRow, IngressChannelRow, UpdateAgentChannelRow};
use everruns_contracts::typed_id::AgentId;
use serde::Deserialize;
use serde_json::{Value, json};
use utoipa::ToSchema;

async fn resolve_agent(
    ctx: &Ctx,
    id_or_name: &str,
) -> Result<crate::storage::models::AgentRow, CommandError> {
    let row = find_agent(ctx, id_or_name).await?;
    if row.status != "active" {
        return Err(CommandError::bad_request(
            "Archived or deleted agents cannot manage channels",
        ));
    }
    Ok(row)
}

pub(crate) async fn find_agent(
    ctx: &Ctx,
    id_or_name: &str,
) -> Result<crate::storage::models::AgentRow, CommandError> {
    let row = if let Ok(agent_id) = id_or_name.parse::<AgentId>() {
        ctx.db
            .get_agent_by_public_id(ctx.org_id(), &agent_id.to_string())
            .await
    } else {
        ctx.db.get_agent_by_name(ctx.org_id(), id_or_name).await
    }?
    .ok_or_else(|| CommandError::not_found("Agent"))?;
    Ok(row)
}

fn row_to_channel(ctx: &Ctx, row: IngressChannelRow) -> Result<AgentChannel, CommandError> {
    let (_, channel) = row_to_ingress(ctx.encryption.as_ref(), row)?;
    Ok(redact_channel_for_response(channel.into_channel()))
}

fn decrypted_config(ctx: &Ctx, row: IngressChannelRow) -> Result<Value, CommandError> {
    let (_, channel) = row_to_ingress(ctx.encryption.as_ref(), row)?;
    let mut config = channel.channel_config;
    if let Some(auth) = channel.auth
        && let Some(object) = config.as_object_mut()
    {
        object.insert(
            "auth".to_string(),
            serde_json::to_value(auth).map_err(|error| {
                CommandError::bad_request(format!("Invalid channel auth configuration: {error}"))
            })?,
        );
    }
    Ok(config)
}

// THREAT[TM-AUTHZ-022]: package imports preflight this same live-channel gate
// before mutating the agent, so a denied ingress update cannot partially apply.
fn prepare_updated_config(
    ctx: &Ctx,
    existing: &IngressChannelRow,
    channel_type: ChannelType,
    mut config: Value,
    enabled: Option<bool>,
) -> Result<Value, CommandError> {
    let current = decrypted_config(ctx, existing.clone())?;
    merge_preserved_secret_fields(channel_type.clone(), &mut config, &current);
    let config = normalize_and_validate_channel_config(channel_type.clone(), config)?;
    let changed = super::exposure::config_changed(
        &config,
        normalize_and_validate_channel_config(channel_type, current).ok(),
    );
    super::exposure::require_live_change_permission(
        ctx,
        &existing.channel_status,
        changed,
        enabled == Some(false),
    )?;
    Ok(config)
}

pub(crate) async fn preflight_package_channel_config(
    ctx: &Ctx,
    agent_id: uuid::Uuid,
    channel_id: &str,
    config: Value,
    enabled: Option<bool>,
) -> Result<(), CommandError> {
    let existing = ctx
        .db
        .get_agent_channel(ctx.org_id(), agent_id, channel_id)
        .await?
        .ok_or_else(|| CommandError::not_found("Channel"))?;
    let channel_type = ChannelType::from_str_opt(&existing.channel_type)
        .ok_or_else(|| CommandError::bad_request("Channel has an unsupported channel type"))?;
    prepare_updated_config(ctx, &existing, channel_type, config, enabled)?;
    Ok(())
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct ListAgentChannels {
    pub agent_id: String,
}

#[command(
    name = "list_agent_channels",
    category = "agent_channels",
    description = "List an agent's ingress channels.",
    method = "GET",
    path = "/v1/agents/{agent_id}/channels",
    policy = AGENT_VIEW,
)]
impl Command for ListAgentChannels {
    type Output = Vec<AgentChannel>;

    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        let agent = find_agent(ctx, &self.agent_id).await?;
        ctx.db
            .list_agent_channels(ctx.org_id(), agent.id.uuid())
            .await?
            .into_iter()
            .map(|row| row_to_channel(ctx, row))
            .collect()
    }
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct GetAgentChannel {
    pub agent_id: String,
    pub channel_id: String,
}

#[command(
    name = "get_agent_channel",
    category = "agent_channels",
    description = "Get an agent ingress channel.",
    method = "GET",
    path = "/v1/agents/{agent_id}/channels/{channel_id}",
    policy = AGENT_VIEW,
)]
impl Command for GetAgentChannel {
    type Output = AgentChannel;

    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        let agent = find_agent(ctx, &self.agent_id).await?;
        let row = ctx
            .db
            .get_agent_channel(ctx.org_id(), agent.id.uuid(), &self.channel_id)
            .await?
            .ok_or_else(|| CommandError::not_found("Channel"))?;
        row_to_channel(ctx, row)
    }
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct CreateAgentChannel {
    pub agent_id: String,
    #[serde(flatten)]
    pub req: CreateAgentChannelRequest,
}

#[command(
    name = "create_agent_channel",
    category = "agent_channels",
    description = "Create an ingress channel for an agent.",
    method = "POST",
    path = "/v1/agents/{agent_id}/channels",
    policy = AGENT_MANAGE,
)]
impl Command for CreateAgentChannel {
    type Output = AgentChannel;

    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        if self.req.channel_type == ChannelType::PublicChat && !ctx.feature_flags.public_chat {
            return Err(CommandError::feature_not_enabled("public_chat"));
        }
        if self.req.channel_type == ChannelType::Voice && !ctx.feature_flags.voice {
            return Err(CommandError::feature_not_enabled("voice"));
        }
        if self.req.channel_type == ChannelType::Schedule {
            return Err(CommandError::bad_request(
                "Create schedules through agent triggers",
            ));
        }
        let agent = resolve_agent(ctx, &self.agent_id).await?;
        let (identity_id, owner) = ensure_identity_for_agent(&ctx.db, ctx.org_id(), &agent).await?;
        let mut config = self.req.channel_config;
        // A builder may configure transport credentials, but cannot select an
        // arbitrary managed app for deletion via forged install metadata.
        if self.req.channel_type == ChannelType::Slack {
            merge_preserved_secret_fields(ChannelType::Slack, &mut config, &json!({}));
        }
        let config = normalize_and_validate_channel_config(self.req.channel_type.clone(), config)?;
        let prepared = super::queries::prepare_channel_storage(ctx.encryption.as_ref(), &config)?;
        let channel_id = AgentChannelId::new();
        let row = ctx
            .db
            .create_agent_channel(
                ctx.org_id(),
                CreateAgentChannelRow {
                    agent_id: agent.id.uuid(),
                    public_id: channel_id.to_string(),
                    channel_type: self.req.channel_type.to_string(),
                    channel_config: prepared.channel_config,
                    channel_config_encrypted: prepared.channel_config_encrypted,
                    auth: prepared.auth,
                    auth_encrypted: prepared.auth_encrypted,
                    enabled: self.req.enabled,
                    status: if self.req.enabled {
                        "draft"
                    } else {
                        "disabled"
                    }
                    .to_string(),
                    virtual_user_id: Some(identity_id.uuid()),
                    owner_principal_id: owner.id.uuid(),
                    resolved_owner_user_id: owner.resolved_user_id,
                },
            )
            .await?;
        row_to_channel(ctx, row)
    }
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct UpdateAgentChannelCmd {
    pub agent_id: String,
    pub channel_id: String,
    #[serde(flatten)]
    pub req: UpdateAgentChannelRequest,
}

#[command(
    name = "update_agent_channel",
    category = "agent_channels",
    description = "Update an agent ingress channel.",
    method = "PATCH",
    path = "/v1/agents/{agent_id}/channels/{channel_id}",
    policy = AGENT_MANAGE,
)]
impl Command for UpdateAgentChannelCmd {
    type Output = AgentChannel;

    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        let agent = resolve_agent(ctx, &self.agent_id).await?;
        let mut existing = ctx
            .db
            .get_agent_channel(ctx.org_id(), agent.id.uuid(), &self.channel_id)
            .await?
            .ok_or_else(|| CommandError::not_found("Channel"))?;
        if existing.channel_type == "slack" {
            // Cleanup must not be followed by a stale settings write restoring
            // the deleted app's credentials. Hold through the command commit.
            let guard = ctx.db.lock_slack_install(existing.channel_id).await?;
            resolve_agent(ctx, &self.agent_id).await?;
            existing = ctx
                .db
                .get_agent_channel(ctx.org_id(), agent.id.uuid(), &self.channel_id)
                .await?
                .ok_or_else(|| CommandError::not_found("Channel"))?;
            super::slack_cleanup::release_after_commit(guard).await;
        }
        let channel_type = ChannelType::from_str_opt(&existing.channel_type)
            .ok_or_else(|| CommandError::bad_request("Channel has an unsupported channel type"))?;
        let (channel_config, channel_config_encrypted, auth, auth_encrypted) =
            if let Some(config) = self.req.channel_config {
                let config =
                    prepare_updated_config(ctx, &existing, channel_type, config, self.req.enabled)?;
                let prepared =
                    super::queries::prepare_channel_storage(ctx.encryption.as_ref(), &config)?;
                (
                    Some(prepared.channel_config),
                    UpdateField::from_option(prepared.channel_config_encrypted),
                    UpdateField::from_option(prepared.auth),
                    UpdateField::from_option(prepared.auth_encrypted),
                )
            } else {
                (
                    None,
                    UpdateField::Unchanged,
                    UpdateField::Unchanged,
                    UpdateField::Unchanged,
                )
            };
        super::exposure::require_live_change_permission(
            ctx,
            &existing.channel_status,
            false,
            self.req.enabled == Some(false),
        )?;
        let status = self.req.enabled.map(|enabled| {
            if enabled {
                if existing.channel_status == "disabled" {
                    "draft".to_string()
                } else {
                    existing.channel_status.clone()
                }
            } else {
                "disabled".to_string()
            }
        });
        let row = ctx
            .db
            .update_agent_channel(
                ctx.org_id(),
                agent.id.uuid(),
                &self.channel_id,
                UpdateAgentChannelRow {
                    channel_config,
                    channel_config_encrypted,
                    auth,
                    auth_encrypted,
                    enabled: self.req.enabled,
                    status,
                    ..Default::default()
                },
            )
            .await?
            .ok_or_else(|| CommandError::not_found("Channel"))?;
        if row.channel_type == "slack" {
            let service = crate::domains::health_issues::service::SlackHealthService::new(
                ctx.db.clone(),
                ctx.encryption.clone(),
            );
            let id = row.channel_public_id.clone();
            // It reads the changed row, so it starts once that commits.
            crate::storage::transaction::spawn_after_commit(async move {
                if let Err(error) = service.check(&id).await {
                    tracing::warn!(%error,"Could not reconcile changed Slack channel");
                }
            });
        }
        row_to_channel(ctx, row)
    }
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct PublishAgentChannel {
    pub agent_id: String,
    pub channel_id: String,
}

#[command(
    name = "publish_agent_channel",
    category = "agent_channels",
    description = "Publish an agent ingress channel.",
    method = "POST",
    path = "/v1/agents/{agent_id}/channels/{channel_id}/publish",
    policy = AGENT_DANGEROUS,
)]
impl Command for PublishAgentChannel {
    type Output = AgentChannel;

    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        set_channel_status(ctx, &self.agent_id, &self.channel_id, true, "live").await
    }
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct UnpublishAgentChannel {
    pub agent_id: String,
    pub channel_id: String,
}

#[command(
    name = "unpublish_agent_channel",
    category = "agent_channels",
    description = "Unpublish an agent ingress channel.",
    method = "POST",
    path = "/v1/agents/{agent_id}/channels/{channel_id}/unpublish",
    policy = AGENT_DANGEROUS,
)]
impl Command for UnpublishAgentChannel {
    type Output = AgentChannel;

    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        set_channel_status(ctx, &self.agent_id, &self.channel_id, true, "draft").await
    }
}

async fn set_channel_status(
    ctx: &Ctx,
    agent_id: &str,
    channel_id: &str,
    enabled: bool,
    status: &str,
) -> Result<AgentChannel, CommandError> {
    let agent = resolve_agent(ctx, agent_id).await?;
    let row = ctx
        .db
        .update_agent_channel(
            ctx.org_id(),
            agent.id.uuid(),
            channel_id,
            UpdateAgentChannelRow {
                enabled: Some(enabled),
                status: Some(status.to_string()),
                ..Default::default()
            },
        )
        .await?
        .ok_or_else(|| CommandError::not_found("Channel"))?;
    row_to_channel(ctx, row)
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct DeleteAgentChannel {
    pub agent_id: String,
    pub channel_id: String,
}

#[command(
    name = "delete_agent_channel",
    category = "agent_channels",
    description = "Delete an agent ingress channel.",
    method = "DELETE",
    path = "/v1/agents/{agent_id}/channels/{channel_id}",
    policy = AGENT_DANGEROUS,
)]
impl Command for DeleteAgentChannel {
    type Output = Value;

    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        let agent = resolve_agent(ctx, &self.agent_id).await?;
        super::slack_cleanup::remove_channel_app(ctx, agent.id.uuid(), &self.channel_id).await?;
        let deleted = ctx
            .db
            .delete_agent_channel(ctx.org_id(), agent.id.uuid(), &self.channel_id)
            .await?;
        if !deleted {
            return Err(CommandError::not_found("Channel"));
        }
        Ok(json!({ "deleted": true }))
    }
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct TriggerAgentChannel {
    pub agent_id: String,
    pub channel_id: String,
}

/// Result of running an Agent schedule channel immediately.
#[derive(Debug, serde::Serialize, ToSchema)]
pub struct TriggerAgentChannelOutput {
    /// Session started or reused by the invocation.
    pub session_id: everruns_contracts::typed_id::SessionId,
    /// Whether the invocation created a new session.
    pub created_session: bool,
}

#[command(
    name = "trigger_agent_channel",
    category = "agent_channels",
    description = "Run an agent schedule channel now.",
    method = "POST",
    path = "/v1/agents/{agent_id}/channels/{channel_id}/trigger",
    policy = AGENT_MANAGE,
)]
impl Command for TriggerAgentChannel {
    type Output = TriggerAgentChannelOutput;

    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        let agent = resolve_agent(ctx, &self.agent_id).await?;
        let row = ctx
            .db
            .get_agent_channel(ctx.org_id(), agent.id.uuid(), &self.channel_id)
            .await?
            .ok_or_else(|| CommandError::not_found("Channel"))?;
        let (context, channel) = row_to_ingress(ctx.encryption.as_ref(), row)?;
        if channel.channel_type != ChannelType::Schedule {
            return Err(CommandError::bad_request(
                "Only schedule channels can run now",
            ));
        }
        channel_liveness(&context, &channel)
            .map_err(|reason| CommandError::bad_request(reason.as_str()))?;
        let session_service = ctx.session_service.as_ref().ok_or_else(|| {
            CommandError::internal(anyhow::anyhow!("Session service not available"))
        })?;
        let message_service = ctx.message_service.as_ref().ok_or_else(|| {
            CommandError::internal(anyhow::anyhow!("Message service not available"))
        })?;
        let result = super::invoke_scheduled_agent_channel(
            &ctx.db,
            ctx.encryption.as_ref(),
            session_service,
            message_service,
            ctx.org_id(),
            &self.channel_id,
        )
        .await?;
        Ok(TriggerAgentChannelOutput {
            session_id: result.session_id,
            created_session: result.created_session,
        })
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

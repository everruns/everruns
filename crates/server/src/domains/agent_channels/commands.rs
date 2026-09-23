use super::types::{CreateAgentChannelRequest, UpdateAgentChannelRequest};
use super::validation::{merge_preserved_secret_fields, normalize_and_validate_channel_config};
use crate::api::app_ingress::{channel_liveness, row_to_ingress};
use crate::domains::agent_identities::lifecycle::ensure_identity_for_agent;
use crate::domains::agents::{AGENT_DANGEROUS, AGENT_MANAGE, AGENT_VIEW};
use crate::domains::apps::redact_channel_for_response;
use crate::domains::common::*;
use crate::storage::{CreateAgentChannelRow, IngressChannelRow, UpdateAgentChannelRow};
use everruns_durable::UpdateField;
use everruns_platform::{AppChannel, ChannelType};
use everruns_provider::typed_id::{AgentId, AppChannelId};
use serde::Deserialize;
use serde_json::{Value, json};
use utoipa::ToSchema;

async fn resolve_agent(
    ctx: &Ctx,
    id_or_name: &str,
) -> Result<crate::storage::models::AgentRow, CommandError> {
    let row = if let Ok(agent_id) = id_or_name.parse::<AgentId>() {
        ctx.db
            .get_agent_by_public_id(ctx.org_id(), &agent_id.to_string())
            .await
    } else {
        ctx.db.get_agent_by_name(ctx.org_id(), id_or_name).await
    }
    .map_err(classify_anyhow)?
    .ok_or_else(|| CommandError::not_found("Agent"))?;
    if row.status != "active" {
        return Err(CommandError::bad_request(
            "Archived or deleted agents cannot manage channels",
        ));
    }
    Ok(row)
}

fn row_to_channel(ctx: &Ctx, row: IngressChannelRow) -> Result<AppChannel, CommandError> {
    let (_, channel) = row_to_ingress(ctx.encryption.as_ref(), row).map_err(classify_anyhow)?;
    Ok(redact_channel_for_response(channel.into_channel()))
}

fn decrypted_config(ctx: &Ctx, row: IngressChannelRow) -> Result<Value, CommandError> {
    let (_, channel) = row_to_ingress(ctx.encryption.as_ref(), row).map_err(classify_anyhow)?;
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

#[derive(Debug, Deserialize, ToSchema)]
pub struct ListAgentChannels {
    pub agent_id: String,
}

impl Command for ListAgentChannels {
    type Output = Vec<AppChannel>;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "list_agent_channels",
            category: "agent_channels",
            description: "List an agent's ingress channels.",
            method: "GET",
            path: "/v1/agents/{agent_id}/channels",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&AGENT_VIEW)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        let agent = resolve_agent(ctx, &self.agent_id).await?;
        ctx.db
            .list_agent_channels(ctx.org_id(), agent.id.uuid())
            .await
            .map_err(classify_anyhow)?
            .into_iter()
            .map(|row| row_to_channel(ctx, row))
            .collect()
    }
}

inventory::submit! { CommandDescriptor::of::<ListAgentChannels>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct GetAgentChannel {
    pub agent_id: String,
    pub channel_id: String,
}

impl Command for GetAgentChannel {
    type Output = AppChannel;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "get_agent_channel",
            category: "agent_channels",
            description: "Get an agent ingress channel.",
            method: "GET",
            path: "/v1/agents/{agent_id}/channels/{channel_id}",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&AGENT_VIEW)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        let agent = resolve_agent(ctx, &self.agent_id).await?;
        let row = ctx
            .db
            .get_agent_channel(ctx.org_id(), agent.id.uuid(), &self.channel_id)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Channel"))?;
        row_to_channel(ctx, row)
    }
}

inventory::submit! { CommandDescriptor::of::<GetAgentChannel>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateAgentChannel {
    pub agent_id: String,
    #[serde(flatten)]
    pub req: CreateAgentChannelRequest,
}

impl Command for CreateAgentChannel {
    type Output = AppChannel;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "create_agent_channel",
            category: "agent_channels",
            description: "Create an ingress channel for an agent.",
            method: "POST",
            path: "/v1/agents/{agent_id}/channels",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&AGENT_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        if self.req.channel_type == ChannelType::PublicChat && !ctx.feature_flags.public_chat {
            return Err(CommandError::feature_not_enabled("public_chat"));
        }
        if self.req.channel_type == ChannelType::Schedule {
            return Err(CommandError::bad_request(
                "Create schedules through agent triggers",
            ));
        }
        let agent = resolve_agent(ctx, &self.agent_id).await?;
        let (identity_id, owner) = ensure_identity_for_agent(&ctx.db, ctx.org_id(), &agent)
            .await
            .map_err(classify_anyhow)?;
        let config = normalize_and_validate_channel_config(
            self.req.channel_type.clone(),
            self.req.channel_config,
        )?;
        let prepared = crate::domains::apps::queries::prepare_channel_storage(
            ctx.encryption.as_ref(),
            &config,
        )
        .map_err(classify_anyhow)?;
        let channel_id = AppChannelId::new();
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
                    agent_identity_id: Some(identity_id.uuid()),
                    agent_version_policy: "default".to_string(),
                    agent_version_id: None,
                    owner_principal_id: owner.id.uuid(),
                    resolved_owner_user_id: owner.resolved_user_id,
                },
            )
            .await
            .map_err(classify_anyhow)?;
        row_to_channel(ctx, row)
    }
}

inventory::submit! { CommandDescriptor::of::<CreateAgentChannel>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateAgentChannelCmd {
    pub agent_id: String,
    pub channel_id: String,
    #[serde(flatten)]
    pub req: UpdateAgentChannelRequest,
}

impl Command for UpdateAgentChannelCmd {
    type Output = AppChannel;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "update_agent_channel",
            category: "agent_channels",
            description: "Update an agent ingress channel.",
            method: "PATCH",
            path: "/v1/agents/{agent_id}/channels/{channel_id}",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&AGENT_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        let agent = resolve_agent(ctx, &self.agent_id).await?;
        let existing = ctx
            .db
            .get_agent_channel(ctx.org_id(), agent.id.uuid(), &self.channel_id)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Channel"))?;
        let channel_type = ChannelType::from_str_opt(&existing.channel_type)
            .ok_or_else(|| CommandError::bad_request("Channel has an unsupported channel type"))?;
        let (channel_config, channel_config_encrypted, auth, auth_encrypted) =
            if let Some(mut config) = self.req.channel_config {
                let current = decrypted_config(ctx, existing.clone())?;
                merge_preserved_secret_fields(channel_type.clone(), &mut config, &current);
                let config = normalize_and_validate_channel_config(channel_type.clone(), config)?;
                let prepared = crate::domains::apps::queries::prepare_channel_storage(
                    ctx.encryption.as_ref(),
                    &config,
                )
                .map_err(classify_anyhow)?;
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
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Channel"))?;
        row_to_channel(ctx, row)
    }
}

inventory::submit! { CommandDescriptor::of::<UpdateAgentChannelCmd>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct PublishAgentChannel {
    pub agent_id: String,
    pub channel_id: String,
}

impl Command for PublishAgentChannel {
    type Output = AppChannel;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "publish_agent_channel",
            category: "agent_channels",
            description: "Publish an agent ingress channel.",
            method: "POST",
            path: "/v1/agents/{agent_id}/channels/{channel_id}/publish",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&AGENT_DANGEROUS)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        set_channel_status(ctx, &self.agent_id, &self.channel_id, true, "live").await
    }
}

inventory::submit! { CommandDescriptor::of::<PublishAgentChannel>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct UnpublishAgentChannel {
    pub agent_id: String,
    pub channel_id: String,
}

impl Command for UnpublishAgentChannel {
    type Output = AppChannel;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "unpublish_agent_channel",
            category: "agent_channels",
            description: "Unpublish an agent ingress channel.",
            method: "POST",
            path: "/v1/agents/{agent_id}/channels/{channel_id}/unpublish",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&AGENT_DANGEROUS)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        set_channel_status(ctx, &self.agent_id, &self.channel_id, true, "draft").await
    }
}

inventory::submit! { CommandDescriptor::of::<UnpublishAgentChannel>() }

async fn set_channel_status(
    ctx: &Ctx,
    agent_id: &str,
    channel_id: &str,
    enabled: bool,
    status: &str,
) -> Result<AppChannel, CommandError> {
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
        .await
        .map_err(classify_anyhow)?
        .ok_or_else(|| CommandError::not_found("Channel"))?;
    row_to_channel(ctx, row)
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct DeleteAgentChannel {
    pub agent_id: String,
    pub channel_id: String,
}

impl Command for DeleteAgentChannel {
    type Output = Value;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "delete_agent_channel",
            category: "agent_channels",
            description: "Delete an agent ingress channel.",
            method: "DELETE",
            path: "/v1/agents/{agent_id}/channels/{channel_id}",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&AGENT_DANGEROUS)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        let agent = resolve_agent(ctx, &self.agent_id).await?;
        let deleted = ctx
            .db
            .delete_agent_channel(ctx.org_id(), agent.id.uuid(), &self.channel_id)
            .await
            .map_err(classify_anyhow)?;
        if !deleted {
            return Err(CommandError::not_found("Channel"));
        }
        Ok(json!({ "deleted": true }))
    }
}

inventory::submit! { CommandDescriptor::of::<DeleteAgentChannel>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct TriggerAgentChannel {
    pub agent_id: String,
    pub channel_id: String,
}

/// Result of running an Agent schedule channel immediately.
#[derive(Debug, serde::Serialize, ToSchema)]
pub struct TriggerAgentChannelOutput {
    /// Session started or reused by the invocation.
    pub session_id: everruns_provider::typed_id::SessionId,
    /// Whether the invocation created a new session.
    pub created_session: bool,
}

impl Command for TriggerAgentChannel {
    type Output = TriggerAgentChannelOutput;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "trigger_agent_channel",
            category: "agent_channels",
            description: "Run an agent schedule channel now.",
            method: "POST",
            path: "/v1/agents/{agent_id}/channels/{channel_id}/trigger",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&AGENT_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        let agent = resolve_agent(ctx, &self.agent_id).await?;
        let row = ctx
            .db
            .get_agent_channel(ctx.org_id(), agent.id.uuid(), &self.channel_id)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Channel"))?;
        let (context, channel) =
            row_to_ingress(ctx.encryption.as_ref(), row).map_err(classify_anyhow)?;
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
        let result = crate::domains::apps::invoke_scheduled_agent_channel(
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

inventory::submit! { CommandDescriptor::of::<TriggerAgentChannel>() }

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

use super::types::{CreateAgentEndpointRequest, UpdateAgentEndpointRequest};
use super::validation::{merge_preserved_secret_fields, normalize_and_validate_channel_config};
use crate::api::app_ingress::{endpoint_liveness, row_to_ingress};
use crate::domains::agent_identities::lifecycle::ensure_identity_for_agent;
use crate::domains::agents::{AGENT_DANGEROUS, AGENT_MANAGE, AGENT_VIEW};
use crate::domains::apps::redact_channel_for_response;
use crate::domains::common::*;
use crate::storage::{CreateAgentEndpointRow, IngressEndpointRow, UpdateAgentEndpointRow};
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
            "Archived or deleted agents cannot manage endpoints",
        ));
    }
    Ok(row)
}

fn row_to_endpoint(ctx: &Ctx, row: IngressEndpointRow) -> Result<AppChannel, CommandError> {
    let (_, endpoint) = row_to_ingress(ctx.encryption.as_ref(), row).map_err(classify_anyhow)?;
    Ok(redact_channel_for_response(endpoint.into_channel()))
}

fn decrypted_config(ctx: &Ctx, row: IngressEndpointRow) -> Result<Value, CommandError> {
    let (_, endpoint) = row_to_ingress(ctx.encryption.as_ref(), row).map_err(classify_anyhow)?;
    let mut config = endpoint.channel_config;
    if let Some(auth) = endpoint.auth
        && let Some(object) = config.as_object_mut()
    {
        object.insert(
            "auth".to_string(),
            serde_json::to_value(auth).map_err(|error| {
                CommandError::bad_request(format!("Invalid endpoint auth configuration: {error}"))
            })?,
        );
    }
    Ok(config)
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ListAgentEndpoints {
    pub agent_id: String,
}

impl Command for ListAgentEndpoints {
    type Output = Vec<AppChannel>;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "list_agent_endpoints",
            category: "agent_endpoints",
            description: "List an agent's ingress endpoints.",
            method: "GET",
            path: "/v1/agents/{agent_id}/endpoints",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&AGENT_VIEW)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        let agent = resolve_agent(ctx, &self.agent_id).await?;
        ctx.db
            .list_agent_endpoints(ctx.org_id(), agent.id.uuid())
            .await
            .map_err(classify_anyhow)?
            .into_iter()
            .map(|row| row_to_endpoint(ctx, row))
            .collect()
    }
}

inventory::submit! { CommandDescriptor::of::<ListAgentEndpoints>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct GetAgentEndpoint {
    pub agent_id: String,
    pub endpoint_id: String,
}

impl Command for GetAgentEndpoint {
    type Output = AppChannel;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "get_agent_endpoint",
            category: "agent_endpoints",
            description: "Get an agent ingress endpoint.",
            method: "GET",
            path: "/v1/agents/{agent_id}/endpoints/{endpoint_id}",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&AGENT_VIEW)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        let agent = resolve_agent(ctx, &self.agent_id).await?;
        let row = ctx
            .db
            .get_agent_endpoint(ctx.org_id(), agent.id.uuid(), &self.endpoint_id)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Endpoint"))?;
        row_to_endpoint(ctx, row)
    }
}

inventory::submit! { CommandDescriptor::of::<GetAgentEndpoint>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateAgentEndpoint {
    pub agent_id: String,
    #[serde(flatten)]
    pub req: CreateAgentEndpointRequest,
}

impl Command for CreateAgentEndpoint {
    type Output = AppChannel;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "create_agent_endpoint",
            category: "agent_endpoints",
            description: "Create an ingress endpoint for an agent.",
            method: "POST",
            path: "/v1/agents/{agent_id}/endpoints",
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
        let endpoint_id = AppChannelId::new();
        let row = ctx
            .db
            .create_agent_endpoint(
                ctx.org_id(),
                CreateAgentEndpointRow {
                    agent_id: agent.id.uuid(),
                    public_id: endpoint_id.to_string(),
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
        row_to_endpoint(ctx, row)
    }
}

inventory::submit! { CommandDescriptor::of::<CreateAgentEndpoint>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateAgentEndpointCmd {
    pub agent_id: String,
    pub endpoint_id: String,
    #[serde(flatten)]
    pub req: UpdateAgentEndpointRequest,
}

impl Command for UpdateAgentEndpointCmd {
    type Output = AppChannel;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "update_agent_endpoint",
            category: "agent_endpoints",
            description: "Update an agent ingress endpoint.",
            method: "PATCH",
            path: "/v1/agents/{agent_id}/endpoints/{endpoint_id}",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&AGENT_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        let agent = resolve_agent(ctx, &self.agent_id).await?;
        let existing = ctx
            .db
            .get_agent_endpoint(ctx.org_id(), agent.id.uuid(), &self.endpoint_id)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Endpoint"))?;
        let channel_type = ChannelType::from_str_opt(&existing.channel_type)
            .ok_or_else(|| CommandError::bad_request("Endpoint has an unsupported channel type"))?;
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
                if existing.endpoint_status == "disabled" {
                    "draft".to_string()
                } else {
                    existing.endpoint_status.clone()
                }
            } else {
                "disabled".to_string()
            }
        });
        let row = ctx
            .db
            .update_agent_endpoint(
                ctx.org_id(),
                agent.id.uuid(),
                &self.endpoint_id,
                UpdateAgentEndpointRow {
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
            .ok_or_else(|| CommandError::not_found("Endpoint"))?;
        row_to_endpoint(ctx, row)
    }
}

inventory::submit! { CommandDescriptor::of::<UpdateAgentEndpointCmd>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct PublishAgentEndpoint {
    pub agent_id: String,
    pub endpoint_id: String,
}

impl Command for PublishAgentEndpoint {
    type Output = AppChannel;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "publish_agent_endpoint",
            category: "agent_endpoints",
            description: "Publish an agent ingress endpoint.",
            method: "POST",
            path: "/v1/agents/{agent_id}/endpoints/{endpoint_id}/publish",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&AGENT_DANGEROUS)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        set_endpoint_status(ctx, &self.agent_id, &self.endpoint_id, true, "live").await
    }
}

inventory::submit! { CommandDescriptor::of::<PublishAgentEndpoint>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct UnpublishAgentEndpoint {
    pub agent_id: String,
    pub endpoint_id: String,
}

impl Command for UnpublishAgentEndpoint {
    type Output = AppChannel;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "unpublish_agent_endpoint",
            category: "agent_endpoints",
            description: "Unpublish an agent ingress endpoint.",
            method: "POST",
            path: "/v1/agents/{agent_id}/endpoints/{endpoint_id}/unpublish",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&AGENT_DANGEROUS)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        set_endpoint_status(ctx, &self.agent_id, &self.endpoint_id, true, "draft").await
    }
}

inventory::submit! { CommandDescriptor::of::<UnpublishAgentEndpoint>() }

async fn set_endpoint_status(
    ctx: &Ctx,
    agent_id: &str,
    endpoint_id: &str,
    enabled: bool,
    status: &str,
) -> Result<AppChannel, CommandError> {
    let agent = resolve_agent(ctx, agent_id).await?;
    let row = ctx
        .db
        .update_agent_endpoint(
            ctx.org_id(),
            agent.id.uuid(),
            endpoint_id,
            UpdateAgentEndpointRow {
                enabled: Some(enabled),
                status: Some(status.to_string()),
                ..Default::default()
            },
        )
        .await
        .map_err(classify_anyhow)?
        .ok_or_else(|| CommandError::not_found("Endpoint"))?;
    row_to_endpoint(ctx, row)
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct DeleteAgentEndpoint {
    pub agent_id: String,
    pub endpoint_id: String,
}

impl Command for DeleteAgentEndpoint {
    type Output = Value;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "delete_agent_endpoint",
            category: "agent_endpoints",
            description: "Delete an agent ingress endpoint.",
            method: "DELETE",
            path: "/v1/agents/{agent_id}/endpoints/{endpoint_id}",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&AGENT_DANGEROUS)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        let agent = resolve_agent(ctx, &self.agent_id).await?;
        let deleted = ctx
            .db
            .delete_agent_endpoint(ctx.org_id(), agent.id.uuid(), &self.endpoint_id)
            .await
            .map_err(classify_anyhow)?;
        if !deleted {
            return Err(CommandError::not_found("Endpoint"));
        }
        Ok(json!({ "deleted": true }))
    }
}

inventory::submit! { CommandDescriptor::of::<DeleteAgentEndpoint>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct TriggerAgentEndpoint {
    pub agent_id: String,
    pub endpoint_id: String,
}

/// Result of running an Agent schedule endpoint immediately.
#[derive(Debug, serde::Serialize, ToSchema)]
pub struct TriggerAgentEndpointOutput {
    /// Session started or reused by the invocation.
    pub session_id: everruns_provider::typed_id::SessionId,
    /// Whether the invocation created a new session.
    pub created_session: bool,
}

impl Command for TriggerAgentEndpoint {
    type Output = TriggerAgentEndpointOutput;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "trigger_agent_endpoint",
            category: "agent_endpoints",
            description: "Run an agent schedule endpoint now.",
            method: "POST",
            path: "/v1/agents/{agent_id}/endpoints/{endpoint_id}/trigger",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&AGENT_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        let agent = resolve_agent(ctx, &self.agent_id).await?;
        let row = ctx
            .db
            .get_agent_endpoint(ctx.org_id(), agent.id.uuid(), &self.endpoint_id)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Endpoint"))?;
        let (context, endpoint) =
            row_to_ingress(ctx.encryption.as_ref(), row).map_err(classify_anyhow)?;
        if endpoint.channel_type != ChannelType::Schedule {
            return Err(CommandError::bad_request(
                "Only schedule endpoints can run now",
            ));
        }
        endpoint_liveness(&context, &endpoint)
            .map_err(|reason| CommandError::bad_request(reason.as_str()))?;
        let session_service = ctx.session_service.as_ref().ok_or_else(|| {
            CommandError::internal(anyhow::anyhow!("Session service not available"))
        })?;
        let message_service = ctx.message_service.as_ref().ok_or_else(|| {
            CommandError::internal(anyhow::anyhow!("Message service not available"))
        })?;
        let result = crate::domains::apps::invoke_scheduled_agent_endpoint(
            &ctx.db,
            ctx.encryption.as_ref(),
            session_service,
            message_service,
            ctx.org_id(),
            &self.endpoint_id,
        )
        .await?;
        Ok(TriggerAgentEndpointOutput {
            session_id: result.session_id,
            created_session: result.created_session,
        })
    }
}

inventory::submit! { CommandDescriptor::of::<TriggerAgentEndpoint>() }

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

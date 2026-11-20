//! A revisioned view definition owned by an Agent. The renderer never gets management credentials.

use super::{AGENT_MANAGE, AGENT_VIEW};
use crate::domains::common::*;
use everruns_contracts::typed_id::AgentId;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

const MAX_SOURCE_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(tag = "format", rename_all = "snake_case", deny_unknown_fields)]
pub enum AgentViewLayout {
    /// A2UI 1.0 operations for one surface using the standard catalog.
    A2ui { operations: Vec<Value> },
    /// Static markup rendered in a sandbox without scripts or network access.
    Html { source: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentViewDefinition {
    pub title: String,
    pub layout: AgentViewLayout,
    /// Whether viewers may use a personal layout for this view.
    #[serde(default)]
    pub allow_personalization: bool,
    /// Top-level keys that an agent is allowed to publish for this layout.
    #[serde(default)]
    pub data_fields: Vec<String>,
    /// Whether a successful run may also publish a replacement layout.
    #[serde(default)]
    pub allow_agent_layout: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AgentViewSettings {
    /// Revision zero represents an Agent with no saved view settings.
    pub revision: i64,
    pub view: Option<AgentViewDefinition>,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ReplaceAgentViewRequest {
    pub revision: i64,
    /// Pass null to remove the view.
    #[serde(deserialize_with = "Deserialize::deserialize")]
    #[schema(required = true)]
    pub view: Option<AgentViewDefinition>,
}

fn validate_view(view: &mut AgentViewDefinition) -> Result<(), CommandError> {
    view.title = view.title.trim().to_owned();
    if view.title.is_empty() || view.title.chars().count() > 120 {
        return Err(CommandError::bad_request(
            "Enter a view title of 1 to 120 characters.",
        ));
    }
    if view.data_fields.len() > 32
        || view.data_fields.iter().any(|field| {
            field.is_empty()
                || field.len() > 64
                || !field
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_')
        })
    {
        return Err(CommandError::bad_request(
            "Use up to 32 data field names containing only letters, numbers and underscores.",
        ));
    }
    view.data_fields.sort_unstable();
    view.data_fields.dedup();
    validate_layout(&view.layout)
}

pub(crate) fn validate_layout(layout: &AgentViewLayout) -> Result<(), CommandError> {
    match layout {
        AgentViewLayout::Html { source } => {
            if source.trim().is_empty() || source.len() > MAX_SOURCE_BYTES {
                return Err(CommandError::bad_request("Enter HTML markup up to 64 KiB."));
            }
        }
        AgentViewLayout::A2ui { operations } => {
            if serde_json::to_vec(operations)
                .map_err(anyhow::Error::from)?
                .len()
                > MAX_SOURCE_BYTES
            {
                return Err(CommandError::bad_request(
                    "A2UI operations must fit within 64 KiB.",
                ));
            }
            validate_a2ui(operations)?;
        }
    }
    Ok(())
}

pub(crate) async fn describe_for_execution(
    db: &crate::storage::StorageBackend,
    org_id: i64,
    agent: &mut super::record::Agent,
) -> anyhow::Result<()> {
    let flags = crate::services::org_feature_flags::resolve_org_feature_flags(
        db,
        org_id,
        &crate::records::FeatureFlagPolicy::current(),
    )
    .await?;
    if !flags.agent_views {
        return Ok(());
    }
    let Some((_, Some(view))) = db.get_agent_view_settings(org_id, agent.public_id).await? else {
        return Ok(());
    };
    let view: AgentViewDefinition = serde_json::from_value(view)?;
    let format = match view.layout {
        AgentViewLayout::A2ui { .. } => "a2ui",
        AgentViewLayout::Html { .. } => "html",
    };
    use std::fmt::Write;
    write!(
        agent.system_prompt,
        "\n\nPublished view configuration: default format {}; authorized top-level data fields {}; replacement layouts {}. Use publish_view with these fields.",
        format,
        serde_json::to_string(&view.data_fields)?,
        if view.allow_agent_layout {
            "allowed"
        } else {
            "disabled"
        },
    )?;
    Ok(())
}

fn validate_a2ui(operations: &[Value]) -> Result<(), CommandError> {
    let invalid = || {
        CommandError::bad_request(
            "Use A2UI 1.0 operations for one surface. Begin with createSurface and use the same surfaceId for every update.",
        )
    };
    let surface = operations
        .first()
        .and_then(|operation| operation.pointer("/createSurface/surfaceId"))
        .and_then(Value::as_str)
        .filter(|id| !id.trim().is_empty())
        .ok_or_else(invalid)?;
    let mut has_components = false;
    for (index, operation) in operations.iter().enumerate() {
        let object = operation.as_object().ok_or_else(invalid)?;
        if object.len() != 2 || object.get("version").and_then(Value::as_str) != Some("1.0") {
            return Err(invalid());
        }
        let body = if index == 0 {
            let create = object.get("createSurface").ok_or_else(invalid)?;
            if create.get("catalogId").and_then(Value::as_str) != Some("copilotkit/standard@1") {
                return Err(CommandError::bad_request(
                    "Use the copilotkit/standard@1 catalog.",
                ));
            }
            create
        } else if let Some(update) = object.get("updateComponents") {
            update
        } else {
            object.get("updateDataModel").ok_or_else(invalid)?
        };
        if body.get("surfaceId").and_then(Value::as_str) != Some(surface) {
            return Err(invalid());
        }
        if let Some(components) = body.get("components") {
            let components = components.as_array().ok_or_else(invalid)?;
            has_components |= components.iter().any(|component| {
                component.get("id").and_then(Value::as_str) == Some("root")
                    && component.get("component").is_some_and(Value::is_string)
            });
        }
    }
    if !has_components {
        return Err(CommandError::bad_request(
            "The A2UI layout needs a component with id root.",
        ));
    }
    Ok(())
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct GetAgentViewSettings {
    pub agent_id: AgentId,
}

#[command(
    name = "get_agent_view_settings",
    category = "agent_views",
    description = "Read the view definition and its revision for an Agent.",
    method = "GET",
    path = "/v1/agents/{agent_id}/view",
    policy = AGENT_VIEW,
    http = plain,
)]
impl Command for GetAgentViewSettings {
    type Output = AgentViewSettings;

    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        let (revision, view) = ctx
            .db
            .get_agent_view_settings(ctx.org_id(), self.agent_id)
            .await?
            .ok_or_else(|| CommandError::not_found("Agent"))?;
        Ok(AgentViewSettings {
            revision,
            view: view
                .map(serde_json::from_value)
                .transpose()
                .map_err(anyhow::Error::from)?,
        })
    }
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct ReplaceAgentView {
    pub agent_id: AgentId,
    #[serde(flatten)]
    pub request: ReplaceAgentViewRequest,
}

#[command(
    name = "replace_agent_view",
    category = "agent_views",
    description = "Replace or remove an Agent view using its current revision.",
    method = "PUT",
    path = "/v1/agents/{agent_id}/view",
    policy = AGENT_MANAGE,
    http = plain,
    request_body(ReplaceAgentViewRequest),
)]
impl Command for ReplaceAgentView {
    type Output = AgentViewSettings;

    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        let ReplaceAgentViewRequest { revision, mut view } = self.request;
        let agent = ctx
            .db
            .get_agent_by_public_id(ctx.org_id(), &self.agent_id.to_string())
            .await?
            .filter(|agent| agent.status != "deleted" && !agent.is_built_in)
            .ok_or_else(|| CommandError::not_found("Agent"))?;
        if agent.status != "active" {
            return Err(CommandError::bad_request(
                "Only active Agents can change their view.",
            ));
        }
        if let Some(view) = &mut view {
            validate_view(view)?;
        }
        let revision = ctx
            .db
            .replace_agent_view_settings(
                ctx.org_id(),
                agent.id,
                revision,
                view.as_ref()
                    .map(serde_json::to_value)
                    .transpose()
                    .map_err(anyhow::Error::from)?,
            )
            .await?
            .ok_or_else(|| CommandError::conflict("The view changed. Reload it before saving."))?;
        if let Err(error) = crate::domains::data_plane::event_tap::sync(&ctx.db, ctx.org_id()).await
        {
            tracing::warn!(%error, org_id = ctx.org_id(), "Could not refresh the Agent view event tap");
        }
        if let Err(error) = ctx
            .db
            .enqueue_engine_agent_publish(ctx.org_id(), Some(agent.id.uuid()), None, "agent_view")
            .await
        {
            tracing::warn!(%error, agent_id = %agent.id, "Could not refresh the Agent view descriptor");
        }
        Ok(AgentViewSettings { revision, view })
    }
}

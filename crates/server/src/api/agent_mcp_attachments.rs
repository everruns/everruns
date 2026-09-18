use crate::auth::ResolvedOrg;
use crate::domains::common::Command;
use crate::domains::mcp_servers::MCP_SERVER_MANAGE;
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::get,
};
use everruns_core::{Caller, McpServerActsAs, ScopedMcpServer, ScopedMcpServers};
use everruns_provider::typed_id::AgentId;
use serde::Serialize;
use std::collections::BTreeMap;
use utoipa::ToSchema;

use super::agents::AppState;
use super::common::{ApiResult, ApiResultExt, ErrorResponse};

pub fn routes(state: AppState) -> Router {
    Router::new()
        .route(
            "/v1/agents/{agent_id}/mcp-attachments",
            get(list_agent_mcp_attachments),
        )
        .route(
            "/v1/agents/{agent_id}/mcp-attachments/{name}/connection",
            axum::routing::delete(revoke_agent_mcp_connection),
        )
        .with_state(state)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AgentMcpAttachmentSource {
    Capability,
    Harness,
    Agent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct AgentMcpAttachmentSourceInfo {
    pub source: AgentMcpAttachmentSource,
    pub source_label: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AgentMcpAttachmentState {
    Ready,
    ConnectionMissing,
    PresetMissing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AgentMcpAttachmentAction {
    None,
    Connect,
    Authorize,
    AskAdmin,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AgentMcpAttachment {
    pub name: String,
    pub source: AgentMcpAttachmentSource,
    pub source_label: String,
    pub overridden_sources: Vec<AgentMcpAttachmentSourceInfo>,
    pub acts_as: McpServerActsAs,
    pub preset_name: Option<String>,
    pub preset_id: Option<String>,
    pub connection_provider: Option<String>,
    pub url: Option<String>,
    pub header_names: Vec<String>,
    pub tools_available: bool,
    pub tools: Vec<String>,
    pub state: AgentMcpAttachmentState,
    pub action: AgentMcpAttachmentAction,
    pub connected_as: Option<String>,
    pub editable: bool,
}

#[derive(Debug, Clone)]
struct SourcedMcpAttachment {
    server: ScopedMcpServer,
    source: AgentMcpAttachmentSource,
    source_label: String,
    overridden_sources: Vec<AgentMcpAttachmentSourceInfo>,
}

fn merge_sourced_mcp_layer(
    effective: &mut BTreeMap<String, SourcedMcpAttachment>,
    layer: &ScopedMcpServers,
    source: AgentMcpAttachmentSource,
    source_label: String,
) {
    for (name, server) in layer {
        let overridden_sources = effective
            .remove(name)
            .map(|previous| {
                let mut sources = previous.overridden_sources;
                sources.push(AgentMcpAttachmentSourceInfo {
                    source: previous.source,
                    source_label: previous.source_label,
                });
                sources
            })
            .unwrap_or_default();
        effective.insert(
            name.clone(),
            SourcedMcpAttachment {
                server: server.clone(),
                source,
                source_label: source_label.clone(),
                overridden_sources,
            },
        );
    }
}

#[utoipa::path(
    get,
    path = "/v1/agents/{agent_id}/mcp-attachments",
    params(("agent_id" = String, Path, description = "Agent ID (prefixed) or name")),
    responses(
        (status = 200, description = "Effective MCP attachments for the agent", body = Vec<AgentMcpAttachment>),
        (status = 404, description = "Agent or harness not found", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse),
    ),
    tag = "agents"
)]
pub async fn list_agent_mcp_attachments(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(agent_id_or_name): Path<String>,
) -> ApiResult<Vec<AgentMcpAttachment>> {
    let agent = crate::domains::agents::GetAgent {
        id: agent_id_or_name,
    }
    .run(&state.ctx(&org))
    .await?;
    let row = state
        .db
        .get_agent(org.org_id, AgentId::from_uuid(agent.internal_id))
        .await
        .log_internal_error_json("load agent MCP attachment identity")?
        .ok_or_else(|| ErrorResponse::not_found("Agent"))?;
    let harness = crate::domains::harnesses::queries::resolve_effective(
        &state.db,
        org.org_id,
        agent.harness_id,
    )
    .await
    .log_internal_error_json("load effective agent harness")?
    .ok_or_else(|| ErrorResponse::not_found("Harness"))?;

    let effective_capabilities =
        everruns_core::merge_capabilities(&harness.capabilities, &agent.capabilities);
    let capability_servers = everruns_core::capabilities::collect_capability_mcp_servers(
        &effective_capabilities,
        state.host_composition.capability_registry().as_ref(),
    );
    let mut effective = BTreeMap::new();
    merge_sourced_mcp_layer(
        &mut effective,
        &capability_servers,
        AgentMcpAttachmentSource::Capability,
        "Capability".to_string(),
    );
    merge_sourced_mcp_layer(
        &mut effective,
        &harness.mcp_servers,
        AgentMcpAttachmentSource::Harness,
        harness
            .display_name
            .clone()
            .unwrap_or_else(|| harness.name.clone()),
    );
    merge_sourced_mcp_layer(
        &mut effective,
        &agent.mcp_servers,
        AgentMcpAttachmentSource::Agent,
        "Agent".to_string(),
    );

    let caller = Caller::from(&org);
    let can_authorize_service = MCP_SERVER_MANAGE
        .evaluate_with(state.auth.permission_resolver.as_ref(), &caller)
        .is_ok();
    let mut attachments = Vec::with_capacity(effective.len());
    for (name, sourced) in effective {
        let preset_name = sourced
            .server
            .preset
            .as_ref()
            .map(|preset| preset.catalog_name().to_string());
        let preset_row = match preset_name.as_deref() {
            Some(name) => state
                .db
                .get_mcp_server_by_name(org.org_id, name)
                .await
                .log_internal_error_json("load MCP attachment preset")?
                .filter(|row| row.status == "active"),
            None => None,
        };
        let preset_missing = preset_name.is_some() && preset_row.is_none();
        let provider = preset_row
            .as_ref()
            .map(|row| everruns_core::mcp_oauth_provider_id_for_uuid(row.id.uuid()));
        let connection = match (sourced.server.acts_as, provider.as_deref()) {
            (McpServerActsAs::User, Some(provider)) => match org.user_id {
                Some(user_id) => state
                    .db
                    .get_user_connection(user_id, provider)
                    .await
                    .log_internal_error_json("load user MCP connection")?
                    .map(|row| row.provider_username),
                None => None,
            },
            (McpServerActsAs::Service, Some(provider)) => match row.agent_identity_id {
                Some(identity_id) => state
                    .db
                    .get_agent_identity_connection(identity_id, provider)
                    .await
                    .log_internal_error_json("load service MCP connection")?
                    .map(|row| row.provider_username),
                None => None,
            },
            _ => None,
        };
        let has_connection = connection.is_some();
        let connected_as = connection
            .flatten()
            .or_else(|| has_connection.then(|| preset_name.clone()).flatten());
        let needs_connection = !sourced.server.acts_as.is_none();
        let state_value = if preset_missing {
            AgentMcpAttachmentState::PresetMissing
        } else if needs_connection && !has_connection {
            AgentMcpAttachmentState::ConnectionMissing
        } else {
            AgentMcpAttachmentState::Ready
        };
        let action = if preset_missing || !needs_connection || has_connection {
            AgentMcpAttachmentAction::None
        } else {
            match sourced.server.acts_as {
                McpServerActsAs::User => AgentMcpAttachmentAction::Connect,
                McpServerActsAs::Service if can_authorize_service => {
                    AgentMcpAttachmentAction::Authorize
                }
                McpServerActsAs::Service => AgentMcpAttachmentAction::AskAdmin,
                McpServerActsAs::None => AgentMcpAttachmentAction::None,
            }
        };
        let tools = preset_row
            .as_ref()
            .and_then(|row| row.cached_tools.as_array())
            .map(|tools| {
                tools
                    .iter()
                    .filter_map(|tool| tool.get("name").and_then(|name| name.as_str()))
                    .map(ToOwned::to_owned)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let header_names = if let Some(row) = preset_row.as_ref() {
            row.headers
                .as_object()
                .map(|headers| headers.keys().cloned().collect())
                .unwrap_or_default()
        } else {
            sourced.server.headers.keys().cloned().collect()
        };

        attachments.push(AgentMcpAttachment {
            name,
            source: sourced.source,
            source_label: sourced.source_label,
            overridden_sources: sourced.overridden_sources,
            acts_as: sourced.server.acts_as,
            preset_name,
            preset_id: preset_row.as_ref().map(|row| row.id.to_string()),
            connection_provider: provider,
            url: preset_row
                .as_ref()
                .map(|row| row.url.clone())
                .or_else(|| (!sourced.server.url.is_empty()).then(|| sourced.server.url.clone())),
            header_names,
            tools_available: !tools.is_empty(),
            tools,
            state: state_value,
            action,
            connected_as,
            editable: matches!(sourced.source, AgentMcpAttachmentSource::Agent),
        });
    }

    Ok(Json(attachments))
}

#[utoipa::path(
    delete,
    path = "/v1/agents/{agent_id}/mcp-attachments/{name}/connection",
    params(
        ("agent_id" = String, Path, description = "Agent ID (prefixed) or name"),
        ("name" = String, Path, description = "Effective MCP attachment name"),
    ),
    responses(
        (status = 204, description = "MCP connection revoked"),
        (status = 400, description = "Attachment does not use a connection", body = ErrorResponse),
        (status = 403, description = "Permission denied", body = ErrorResponse),
        (status = 404, description = "Agent, attachment, preset, or connection not found", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse),
    ),
    tag = "agents"
)]
pub async fn revoke_agent_mcp_connection(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((agent_id_or_name, name)): Path<(String, String)>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let agent = crate::domains::agents::GetAgent {
        id: agent_id_or_name,
    }
    .run(&state.ctx(&org))
    .await?;
    let row = state
        .db
        .get_agent(org.org_id, AgentId::from_uuid(agent.internal_id))
        .await
        .log_internal_error_json("load agent MCP attachment identity")?
        .ok_or_else(|| ErrorResponse::not_found("Agent"))?;
    let harness = crate::domains::harnesses::queries::resolve_effective(
        &state.db,
        org.org_id,
        agent.harness_id,
    )
    .await
    .log_internal_error_json("load effective agent harness")?
    .ok_or_else(|| ErrorResponse::not_found("Harness"))?;
    let effective_capabilities =
        everruns_core::merge_capabilities(&harness.capabilities, &agent.capabilities);
    let capability_servers = everruns_core::capabilities::collect_capability_mcp_servers(
        &effective_capabilities,
        state.host_composition.capability_registry().as_ref(),
    );
    let mut servers =
        everruns_core::merge_scoped_mcp_servers(&capability_servers, &harness.mcp_servers);
    servers = everruns_core::merge_scoped_mcp_servers(&servers, &agent.mcp_servers);
    let attachment = servers
        .get(&name)
        .ok_or_else(|| ErrorResponse::not_found("MCP attachment"))?;
    let preset_name = attachment
        .preset
        .as_ref()
        .map(|preset| preset.catalog_name())
        .ok_or_else(|| {
            ErrorResponse::new("This MCP attachment does not use a connection")
                .into_response(StatusCode::BAD_REQUEST)
        })?;
    let preset = state
        .db
        .get_mcp_server_by_name(org.org_id, preset_name)
        .await
        .log_internal_error_json("load MCP attachment preset")?
        .ok_or_else(|| ErrorResponse::not_found("MCP server preset"))?;
    let provider = everruns_core::mcp_oauth_provider_id_for_uuid(preset.id.uuid());

    let deleted = match attachment.acts_as {
        McpServerActsAs::User => match org.user_id {
            Some(user_id) => state
                .db
                .delete_user_connection(user_id, &provider)
                .await
                .log_internal_error_json("revoke user MCP connection")?,
            None => false,
        },
        McpServerActsAs::Service => {
            let caller = Caller::from(&org);
            MCP_SERVER_MANAGE
                .evaluate_with(state.auth.permission_resolver.as_ref(), &caller)
                .map_err(|error| {
                    ErrorResponse::new(error.message).into_response(StatusCode::FORBIDDEN)
                })?;
            match row.agent_identity_id {
                Some(identity_id) => state
                    .db
                    .delete_agent_identity_connection(identity_id, &provider)
                    .await
                    .log_internal_error_json("revoke service MCP connection")?,
                None => false,
            }
        }
        McpServerActsAs::None => {
            return Err(
                ErrorResponse::new("This MCP attachment does not use a connection")
                    .into_response(StatusCode::BAD_REQUEST),
            );
        }
    };

    if deleted {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ErrorResponse::not_found("MCP connection"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sourced_mcp_layers_keep_winner_and_override_history() {
        let capability: ScopedMcpServers = serde_json::from_value(serde_json::json!({
            "search": { "url": "https://capability.example/mcp" }
        }))
        .unwrap();
        let harness: ScopedMcpServers = serde_json::from_value(serde_json::json!({
            "search": { "url": "https://harness.example/mcp" }
        }))
        .unwrap();
        let agent: ScopedMcpServers = serde_json::from_value(serde_json::json!({
            "search": { "url": "https://agent.example/mcp" }
        }))
        .unwrap();
        let mut effective = BTreeMap::new();

        merge_sourced_mcp_layer(
            &mut effective,
            &capability,
            AgentMcpAttachmentSource::Capability,
            "Capability".to_string(),
        );
        merge_sourced_mcp_layer(
            &mut effective,
            &harness,
            AgentMcpAttachmentSource::Harness,
            "Generic".to_string(),
        );
        merge_sourced_mcp_layer(
            &mut effective,
            &agent,
            AgentMcpAttachmentSource::Agent,
            "Agent".to_string(),
        );

        let search = effective.get("search").unwrap();
        assert_eq!(search.source, AgentMcpAttachmentSource::Agent);
        assert_eq!(search.server.url, "https://agent.example/mcp");
        assert_eq!(
            search
                .overridden_sources
                .iter()
                .map(|source| source.source)
                .collect::<Vec<_>>(),
            vec![
                AgentMcpAttachmentSource::Capability,
                AgentMcpAttachmentSource::Harness
            ]
        );
    }
}

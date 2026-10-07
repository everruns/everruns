use crate::auth::ResolvedOrg;
use crate::domains::common::Command;
use crate::domains::mcp_servers::MCP_SERVER_MANAGE;
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::get,
};
use everruns_contracts::CapabilityRef as AgentCapabilityConfig;
use everruns_contracts::typed_id::AgentId;
use everruns_core::{
    Caller, CapabilityRegistry, McpConnectInChat, McpServerActsAs, ScopedMcpServer,
    ScopedMcpServers,
};
use serde::Serialize;
use std::collections::BTreeMap;
use utoipa::ToSchema;

use super::agents::AppState;
use super::common::{ApiResult, ApiResultExt, ErrorResponse};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/v1/agents/{agent_id}/mcp-attachments",
            get(list_agent_mcp_attachments),
        )
        .route(
            "/v1/agents/{agent_id}/mcp-attachments/{name}/connection",
            axum::routing::delete(revoke_agent_mcp_connection),
        )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
/// Configuration layer that supplied an effective MCP attachment.
pub enum AgentMcpAttachmentSource {
    /// The attachment came from an enabled capability.
    Capability,
    /// The attachment came from the agent's effective harness.
    Harness,
    /// The attachment came directly from the agent configuration.
    Agent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
/// Configuration layer that was overridden by the effective MCP attachment.
pub struct AgentMcpAttachmentSourceInfo {
    /// Overridden configuration layer.
    pub source: AgentMcpAttachmentSource,
    /// Human-readable name of the overridden capability, harness, or agent layer.
    pub source_label: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
/// Availability state of an effective MCP attachment.
pub enum AgentMcpAttachmentState {
    /// The attachment is ready to use.
    Ready,
    /// The attachment needs a user or service connection.
    ConnectionMissing,
    /// The attachment references a catalog preset that is missing or archived.
    PresetMissing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
/// Action the current caller can take to make an MCP attachment usable.
pub enum AgentMcpAttachmentAction {
    /// No connection action is available or required.
    None,
    /// The current user can connect their own account.
    Connect,
    /// The current caller can authorize a shared agent connection.
    Authorize,
    /// An administrator must authorize the shared agent connection.
    AskAdmin,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
/// Effective MCP attachment projected for an agent and the current caller.
pub struct AgentMcpAttachment {
    /// Logical attachment name used in the agent's MCP configuration.
    pub name: String,
    /// Highest-precedence configuration layer that supplied this attachment.
    pub source: AgentMcpAttachmentSource,
    /// Human-readable name of the winning capability, harness, or agent layer.
    pub source_label: String,
    /// Capability that contributed the attachment, when the winning source is a capability.
    pub contributor: Option<AgentMcpAttachmentContributor>,
    /// Lower-precedence configuration layers overridden by this attachment.
    pub overridden_sources: Vec<AgentMcpAttachmentSourceInfo>,
    /// Identity whose connection is used when the attachment calls the MCP server.
    pub acts_as: McpServerActsAs,
    /// Whether a missing sign-in pauses the turn with an in-chat card (`ask`)
    /// or fails the call with a settings link (`never`).
    pub connect_in_chat: McpConnectInChat,
    /// Whether the attachment's tools are listed only when the model reveals
    /// the server through tool search (`false` lists them at turn start).
    pub deferred: bool,
    /// Catalog preset name referenced by the attachment, including a missing preset.
    pub preset_name: Option<String>,
    /// ID of the active catalog preset when the reference resolves.
    pub preset_id: Option<String>,
    /// OAuth provider key used to create or revoke the attachment connection.
    pub connection_provider: Option<String>,
    /// Connection provider (e.g. `github`) whose connection on the agent
    /// supplies the service credential, when the preset names one. The agent's
    /// service login is then set up on the agent's integrations, not by an MCP
    /// sign-in.
    pub service_connection_provider: Option<String>,
    /// Effective MCP endpoint URL from the catalog preset or inline configuration.
    pub url: Option<String>,
    /// Header names configured for the endpoint; secret header values are omitted.
    pub header_names: Vec<String>,
    /// Whether at least one cached tool name is available.
    pub tools_available: bool,
    /// Cached names of tools exposed by the MCP server.
    pub tools: Vec<String>,
    /// Current preset and connection availability.
    pub state: AgentMcpAttachmentState,
    /// Connection action available to the current caller.
    pub action: AgentMcpAttachmentAction,
    /// Connected account name, or the preset name when the provider did not supply one.
    pub connected_as: Option<String>,
    /// Whether the current caller can revoke the active connection.
    pub can_revoke: bool,
    /// Whether the attachment is defined directly on the agent and can be removed there.
    pub editable: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
/// Capability that contributed an effective MCP attachment.
pub struct AgentMcpAttachmentContributor {
    /// Canonical capability ID.
    pub id: String,
    /// Human-readable capability name.
    pub name: String,
    /// UI path for the capability detail page.
    pub href: String,
}

#[derive(Debug, Clone)]
struct SourcedMcpAttachment {
    server: ScopedMcpServer,
    source: AgentMcpAttachmentSource,
    source_label: String,
    contributor: Option<AgentMcpAttachmentContributor>,
    overridden_sources: Vec<AgentMcpAttachmentSourceInfo>,
}

fn merge_sourced_mcp_layer(
    effective: &mut BTreeMap<String, SourcedMcpAttachment>,
    layer: &ScopedMcpServers,
    source: AgentMcpAttachmentSource,
    source_label: String,
    contributor: Option<AgentMcpAttachmentContributor>,
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
                contributor: contributor.clone(),
                overridden_sources,
            },
        );
    }
}
fn capability_contributor(
    capability: &AgentCapabilityConfig,
    registry: &CapabilityRegistry,
) -> AgentMcpAttachmentContributor {
    let id = capability.capability_id().to_string();
    let name = registry
        .get(&id)
        .map(|capability| capability.localized_name(None))
        .or_else(|| {
            serde_json::from_value::<everruns_core::DeclarativeCapabilityDefinition>(
                capability.config_value().clone(),
            )
            .ok()
            .map(|definition| definition.display_name.unwrap_or(definition.name))
        })
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| id.clone());
    AgentMcpAttachmentContributor {
        href: format!("/capabilities/{id}"),
        id,
        name,
    }
}

fn merge_capability_mcp_layers(
    effective: &mut BTreeMap<String, SourcedMcpAttachment>,
    capabilities: &[AgentCapabilityConfig],
    registry: &CapabilityRegistry,
) {
    for capability in capabilities {
        let servers = everruns_core::capabilities::collect_capability_mcp_servers(
            std::slice::from_ref(capability),
            registry,
        );
        if servers.is_empty() {
            continue;
        }
        let contributor = capability_contributor(capability, registry);
        merge_sourced_mcp_layer(
            effective,
            &servers,
            AgentMcpAttachmentSource::Capability,
            contributor.name.clone(),
            Some(contributor),
        );
    }
}

async fn resolve_effective_capabilities(
    state: &AppState,
    org_id: i64,
    harness_capabilities: &[AgentCapabilityConfig],
    agent_capabilities: &[AgentCapabilityConfig],
) -> Result<Vec<AgentCapabilityConfig>, (StatusCode, Json<ErrorResponse>)> {
    let merged = everruns_core::merge_capabilities(harness_capabilities, agent_capabilities);
    let hydrated = crate::domains::capabilities::queries::hydrate_declarative_capability_configs(
        state.db.as_ref(),
        org_id,
        merged,
    )
    .await
    .log_internal_error_json("hydrate agent MCP attachment capabilities")?;
    everruns_core::capabilities::resolve_capability_configs(
        &hydrated,
        state.host_composition.capability_registry().as_ref(),
    )
    .map_err(|error| anyhow::anyhow!(error))
    .log_internal_error_json("resolve agent MCP attachment capability dependencies")
}

fn merge_effective_mcp_attachments(
    capabilities: &[AgentCapabilityConfig],
    registry: &CapabilityRegistry,
    harness_servers: &ScopedMcpServers,
    harness_label: String,
    agent_servers: &ScopedMcpServers,
) -> BTreeMap<String, SourcedMcpAttachment> {
    let mut effective = BTreeMap::new();
    merge_capability_mcp_layers(&mut effective, capabilities, registry);
    merge_sourced_mcp_layer(
        &mut effective,
        harness_servers,
        AgentMcpAttachmentSource::Harness,
        harness_label,
        None,
    );
    merge_sourced_mcp_layer(
        &mut effective,
        agent_servers,
        AgentMcpAttachmentSource::Agent,
        "Agent".to_string(),
        None,
    );
    effective
}

/// Which grants exist for an attachment: the current caller's own, and the
/// agent's shared service grant.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct ConnectionPresence {
    user: bool,
    service: bool,
}

impl ConnectionPresence {
    /// Whether a call made now would find a credential.
    fn usable(self, acts_as: McpServerActsAs) -> bool {
        match acts_as {
            McpServerActsAs::None => true,
            McpServerActsAs::User => self.user,
            McpServerActsAs::Service => self.service,
            McpServerActsAs::UserOrService => self.user || self.service,
        }
    }
}

fn project_connection_permissions(
    source: AgentMcpAttachmentSource,
    acts_as: McpServerActsAs,
    preset_missing: bool,
    presence: ConnectionPresence,
    can_manage_service_connections: bool,
) -> (AgentMcpAttachmentAction, bool) {
    if matches!(source, AgentMcpAttachmentSource::Capability) {
        return (AgentMcpAttachmentAction::None, false);
    }
    // Revoking removes the caller's own grant when there is one; otherwise the
    // agent's, which needs MCP management permission.
    let can_revoke = (acts_as.uses_user_grant() && presence.user)
        || (acts_as.uses_service_grant() && presence.service && can_manage_service_connections);
    if preset_missing {
        return (AgentMcpAttachmentAction::None, can_revoke);
    }
    let action = match acts_as {
        McpServerActsAs::None => AgentMcpAttachmentAction::None,
        McpServerActsAs::User if presence.user => AgentMcpAttachmentAction::None,
        McpServerActsAs::User => AgentMcpAttachmentAction::Connect,
        McpServerActsAs::Service if presence.service => AgentMcpAttachmentAction::None,
        McpServerActsAs::Service if can_manage_service_connections => {
            AgentMcpAttachmentAction::Authorize
        }
        McpServerActsAs::Service => AgentMcpAttachmentAction::AskAdmin,
        // The agent's login is what everyone without their own falls back
        // to, so a manager is offered that first. Anyone else can switch
        // their own calls to their own account.
        McpServerActsAs::UserOrService if !presence.service && can_manage_service_connections => {
            AgentMcpAttachmentAction::Authorize
        }
        McpServerActsAs::UserOrService if !presence.user => AgentMcpAttachmentAction::Connect,
        McpServerActsAs::UserOrService => AgentMcpAttachmentAction::None,
    };
    (action, can_revoke)
}
#[utoipa::path(
    get,
    path = "/v1/agents/{agent_id}/mcp-attachments",
    description = "Lists the effective MCP attachments after capability, harness, and agent layers are merged. Connection state and permitted actions are resolved for the current caller.",
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

    let effective_capabilities = resolve_effective_capabilities(
        &state,
        org.org_id,
        &harness.capabilities,
        &agent.capabilities,
    )
    .await?;
    let effective = merge_effective_mcp_attachments(
        &effective_capabilities,
        state.host_composition.capability_registry().as_ref(),
        &harness.mcp_servers,
        harness
            .display_name
            .clone()
            .unwrap_or_else(|| harness.name.clone()),
        &agent.mcp_servers,
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
        let service_connection_provider = preset_row.as_ref().and_then(|row| {
            crate::domains::mcp_servers::McpServerService::settings_from_row(row)
                .service_connection_provider
        });
        let acts_as = sourced.server.acts_as;
        let user_connection = match (acts_as.uses_user_grant(), provider.as_deref(), org.user_id) {
            (true, Some(provider), Some(user_id)) => state
                .db
                .get_user_connection(user_id, provider)
                .await
                .log_internal_error_json("load user MCP connection")?
                .map(|row| row.provider_username),
            _ => None,
        };
        // A connection-backed preset's service credential is the agent's
        // connection for that provider (its GitHub App), not an MCP grant.
        let service_provider = service_connection_provider.clone().or(provider.clone());
        let service_connection = match (
            acts_as.uses_service_grant(),
            service_provider.as_deref(),
            row.virtual_user_id,
        ) {
            (true, Some(service_provider), Some(identity_id)) => state
                .db
                .get_virtual_user_connection(identity_id, service_provider)
                .await
                .log_internal_error_json("load service MCP connection")?
                .map(|row| row.provider_username),
            _ => None,
        };
        let presence = ConnectionPresence {
            user: user_connection.is_some(),
            service: service_connection.is_some(),
        };
        // The caller's own account when they connected one, else the agent's.
        let connected_as = user_connection
            .or(service_connection)
            .and_then(|username| username.or_else(|| preset_name.clone()));
        let state_value = if preset_missing {
            AgentMcpAttachmentState::PresetMissing
        } else if !presence.usable(acts_as) {
            AgentMcpAttachmentState::ConnectionMissing
        } else {
            AgentMcpAttachmentState::Ready
        };
        let (action, can_revoke) = project_connection_permissions(
            sourced.source,
            acts_as,
            preset_missing,
            presence,
            can_authorize_service,
        );
        // The agent's GitHub App (or other backing connection) is removed from
        // the agent's integrations, not from one MCP row that borrows it.
        let can_revoke = can_revoke
            && (presence.user && acts_as.uses_user_grant()
                || service_connection_provider.is_none());
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
            contributor: sourced.contributor,
            overridden_sources: sourced.overridden_sources,
            acts_as: sourced.server.acts_as,
            connect_in_chat: sourced.server.connect_in_chat,
            deferred: sourced.server.deferred,
            preset_name,
            preset_id: preset_row.as_ref().map(|row| row.id.to_string()),
            connection_provider: provider,
            service_connection_provider,
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
            can_revoke,
            editable: matches!(sourced.source, AgentMcpAttachmentSource::Agent),
        });
    }

    Ok(Json(attachments))
}

#[utoipa::path(
    delete,
    path = "/v1/agents/{agent_id}/mcp-attachments/{name}/connection",
    description = "Revokes the current caller's user connection or the virtual user's shared service connection for an effective MCP attachment. The attachment configuration remains unchanged.",
    params(
        ("agent_id" = String, Path, description = "Agent ID (prefixed) or name"),
        ("name" = String, Path, description = "Effective MCP attachment name"),
    ),
    responses(
        (status = 204, description = "MCP connection revoked or already absent"),
        (status = 400, description = "Attachment does not use a connection", body = ErrorResponse),
        (status = 403, description = "Permission denied", body = ErrorResponse),
        (status = 404, description = "Agent, attachment, or preset not found", body = ErrorResponse),
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
    let effective_capabilities = resolve_effective_capabilities(
        &state,
        org.org_id,
        &harness.capabilities,
        &agent.capabilities,
    )
    .await?;
    let effective = merge_effective_mcp_attachments(
        &effective_capabilities,
        state.host_composition.capability_registry().as_ref(),
        &harness.mcp_servers,
        harness
            .display_name
            .clone()
            .unwrap_or_else(|| harness.name.clone()),
        &agent.mcp_servers,
    );
    let attachment = effective
        .get(&name)
        .ok_or_else(|| ErrorResponse::not_found("MCP attachment"))?;
    let attachment = &attachment.server;
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

    // `user_or_service` revokes the caller's own grant when they have one
    // (their calls fall back to the agent's), otherwise the agent's.
    let user_grant = match (attachment.acts_as.uses_user_grant(), org.user_id) {
        (true, Some(user_id)) => state
            .db
            .get_user_connection(user_id, &provider)
            .await
            .log_internal_error_json("load user MCP connection")?
            .map(|_| user_id),
        _ => None,
    };
    let revoke_as = match (attachment.acts_as, user_grant) {
        (McpServerActsAs::UserOrService, Some(_)) => McpServerActsAs::User,
        (McpServerActsAs::UserOrService, None) => McpServerActsAs::Service,
        (acts_as, _) => acts_as,
    };
    if revoke_as == McpServerActsAs::Service
        && crate::domains::mcp_servers::McpServerService::settings_from_row(&preset)
            .service_connection_provider
            .is_some()
    {
        return Err(ErrorResponse::new(
            "This MCP server uses the agent's own connection; manage it in the agent's integrations",
        )
        .into_response(StatusCode::BAD_REQUEST));
    }

    match revoke_as {
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
            match row.virtual_user_id {
                Some(identity_id) => state
                    .db
                    .delete_virtual_user_connection(identity_id, &provider)
                    .await
                    .log_internal_error_json("revoke service MCP connection")?,
                None => false,
            }
        }
        McpServerActsAs::None | McpServerActsAs::UserOrService => {
            return Err(
                ErrorResponse::new("This MCP attachment does not use a connection")
                    .into_response(StatusCode::BAD_REQUEST),
            );
        }
    };

    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_core::{Capability, CapabilityMcpServer, CapabilityMcpServers};

    struct DependencyMcpCapability;

    impl Capability for DependencyMcpCapability {
        fn id(&self) -> &str {
            "dependency_mcp"
        }

        fn name(&self) -> &str {
            "Dependency MCP"
        }

        fn description(&self) -> &str {
            "Contributes an MCP server for projection tests."
        }

        fn mcp_servers(&self) -> CapabilityMcpServers {
            CapabilityMcpServers::from([(
                "dependency-server".to_string(),
                CapabilityMcpServer::new(
                    ScopedMcpServer {
                        url: "https://dependency.example/mcp".to_string(),
                        ..Default::default()
                    },
                    McpServerActsAs::None,
                ),
            )])
        }
    }

    struct ParentCapability;

    impl Capability for ParentCapability {
        fn id(&self) -> &str {
            "parent"
        }

        fn name(&self) -> &str {
            "Parent"
        }

        fn description(&self) -> &str {
            "Depends on the MCP-contributing test capability."
        }

        fn dependencies(&self) -> Vec<&'static str> {
            vec!["dependency_mcp"]
        }
    }

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
            None,
        );
        merge_sourced_mcp_layer(
            &mut effective,
            &harness,
            AgentMcpAttachmentSource::Harness,
            "Generic".to_string(),
            None,
        );
        merge_sourced_mcp_layer(
            &mut effective,
            &agent,
            AgentMcpAttachmentSource::Agent,
            "Agent".to_string(),
            None,
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

    #[test]
    fn connected_service_connection_requires_manage_permission_to_revoke() {
        assert_eq!(
            project_connection_permissions(
                AgentMcpAttachmentSource::Agent,
                McpServerActsAs::Service,
                false,
                ConnectionPresence {
                    user: false,
                    service: true,
                },
                false,
            ),
            (AgentMcpAttachmentAction::None, false)
        );
        assert_eq!(
            project_connection_permissions(
                AgentMcpAttachmentSource::Agent,
                McpServerActsAs::Service,
                false,
                ConnectionPresence::default(),
                false,
            ),
            (AgentMcpAttachmentAction::AskAdmin, false)
        );
    }

    #[test]
    fn dependency_contributed_server_keeps_actual_contributor() {
        let mut registry = CapabilityRegistry::new();
        registry.register(DependencyMcpCapability);
        registry.register(ParentCapability);
        let selected = vec![AgentCapabilityConfig::new("parent")];
        let resolved =
            everruns_core::capabilities::resolve_capability_configs(&selected, &registry).unwrap();

        let effective = merge_effective_mcp_attachments(
            &resolved,
            &registry,
            &ScopedMcpServers::default(),
            "Harness".to_string(),
            &ScopedMcpServers::default(),
        );

        let attachment = effective.get("dependency-server").unwrap();
        assert_eq!(attachment.server.url, "https://dependency.example/mcp");
        assert_eq!(
            attachment.contributor,
            Some(AgentMcpAttachmentContributor {
                id: "dependency_mcp".to_string(),
                name: "Dependency MCP".to_string(),
                href: "/capabilities/dependency_mcp".to_string(),
            })
        );
    }

    #[test]
    fn capability_connections_are_always_read_only() {
        assert_eq!(
            project_connection_permissions(
                AgentMcpAttachmentSource::Capability,
                McpServerActsAs::User,
                false,
                ConnectionPresence::default(),
                true,
            ),
            (AgentMcpAttachmentAction::None, false)
        );
        assert_eq!(
            project_connection_permissions(
                AgentMcpAttachmentSource::Capability,
                McpServerActsAs::Service,
                false,
                ConnectionPresence {
                    user: false,
                    service: true,
                },
                true,
            ),
            (AgentMcpAttachmentAction::None, false)
        );
    }

    #[test]
    fn user_or_service_offers_the_agent_login_to_managers_and_the_own_login_to_everyone() {
        let none = ConnectionPresence::default();
        let service_only = ConnectionPresence {
            user: false,
            service: true,
        };
        let user_only = ConnectionPresence {
            user: true,
            service: false,
        };
        let agent = AgentMcpAttachmentSource::Agent;
        let acts_as = McpServerActsAs::UserOrService;

        // Nobody connected: a manager authorizes the shared login, anyone else
        // connects their own.
        assert_eq!(
            project_connection_permissions(agent, acts_as, false, none, true),
            (AgentMcpAttachmentAction::Authorize, false)
        );
        assert_eq!(
            project_connection_permissions(agent, acts_as, false, none, false),
            (AgentMcpAttachmentAction::Connect, false)
        );
        // The agent's login works; anyone can still switch to their own, and
        // only a manager can revoke the shared one.
        assert_eq!(
            project_connection_permissions(agent, acts_as, false, service_only, false),
            (AgentMcpAttachmentAction::Connect, false)
        );
        assert_eq!(
            project_connection_permissions(agent, acts_as, false, service_only, true),
            (AgentMcpAttachmentAction::Connect, true)
        );
        // The caller's own login: nothing to do, and they can revoke it.
        assert_eq!(
            project_connection_permissions(agent, acts_as, false, user_only, false),
            (AgentMcpAttachmentAction::None, true)
        );
        assert!(none.usable(McpServerActsAs::None));
        assert!(!none.usable(acts_as));
        assert!(service_only.usable(acts_as));
        assert!(user_only.usable(acts_as));
        assert!(!service_only.usable(McpServerActsAs::User));
        assert!(!user_only.usable(McpServerActsAs::Service));
    }
}

//! Effective MCP attachments of an agent, and each one's sign-in state for
//! the caller (knowledge/integrations/mcp-oauth-grant-health.md).
//!
//! Decisions:
//! - These are commands, so the console, the CLI (`everruns agents mcp ...`),
//!   MCP clients and Platform Chat read the same states.
//! - `sign_in` reports what the agent's own sign-in can still do, read from
//!   the stored grant: nothing is sent to the server to show it. A sign-in
//!   that works today but cannot renew is reported as soon as it is seen,
//!   while Health only warns in the last `EXPIRY_WARNING`.
use super::AGENT_VIEW;
use crate::domains::common::*;
use crate::domains::mcp_servers::MCP_SERVER_MANAGE;
use crate::storage::GrantHealth;
use chrono::{DateTime, Utc};
use everruns_contracts::CapabilityRef as AgentCapabilityConfig;
use everruns_contracts::typed_id::AgentId;
use everruns_core::{
    CapabilityRegistry, McpConnectInChat, McpServerActsAs, ScopedMcpServer, ScopedMcpServers,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use utoipa::ToSchema;

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
    /// The agent's sign-in stopped working (it expired and cannot renew, or
    /// the server refused it) and must be done again. Its tools are missing
    /// from turns until then.
    ReconnectRequired,
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
    /// The agent's own sign-in to this catalog server and what it can still
    /// do. Absent when the attachment does not sign in as the agent, or the
    /// agent never signed in.
    pub sign_in: Option<AgentMcpSignIn>,
    /// Console path where a person signs in (again), when a sign-in is
    /// missing, stopped working or will stop.
    pub sign_in_href: Option<String>,
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
    ctx: &Ctx,
    harness_capabilities: &[AgentCapabilityConfig],
    agent_capabilities: &[AgentCapabilityConfig],
) -> Result<Vec<AgentCapabilityConfig>, CommandError> {
    let merged = everruns_core::merge_capabilities(harness_capabilities, agent_capabilities);
    let hydrated = crate::domains::capabilities::queries::hydrate_declarative_capability_configs(
        ctx.db.as_ref(),
        ctx.org_id(),
        merged,
    )
    .await?;
    everruns_core::capabilities::resolve_capability_configs(
        &hydrated,
        ctx.capability_service.registry(),
    )
    .map_err(|error| CommandError::internal(anyhow::anyhow!(error)))
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
/// What the agent's own sign-in to a catalog server can still do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AgentMcpSignInState {
    /// It produces tokens and renews them as needed.
    Working,
    /// It works until `expires_at`, and nothing can renew it.
    ExpiresSoon,
    /// It produces no token until someone signs in again; the server's tools
    /// are missing from turns.
    NeedsSignIn,
}

/// The agent's own (service) sign-in to an OAuth catalog server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct AgentMcpSignIn {
    /// What the sign-in can still do.
    pub state: AgentMcpSignInState,
    /// When it stops working, for `expires_soon`.
    pub expires_at: Option<DateTime<Utc>>,
    /// When it stopped working, for `needs_sign_in`, when known.
    pub since: Option<DateTime<Utc>>,
    /// Why it cannot renew or stopped working: `unbound`, `settings_changed`,
    /// `no_refresh_token`, `refused` or `rejected`.
    pub reason: Option<String>,
    /// The reason in plain words.
    pub detail: Option<String>,
}

impl AgentMcpSignIn {
    fn from_health(health: GrantHealth) -> Self {
        let words = |reason: &str| {
            Some(crate::domains::health_issues::mcp_grants::reason_text(reason).to_string())
        };
        match health {
            GrantHealth::Usable => Self {
                state: AgentMcpSignInState::Working,
                expires_at: None,
                since: None,
                reason: None,
                detail: None,
            },
            GrantHealth::ExpiresSoon { at, reason } => Self {
                state: AgentMcpSignInState::ExpiresSoon,
                expires_at: Some(at),
                since: None,
                detail: words(&reason),
                reason: Some(reason),
            },
            GrantHealth::NeedsSignIn { reason, since } => Self {
                state: AgentMcpSignInState::NeedsSignIn,
                expires_at: None,
                since,
                detail: words(&reason),
                reason: Some(reason),
            },
        }
    }
}

/// The agent's service grant for catalog server `server_id`, read without
/// using it. Unknown (no encryption key, a lookup failure) reads as working:
/// turns then behave as before and Health reports the grant.
async fn service_sign_in(
    ctx: &Ctx,
    server_id: uuid::Uuid,
    grant: &crate::storage::VirtualUserConnectionRow,
) -> AgentMcpSignIn {
    if let Some((reason, since)) =
        crate::storage::connection_resolver::grant_health::reconnect_mark(grant)
    {
        return AgentMcpSignIn::from_health(GrantHealth::NeedsSignIn { reason, since });
    }
    let Some(encryption) = ctx.encryption.as_ref() else {
        return AgentMcpSignIn::from_health(GrantHealth::Usable);
    };
    let resolver = crate::storage::DbConnectionResolver::new(
        ctx.db.as_ref().clone(),
        encryption.as_ref().clone(),
        None,
        // Never used: reading a grant's health sends nothing.
        std::sync::Arc::new(
            everruns_core::host::DirectEgressService::for_runtime_traffic_from_env(),
        ),
    );
    let health = resolver
        .mcp_grant_health(ctx.org_id(), server_id, grant)
        .await
        .unwrap_or(GrantHealth::Usable);
    AgentMcpSignIn::from_health(health)
}

/// The agent row and its effective MCP attachments, all layers merged.
async fn effective_attachments(
    ctx: &Ctx,
    agent_id_or_name: &str,
) -> Result<
    (
        crate::storage::AgentRow,
        String,
        BTreeMap<String, SourcedMcpAttachment>,
    ),
    CommandError,
> {
    let agent = super::queries::resolve(&ctx.db, ctx.org_id(), agent_id_or_name)
        .await?
        .ok_or_else(|| CommandError::not_found("Agent"))?;
    let row = ctx
        .db
        .get_agent(ctx.org_id(), AgentId::from_uuid(agent.internal_id))
        .await?
        .ok_or_else(|| CommandError::not_found("Agent"))?;
    let harness = crate::domains::harnesses::queries::resolve_effective(
        &ctx.db,
        ctx.org_id(),
        agent.harness_id,
    )
    .await?
    .ok_or_else(|| CommandError::not_found("Harness"))?;
    let effective_capabilities =
        resolve_effective_capabilities(ctx, &harness.capabilities, &agent.capabilities).await?;
    let effective = merge_effective_mcp_attachments(
        &effective_capabilities,
        ctx.capability_service.registry(),
        &harness.mcp_servers,
        harness
            .display_name
            .clone()
            .unwrap_or_else(|| harness.name.clone()),
        &agent.mcp_servers,
    );
    Ok((row, agent.public_id.to_string(), effective))
}

/// Where a person signs the agent in again: the agent's MCP sheet.
pub fn sign_in_href(agent_public_id: &str) -> String {
    crate::domains::health_issues::mcp_grants::sign_in_href(agent_public_id)
}

/// List an agent's effective MCP servers and whether each one's sign-in works.
#[derive(Debug, Deserialize, ToSchema, Serialize)]
pub struct ListAgentMcpAttachments {
    /// Agent ID (prefixed) or name.
    pub id: String,
}

#[command(
    name = "list_agent_mcp_attachments",
    category = "agents",
    description = "List an agent's MCP servers after capability, harness and agent layers are merged, with each server's sign-in state for the caller: whether it works, renews, expires or needs a person to sign in again.",
    method = "GET",
    path = "/v1/platform/agents/{id}/mcp-attachments",
    policy = AGENT_VIEW,
    positional = "id",
    cli = CliRoute::new(&["agents", "mcp"], "list").with_args(&[CliArg::new("id").at(1)]).with_examples(&[CliExample::new("Check whether an agent's MCP sign-ins still work", "everruns agents mcp list agent_01h9")]),
)]
impl Command for ListAgentMcpAttachments {
    type Output = Vec<AgentMcpAttachment>;

    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        let (row, agent_public_id, effective) = effective_attachments(ctx, &self.id).await?;
        let org_id = ctx.org_id();
        let user_id = ctx.caller.user_id;
        let can_authorize_service = MCP_SERVER_MANAGE
            .evaluate_with(ctx.permission_resolver.as_ref(), &ctx.caller)
            .is_ok();
        let mut attachments = Vec::with_capacity(effective.len());
        for (name, sourced) in effective {
            let preset_name = sourced
                .server
                .preset
                .as_ref()
                .map(|preset| preset.catalog_name().to_string());
            let preset_row = match preset_name.as_deref() {
                Some(name) => ctx
                    .db
                    .get_mcp_server_by_name(org_id, name)
                    .await?
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
            let user_connection = match (acts_as.uses_user_grant(), provider.as_deref(), user_id) {
                (true, Some(provider), Some(user_id)) => ctx
                    .db
                    .get_user_connection(user_id, provider)
                    .await?
                    .map(|row| row.provider_username),
                _ => None,
            };
            // A connection-backed preset's service credential is the agent's
            // connection for that provider (its GitHub App), not an MCP grant.
            let service_provider = service_connection_provider.clone().or(provider.clone());
            let service_row = match (
                acts_as.uses_service_grant(),
                service_provider.as_deref(),
                row.virtual_user_id,
            ) {
                (true, Some(service_provider), Some(identity_id)) => {
                    ctx.db
                        .get_virtual_user_connection(identity_id, service_provider)
                        .await?
                }
                _ => None,
            };
            // The agent's own MCP sign-in, read from its grant (grant_health.rs).
            let sign_in = match (&service_row, &preset_row, &service_connection_provider) {
                (Some(grant), Some(preset), None) => {
                    Some(service_sign_in(ctx, preset.id.uuid(), grant).await)
                }
                _ => None,
            };
            let service_needs_sign_in = sign_in
                .as_ref()
                .is_some_and(|s| s.state == AgentMcpSignInState::NeedsSignIn);
            // A sign-in that stopped working counts as missing, so a manager
            // is offered to sign in again.
            let has_service_grant = service_row.is_some();
            let service_connection = service_row
                .filter(|_| !service_needs_sign_in)
                .map(|row| row.provider_username);
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
            } else if service_needs_sign_in && !presence.usable(acts_as) {
                AgentMcpAttachmentState::ReconnectRequired
            } else if !presence.usable(acts_as) {
                AgentMcpAttachmentState::ConnectionMissing
            } else {
                AgentMcpAttachmentState::Ready
            };
            let (action, _) = project_connection_permissions(
                sourced.source,
                acts_as,
                preset_missing,
                presence,
                can_authorize_service,
            );
            // A sign-in that stopped working can still be revoked.
            let (_, can_revoke) = project_connection_permissions(
                sourced.source,
                acts_as,
                preset_missing,
                ConnectionPresence {
                    service: has_service_grant,
                    ..presence
                },
                can_authorize_service,
            );
            // A working sign-in that will stop: a manager can sign the agent
            // in again ahead of time.
            let action = match (&sign_in, action) {
                (Some(s), AgentMcpAttachmentAction::None)
                    if s.state == AgentMcpSignInState::ExpiresSoon
                        && !matches!(sourced.source, AgentMcpAttachmentSource::Capability) =>
                {
                    if can_authorize_service {
                        AgentMcpAttachmentAction::Authorize
                    } else {
                        AgentMcpAttachmentAction::AskAdmin
                    }
                }
                (_, action) => action,
            };
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
            let sign_in_href = (!matches!(action, AgentMcpAttachmentAction::None)
                || state_value == AgentMcpAttachmentState::ReconnectRequired)
                .then(|| sign_in_href(&agent_public_id));

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
                url: preset_row.as_ref().map(|row| row.url.clone()).or_else(|| {
                    (!sourced.server.url.is_empty()).then(|| sourced.server.url.clone())
                }),
                header_names,
                tools_available: !tools.is_empty(),
                tools,
                state: state_value,
                action,
                connected_as,
                sign_in,
                sign_in_href,
                can_revoke,
                editable: matches!(sourced.source, AgentMcpAttachmentSource::Agent),
            });
        }
        Ok(attachments)
    }
}

/// Whose sign-in a revoke removed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AgentMcpRevokedIdentity {
    /// The caller's own sign-in.
    User,
    /// The agent's shared (service) sign-in.
    Agent,
}

/// Result of revoking an MCP attachment's sign-in.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AgentMcpConnectionRevoked {
    /// Whether a sign-in was removed (`false` when there was none).
    pub revoked: bool,
    /// Whose sign-in it was.
    pub identity: AgentMcpRevokedIdentity,
}

/// Revoke the caller's own sign-in for an agent's MCP server, or the agent's
/// shared sign-in when the caller has none. The attachment stays.
#[derive(Debug, Deserialize, ToSchema, Serialize)]
pub struct RevokeAgentMcpConnection {
    /// Agent ID (prefixed) or name.
    pub id: String,
    /// Effective MCP attachment name.
    pub name: String,
}

#[command(
    name = "revoke_agent_mcp_connection",
    category = "agents",
    description = "Revoke the caller's own sign-in for an agent's MCP server, or the agent's shared sign-in when the caller has none (needs MCP server management). The attachment stays; sign in again on the agent's MCP sheet.",
    method = "DELETE",
    path = "/v1/platform/agents/{id}/mcp-attachments/{name}/connection",
    policy = AGENT_VIEW,
    cli = CliRoute::new(&["agents", "mcp"], "revoke").with_args(&[CliArg::new("id").at(1), CliArg::new("name").at(2)]).with_examples(&[CliExample::new("Remove the agent's stend sign-in before signing in again", "everruns agents mcp revoke agent_01h9 stend --reason 'Sign-in stopped renewing'")]),
)]
impl Command for RevokeAgentMcpConnection {
    type Output = AgentMcpConnectionRevoked;

    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        let (row, _, effective) = effective_attachments(ctx, &self.id).await?;
        let org_id = ctx.org_id();
        let attachment = effective
            .get(&self.name)
            .ok_or_else(|| CommandError::not_found("MCP attachment"))?;
        let attachment = &attachment.server;
        let preset_name = attachment
            .preset
            .as_ref()
            .map(|preset| preset.catalog_name())
            .ok_or_else(|| {
                CommandError::bad_request("This MCP attachment does not use a connection")
            })?;
        let preset = ctx
            .db
            .get_mcp_server_by_name(org_id, preset_name)
            .await?
            .ok_or_else(|| CommandError::not_found("MCP server preset"))?;
        let provider = everruns_core::mcp_oauth_provider_id_for_uuid(preset.id.uuid());

        // `user_or_service` revokes the caller's own grant when they have one
        // (their calls fall back to the agent's), otherwise the agent's.
        let user_grant = match (attachment.acts_as.uses_user_grant(), ctx.caller.user_id) {
            (true, Some(user_id)) => ctx
                .db
                .get_user_connection(user_id, &provider)
                .await?
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
            return Err(CommandError::bad_request(
                "This MCP server uses the agent's own connection; manage it in the agent's integrations",
            ));
        }

        match revoke_as {
            McpServerActsAs::User => Ok(AgentMcpConnectionRevoked {
                revoked: match ctx.caller.user_id {
                    Some(user_id) => ctx.db.delete_user_connection(user_id, &provider).await?,
                    None => false,
                },
                identity: AgentMcpRevokedIdentity::User,
            }),
            McpServerActsAs::Service => {
                MCP_SERVER_MANAGE
                    .evaluate_with(ctx.permission_resolver.as_ref(), &ctx.caller)
                    .map_err(|error| CommandError::forbidden(error.message))?;
                Ok(AgentMcpConnectionRevoked {
                    revoked: match row.virtual_user_id {
                        Some(identity_id) => {
                            ctx.db
                                .delete_virtual_user_connection(identity_id, &provider)
                                .await?
                        }
                        None => false,
                    },
                    identity: AgentMcpRevokedIdentity::Agent,
                })
            }
            McpServerActsAs::None | McpServerActsAs::UserOrService => Err(
                CommandError::bad_request("This MCP attachment does not use a connection"),
            ),
        }
    }
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

    #[test]
    fn sign_in_state_names_what_the_grant_can_still_do() {
        let at = chrono::Utc::now() + chrono::Duration::days(20);
        let soon = AgentMcpSignIn::from_health(GrantHealth::ExpiresSoon {
            at,
            reason: "unbound".into(),
        });
        assert_eq!(soon.state, AgentMcpSignInState::ExpiresSoon);
        assert_eq!(soon.expires_at, Some(at));
        assert_eq!(soon.reason.as_deref(), Some("unbound"));
        assert!(soon.detail.unwrap().contains("cannot renew itself"));

        let gone = AgentMcpSignIn::from_health(GrantHealth::NeedsSignIn {
            reason: "refused".into(),
            since: None,
        });
        assert_eq!(gone.state, AgentMcpSignInState::NeedsSignIn);
        assert_eq!(
            gone.detail.as_deref(),
            Some("the server refused to renew it")
        );

        let ok = AgentMcpSignIn::from_health(GrantHealth::Usable);
        assert_eq!(ok.state, AgentMcpSignInState::Working);
        assert!(ok.reason.is_none());
    }
}

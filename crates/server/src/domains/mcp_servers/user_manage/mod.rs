// The `user_mcp` manage tools' store, served by the control plane.
//
// Spec: knowledge/integrations/user-mcp-servers.md (D2, D3, D5).
//
// Decision: this is the only place a manage tool's call turns into a change,
// for both the in-process worker and a remote one, and it re-derives every
// permission itself instead of trusting the worker: the agent's resolved
// `user_mcp` configuration must have `manage` on (and `allow_custom_urls` for a
// server added by URL), and the person is the turn's verified initiating
// virtual user, looked up from the input message. The operations are the same
// self-service ones `/v1/virtual-users/me/mcp-servers` runs, so the tools never
// need, and never get, organization management authority.
//
// Decision: like the *use* layer, a turn without an initiating person
// (triggers, schedules) or with several people in the session cannot change
// anyone's list; the tools say so instead of guessing whose list was meant.
//
// Decision: "add for this chat only" (D6) writes a session MCP server record
// (`everruns_core::session_mcp_servers`), the record ARD attachments write,
// instead of a list entry. The record is built here, never by the worker: the
// same `manage` / `allow_custom_urls` checks apply, a catalog server signs in
// as the person exactly as it does from their list, a custom server cannot ask
// for OAuth (its sign-in needs the list row that holds the OAuth client), and
// a name the agent already uses is refused, because a session server would
// otherwise win over the agent's own. `remove` and `connect` find these
// records before the list.
//
// Decision: `upsert` never repoints an existing server. A name already in the
// list with a different source is refused, because stored sign-ins are bound to
// the origin they were issued for; re-adding the same server only applies its
// enabled flag.

use std::collections::HashSet;

use everruns_capabilities::capabilities::{
    USER_MCP_CAPABILITY_ID, user_mcp_connect_enabled, user_mcp_custom_urls_allowed,
    user_mcp_manage_enabled,
};
use everruns_core::capabilities::{CapabilityRegistry, collect_capability_mcp_servers};
use everruns_core::mcp::{
    McpLogin, UserMcpLoginStatus, UserMcpServerEntry, UserMcpServerSummary, UserMcpStoreCall,
    UserMcpStoreError, UserMcpStoreReply, UserMcpStoreResult,
};
use uuid::Uuid;

use super::scoped_mcp::merge_effective_scoped_mcp_servers;
use super::user_servers::{
    AddUserMcpServerRequest, UpdateUserMcpServerRequest, UserMcpConnectionStatus, UserMcpServer,
    UserMcpServerError, UserMcpServerSource, UserMcpServers,
};
use crate::kernel_imports::{McpServerActsAs, McpServerAuthMode, resolve_runtime_capabilities};
use crate::records::{Agent, Harness, Session};
use crate::storage::{EncryptionService, StorageBackend};
use everruns_core::session_services::SessionStorageStore;
use everruns_core::{SessionMcpServer, SessionMcpServerSource};

/// Where the Connect card sends a person who cannot use the card itself.
const SETUP_URL: &str = "/settings/connections";

const NO_PERSON: &str = "No person is chatting in this turn, so there is no MCP server list to change. The tools work only while a person is talking to the agent.";
const SHARED: &str = "This conversation has several people in it, so the agent cannot change one person's MCP servers here.";
const NOT_MANAGED: &str =
    "This agent is not allowed to manage MCP servers (user_mcp `manage` is off).";
const PRESET_GONE: &str =
    "This MCP server's catalog preset is no longer available, so there is nothing to sign in to.";

/// One manage call's turn: the loaded records the call is checked against.
pub struct UserMcpManageTurn<'a> {
    pub db: &'a StorageBackend,
    pub encryption: Option<&'a EncryptionService>,
    pub org_id: i64,
    pub harness: &'a Harness,
    pub agent: Option<&'a Agent>,
    pub session: &'a Session,
    pub registry: &'a CapabilityRegistry,
    /// The turn's input message; None for unattended work.
    pub input_message: Option<Uuid>,
    /// Session storage, for servers added to this chat only.
    pub storage: Option<&'a dyn SessionStorageStore>,
}

/// Run one manage call for the turn's initiating person.
pub async fn invoke_user_mcp_store(
    turn: &UserMcpManageTurn<'_>,
    call: UserMcpStoreCall,
) -> UserMcpStoreResult<UserMcpStoreReply> {
    let resolved = resolve_runtime_capabilities(
        &turn.harness.definition(),
        turn.agent.map(|agent| agent.definition()).as_ref(),
        &turn.session.execution_session(),
        turn.registry,
    );
    let config = resolved
        .resolved_capability_configs
        .iter()
        .find(|config| config.capability_id() == USER_MCP_CAPABILITY_ID)
        .map(|config| config.config_value().clone())
        .unwrap_or_default();
    // `connect_mcp_server` alone is available with `manage`, or when one of
    // the agent's own servers signs in as the person chatting (D5); the host
    // derives `connect` for that case. Every other call needs `manage`.
    let allowed = match &call {
        UserMcpStoreCall::StartLogin { .. } => user_mcp_connect_enabled(&config),
        _ => user_mcp_manage_enabled(&config),
    };
    if !allowed {
        return Err(UserMcpStoreError::Unavailable(NOT_MANAGED.into()));
    }
    let person = initiating_person(turn).await?;
    let servers = UserMcpServers {
        db: turn.db,
        encryption: turn.encryption,
        org_id: turn.org_id,
        owner: person,
    };
    let clashes = Clashes::of(turn, &resolved.resolved_capability_configs);

    match call {
        UserMcpStoreCall::List => {
            let mut listed: Vec<_> = servers
                .list()
                .await
                .map_err(store_error)?
                .into_iter()
                .map(|server| clashes.summary(server))
                .collect();
            for record in chat_servers(turn).await {
                listed.push(chat_summary(turn, person, &record).await?);
            }
            Ok(UserMcpStoreReply::Servers { servers: listed })
        }
        UserMcpStoreCall::AddToChat { name, server } => {
            let record = add_to_chat(
                turn,
                &clashes,
                &name,
                *server,
                user_mcp_custom_urls_allowed(&config),
            )
            .await?;
            Ok(UserMcpStoreReply::Server {
                server: chat_summary(turn, person, &record).await?,
            })
        }
        UserMcpStoreCall::Upsert { name, entry } => {
            let server = upsert(
                &servers,
                &name,
                *entry,
                user_mcp_custom_urls_allowed(&config),
            )
            .await?;
            Ok(UserMcpStoreReply::Server {
                server: clashes.summary(server),
            })
        }
        UserMcpStoreCall::Remove { name } => {
            if let Some(storage) = turn.storage
                && chat_server(turn, &name).await.is_some()
            {
                let removed =
                    everruns_core::remove_session_mcp_server(storage, turn.session.id, &name)
                        .await
                        .map_err(|error| UserMcpStoreError::Internal(error.to_string()))?;
                return Ok(UserMcpStoreReply::Removed { removed });
            }
            let removed = match find(&servers, &name).await? {
                Some(server) => {
                    servers
                        .remove(parse_id(&server)?)
                        .await
                        .map_err(store_error)?;
                    true
                }
                None => false,
            };
            Ok(UserMcpStoreReply::Removed { removed })
        }
        UserMcpStoreCall::SetEnabled { name, enabled } => {
            let server = find(&servers, &name)
                .await?
                .ok_or(UserMcpStoreError::NotFound(name))?;
            let server = servers
                .update(
                    parse_id(&server)?,
                    UpdateUserMcpServerRequest {
                        enabled: Some(enabled),
                        ..Default::default()
                    },
                )
                .await
                .map_err(store_error)?;
            Ok(UserMcpStoreReply::Server {
                server: clashes.summary(server),
            })
        }
        UserMcpStoreCall::StartLogin { name } => {
            // The agent's own servers win a name clash, as they do at runtime.
            if let Some(login) = agent_server_login(turn, &resolved, person, &name).await? {
                return Ok(UserMcpStoreReply::Login { login });
            }
            if let Some(record) = chat_server(turn, &name).await {
                return Ok(UserMcpStoreReply::Login {
                    login: server_login(turn, person, None, record.server).await?,
                });
            }
            let server = find(&servers, &name)
                .await?
                .ok_or(UserMcpStoreError::NotFound(name))?;
            Ok(UserMcpStoreReply::Login {
                login: start_login(&server)?,
            })
        }
    }
}

/// The records a manage call is checked against, loaded from storage as the
/// turn sees them: the agent that answers this input message, in its current
/// configuration, and its effective harness. Shared by the in-process worker
/// and the gRPC edge.
pub struct ManageTurnRecords {
    pub harness: Harness,
    pub agent: Option<Agent>,
    pub session: Session,
}

impl ManageTurnRecords {
    pub async fn load(
        db: &StorageBackend,
        org_id: i64,
        session_id: everruns_contracts::typed_id::SessionId,
        input_message: Option<Uuid>,
    ) -> anyhow::Result<Option<Self>> {
        let Some(row) = db.get_session(org_id, session_id).await? else {
            return Ok(None);
        };
        let org_public_id = everruns_core::org_public_id_from_internal(org_id);
        let mut session =
            crate::domains::sessions::SessionService::row_to_session(row, &org_public_id, None);
        // Raw row: `agent_id` is the agent's internal id here.
        if let Some(message) = input_message {
            if !db.runtime_invocation_exists(session.id, message).await? {
                anyhow::bail!("Unknown invocation");
            }
            let responder = db
                .runtime_invocation_responder(session.id, message)
                .await?
                .map(everruns_contracts::typed_id::AgentId::from_uuid);
            if session.agent_id != responder {
                session.agent_revision = None;
            }
            session.agent_id = responder;
        }
        let agent = match session.agent_id {
            Some(agent_id) => match db.get_agent(org_id, agent_id).await? {
                Some(row) if row.status != "deleted" => {
                    let capabilities = crate::domains::agents::queries::get_capabilities(
                        db,
                        org_id,
                        row.id.uuid(),
                    )
                    .await?;
                    Some(crate::domains::agents::queries::row_to_agent(
                        row,
                        capabilities,
                    ))
                }
                _ => None,
            },
            None => None,
        };
        let Some(harness) =
            crate::domains::harnesses::queries::resolve_effective(db, org_id, session.harness_id)
                .await?
        else {
            return Ok(None);
        };
        Ok(Some(Self {
            harness,
            agent,
            session,
        }))
    }
}

/// Load the turn's records and run one manage call: what both worker paths
/// call with nothing but ids.
#[allow(clippy::too_many_arguments)] // Each is a separate host handle; the turn is loaded here.
pub async fn invoke_user_mcp_store_for_session(
    db: &StorageBackend,
    encryption: Option<&EncryptionService>,
    registry: &CapabilityRegistry,
    org_id: i64,
    session_id: everruns_contracts::typed_id::SessionId,
    input_message: Option<Uuid>,
    storage: Option<&dyn SessionStorageStore>,
    call: UserMcpStoreCall,
) -> UserMcpStoreResult<UserMcpStoreReply> {
    let records = ManageTurnRecords::load(db, org_id, session_id, input_message)
        .await
        .map_err(internal)?
        .ok_or_else(|| UserMcpStoreError::Unavailable("Session not found".into()))?;
    invoke_user_mcp_store(
        &UserMcpManageTurn {
            db,
            encryption,
            org_id,
            harness: &records.harness,
            agent: records.agent.as_ref(),
            session: &records.session,
            registry,
            input_message,
            storage,
        },
        call,
    )
    .await
}

async fn initiating_person(turn: &UserMcpManageTurn<'_>) -> UserMcpStoreResult<Uuid> {
    let message = turn
        .input_message
        .ok_or_else(|| UserMcpStoreError::Unavailable(NO_PERSON.into()))?;
    let person = turn
        .db
        .runtime_invocation_subject(turn.session.id, message)
        .await
        .map_err(internal)?
        .ok_or_else(|| UserMcpStoreError::Unavailable(NO_PERSON.into()))?;
    let people = turn
        .db
        .list_session_participants(turn.org_id, turn.session.id)
        .await
        .map_err(internal)?
        .into_iter()
        .filter(|p| p.kind == "user" && p.left_at.is_none())
        .map(|p| p.principal_id)
        .collect::<HashSet<_>>();
    if people.len() > 1 {
        return Err(UserMcpStoreError::Unavailable(SHARED.into()));
    }
    Ok(person.uuid())
}

async fn upsert(
    servers: &UserMcpServers<'_>,
    name: &str,
    entry: UserMcpServerEntry,
    custom_urls_allowed: bool,
) -> UserMcpStoreResult<UserMcpServer> {
    let catalog = entry
        .server
        .preset
        .as_ref()
        .map(|preset| preset.catalog_name().to_string());
    let url = entry.server.url.trim().to_string();
    if catalog.is_none() && !custom_urls_allowed {
        return Err(UserMcpStoreError::Invalid(
            "This agent can only add servers from the organization's MCP catalog.".into(),
        ));
    }
    // Credentials never come through chat: no API keys, no literal headers.
    if !entry.server.headers.is_empty()
        || entry.server.auth_mode == McpServerAuthMode::ApiKey
        || entry.server.command.is_some()
    {
        return Err(UserMcpStoreError::Invalid(
            "Servers added in chat cannot carry keys, headers or commands. The person can add those in Settings > My MCP servers.".into(),
        ));
    }

    if let Some(existing) = find(servers, name).await? {
        let same = match &catalog {
            Some(catalog) => existing.catalog_name.as_deref() == Some(catalog.as_str()),
            None => existing.source == UserMcpServerSource::Custom && existing.url == url,
        };
        if !same {
            return Err(UserMcpStoreError::Invalid(format!(
                "There is already an MCP server named '{name}' in the list. Remove it first or pick another name."
            )));
        }
        if existing.enabled == entry.enabled {
            return Ok(existing);
        }
        return servers
            .update(
                parse_id(&existing)?,
                UpdateUserMcpServerRequest {
                    enabled: Some(entry.enabled),
                    ..Default::default()
                },
            )
            .await
            .map_err(store_error);
    }

    let request = match catalog {
        Some(catalog) => AddUserMcpServerRequest {
            catalog: Some(catalog),
            name: Some(name.to_string()),
            enabled: Some(entry.enabled),
            ..Default::default()
        },
        None => AddUserMcpServerRequest {
            name: Some(name.to_string()),
            // Validated (SSRF, scheme) by the same path the API uses.
            url: Some(url),
            auth_mode: Some(entry.server.auth_mode),
            enabled: Some(entry.enabled),
            ..Default::default()
        },
    };
    servers.add(request).await.map_err(store_error)
}

fn start_login(server: &UserMcpServer) -> UserMcpStoreResult<McpLogin> {
    match (server.connection.status, &server.connection.provider) {
        (UserMcpConnectionStatus::NotNeeded, _) => Ok(McpLogin::NotNeeded),
        (UserMcpConnectionStatus::Connected, _) => Ok(McpLogin::AlreadyConnected),
        (UserMcpConnectionStatus::NotConnected, Some(provider)) => Ok(McpLogin::Pending {
            provider: provider.clone(),
            setup_url: SETUP_URL.into(),
            for_agent: false,
            // A person's own servers have no attachment to opt out with.
            connect_in_chat: Default::default(),
        }),
        (UserMcpConnectionStatus::NotConnected, None) => Err(UserMcpStoreError::Invalid(
            "This server signs in with an API key, which cannot be given in chat. The person can set it in Settings > My MCP servers.".into(),
        )),
    }
}

/// Sign-in for one of the agent's own MCP servers (agent, harness, session or
/// capability layer) named `name`, or None when the agent has no such server.
///
/// `user` and `user_or_service` servers sign in as the person chatting.
/// `service` servers sign in as the agent: the card routes to the agent's MCP
/// servers sheet, where only someone with MCP management permission can
/// authorize it (the existing Authorize / Ask admin split), so a person
/// without it is told to ask an admin rather than authorizing anything.
/// The attachment's `connectInChat` travels with the sign-in, so `never`
/// hands the model the same link instead of a card.
async fn agent_server_login(
    turn: &UserMcpManageTurn<'_>,
    resolved: &everruns_core::ResolvedRuntimeCapabilities,
    person: Uuid,
    name: &str,
) -> UserMcpStoreResult<Option<McpLogin>> {
    let server = merge_effective_scoped_mcp_servers(turn.harness, turn.agent, turn.session)
        .remove(name)
        .or_else(|| {
            collect_capability_mcp_servers(&resolved.resolved_capability_configs, turn.registry)
                .remove(name)
        });
    let Some(server) = server else {
        return Ok(None);
    };
    server_login(turn, person, turn.agent, server)
        .await
        .map(Some)
}

/// The connection providers `server` signs in with, as (the person's, the
/// agent's service login), or None when it needs no sign-in. Shared with the
/// session API's chat-only server listing.
pub(crate) async fn sign_in_providers(
    db: &StorageBackend,
    org_id: i64,
    server: &crate::kernel_imports::ScopedMcpServer,
) -> UserMcpStoreResult<Option<(String, String)>> {
    if server.acts_as.is_none() {
        return Ok(None);
    }
    Ok(match &server.preset {
        Some(preset) => {
            let row = db
                .get_mcp_server_by_name(org_id, preset.catalog_name())
                .await
                .map_err(internal)?
                .filter(|row| row.status == "active")
                .ok_or_else(|| UserMcpStoreError::Invalid(PRESET_GONE.into()))?;
            let provider = everruns_core::mcp_oauth_provider_id_for_uuid(row.id.uuid());
            let backing =
                super::McpServerService::settings_from_row(&row).service_connection_provider;
            Some((provider.clone(), backing.unwrap_or(provider)))
        }
        None => server
            .oauth_provider_id
            .clone()
            .map(|provider| (provider.clone(), provider)),
    })
}

/// Sign-in for one server definition: as the person for `user` and
/// `user_or_service`, otherwise as `agent` (its service virtual user).
async fn server_login(
    turn: &UserMcpManageTurn<'_>,
    person: Uuid,
    agent: Option<&Agent>,
    server: crate::kernel_imports::ScopedMcpServer,
) -> UserMcpStoreResult<McpLogin> {
    let Some((provider, service_provider)) =
        sign_in_providers(turn.db, turn.org_id, &server).await?
    else {
        return Ok(McpLogin::NotNeeded);
    };
    let connected = |identity: everruns_contracts::typed_id::VirtualUserId, provider: String| async move {
        turn.db
            .get_virtual_user_connection(identity, &provider)
            .await
            .map(|row| row.is_some())
            .map_err(internal)
    };
    if server.acts_as.uses_user_grant() {
        let person = everruns_contracts::typed_id::VirtualUserId::from_uuid(person);
        if connected(person, provider.clone()).await? {
            return Ok(McpLogin::AlreadyConnected);
        }
        return Ok(McpLogin::Pending {
            provider,
            setup_url: SETUP_URL.into(),
            for_agent: false,
            connect_in_chat: server.connect_in_chat,
        });
    }
    let agent = agent.ok_or_else(|| {
        UserMcpStoreError::Unavailable(
            "This server signs in as an agent, and no agent is answering.".into(),
        )
    })?;
    if let Some(identity) = agent.service_virtual_user_id
        && connected(identity, service_provider).await?
    {
        return Ok(McpLogin::AlreadyConnected);
    }
    Ok(McpLogin::Pending {
        provider,
        setup_url: format!("/agents/{}?tab=mcp", agent.public_id),
        for_agent: true,
        connect_in_chat: server.connect_in_chat,
    })
}

/// The person's chat-only servers in this session.
async fn chat_servers(turn: &UserMcpManageTurn<'_>) -> Vec<SessionMcpServer> {
    let Some(storage) = turn.storage else {
        return Vec::new();
    };
    everruns_core::load_session_mcp_servers(storage, turn.session.id)
        .await
        .into_iter()
        .filter(|record| record.source == SessionMcpServerSource::UserMcp)
        .collect()
}

/// The chat-only server named `name`, if there is one.
async fn chat_server(turn: &UserMcpManageTurn<'_>, name: &str) -> Option<SessionMcpServer> {
    everruns_core::get_session_mcp_server(turn.storage?, turn.session.id, name)
        .await
        .filter(|record| record.source == SessionMcpServerSource::UserMcp)
}

/// Build, check and write a chat-only server.
async fn add_to_chat(
    turn: &UserMcpManageTurn<'_>,
    clashes: &Clashes,
    name: &str,
    requested: crate::kernel_imports::ScopedMcpServer,
    custom_urls_allowed: bool,
) -> UserMcpStoreResult<SessionMcpServer> {
    let storage = turn.storage.ok_or_else(|| {
        UserMcpStoreError::Unavailable(
            "This conversation cannot hold MCP servers of its own here.".into(),
        )
    })?;
    if !requested.headers.is_empty()
        || requested.auth_mode == McpServerAuthMode::ApiKey
        || requested.command.is_some()
    {
        return Err(UserMcpStoreError::Invalid(
            "Servers added in chat cannot carry keys, headers or commands. The person can add those in Settings > My MCP servers.".into(),
        ));
    }
    // A session server would win over the agent's own server of that name.
    if clashes.skipped(name).is_some() {
        return Err(UserMcpStoreError::Invalid(format!(
            "The agent already has an MCP server named '{name}'. Pick another name."
        )));
    }
    if let Some(existing) =
        everruns_core::get_session_mcp_server(storage, turn.session.id, name).await
        && existing.source != SessionMcpServerSource::UserMcp
    {
        return Err(UserMcpStoreError::Invalid(format!(
            "This conversation already has an MCP server named '{name}'. Pick another name."
        )));
    }

    let mut server = match &requested.preset {
        Some(preset) => {
            let row = turn
                .db
                .get_mcp_server_by_name(turn.org_id, preset.catalog_name())
                .await
                .map_err(internal)?
                .filter(|row| row.status == "active")
                .ok_or_else(|| {
                    UserMcpStoreError::Invalid(format!(
                        "There is no server named '{}' in the organization's MCP catalog.",
                        preset.catalog_name()
                    ))
                })?;
            // Signs in exactly as the same catalog server in the person's list.
            let oauth = super::McpServerService::settings_from_row(&row).auth_mode
                == McpServerAuthMode::OAuth;
            crate::kernel_imports::ScopedMcpServer {
                preset: Some(preset.clone()),
                acts_as: if oauth {
                    McpServerActsAs::User
                } else {
                    McpServerActsAs::None
                },
                ..Default::default()
            }
        }
        None if !custom_urls_allowed => {
            return Err(UserMcpStoreError::Invalid(
                "This agent can only add servers from the organization's MCP catalog.".into(),
            ));
        }
        None if requested.auth_mode == McpServerAuthMode::OAuth => {
            return Err(UserMcpStoreError::Invalid(
                "A server that signs in with OAuth needs a place to keep its sign-in: add it to the person's list instead of this conversation only.".into(),
            ));
        }
        None => crate::kernel_imports::ScopedMcpServer {
            url: requested.url.trim().to_string(),
            ..Default::default()
        },
    };
    // A person's servers load on demand (D6), wherever they were added.
    server.deferred = true;
    let single =
        crate::kernel_imports::ScopedMcpServers::from([(name.to_string(), server.clone())]);
    super::scoped_mcp::validate_scoped_mcp_servers_for_org(turn.db, turn.org_id, &single)
        .await
        .map_err(|error| UserMcpStoreError::Invalid(error.to_string()))?;

    let record = SessionMcpServer {
        name: name.to_string(),
        server,
        source: SessionMcpServerSource::UserMcp,
    };
    everruns_core::put_session_mcp_server(storage, turn.session.id, &record)
        .await
        .map_err(|error| UserMcpStoreError::Internal(error.to_string()))?;
    Ok(record)
}

/// How a chat-only server reads in the manage tools.
async fn chat_summary(
    turn: &UserMcpManageTurn<'_>,
    person: Uuid,
    record: &SessionMcpServer,
) -> UserMcpStoreResult<UserMcpServerSummary> {
    let login = match server_login(turn, person, None, record.server.clone()).await {
        Ok(McpLogin::NotNeeded) => UserMcpLoginStatus::NotNeeded,
        Ok(McpLogin::AlreadyConnected | McpLogin::Completed) => UserMcpLoginStatus::Connected,
        Ok(McpLogin::Pending { .. }) | Err(UserMcpStoreError::Invalid(_)) => {
            UserMcpLoginStatus::NotConnected
        }
        Err(error) => return Err(error),
    };
    Ok(UserMcpServerSummary {
        name: record.name.clone(),
        enabled: true,
        catalog: record
            .server
            .preset
            .as_ref()
            .map(|preset| preset.catalog_name().to_string()),
        url: record.server.url.clone(),
        login,
        skipped: None,
        chat_only: true,
    })
}

async fn find(
    servers: &UserMcpServers<'_>,
    name: &str,
) -> UserMcpStoreResult<Option<UserMcpServer>> {
    Ok(servers
        .list()
        .await
        .map_err(store_error)?
        .into_iter()
        .find(|server| server.name == name))
}

fn parse_id(server: &UserMcpServer) -> UserMcpStoreResult<Uuid> {
    server
        .id
        .parse::<everruns_contracts::typed_id::McpServerId>()
        .map(|id| id.uuid())
        .map_err(|error| UserMcpStoreError::Internal(format!("bad MCP server id: {error}")))
}

fn internal(error: anyhow::Error) -> UserMcpStoreError {
    UserMcpStoreError::Internal(error.to_string())
}

fn store_error(error: UserMcpServerError) -> UserMcpStoreError {
    match error {
        UserMcpServerError::Invalid(message) | UserMcpServerError::Conflict(message) => {
            UserMcpStoreError::Invalid(message)
        }
        UserMcpServerError::NotFound => UserMcpStoreError::NotFound(String::new()),
        UserMcpServerError::Internal(error) => internal(error),
    }
}

/// Names a user server loses to in this turn, and to what.
struct Clashes {
    agent: HashSet<String>,
    capability: HashSet<String>,
}

impl Clashes {
    fn of(
        turn: &UserMcpManageTurn<'_>,
        resolved: &[crate::kernel_imports::AgentCapabilityConfig],
    ) -> Self {
        Self {
            agent: merge_effective_scoped_mcp_servers(turn.harness, turn.agent, turn.session)
                .into_keys()
                .collect(),
            capability: collect_capability_mcp_servers(resolved, turn.registry)
                .into_keys()
                .collect(),
        }
    }

    fn skipped(&self, name: &str) -> Option<String> {
        if self.agent.contains(name) {
            Some(format!(
                "name clash with agent server '{name}': the agent's own server is used instead"
            ))
        } else if self.capability.contains(name) {
            Some(format!(
                "name clash with capability server '{name}': the capability's server is used instead"
            ))
        } else {
            None
        }
    }

    fn summary(&self, server: UserMcpServer) -> UserMcpServerSummary {
        UserMcpServerSummary {
            skipped: self.skipped(&server.name),
            login: match server.connection.status {
                UserMcpConnectionStatus::Connected => UserMcpLoginStatus::Connected,
                UserMcpConnectionStatus::NotConnected => UserMcpLoginStatus::NotConnected,
                UserMcpConnectionStatus::NotNeeded => UserMcpLoginStatus::NotNeeded,
            },
            catalog: server.catalog_name,
            url: server.url,
            enabled: server.enabled,
            name: server.name,
            chat_only: false,
        }
    }
}

#[cfg(test)]
mod tests;

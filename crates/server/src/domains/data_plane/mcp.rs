//! An agent's MCP servers on its org's data plane (data plane v2 step 11.2).
//!
//! A published declaration names its MCP servers the portable way: a
//! catalog preset by name, a credential as a `${MCP_<SERVER>_<KEY>}`
//! placeholder. The data plane cannot look either up, so before an agent
//! goes over, [`prepare`] turns its servers into what an executor runs:
//!
//! - A preset becomes its catalog server's address, headers and auth mode.
//! - Every credential header (a literal the author wrote, or one the
//!   catalog holds) becomes an org secret, `mcp:<BINDING>`, sealed with the
//!   org's data key, and the header the placeholder `${BINDING}`. The
//!   executor fills it per call (`crates/agentd/src/mcp.rs`).
//! - A server that acts as the agent's service identity names the agent's
//!   grant as an `mcp_oauth:<row id>` connection, its token secret the
//!   grant as a self-contained OAuth token set. The data plane refreshes it
//!   (`RefreshOwner::DataPlane`) and reports each refresh on the feed;
//!   [`refreshed`] stores it back on the platform's row.
//!
//! Decisions:
//! - A binding is named for the agent key as well as the server and header
//!   (`MCP_<12 hex of sha256(agent key)>_<SERVER>_<KEY>`): two agents with a
//!   server of the same name never share a secret.
//! - A placeholder the author wrote (`${DOCS_KEY}`) stays: it names a
//!   secret the org sets itself (`mcp:DOCS_KEY`), and with none it is sent
//!   as written, as the platform's workers send it.
//! - A secret is sealed again only when its value changed, and a grant
//!   only when its tokens did (the data plane's own refreshes come back as
//!   the same tokens). An agent's bindings it no longer has are removed.
//! - A server whose grant depends on the turn keeps its OAuth provider and
//!   identity, and the data plane asks for its token per use of the
//!   credentials server (`calls::MCP_CONNECTION_TOKEN`, parity gap 8):
//!   one that acts as the turn's person (`user`, and `user_or_service`,
//!   whose person comes first), and a service one with no grant of the
//!   agent's to push (a connection-backed preset, whose token is the org's
//!   connection). The control plane answers as a v1 worker's lookup does,
//!   and refreshes the grant. With no grant the call is unauthorized, as on
//!   the platform.
//! - A preset that does not resolve (removed from the catalog) leaves its
//!   server out of the copy rather than failing the agent.
//! - Only agents with MCP servers pay for this: the rest of a session
//!   create does not resolve anything.

use std::collections::HashSet;

use everruns_contracts::dataplane::{self as dp, MCP_OAUTH, mcp_binding_secret};
use everruns_contracts::dataplane_seal as seal;
use everruns_contracts::typed_id::{AgentId, HarnessId, VirtualUserId};
use everruns_core::agent_package::{binding, sensitive_key};
use everruns_core::mcp::oauth::TokenSet;
use everruns_core::{McpServerActsAs, McpServerAuthMode, ScopedMcpServer, ScopedMcpServers};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use super::providers::sealed_key;
use super::{ControlPlane, put_connection, put_secret, remove};
use crate::domains::mcp_servers::McpServerService;
use crate::domains::mcp_servers::scoped_mcp::resolve_matched_scoped_mcp_server;
use crate::storage::{DataPlaneGrant, DbConnectionResolver};

/// What an `mcp_oauth` connection's settings carry: where its refreshes
/// are stored back on the platform. The data plane does not read them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GrantSettings {
    pub connection_id: Uuid,
    pub virtual_user_id: VirtualUserId,
    pub provider: String,
    pub config_hash: String,
}

/// The binding name for `agent_key`'s server `server` header `key`.
pub fn binding_name(agent_key: &str, server: &str, key: &str) -> String {
    use sha2::{Digest, Sha256};
    let agent = &hex::encode(Sha256::digest(agent_key.as_bytes()))[..12];
    format!("MCP_{agent}_{server}_{key}")
        .to_ascii_uppercase()
        .replace(|c: char| !c.is_ascii_alphanumeric(), "_")
}

/// Turn `package`'s MCP servers into what `org_id`'s data plane runs, and
/// put the secrets and connections they name.
pub async fn prepare(
    cp: &ControlPlane,
    org_id: i64,
    agent_key: &str,
    package: &mut Value,
) -> anyhow::Result<()> {
    if declared_servers(package)?.is_none() {
        return Ok(());
    }
    let target = cp
        .db
        .engine_agent_key_target(agent_key)
        .await?
        .ok_or_else(|| anyhow::anyhow!("engine agent key {agent_key} vanished"))?;
    let agent_id = target.agent_id.map(AgentId::from_uuid);
    let layers = crate::domains::agent_definitions::resolve(
        &cp.db,
        org_id,
        HarnessId::from_uuid(target.harness_id),
        agent_id,
    )
    .await?;
    prepare_layers(cp, org_id, agent_key, agent_id, &layers, package).await
}

/// `package`'s MCP servers and the field that holds them, if it has any.
/// The canonical package spells it `mcpServers`; the manifest also reads
/// `mcp_servers`.
fn declared_servers(package: &Value) -> anyhow::Result<Option<(&'static str, ScopedMcpServers)>> {
    let Some(field) = ["mcpServers", "mcp_servers"]
        .into_iter()
        .find(|field| package.get(*field).is_some_and(|v| !v.is_null()))
    else {
        return Ok(None);
    };
    let servers: ScopedMcpServers = serde_json::from_value(package[field].clone())?;
    Ok((!servers.is_empty()).then_some((field, servers)))
}

/// [`prepare`] for a declaration compiled from `layers` (the agent's, or a
/// session's own: its agent's with the session's settings on top). Its
/// bindings are named for `binding_key`: the agent key, or the session's
/// own key, so a session's servers never share a secret with its agent's.
pub async fn prepare_layers(
    cp: &ControlPlane,
    org_id: i64,
    binding_key: &str,
    agent_id: Option<AgentId>,
    layers: &crate::domains::agent_definitions::PlatformLayers,
    package: &mut Value,
) -> anyhow::Result<()> {
    let agent_key = binding_key;
    let Some((field, mut servers)) = declared_servers(package)? else {
        return Ok(());
    };
    let written = crate::domains::agent_definitions::mcp_servers(layers);
    let catalog = McpServerService::new(cp.db.clone(), cp.encryption.clone());
    let grants = DbConnectionResolver::new(
        cp.db.as_ref().clone(),
        cp.encryption()?.clone(),
        None,
        catalog.egress_service(),
    );
    let mut live = HashSet::new();
    let mut unresolved = Vec::new();
    for (name, server) in servers.iter_mut() {
        let Some(original) = written.get(name.as_str()) else {
            continue;
        };
        let resolved = match resolve_matched_scoped_mcp_server(
            &catalog,
            org_id,
            Uuid::nil(),
            Some((name.clone(), original.clone())),
        )
        .await
        {
            Ok(Some(resolved)) => resolved,
            // A preset the catalog no longer has: the agent runs without
            // the server, as a turn on the platform's workers fails only
            // that server's tools.
            outcome => {
                let error = outcome.err().map(|e| e.to_string()).unwrap_or_default();
                tracing::warn!(org_id, agent_key, server = %name, %error, "MCP server left out of the data plane's copy");
                unresolved.push(name.clone());
                continue;
            }
        };
        let mut headers = resolved.headers.clone();
        if let Some(api_key) = &resolved.api_key
            && !headers
                .keys()
                .any(|k| k.eq_ignore_ascii_case("authorization"))
        {
            headers.insert("Authorization".into(), format!("Bearer {api_key}"));
        }
        if original.preset.is_some() {
            server.url = resolved.url.clone();
            server.protocol_mode = resolved.protocol_mode;
            server.elicitation_policy = resolved.elicitation_policy;
            server.tool_timeout_ms = resolved.tool_timeout_ms;
            server.preset = None;
        }
        server.acts_as = McpServerActsAs::default();
        server.headers.clear();
        for (key, value) in headers {
            if sensitive_key(&key) && !binding(&value) {
                let name = binding_name(agent_key, name, &key);
                let secret = mcp_binding_secret(&name);
                if sealed_key(cp, org_id, &secret).await?.as_deref() != Some(value.as_bytes()) {
                    put_secret(cp, org_id, &secret, value.as_bytes()).await?;
                }
                live.insert(secret);
                server.headers.insert(key, format!("${{{name}}}"));
            } else {
                server.headers.insert(key, value);
            }
        }
        server.auth_mode = McpServerAuthMode::None;
        server.oauth_provider_id = None;
        if resolved.auth_mode == McpServerAuthMode::OAuth
            && !resolved.acts_as.is_none()
            && let Some(provider) = resolved.oauth_provider_id.as_deref()
            && !server
                .headers
                .keys()
                .any(|k| k.eq_ignore_ascii_case("authorization"))
        {
            // The agent's own grant goes over, refreshed on the data plane;
            // anything else is the turn's to ask for.
            let pushed = match (resolved.acts_as, agent_id) {
                (McpServerActsAs::Service, Some(agent_id)) => grants
                    .agent_service_mcp_grant(org_id, agent_id, provider)
                    .await
                    .map_err(|e| anyhow::anyhow!("MCP server {name}: {e}"))?,
                _ => None,
            };
            server.auth_mode = McpServerAuthMode::OAuth;
            match pushed {
                Some(grant) => {
                    server.oauth_provider_id = Some(put_grant(cp, org_id, &grant).await?);
                }
                None => {
                    server.oauth_provider_id = Some(provider.to_owned());
                    server.acts_as = resolved.acts_as;
                }
            }
        }
        normalize(server);
    }
    for name in unresolved {
        servers.remove(&name);
    }
    // The agent's bindings it no longer has.
    let own = mcp_binding_secret(&binding_name(agent_key, "", ""));
    let own = own.trim_end_matches('_');
    for item in cp.db.dp_items_live(org_id).await? {
        if item.kind == "secret" && item.item_id.starts_with(own) && !live.contains(&item.item_id) {
            remove(&cp.db, org_id, "secret", &item.item_id).await?;
        }
    }
    package[field] = serde_json::to_value(&servers)?;
    Ok(())
}

/// Defaults a server's wire form leaves out stay out: equal servers are
/// equal bytes, however they were resolved.
fn normalize(server: &mut ScopedMcpServer) {
    server.connect_in_chat = Default::default();
}

/// Put `grant` as an `mcp_oauth` connection and its token secret; the
/// connection id.
async fn put_grant(
    cp: &ControlPlane,
    org_id: i64,
    grant: &DataPlaneGrant,
) -> anyhow::Result<String> {
    let id = format!("{MCP_OAUTH}:{}", grant.connection_id);
    let held = sealed_key(cp, org_id, &id)
        .await?
        .and_then(|plain| serde_json::from_slice::<TokenSet>(&plain).ok());
    let same = held.is_some_and(|held| {
        held.access_token == grant.tokens.access_token
            && held.refresh_token == grant.tokens.refresh_token
    });
    if !same {
        put_secret(cp, org_id, &id, &serde_json::to_vec(&grant.tokens)?).await?;
    }
    let connection = dp::Connection {
        connection_id: id.clone(),
        kind: MCP_OAUTH.to_owned(),
        settings: serde_json::to_value(GrantSettings {
            connection_id: grant.connection_id,
            virtual_user_id: grant.virtual_user_id,
            provider: grant.provider.clone(),
            config_hash: grant.config_hash.clone(),
        })?,
        token_secret_id: id.clone(),
        refresh_owner: dp::RefreshOwner::DataPlane,
    };
    let current = cp.db.dp_item(org_id, "connection", &id).await?;
    if current.and_then(|item| item.body)
        != Some(dp::ConfigChange::Connection(connection.clone()).to_value())
    {
        put_connection(&cp.db, org_id, connection).await?;
    }
    Ok(id)
}

/// The data plane refreshed `sealed`: store the new tokens on the
/// platform's row, before the secret itself is kept, so a publish in
/// between never seals the spent ones again. Anything else is left alone.
pub async fn refreshed(
    cp: &ControlPlane,
    org_id: i64,
    sealed: &dp::SealedSecret,
) -> anyhow::Result<()> {
    if !sealed.secret_id.starts_with(&format!("{MCP_OAUTH}:")) {
        return Ok(());
    }
    let id = &sealed.secret_id;
    let held = cp.db.dp_item(org_id, "secret", id).await?;
    if held
        .as_ref()
        .is_some_and(|item| item.revision >= sealed.revision as i64)
    {
        return Ok(());
    }
    let Some(settings) = cp
        .db
        .dp_item(org_id, "connection", id)
        .await?
        .and_then(|item| item.body)
        .and_then(|body| match dp::ConfigChange::parse(&body) {
            Some(dp::ConfigChange::Connection(c)) => {
                serde_json::from_value::<GrantSettings>(c.settings).ok()
            }
            _ => None,
        })
    else {
        return Ok(());
    };
    let Some(org) = cp.db.dp_org(org_id).await? else {
        return Ok(());
    };
    let key = cp.data_key(&org)?;
    let tokens: TokenSet =
        serde_json::from_slice(&seal::unseal(&key, &super::org_public(org_id), sealed)?)?;
    let Some(previous) = sealed_key(cp, org_id, id)
        .await?
        .and_then(|plain| serde_json::from_slice::<TokenSet>(&plain).ok())
    else {
        return Ok(());
    };
    let grants = DbConnectionResolver::new(
        cp.db.as_ref().clone(),
        cp.encryption()?.clone(),
        None,
        McpServerService::new(cp.db.clone(), cp.encryption.clone()).egress_service(),
    );
    let grant = DataPlaneGrant {
        connection_id: settings.connection_id,
        virtual_user_id: settings.virtual_user_id,
        provider: settings.provider,
        config_hash: settings.config_hash,
        tokens,
    };
    let stored = grants
        .store_data_plane_refresh(&grant, &previous.access_token)
        .await
        .map_err(|e| anyhow::anyhow!("storing a data plane refresh: {e}"))?;
    if !stored {
        tracing::info!(org_id, secret = %id, "a data plane refresh was not stored: the grant moved on");
    }
    Ok(())
}

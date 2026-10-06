// The chatting person's own MCP servers, as a scoped-MCP layer for one turn.
//
// Spec: knowledge/integrations/user-mcp-servers.md (D1, D2).
//
// Decision: this layer is trusted (it comes from the person's own records, not
// from request input), so it skips the inline-OAuth strip applied to explicit
// harness/agent/session servers, and it merges *under* capability-contributed
// and explicit servers: an agent server with the same name always wins.
//
// It is empty unless the agent has the `user_mcp` capability with `use` on,
// the turn has a verified initiating person (an end-user virtual user recorded
// for the input message), and no other person takes part in the session.
// Every failure degrades to an empty layer: the person's servers are an
// addition to the turn and must never break it.

use crate::kernel_imports::{
    McpServerActsAs, McpServerAuthMode, ScopedMcpServer, ScopedMcpServers,
    merge_scoped_mcp_servers, resolve_runtime_capabilities,
};
use crate::records::{Agent, Harness, Session};
use crate::storage::{EncryptionService, StorageBackend, UserMcpServerRow};
use everruns_capabilities::capabilities::{USER_MCP_CAPABILITY_ID, user_mcp_use_enabled};
use everruns_core::capabilities::{CapabilityRegistry, collect_capability_mcp_servers};
use everruns_core::mcp_oauth_provider_id_for_uuid;
use uuid::Uuid;

use super::McpServerService;
use super::scoped_mcp::{merge_effective_scoped_mcp_servers, validate_effective_mcp_servers};

/// Inputs for resolving the user layer of one turn.
pub struct UserMcpTurn<'a> {
    pub db: &'a StorageBackend,
    pub encryption: Option<&'a EncryptionService>,
    pub org_id: i64,
    pub harness: &'a Harness,
    pub agent: Option<&'a Agent>,
    pub session: &'a Session,
    pub registry: &'a CapabilityRegistry,
    /// The turn's input message; None for unattended work.
    pub input_message: Option<Uuid>,
}

/// Whether the agent's resolved capabilities ask for the person's servers.
fn wants_user_servers(turn: &UserMcpTurn<'_>) -> bool {
    let resolved = resolve_runtime_capabilities(
        &turn.harness.definition(),
        turn.agent.map(|agent| agent.definition()).as_ref(),
        &turn.session.execution_session(),
        turn.registry,
    );
    resolved
        .resolved_capability_configs
        .iter()
        .find(|config| config.capability_id() == USER_MCP_CAPABILITY_ID)
        .is_some_and(|config| user_mcp_use_enabled(config.config_value()))
}

/// The initiating person's enabled MCP servers for this turn, or an empty
/// layer when the turn should not see them.
pub async fn user_mcp_layer(turn: &UserMcpTurn<'_>) -> ScopedMcpServers {
    match try_user_mcp_layer(turn).await {
        Ok(layer) => layer,
        Err(error) => {
            tracing::warn!(%error, session_id = %turn.session.id, "Skipping user MCP servers for this turn");
            ScopedMcpServers::new()
        }
    }
}

async fn try_user_mcp_layer(turn: &UserMcpTurn<'_>) -> anyhow::Result<ScopedMcpServers> {
    let Some(message) = turn.input_message else {
        return Ok(ScopedMcpServers::new());
    };
    if !wants_user_servers(turn) {
        return Ok(ScopedMcpServers::new());
    }
    let Some(person) = turn
        .db
        .runtime_invocation_subject(turn.session.id, message)
        .await?
    else {
        return Ok(ScopedMcpServers::new());
    };
    let people = turn
        .db
        .list_session_participants(turn.org_id, turn.session.id)
        .await?
        .into_iter()
        .filter(|p| p.kind == "user" && p.left_at.is_none())
        .map(|p| p.principal_id)
        .collect::<std::collections::HashSet<_>>();
    if people.len() > 1 {
        return Ok(ScopedMcpServers::new());
    }

    let mut layer = ScopedMcpServers::new();
    for row in turn
        .db
        .list_user_mcp_servers(turn.org_id, person.uuid())
        .await?
    {
        if row.row.status != "active" {
            continue;
        }
        let name = row.row.name.clone();
        let Some(server) = scoped_server(turn, row).await? else {
            continue;
        };
        // One bad entry must not take the whole turn's MCP tools down.
        let single = ScopedMcpServers::from([(name.clone(), server.clone())]);
        if validate_effective_mcp_servers(&single).is_ok() {
            layer.insert(name, server);
        }
    }
    Ok(layer)
}

async fn scoped_server(
    turn: &UserMcpTurn<'_>,
    user: UserMcpServerRow,
) -> anyhow::Result<Option<ScopedMcpServer>> {
    if let Some(catalog_id) = user.catalog_mcp_server_id {
        // A catalog server signs in exactly as its preset does; a preset that
        // has been archived simply drops out.
        let Some(preset) = turn
            .db
            .get_mcp_server(turn.org_id, catalog_id)
            .await?
            .filter(|preset| preset.status == "active")
        else {
            return Ok(None);
        };
        let acts_as =
            if McpServerService::settings_from_row(&preset).auth_mode == McpServerAuthMode::OAuth {
                McpServerActsAs::User
            } else {
                McpServerActsAs::None
            };
        return Ok(Some(ScopedMcpServer {
            preset: Some(
                format!("catalog:{}", preset.name)
                    .parse()
                    .map_err(anyhow::Error::msg)?,
            ),
            acts_as,
            ..Default::default()
        }));
    }

    let row = user.row;
    let settings = McpServerService::settings_from_row(&row);
    let mut headers: std::collections::HashMap<String, String> =
        serde_json::from_value(row.headers.clone()).unwrap_or_default();
    let mut server = ScopedMcpServer {
        url: row.url.clone(),
        protocol_mode: settings.protocol_mode,
        elicitation_policy: settings.elicitation_policy,
        ..Default::default()
    };
    match settings.auth_mode {
        McpServerAuthMode::OAuth => {
            server.auth_mode = McpServerAuthMode::OAuth;
            server.acts_as = McpServerActsAs::User;
            server.oauth_provider_id = Some(mcp_oauth_provider_id_for_uuid(row.id.uuid()));
        }
        McpServerAuthMode::ApiKey => {
            let (Some(encrypted), Some(encryption)) =
                (row.api_key_encrypted.as_deref(), turn.encryption)
            else {
                return Ok(None);
            };
            // The key belongs to the person whose turn this is; it travels as a
            // literal header, which only `actsAs: none` servers keep.
            headers.retain(|key, _| !key.eq_ignore_ascii_case("authorization"));
            headers.insert(
                "Authorization".to_string(),
                format!("Bearer {}", encryption.decrypt_to_string(encrypted)?),
            );
        }
        _ => {}
    }
    server.headers = headers;
    Ok(Some(server))
}

/// Effective scoped servers for a turn: the person's servers, under
/// capability-contributed servers, under explicit harness/agent/session ones.
pub fn merge_turn_scoped_mcp_servers(
    harness: &Harness,
    agent: Option<&Agent>,
    session: &Session,
    registry: &CapabilityRegistry,
    user_layer: &ScopedMcpServers,
) -> ScopedMcpServers {
    let explicit = merge_effective_scoped_mcp_servers(harness, agent, session);
    let resolved = resolve_runtime_capabilities(
        &harness.definition(),
        agent.map(|a| a.definition()).as_ref(),
        &session.execution_session(),
        registry,
    );
    let contributed =
        collect_capability_mcp_servers(&resolved.resolved_capability_configs, registry);
    merge_scoped_mcp_servers(
        &merge_scoped_mcp_servers(user_layer, &contributed),
        &explicit,
    )
}

#[cfg(test)]
#[path = "user_layer_tests.rs"]
mod tests;

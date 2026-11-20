//! The person's own MCP servers (`user_mcp`) for a v2 session (data plane
//! v2 parity, Phase B: Console Chat).
//!
//! Decisions:
//! - The person's server list stays in the control plane: it is theirs
//!   across every chat and agent of the org, so no session's cell owns it.
//!   A cell reaches it with two host calls, both acting as the turn's
//!   person ([`USER_MCP_STORE`], [`USER_MCP_SERVERS`]). As for every such
//!   call, the person is the one the session's cell recorded for the open
//!   turn, and the dispatcher records the turn's invocation first
//!   (`host_calls::record_turn`); the call itself names no one
//!   (THREAT[TM-AGENT-017]).
//! - The manage tools run the v1 worker's own store
//!   (`invoke_user_mcp_store_for_session`), with its checks: the agent must
//!   have `user_mcp` with `manage` on, and the turn a single person.
//! - "Add to this chat only" keeps a server in the session's storage, which
//!   for a v2 session lives in its cell: the store reads and writes it there
//!   (`session_servers::session_store`).
//! - The session's own MCP records (chat-only servers and ARD MCP targets,
//!   `everruns_core::session_mcp_servers`) join the same answer when the
//!   cell says the session has any (`session_servers: true`). They are read
//!   from the cell here, never taken from the call, so an executor cannot
//!   name a server the session does not have. As on v1, where they fold into
//!   the session's own `mcpServers`, a record wins a name clash with the
//!   person's server, and they need no person: an unattended turn gets them
//!   too.
//! - The turn's layer is the v1 one (`user_mcp_layer`: `use` on, a single
//!   person, their enabled servers), resolved into what an executor runs:
//!   a catalog preset becomes its server's address and headers, a key a
//!   literal `Authorization` header (the answer is sealed for the org, and
//!   the executor keeps it for the turn only), and a server that signs in
//!   as the person keeps its OAuth provider, so its token is a
//!   `MCP_CONNECTION_TOKEN` call per use that the control plane checks
//!   against this same layer.
//! - A server that does not resolve (a preset archived meanwhile) is left
//!   out, as on v1 one bad entry never takes the turn's tools down.

use everruns_contracts::host_calls::{USER_MCP_SERVERS, USER_MCP_STORE};
use everruns_contracts::typed_id::SessionId;
use everruns_core::mcp::{UserMcpStoreCall, UserMcpStoreError};
use everruns_core::{McpServerActsAs, McpServerAuthMode, ScopedMcpServers};
use serde_json::{Value, json};
use uuid::Uuid;

use everruns_core::merge_scoped_mcp_servers;
use everruns_core::session_mcp_servers::{SESSION_MCP_SERVER_KV_PREFIX, SessionMcpServer};

use crate::domains::common::Ctx;
use crate::domains::mcp_servers::McpServerService;
use crate::domains::mcp_servers::scoped_mcp::resolve_matched_scoped_mcp_server;
use crate::domains::mcp_servers::session_servers::session_store;
use crate::domains::mcp_servers::user_layer::{UserMcpTurn, user_mcp_layer};
use crate::domains::mcp_servers::user_manage::{
    ManageTurnRecords, invoke_user_mcp_store_for_session,
};
use crate::domains::session_storage::plane::PlaneStorage;

/// The session and the turn's input message a call names (the session's
/// cell has put the open turn's message in place of `"open"`).
fn turn(params: &Value) -> Result<(SessionId, Option<Uuid>), String> {
    let session = params["session_id"]
        .as_str()
        .and_then(|id| SessionId::parse(id).ok())
        .ok_or("invalid session_id")?;
    let message = params["input_message_id"]
        .as_str()
        .and_then(|id| Uuid::parse_str(id).ok());
    Ok((session, message))
}

/// Answer a [`USER_MCP_STORE`] call: `{"ok": reply}` or `{"error": ...}`.
pub async fn store(ctx: &Ctx, params: &Value) -> Value {
    let outcome = async {
        let (session, message) = turn(params).map_err(UserMcpStoreError::Internal)?;
        let call: UserMcpStoreCall = serde_json::from_value(params["call"].clone())
            .map_err(|error| UserMcpStoreError::Internal(format!("invalid call: {error}")))?;
        let storage = session_store(ctx, session)
            .await
            .map_err(|error| UserMcpStoreError::Internal(error.to_string()))?;
        invoke_user_mcp_store_for_session(
            &ctx.db,
            ctx.encryption.as_deref(),
            ctx.capability_service.registry(),
            ctx.org_id(),
            session,
            message,
            Some(storage.as_ref()),
            call,
        )
        .await
    }
    .await;
    match outcome {
        Ok(reply) => json!({ "ok": reply }),
        Err(error) => {
            if let UserMcpStoreError::Internal(message) = &error {
                tracing::warn!(call = USER_MCP_STORE, %message, "user MCP store call failed");
            }
            json!({ "error": error })
        }
    }
}

/// Answer a [`USER_MCP_SERVERS`] call: `{"ok": servers}` (empty when the
/// turn should not see the person's servers) or `{"error": message}`.
pub async fn servers(ctx: &Ctx, params: &Value) -> Value {
    match layer(ctx, params).await {
        Ok(servers) => json!({ "ok": servers }),
        Err(message) => json!({ "error": message }),
    }
}

async fn layer(ctx: &Ctx, params: &Value) -> Result<ScopedMcpServers, String> {
    let (session, message) = turn(params)?;
    let person = match message {
        Some(message) => person_layer(ctx, session, message).await?,
        None => ScopedMcpServers::new(),
    };
    let records = if params["session_servers"].as_bool() == Some(true) {
        session_records(ctx, session).await
    } else {
        ScopedMcpServers::new()
    };
    let layer = merge_scoped_mcp_servers(&person, &records);
    if layer.is_empty() {
        return Ok(layer);
    }
    resolve(ctx, session, layer).await
}

/// The person's own servers for the turn of `message` (`user_mcp_layer`).
async fn person_layer(
    ctx: &Ctx,
    session: SessionId,
    message: Uuid,
) -> Result<ScopedMcpServers, String> {
    let org_id = ctx.org_id();
    // Another org's session, or a turn not recorded: nothing.
    let records = match ManageTurnRecords::load(&ctx.db, org_id, session, Some(message)).await {
        Ok(Some(records)) => records,
        Ok(None) => return Err("session not found".into()),
        Err(error) => {
            tracing::warn!(call = USER_MCP_SERVERS, %error, "no turn for the person's MCP servers");
            return Ok(ScopedMcpServers::new());
        }
    };
    Ok(user_mcp_layer(&UserMcpTurn {
        db: &ctx.db,
        encryption: ctx.encryption.as_deref(),
        org_id,
        harness: &records.harness,
        agent: records.agent.as_ref(),
        session: &records.session,
        registry: ctx.capability_service.registry(),
        input_message: Some(message),
    })
    .await)
}

/// The session's own MCP records, read from its cell (none for a session
/// of another org, or not on a data plane).
async fn session_records(ctx: &Ctx, session: SessionId) -> ScopedMcpServers {
    let store = match PlaneStorage::of(ctx, session).await {
        Ok(Some(plane)) => plane,
        Ok(None) => return ScopedMcpServers::new(),
        Err(error) => {
            tracing::warn!(call = USER_MCP_SERVERS, %error, "no session MCP records for the turn");
            return ScopedMcpServers::new();
        }
    };
    let listed = match store.listed().await {
        Ok(listed) => listed,
        Err(error) => {
            tracing::warn!(call = USER_MCP_SERVERS, %error, "no session MCP records for the turn");
            return ScopedMcpServers::new();
        }
    };
    records_of(&listed.values)
}

/// The MCP records among a session's storage values.
pub(crate) fn records_of(values: &std::collections::BTreeMap<String, String>) -> ScopedMcpServers {
    values
        .iter()
        .filter(|(key, _)| key.starts_with(SESSION_MCP_SERVER_KV_PREFIX))
        .filter_map(
            |(key, value)| match serde_json::from_str::<SessionMcpServer>(value) {
                Ok(record) => Some((record.name, record.server)),
                Err(error) => {
                    tracing::warn!(%key, %error, "a session MCP record did not parse");
                    None
                }
            },
        )
        .collect()
}

/// `layer` as an executor runs it.
async fn resolve(
    ctx: &Ctx,
    session: SessionId,
    layer: ScopedMcpServers,
) -> Result<ScopedMcpServers, String> {
    let org_id = ctx.org_id();
    let catalog = McpServerService::with_egress_service(
        ctx.db.clone(),
        ctx.encryption.clone(),
        ctx.capability_service.egress_service(),
    );
    let mut servers = ScopedMcpServers::new();
    for (name, server) in layer {
        let resolved = match resolve_matched_scoped_mcp_server(
            &catalog,
            org_id,
            session.uuid(),
            Some((name.clone(), server.clone())),
        )
        .await
        {
            Ok(Some(resolved)) => resolved,
            outcome => {
                let error = outcome.err().map(|e| e.to_string()).unwrap_or_default();
                tracing::warn!(org_id, server = %name, %error, "a person's MCP server left out of the turn");
                continue;
            }
        };
        let mut concrete = server;
        concrete.preset = None;
        concrete.url = resolved.url;
        concrete.protocol_mode = resolved.protocol_mode;
        concrete.elicitation_policy = resolved.elicitation_policy;
        concrete.tool_timeout_ms = resolved.tool_timeout_ms;
        concrete.headers = resolved.headers.into_iter().collect();
        if let Some(api_key) = resolved.api_key
            && !concrete
                .headers
                .keys()
                .any(|key| key.eq_ignore_ascii_case("authorization"))
        {
            concrete
                .headers
                .insert("Authorization".into(), format!("Bearer {api_key}"));
        }
        if resolved.auth_mode == McpServerAuthMode::OAuth && !resolved.acts_as.is_none() {
            concrete.auth_mode = McpServerAuthMode::OAuth;
            concrete.oauth_provider_id = resolved.oauth_provider_id;
            concrete.acts_as = resolved.acts_as;
        } else {
            concrete.auth_mode = McpServerAuthMode::None;
            concrete.oauth_provider_id = None;
            concrete.acts_as = McpServerActsAs::default();
        }
        servers.insert(name, concrete);
    }
    Ok(servers)
}

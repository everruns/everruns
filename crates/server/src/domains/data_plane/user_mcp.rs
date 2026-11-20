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
//!   for a v2 session lives in its cell: it is refused here until session
//!   MCP records join cell turns (parity step 9).
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

use crate::domains::common::Ctx;
use crate::domains::mcp_servers::McpServerService;
use crate::domains::mcp_servers::scoped_mcp::resolve_matched_scoped_mcp_server;
use crate::domains::mcp_servers::user_layer::{UserMcpTurn, user_mcp_layer};
use crate::domains::mcp_servers::user_manage::{
    ManageTurnRecords, invoke_user_mcp_store_for_session,
};

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
        if matches!(call, UserMcpStoreCall::AddToChat { .. }) {
            return Err(UserMcpStoreError::Unavailable(
                "adding a server to this chat only is not available here yet; add it to your servers instead".into(),
            ));
        }
        invoke_user_mcp_store_for_session(
            &ctx.db,
            ctx.encryption.as_deref(),
            ctx.capability_service.registry(),
            ctx.org_id(),
            session,
            message,
            None,
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
    let Some(message) = message else {
        return Ok(ScopedMcpServers::new());
    };
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
    let layer = user_mcp_layer(&UserMcpTurn {
        db: &ctx.db,
        encryption: ctx.encryption.as_deref(),
        org_id,
        harness: &records.harness,
        agent: records.agent.as_ref(),
        session: &records.session,
        registry: ctx.capability_service.registry(),
        input_message: Some(message),
    })
    .await;
    if layer.is_empty() {
        return Ok(layer);
    }
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

//! The MCP server a worker's tool prefix names, resolved for one org.
//!
//! Decision: one resolution serves both the `GetMcpServerByPrefix` RPC (which
//! hands the worker the server to call) and the worker MCP credential commands
//! (which re-resolve it to confirm the attachment a token request names). Two
//! copies could disagree about which server a prefix means, and a credential
//! check that disagrees with the server it guards is worse than none.
//!
//! With a session, the session's effective scope wins: the harness, agent and
//! session layers, the run-time session records, and the invoking person's own
//! servers. A named invocation must exist on the session, and its responder is
//! the agent whose layers apply. Without a scoped match the org's servers are
//! searched by prefix.

use crate::domains::mcp_servers::{McpServerResolved, McpServerService};
use crate::domains::sessions::SessionService;
use crate::storage::{EncryptionService, StorageBackend};
use everruns_contracts::typed_id::AgentId;
use everruns_core::capabilities::CapabilityRegistry;
use everruns_core::session_services::SessionStorageStore;
use std::sync::Arc;
use uuid::Uuid;

/// What resolution reads.
pub struct WorkerMcpLookup<'a> {
    pub db: &'a Arc<StorageBackend>,
    pub session_service: &'a SessionService,
    pub mcp_server_service: &'a McpServerService,
    pub registry: &'a CapabilityRegistry,
    pub encryption: Option<&'a EncryptionService>,
    /// Session storage holding the run-time attachment records, when present.
    pub storage: Option<&'a dyn SessionStorageStore>,
}

/// The server, and the agent whose credentials its secret bindings use.
pub struct WorkerMcpServer {
    pub server: McpServerResolved,
    pub runtime_agent_id: Option<AgentId>,
}

/// Why resolution failed. Messages are fixed text: no server detail leaks.
#[derive(Debug)]
pub enum WorkerMcpLookupError {
    /// The named invocation is not this session's.
    UnknownInvocation,
    Internal(&'static str),
}

pub async fn resolve(
    lookup: &WorkerMcpLookup<'_>,
    org_id: i64,
    session_id: Option<Uuid>,
    input_message: Option<Uuid>,
    server_prefix: &str,
) -> Result<Option<WorkerMcpServer>, WorkerMcpLookupError> {
    use WorkerMcpLookupError::{Internal, UnknownInvocation};
    let mut runtime_agent_id = None;
    let internal_caller = everruns_core::Caller::internal(org_id);

    if let Some(session_id) = session_id
        && let Some(mut session) = lookup
            .session_service
            .get_for_worker(&internal_caller, session_id)
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "Failed to get session for scoped MCP lookup");
                Internal("Failed to resolve scoped MCP server")
            })?
        && let Some(harness) = crate::domains::harnesses::queries::resolve_effective(
            lookup.db,
            org_id,
            session.harness_id,
        )
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "Failed to get harness for scoped MCP lookup");
            Internal("Failed to resolve scoped MCP server")
        })?
    {
        if let Some(message) = input_message {
            if !lookup
                .db
                .runtime_invocation_exists(session.id, message)
                .await
                .map_err(|_| Internal("Invocation unavailable"))?
            {
                return Err(UnknownInvocation);
            }
            session.agent_id = lookup
                .db
                .runtime_invocation_responder(session.id, message)
                .await
                .map_err(|_| Internal("Invocation unavailable"))?
                .map(AgentId::from_uuid);
        }
        runtime_agent_id = session.agent_id;
        // The same run-time records the turn context folded, so an
        // ARD-attached or chat-only server's tools resolve here too.
        if let Some(store) = lookup.storage {
            super::session_servers::fold_session_records(store, &mut session).await;
        }
        let agent = match session.agent_id {
            Some(agent_id) => crate::domains::agents::queries::get_by_public_id(
                lookup.db,
                org_id,
                &agent_id.to_string(),
            )
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "Failed to get agent for scoped MCP lookup");
                Internal("Failed to resolve scoped MCP server")
            })?,
            None => None,
        };

        let user_layer = super::user_layer::user_mcp_layer(&super::user_layer::UserMcpTurn {
            db: lookup.db,
            encryption: lookup.encryption,
            org_id,
            harness: &harness,
            agent: agent.as_ref(),
            session: &session,
            registry: lookup.registry,
            input_message,
        })
        .await;
        if let Some(server) = super::scoped_mcp::resolve_scoped_mcp_server_with_capabilities(
            lookup.mcp_server_service,
            org_id,
            &harness,
            agent.as_ref(),
            &session,
            server_prefix,
            lookup.registry,
            &user_layer,
        )
        .await
        .map_err(|error| {
            tracing::error!(%error, "Failed to resolve scoped MCP server");
            Internal("Failed to resolve scoped MCP server")
        })? {
            return Ok(Some(WorkerMcpServer {
                server,
                runtime_agent_id,
            }));
        }
    }

    let server = lookup
        .mcp_server_service
        .resolve_by_prefix(&internal_caller, server_prefix)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "Failed to resolve MCP server");
            Internal("Failed to resolve MCP server")
        })?;
    Ok(server.map(|server| WorkerMcpServer {
        server,
        runtime_agent_id,
    }))
}

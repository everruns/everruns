//! Runtime MCP wiring (specs/runtime-mcp.md D4).
//!
//! Resolves the effective scoped MCP servers for a session (harness chain →
//! agent → session, last wins), turns them into transport connections, and
//! drives discovery + execution through the shared `everruns-mcp` client.
//! Only HTTP scoped servers are wired today; stdio scoped config awaits the
//! core `ScopedMcpServer` command/args/env fields (the stdio transport itself
//! already exists behind the `mcp-stdio` feature).

use std::sync::Arc;

use everruns_core::capabilities::Capability;
use everruns_core::{
    Agent, Harness, McpCapability, ScopedMcpServer, ScopedMcpServers, Session, ToolDefinition,
    merge_scoped_mcp_servers,
};
use everruns_mcp::{McpClient, McpConnection, McpEndpoint, McpExecutor, StaticConnectionResolver};
use uuid::Uuid;

/// Merge harness-chain → agent → session scoped MCP servers (last layer wins).
pub(crate) fn merge_session_scoped_servers(
    harness_chain: &[Harness],
    agent: Option<&Agent>,
    session: &Session,
) -> ScopedMcpServers {
    let mut merged = ScopedMcpServers::default();
    for harness in harness_chain {
        merged = merge_scoped_mcp_servers(&merged, &harness.mcp_servers);
    }
    if let Some(agent) = agent {
        merged = merge_scoped_mcp_servers(&merged, &agent.mcp_servers);
    }
    merge_scoped_mcp_servers(&merged, &session.mcp_servers)
}

/// A resolved scoped server plus whether to discover its tools.
struct ResolvedServer {
    name: String,
    connection: McpConnection,
    tool_discovery: bool,
}

fn resolve_servers(servers: &ScopedMcpServers) -> Vec<ResolvedServer> {
    servers
        .iter()
        .map(|(name, server)| ResolvedServer {
            name: name.clone(),
            connection: McpConnection {
                name: name.clone(),
                endpoint: endpoint_for(server),
                auth_mode: server.auth_mode.clone(),
                oauth_provider_id: server.oauth_provider_id.clone(),
            },
            tool_discovery: server.tool_discovery,
        })
        .collect()
}

fn endpoint_for(server: &ScopedMcpServer) -> McpEndpoint {
    // `ScopedMcpServer.transport_type` only has an HTTP variant today.
    McpEndpoint::Http {
        url: server.url.clone(),
        headers: server.headers.clone(),
    }
}

/// Discover tool definitions for all scoped servers with `tool_discovery`
/// enabled, prefixed via [`McpCapability`]. Failures are logged and skipped so
/// one unreachable server doesn't fail the whole turn.
pub(crate) async fn discover_tool_definitions(
    client: &McpClient,
    session_uuid: Uuid,
    servers: &ScopedMcpServers,
) -> Vec<ToolDefinition> {
    let mut definitions = Vec::new();
    for resolved in resolve_servers(servers) {
        if !resolved.tool_discovery {
            continue;
        }
        match client.discover(&resolved.connection).await {
            Ok(tools) => {
                let id = Uuid::new_v5(&session_uuid, resolved.name.as_bytes());
                let capability = McpCapability::new(id, resolved.name.clone(), None, tools);
                definitions.extend(capability.tool_definitions());
            }
            Err(error) => {
                tracing::warn!(
                    server = %resolved.name,
                    %error,
                    "scoped MCP tool discovery failed; skipping server"
                );
            }
        }
    }
    definitions
}

/// Build an MCP executor for the session's scoped servers, or `None` when no
/// servers are configured (so callers keep the plain tool registry).
pub(crate) fn build_executor(
    client: Arc<McpClient>,
    servers: &ScopedMcpServers,
) -> Option<Arc<McpExecutor>> {
    let connections: Vec<McpConnection> = resolve_servers(servers)
        .into_iter()
        .map(|resolved| resolved.connection)
        .collect();
    if connections.is_empty() {
        return None;
    }
    let resolver = Arc::new(StaticConnectionResolver::from_connections(connections));
    Some(Arc::new(McpExecutor::new(client, resolver)))
}

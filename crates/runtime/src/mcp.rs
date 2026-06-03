//! Runtime MCP wiring (specs/runtime-mcp.md D4).
//!
//! Resolves the effective scoped MCP servers for a session (harness chain →
//! agent → session, last wins), turns them into transport connections, and
//! drives discovery + execution through the shared `everruns-mcp` client.
//! HTTP scoped servers are always wired; stdio scoped servers are wired only
//! when the crate is built with the `mcp-stdio` feature (off in hosted
//! builds), and are otherwise skipped with a warning.

use std::sync::Arc;

use everruns_core::capabilities::Capability;
use everruns_core::{
    Agent, Harness, McpCapability, McpServerTransportType, ScopedMcpServer, ScopedMcpServers,
    Session, ToolDefinition, merge_scoped_mcp_servers,
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
        .filter_map(|(name, server)| {
            let endpoint = endpoint_for(name, server)?;
            Some(ResolvedServer {
                name: name.clone(),
                connection: McpConnection {
                    name: name.clone(),
                    endpoint,
                    auth_mode: server.auth_mode.clone(),
                    oauth_provider_id: server.oauth_provider_id.clone(),
                },
                tool_discovery: server.tool_discovery,
            })
        })
        .collect()
}

/// Map a scoped server to a transport endpoint. Returns `None` (and logs) for
/// servers this build cannot serve — e.g. a stdio server when the `mcp-stdio`
/// feature is off, keeping stdio out of hosted builds (specs/runtime-mcp.md D2).
fn endpoint_for(name: &str, server: &ScopedMcpServer) -> Option<McpEndpoint> {
    match server.transport_type {
        McpServerTransportType::Http => Some(McpEndpoint::Http {
            url: server.url.clone(),
            headers: server.headers.clone(),
        }),
        McpServerTransportType::Stdio => stdio_endpoint(name, server),
    }
}

#[cfg(feature = "mcp-stdio")]
fn stdio_endpoint(name: &str, server: &ScopedMcpServer) -> Option<McpEndpoint> {
    let Some(command) = server.command.clone() else {
        tracing::warn!(server = %name, "stdio MCP server ignored: missing `command`");
        return None;
    };
    Some(McpEndpoint::Stdio {
        command,
        args: server.args.clone(),
        env: server.env.clone(),
    })
}

#[cfg(not(feature = "mcp-stdio"))]
fn stdio_endpoint(name: &str, _server: &ScopedMcpServer) -> Option<McpEndpoint> {
    tracing::warn!(
        server = %name,
        "stdio MCP server ignored: runtime built without the `mcp-stdio` feature"
    );
    None
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

#[cfg(test)]
mod tests {
    use super::*;

    fn servers_with(name: &str, server: ScopedMcpServer) -> ScopedMcpServers {
        let mut servers = ScopedMcpServers::default();
        servers.insert(name.to_string(), server);
        servers
    }

    #[test]
    fn http_server_maps_to_http_endpoint() {
        let servers = servers_with(
            "docs",
            ScopedMcpServer {
                url: "https://example.com/mcp".into(),
                ..Default::default()
            },
        );
        let resolved = resolve_servers(&servers);
        assert_eq!(resolved.len(), 1);
        assert!(matches!(
            resolved[0].connection.endpoint,
            McpEndpoint::Http { .. }
        ));
    }

    #[cfg(feature = "mcp-stdio")]
    #[test]
    fn stdio_server_maps_to_stdio_endpoint_when_feature_enabled() {
        let servers = servers_with(
            "fs",
            ScopedMcpServer {
                transport_type: McpServerTransportType::Stdio,
                command: Some("mcp-server-filesystem".into()),
                args: vec!["/work".into()],
                ..Default::default()
            },
        );
        let resolved = resolve_servers(&servers);
        assert_eq!(resolved.len(), 1);
        match &resolved[0].connection.endpoint {
            McpEndpoint::Stdio { command, args, .. } => {
                assert_eq!(command, "mcp-server-filesystem");
                assert_eq!(args, &["/work".to_string()]);
            }
            other => panic!("expected stdio endpoint, got {other:?}"),
        }
    }

    #[cfg(feature = "mcp-stdio")]
    #[test]
    fn stdio_server_without_command_is_skipped() {
        let servers = servers_with(
            "fs",
            ScopedMcpServer {
                transport_type: McpServerTransportType::Stdio,
                command: None,
                ..Default::default()
            },
        );
        assert!(resolve_servers(&servers).is_empty());
    }

    #[cfg(not(feature = "mcp-stdio"))]
    #[test]
    fn stdio_server_is_skipped_without_feature() {
        let servers = servers_with(
            "fs",
            ScopedMcpServer {
                transport_type: McpServerTransportType::Stdio,
                command: Some("mcp-server-filesystem".into()),
                ..Default::default()
            },
        );
        assert!(resolve_servers(&servers).is_empty());
    }
}

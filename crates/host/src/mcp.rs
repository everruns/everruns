//! Host MCP wiring (knowledge/integrations/runtime-mcp.md D4).
//!
//! Resolves the effective scoped MCP servers for a session (harness chain →
//! agent → session, last wins), turns them into transport connections, and
//! drives discovery + execution through the shared `everruns-mcp` client.
//! HTTP scoped servers are always wired; stdio scoped servers are wired only
//! when the crate is built with the `mcp-stdio` feature (off in hosted
//! builds), and are otherwise skipped with a warning.

use std::{collections::HashMap, sync::Arc};

use everruns_core::capabilities::Capability;
use everruns_core::{
    AgentDefinition, ExecutionSession, HarnessDefinition, McpServerTransportType, ScopedMcpServer,
    ScopedMcpServers, ToolDefinition, merge_scoped_mcp_servers,
};
use everruns_mcp::{
    McpCapability, McpClient, McpConnection, McpEndpoint, McpExecutor, StaticConnectionResolver,
};
use futures::{StreamExt, stream};
use uuid::Uuid;

use crate::mcp_cache::McpDiscoveryCache;

/// Maximum scoped MCP server discoveries run concurrently for one turn.
const MAX_DISCOVERY_CONCURRENCY: usize = 16;

/// Merge harness → agent → session scoped MCP servers (last layer wins). The
/// harness definition is already inheritance-resolved (EVE-881).
pub(crate) fn merge_session_scoped_servers(
    harness: &HarnessDefinition,
    agent: Option<&AgentDefinition>,
    session: &ExecutionSession,
) -> ScopedMcpServers {
    let mut merged = merge_scoped_mcp_servers(&ScopedMcpServers::default(), &harness.mcp_servers);
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
                    protocol_mode: server.protocol_mode,
                    oauth_provider_id: server.oauth_provider_id.clone(),
                    pending_oauth_provider: None,
                    secret_bindings: HashMap::new(),
                },
                tool_discovery: server.tool_discovery,
            })
        })
        .collect()
}

/// Map a scoped server to a transport endpoint. Returns `None` (and logs) for
/// servers this build cannot serve — e.g. a stdio server when the `mcp-stdio`
/// feature is off, keeping stdio out of hosted builds (knowledge/integrations/runtime-mcp.md D2).
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
/// enabled, prefixed via [`McpCapability`].
///
/// Discovery is concurrent across servers (a turn's latency is the slowest
/// server, not the sum) and cached per session via [`McpDiscoveryCache`]:
/// fresh entries are served from memory, stale entries are served immediately
/// and revalidated in the background, and a cold entry blocks on a
/// single-flight `tools/list`. Per-server failures degrade to whatever is
/// cached (empty when cold) so one unreachable server doesn't fail the turn.
pub(crate) async fn discover_tool_definitions(
    cache: &Arc<McpDiscoveryCache>,
    client: Arc<McpClient>,
    session_uuid: Uuid,
    servers: &ScopedMcpServers,
) -> Vec<ToolDefinition> {
    let discoveries = resolve_servers(servers)
        .into_iter()
        .filter(|resolved| resolved.tool_discovery)
        .map(|resolved| {
            let cache = cache.clone();
            let client = client.clone();
            async move {
                let key = (session_uuid, resolved.name.clone());
                let name = resolved.name.clone();
                let connection = resolved.connection.clone();
                cache
                    .resolve(key, move || {
                        let client = client.clone();
                        let connection = connection.clone();
                        let name = name.clone();
                        async move {
                            match client.discover(&connection).await {
                                Ok(tools) => Some(build_definitions(session_uuid, &name, tools)),
                                Err(error) => {
                                    tracing::warn!(
                                        server = %name,
                                        %error,
                                        "scoped MCP tool discovery failed; skipping server"
                                    );
                                    None
                                }
                            }
                        }
                    })
                    .await
            }
        });
    stream::iter(discoveries)
        // `buffered` (not `buffer_unordered`) caps concurrency while preserving
        // input order, so the merged tool list stays deterministic — scoped MCP
        // servers iterate from a `BTreeMap`, and stable ordering keeps prompts /
        // provider KV-cache reuse stable.
        .buffered(MAX_DISCOVERY_CONCURRENCY)
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .flatten()
        .collect()
}

/// Map a server's raw `tools/list` result into prefixed tool definitions.
fn build_definitions(
    session_uuid: Uuid,
    name: &str,
    tools: Vec<everruns_core::McpToolDefinition>,
) -> Vec<ToolDefinition> {
    let id = Uuid::new_v5(&session_uuid, name.as_bytes());
    McpCapability::new(id, name.to_string(), None, tools).tool_definitions()
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

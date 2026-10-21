//! Host MCP wiring.
//!
//! Resolves the effective scoped MCP servers for a session (harness chain →
//! agent → session, last wins), turns them into transport connections, and
//! drives discovery + execution through the shared `everruns_core::mcp` client.
//! HTTP scoped servers are always wired; stdio scoped servers are wired only
//! when the crate is built with the `mcp-stdio` feature (off in hosted
//! builds), and are otherwise skipped with a warning.

use std::{collections::HashMap, sync::Arc};

use crate::capabilities::Capability;
use crate::mcp::{
    McpCapability, McpClient, McpConnection, McpEndpoint, McpExecutor, StaticConnectionResolver,
};
use crate::{
    AgentDefinition, ExecutionSession, HarnessDefinition, McpServerTransportType, ScopedMcpServer,
    ScopedMcpServers, merge_scoped_mcp_servers,
};
use everruns_contracts::tool_types::ToolDefinition;
use futures::{StreamExt, TryStreamExt, stream};
use uuid::Uuid;

use crate::host::mcp_cache::{DiscoveryFailure, McpDiscoveryCache};
use sha2::{Digest, Sha256};

/// Maximum scoped MCP server discoveries run concurrently for one turn.
const MAX_DISCOVERY_CONCURRENCY: usize = 16;

struct CatalogByteCounter {
    written: usize,
    limit: usize,
}

impl std::io::Write for CatalogByteCounter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.written = self.written.saturating_add(bytes.len());
        if self.written > self.limit {
            return Err(std::io::Error::other(
                "MCP session catalog exceeds byte limit",
            ));
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn append_discovered_tools(
    catalog: &mut Vec<ToolDefinition>,
    catalog_bytes: usize,
    tools: Vec<ToolDefinition>,
) -> anyhow::Result<usize> {
    let tool_count = catalog.len().saturating_add(tools.len());
    if tool_count > crate::mcp::MCP_SESSION_DISCOVERY_MAX_TOOLS {
        return Err(crate::mcp::McpDiscoveryError::LimitExceeded {
            resource: "session tools",
            limit: crate::mcp::MCP_SESSION_DISCOVERY_MAX_TOOLS,
        }
        .into());
    }

    let remaining = crate::mcp::MCP_SESSION_DISCOVERY_MAX_BYTES.saturating_sub(catalog_bytes);
    let added_bytes = serialized_tool_bytes(&tools, remaining)?;
    catalog.extend(tools);
    Ok(catalog_bytes.saturating_add(added_bytes))
}

fn serialized_tool_bytes(tools: &[ToolDefinition], limit: usize) -> anyhow::Result<usize> {
    let mut writer = CatalogByteCounter { written: 0, limit };
    for tool in tools {
        if serde_json::to_writer(&mut writer, tool).is_err() {
            return Err(crate::mcp::McpDiscoveryError::LimitExceeded {
                resource: "session serialized bytes",
                limit: crate::mcp::MCP_SESSION_DISCOVERY_MAX_BYTES,
            }
            .into());
        }
    }
    Ok(writer.written)
}

fn validate_session_catalog(tools: &[ToolDefinition]) -> anyhow::Result<usize> {
    if tools.len() > crate::mcp::MCP_SESSION_DISCOVERY_MAX_TOOLS {
        return Err(crate::mcp::McpDiscoveryError::LimitExceeded {
            resource: "session tools",
            limit: crate::mcp::MCP_SESSION_DISCOVERY_MAX_TOOLS,
        }
        .into());
    }
    serialized_tool_bytes(tools, crate::mcp::MCP_SESSION_DISCOVERY_MAX_BYTES)
}

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
    configuration: [u8; 32],
}

fn resolve_servers(servers: &ScopedMcpServers) -> Vec<ResolvedServer> {
    servers
        .iter()
        .filter_map(|(name, server)| {
            if !crate::mcp_server::is_valid_mcp_server_name(name) {
                tracing::warn!(server = %name, "MCP server ignored: ambiguous tool prefix");
                return None;
            }
            let endpoint = endpoint_for(name, server)?;
            let mut configuration = serde_json::to_value(server).expect("scoped MCP configuration");
            configuration.sort_all_objects();
            let configuration = Sha256::digest(
                serde_json::to_vec(&configuration).expect("scoped MCP configuration"),
            )
            .into();
            Some(ResolvedServer {
                name: name.clone(),
                connection: McpConnection {
                    name: name.clone(),
                    endpoint,
                    auth_mode: server.auth_mode.clone(),
                    protocol_mode: server.protocol_mode,
                    elicitation_policy: server.elicitation_policy,
                    oauth_provider_id: server.oauth_provider_id.clone(),
                    pending_oauth_provider: None,
                    connect_in_chat: server.connect_in_chat,
                    secret_bindings: HashMap::new(),
                    acted_as: None,
                },
                tool_discovery: server.tool_discovery,
                configuration,
            })
        })
        .collect()
}

/// Map a scoped server to a transport endpoint. Returns `None` (and logs) for
/// servers this build cannot serve — e.g. a stdio server when the `mcp-stdio`
/// feature is off, keeping stdio out of hosted builds.
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
                let key = (session_uuid, resolved.name.clone(), resolved.configuration);
                let name = resolved.name.clone();
                let connection = resolved.connection.clone();
                cache
                    .resolve(key, move || {
                        let client = client.clone();
                        let connection = connection.clone();
                        let name = name.clone();
                        async move {
                            match client.discover(&connection).await {
                                Ok(tools) => Ok(build_definitions(session_uuid, &name, tools)),
                                Err(error) => {
                                    let failure = if error.downcast_ref::<crate::mcp::McpHttpStatusError>()
                                        .is_some_and(|error| matches!(error.status, 401 | 403)) {
                                        tracing::warn!(server = %name,
                                            "MCP authentication rejected; reconnect and reload MCP to retry discovery");
                                        DiscoveryFailure::Authentication
                                    } else {
                                        tracing::warn!(server = %name, %error,
                                            "scoped MCP discovery failed; retrying after a short cooldown");
                                        DiscoveryFailure::Transient
                                    };
                                    Err(failure)
                                }
                            }
                        }
                    })
                    .await
            }
        });
    let result = stream::iter(discoveries)
        // `buffered` (not `buffer_unordered`) caps concurrency while preserving
        // input order, so the merged tool list stays deterministic — scoped MCP
        // servers iterate from a `BTreeMap`, and stable ordering keeps prompts /
        // provider KV-cache reuse stable.
        .buffered(MAX_DISCOVERY_CONCURRENCY)
        .map(Ok::<_, anyhow::Error>)
        .try_fold(
            (Vec::new(), 0usize),
            |(mut catalog, bytes), tools| async move {
                let bytes = append_discovered_tools(&mut catalog, bytes, tools)?;
                Ok((catalog, bytes))
            },
        )
        .await;
    match result {
        Ok((catalog, _bytes)) => catalog,
        Err(error) => {
            tracing::error!(
                %error,
                max_tools = crate::mcp::MCP_SESSION_DISCOVERY_MAX_TOOLS,
                max_bytes = crate::mcp::MCP_SESSION_DISCOVERY_MAX_BYTES,
                "MCP session discovery exceeded aggregate bounds; exposing no MCP tools"
            );
            Vec::new()
        }
    }
}

/// A turn's MCP tool definitions: listed tools for eager and revealed
/// servers, one placeholder per deferred server that has not been revealed in
/// this session yet (user MCP servers D6). No servers, no discovery.
pub(crate) async fn discover_turn_tool_definitions(
    cache: &Arc<McpDiscoveryCache>,
    client: Arc<McpClient>,
    session_id: crate::typed_id::SessionId,
    servers: &ScopedMcpServers,
    storage: &dyn crate::session_services::SessionStorageStore,
) -> Vec<ToolDefinition> {
    if servers.is_empty() {
        return Vec::new();
    }
    let revealed = if servers.values().any(|server| server.deferred) {
        crate::revealed_mcp_servers(storage, session_id).await
    } else {
        Default::default()
    };
    let (listed, deferred) = crate::partition_deferred_mcp_servers(servers, &revealed);
    let mut definitions =
        discover_tool_definitions(cache, client, session_id.uuid(), &listed).await;
    let descriptions: HashMap<_, _> = servers
        .iter()
        .map(|(name, server)| {
            (
                crate::mcp_server::sanitize_mcp_server_name(name),
                (name, server),
            )
        })
        .collect();
    for definition in &mut definitions {
        if let Some((prefix, _)) = crate::mcp_server::parse_mcp_tool_name(definition.name())
            && let Some((name, server)) = descriptions.get(&prefix)
        {
            let metadata = serde_json::json!({"mcp_server": {"name": name, "description": server.description.as_deref().unwrap_or_default()}});
            match definition {
                ToolDefinition::Builtin(tool) => tool.hints.metadata = Some(metadata),
                ToolDefinition::ClientSide(_) => {}
            }
        }
    }
    if definitions.len().saturating_add(deferred.len())
        > crate::mcp::MCP_SESSION_DISCOVERY_MAX_TOOLS
    {
        tracing::error!(
            current_tools = definitions.len(),
            deferred_servers = deferred.len(),
            max_tools = crate::mcp::MCP_SESSION_DISCOVERY_MAX_TOOLS,
            "MCP session discovery exceeded aggregate tool bound; exposing no MCP tools"
        );
        return Vec::new();
    }
    definitions.extend(deferred.iter().map(|name| {
        crate::deferred_mcp_server_definition(
            name,
            servers
                .get(name)
                .and_then(|server| server.description.as_deref()),
        )
    }));
    if let Err(error) = validate_session_catalog(&definitions) {
        tracing::error!(
            %error,
            tool_count = definitions.len(),
            max_tools = crate::mcp::MCP_SESSION_DISCOVERY_MAX_TOOLS,
            max_bytes = crate::mcp::MCP_SESSION_DISCOVERY_MAX_BYTES,
            "MCP session definitions exceeded aggregate bounds after server metadata; exposing no MCP tools"
        );
        return Vec::new();
    }
    definitions
}

/// Map a server's raw `tools/list` result into prefixed tool definitions.
fn build_definitions(
    session_uuid: Uuid,
    name: &str,
    tools: Vec<crate::McpToolDefinition>,
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
    use everruns_contracts::tool_types::{BuiltinTool, DeferrablePolicy, ToolHints, ToolPolicy};

    fn test_tool(name: &str, description: impl Into<String>) -> ToolDefinition {
        ToolDefinition::Builtin(BuiltinTool {
            name: name.into(),
            display_name: None,
            description: description.into(),
            parameters: serde_json::json!({"type":"object"}),
            policy: ToolPolicy::default(),
            category: None,
            deferrable: DeferrablePolicy::default(),
            hints: ToolHints::default(),
            full_parameters: None,
        })
    }

    #[test]
    fn aggregate_catalog_accepts_ten_thousand_tools_and_rejects_one_more() {
        let mut catalog = Vec::new();
        let definitions = (0..crate::mcp::MCP_SESSION_DISCOVERY_MAX_TOOLS)
            .map(|index| test_tool(&format!("server__tool_{index}"), ""))
            .collect();
        let bytes = append_discovered_tools(&mut catalog, 0, definitions).unwrap();
        assert_eq!(catalog.len(), crate::mcp::MCP_SESSION_DISCOVERY_MAX_TOOLS);
        assert!(bytes <= crate::mcp::MCP_SESSION_DISCOVERY_MAX_BYTES);

        let error =
            append_discovered_tools(&mut catalog, bytes, vec![test_tool("another__tool", "")])
                .unwrap_err();
        assert!(matches!(
            error.downcast_ref::<crate::mcp::McpDiscoveryError>(),
            Some(crate::mcp::McpDiscoveryError::LimitExceeded {
                resource: "session tools",
                limit: crate::mcp::MCP_SESSION_DISCOVERY_MAX_TOOLS,
            })
        ));
        assert_eq!(catalog.len(), crate::mcp::MCP_SESSION_DISCOVERY_MAX_TOOLS);
    }

    #[test]
    fn aggregate_catalog_accepts_exact_byte_limit_and_rejects_one_more() {
        let empty = test_tool("server__tool", "");
        let base_bytes = serialized_tool_bytes(std::slice::from_ref(&empty), usize::MAX).unwrap();
        let exact = test_tool(
            "server__tool",
            "x".repeat(crate::mcp::MCP_SESSION_DISCOVERY_MAX_BYTES - base_bytes),
        );
        let mut catalog = Vec::new();
        let bytes = append_discovered_tools(&mut catalog, 0, vec![exact]).unwrap();
        assert_eq!(bytes, crate::mcp::MCP_SESSION_DISCOVERY_MAX_BYTES);

        let error =
            append_discovered_tools(&mut catalog, bytes, vec![test_tool("other__tool", "")])
                .unwrap_err();
        assert!(matches!(
            error.downcast_ref::<crate::mcp::McpDiscoveryError>(),
            Some(crate::mcp::McpDiscoveryError::LimitExceeded {
                resource: "session serialized bytes",
                limit: crate::mcp::MCP_SESSION_DISCOVERY_MAX_BYTES,
            })
        ));
        assert_eq!(catalog.len(), 1);
    }

    fn servers_with(name: &str, server: ScopedMcpServer) -> ScopedMcpServers {
        let mut servers = ScopedMcpServers::default();
        servers.insert(name.to_string(), server);
        servers
    }

    struct DiscoveryEgress {
        requests: std::sync::atomic::AtomicUsize,
        status: std::sync::atomic::AtomicU16,
    }

    #[async_trait::async_trait]
    impl crate::EgressService for DiscoveryEgress {
        async fn send(
            &self,
            request: crate::EgressRequest,
        ) -> crate::EgressResult<crate::EgressResponse> {
            self.requests
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let id = serde_json::from_slice::<serde_json::Value>(&request.body)
                .ok()
                .and_then(|body| body.get("id").cloned());
            Ok(crate::EgressResponse {
                status: self.status.load(std::sync::atomic::Ordering::SeqCst),
                headers: Default::default(),
                body: serde_json::to_vec(&serde_json::json!({"jsonrpc":"2.0", "id":id,
                    "result":{"tools":[{"name":"ping", "inputSchema":{"type":"object"}}]}}))
                .unwrap(),
            })
        }
        async fn send_stream(
            &self,
            request: crate::EgressRequest,
        ) -> crate::EgressResult<crate::EgressStreamResponse> {
            let response = self.send(request).await?;
            Ok(crate::EgressStreamResponse {
                status: response.status,
                headers: response.headers,
                body: Box::pin(futures::stream::iter([Ok(response.body)])),
            })
        }
    }

    #[tokio::test]
    async fn rejected_http_discovery_stays_paused_until_reload_or_new_credentials() {
        use std::sync::atomic::{AtomicU16, AtomicUsize, Ordering};
        for status in [401, 403] {
            let egress = Arc::new(DiscoveryEgress {
                requests: AtomicUsize::new(0),
                status: AtomicU16::new(status),
            });
            let client = Arc::new(McpClient::new(
                egress.clone(),
                Arc::new(crate::mcp::NoAuthProvider),
            ));
            let cache = Arc::new(McpDiscoveryCache::new());
            let session = Uuid::new_v4();
            let mut servers = servers_with(
                "private",
                ScopedMcpServer {
                    url: "https://93.184.216.34/mcp".into(),
                    ..Default::default()
                },
            );
            assert!(
                discover_tool_definitions(&cache, client.clone(), session, &servers)
                    .await
                    .is_empty()
            );
            let failed_requests = egress.requests.load(Ordering::SeqCst);
            assert!(failed_requests > 0);
            for _ in 0..20 {
                assert!(
                    discover_tool_definitions(&cache, client.clone(), session, &servers)
                        .await
                        .is_empty()
                );
            }
            assert_eq!(egress.requests.load(Ordering::SeqCst), failed_requests);
            servers
                .get_mut("private")
                .unwrap()
                .headers
                .insert("Authorization".into(), "replacement-fixture".into());
            egress.status.store(200, Ordering::SeqCst);
            assert_eq!(
                discover_tool_definitions(&cache, client.clone(), session, &servers)
                    .await
                    .len(),
                1
            );
            assert!(egress.requests.load(Ordering::SeqCst) > failed_requests);
            let before_reload = egress.requests.load(Ordering::SeqCst);
            cache.invalidate_session(session);
            assert_eq!(
                discover_tool_definitions(&cache, client, session, &servers)
                    .await
                    .len(),
                1
            );
            assert!(egress.requests.load(Ordering::SeqCst) > before_reload);
        }
    }

    #[test]
    fn http_server_maps_to_http_endpoint() {
        let headers = HashMap::from([("X-Trace".into(), "request-trace".into())]);
        let servers = servers_with(
            "docs",
            ScopedMcpServer {
                url: "https://example.com/mcp".into(),
                headers: headers.clone(),
                auth_mode: crate::McpServerAuthMode::OAuth,
                protocol_mode: crate::McpProtocolMode::V2025March,
                oauth_provider_id: Some("docs-provider".into()),
                tool_discovery: false,
                ..Default::default()
            },
        );
        let resolved = resolve_servers(&servers);
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].name, "docs");
        assert!(!resolved[0].tool_discovery);
        let connection = &resolved[0].connection;
        assert_eq!(connection.name, "docs");
        assert_eq!(connection.auth_mode, crate::McpServerAuthMode::OAuth);
        assert_eq!(connection.protocol_mode, crate::McpProtocolMode::V2025March);
        assert_eq!(
            connection.oauth_provider_id.as_deref(),
            Some("docs-provider")
        );
        match &connection.endpoint {
            McpEndpoint::Http {
                url,
                headers: actual_headers,
            } => {
                assert_eq!(url, "https://example.com/mcp");
                assert_eq!(actual_headers, &headers);
            }
            #[cfg(feature = "mcp-stdio")]
            other => panic!("expected HTTP, got {other:?}"),
        }
    }

    #[test]
    fn discovery_identity_is_stable_for_header_order_and_changes_with_credentials() {
        let first = ScopedMcpServer {
            url: "https://example.com/mcp".into(),
            headers: HashMap::from([
                ("Authorization".into(), "private-one".into()),
                ("X-Mode".into(), "read".into()),
            ]),
            ..Default::default()
        };
        let mut reordered = first.clone();
        reordered.headers = HashMap::from([
            ("X-Mode".into(), "read".into()),
            ("Authorization".into(), "private-one".into()),
        ]);
        let fingerprint = |server| resolve_servers(&servers_with("docs", server))[0].configuration;
        assert_eq!(fingerprint(first.clone()), fingerprint(reordered));
        let mut changed = first.clone();
        changed
            .headers
            .insert("Authorization".into(), "private-two".into());
        assert_ne!(fingerprint(first), fingerprint(changed));
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
                env: HashMap::from([("MODE".into(), "test".into())]),
                ..Default::default()
            },
        );
        let resolved = resolve_servers(&servers);
        assert_eq!(resolved.len(), 1);
        match &resolved[0].connection.endpoint {
            McpEndpoint::Stdio { command, args, env } => {
                assert_eq!(command, "mcp-server-filesystem");
                assert_eq!(args, &["/work".to_string()]);
                assert_eq!(env, &HashMap::from([("MODE".into(), "test".into())]));
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
    #[test]
    fn ambiguous_server_names_are_excluded_from_runtime_connections() {
        let server = ScopedMcpServer {
            url: "https://example.com/mcp".into(),
            ..Default::default()
        };
        for name in ["docs_", "docs__private", "docs-", "_"] {
            assert!(
                resolve_servers(&servers_with(name, server.clone())).is_empty(),
                "{name}"
            );
        }
        let resolved = resolve_servers(&servers_with("docs_api", server));
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].connection.name, "docs_api");
    }
}

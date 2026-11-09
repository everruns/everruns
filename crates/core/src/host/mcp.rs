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
                    tool_timeout_ms: server.tool_timeout_ms,
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

/// Readiness of every configured server that discovers tools, read from the
/// discovery cache without starting discovery. `deferred` names the deferred
/// servers not yet revealed in the session.
pub(crate) fn server_readiness(
    cache: &McpDiscoveryCache,
    session: Uuid,
    servers: &ScopedMcpServers,
    deferred: &[String],
) -> crate::host::McpReadiness {
    // Read the revision first: a change racing this read bumps past it, so a
    // host comparing revisions never misses an update.
    let revision = cache.revision();
    let mut resolved: HashMap<String, ResolvedServer> = resolve_servers(servers)
        .into_iter()
        .map(|server| (server.name.clone(), server))
        .collect();
    let servers = servers
        .iter()
        .filter(|(_, server)| server.tool_discovery)
        .map(|(name, _)| {
            let Some(server) = resolved.remove(name) else {
                // resolve_servers dropped it: a name that is not a usable tool
                // prefix, or a transport this build cannot serve.
                let mut entry = crate::host::McpServerReadiness::new(
                    name.clone(),
                    crate::host::McpDiscoveryStatus::Failed,
                );
                entry.reason = Some(UNSERVABLE_SERVER_REASON.to_string());
                return entry;
            };
            let state = cache.readiness(&(session, server.name.clone(), server.configuration));
            let mut entry = crate::host::McpServerReadiness::new(server.name, state.status);
            entry.deferred = deferred.contains(name);
            entry.probing = state.probing;
            entry.tool_count = state.tool_count;
            entry.reason = state.reason;
            entry.last_probe = state.last_probe;
            entry
        })
        .collect();
    crate::host::McpReadiness { revision, servers }
}

const UNSERVABLE_SERVER_REASON: &str =
    "server ignored: its name is not a valid tool prefix or this build cannot serve its transport";

/// Run one discovery attempt now for each named server, bypassing cached
/// freshness, cooldowns and authentication holds. Unknown, non-discovering or
/// unservable names are rejected before any attempt starts.
pub(crate) async fn probe_servers(
    cache: &Arc<McpDiscoveryCache>,
    client: Arc<McpClient>,
    session_uuid: Uuid,
    servers: &ScopedMcpServers,
    names: &[String],
) -> anyhow::Result<()> {
    let mut resolved: HashMap<String, ResolvedServer> = resolve_servers(servers)
        .into_iter()
        .map(|server| (server.name.clone(), server))
        .collect();
    let mut targets = Vec::with_capacity(names.len());
    for name in names {
        match servers.get(name) {
            None => anyhow::bail!("session has no MCP server {name:?}"),
            Some(server) if !server.tool_discovery => {
                anyhow::bail!("MCP server {name:?} does not discover tools")
            }
            Some(_) => {}
        }
        let Some(server) = resolved.remove(name) else {
            anyhow::bail!("MCP server {name:?} {UNSERVABLE_SERVER_REASON}");
        };
        targets.push(server);
    }
    stream::iter(targets.into_iter().map(|server| {
        let cache = cache.clone();
        let client = client.clone();
        async move {
            let key = (session_uuid, server.name.clone(), server.configuration);
            cache
                .probe(key, || {
                    discover_once(client, session_uuid, server.name, server.connection)
                })
                .await;
        }
    }))
    .buffer_unordered(MAX_DISCOVERY_CONCURRENCY)
    .collect::<()>()
    .await;
    Ok(())
}

/// One upstream `tools/list` for a server, classified for the cache.
async fn discover_once(
    client: Arc<McpClient>,
    session_uuid: Uuid,
    name: String,
    connection: McpConnection,
) -> Result<Vec<ToolDefinition>, DiscoveryFailure> {
    match client.discover(&connection).await {
        Ok(tools) => Ok(build_definitions(session_uuid, &name, tools)),
        Err(error) => {
            if let Some(status) = error
                .downcast_ref::<crate::mcp::McpHttpStatusError>()
                .map(|error| error.status)
                .filter(|status| matches!(status, 401 | 403))
            {
                tracing::warn!(server = %name,
                    "MCP authentication rejected; reconnect and reload MCP to retry discovery");
                Err(DiscoveryFailure::authentication(format!(
                    "server rejected credentials (HTTP {status}); reconnect to retry"
                )))
            } else {
                tracing::warn!(server = %name, %error,
                    "scoped MCP discovery failed; retrying after a short cooldown");
                Err(DiscoveryFailure::transient(error.to_string()))
            }
        }
    }
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
    let deadline = cache
        .readiness_budget
        .map(|budget| tokio::time::Instant::now() + budget.min(crate::mcp::MCP_DISCOVERY_DEADLINE));
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
                    .resolve_until(
                        key,
                        move || {
                            discover_once(
                                client.clone(),
                                session_uuid,
                                name.clone(),
                                connection.clone(),
                            )
                        },
                        deadline,
                    )
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
    attach_server_search_metadata(&mut definitions, servers);
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

/// Apply current source context after discovery-cache lookup and placeholder assembly.
fn attach_server_search_metadata(definitions: &mut [ToolDefinition], servers: &ScopedMcpServers) {
    let by_prefix: HashMap<_, _> = servers
        .iter()
        .map(|(name, server)| {
            (
                crate::mcp_server::sanitize_mcp_server_name(name),
                (name, server),
            )
        })
        .collect();
    for definition in definitions {
        let prefix = crate::deferred_mcp_server_prefix(definition.name())
            .map(str::to_owned)
            .or_else(|| {
                crate::mcp_server::parse_mcp_tool_name(definition.name()).map(|(prefix, _)| prefix)
            });
        let Some((name, server)) = prefix
            .as_deref()
            .and_then(|prefix| by_prefix.get(prefix).copied())
        else {
            continue;
        };
        let ToolDefinition::Builtin(tool) = definition else {
            continue;
        };
        if let Some(context) = &server.capability_context {
            tool.hints.capability_id = Some(context.id.clone());
            tool.hints.capability_name = Some(context.name.clone());
            tool.hints.capability_description = Some(context.description.clone());
        }
        let metadata = tool
            .hints
            .metadata
            .get_or_insert_with(|| serde_json::json!({}));
        if let Some(metadata) = metadata.as_object_mut() {
            metadata.insert(
                "mcp_server".into(),
                serde_json::json!({
                    "name": name, "description": server.description.as_deref().unwrap_or_default(),
                }),
            );
        }
    }
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
        slow_gate: Option<Arc<tokio::sync::Semaphore>>,
    }

    #[async_trait::async_trait]
    impl crate::EgressService for DiscoveryEgress {
        async fn send(
            &self,
            request: crate::EgressRequest,
        ) -> crate::EgressResult<crate::EgressResponse> {
            self.requests
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if request.url.ends_with("/slow")
                && let Some(gate) = &self.slow_gate
            {
                gate.acquire().await.unwrap().forget();
            }
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
    async fn readiness_keeps_fast_tools_and_admits_late_tools_in_stable_order() {
        use std::sync::atomic::{AtomicU16, AtomicUsize};
        use std::time::Duration;
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        let egress = Arc::new(DiscoveryEgress {
            requests: AtomicUsize::new(0),
            status: AtomicU16::new(200),
            slow_gate: Some(gate.clone()),
        });
        let client = Arc::new(McpClient::new(egress, Arc::new(crate::mcp::NoAuthProvider)));
        let cache = Arc::new(McpDiscoveryCache::with_readiness_budget(Some(
            Duration::from_millis(20),
        )));
        let mut servers = servers_with(
            "a_slow",
            ScopedMcpServer {
                url: "https://93.184.216.34/slow".into(),
                ..Default::default()
            },
        );
        servers.insert(
            "z_fast".into(),
            ScopedMcpServer {
                url: "https://93.184.216.34/fast".into(),
                ..Default::default()
            },
        );
        let session = Uuid::new_v4();
        let first = tokio::time::timeout(
            Duration::from_secs(1),
            discover_tool_definitions(&cache, client.clone(), session, &servers),
        )
        .await
        .expect("slow server must not hold discovery");
        assert_eq!(
            first.iter().map(|tool| tool.name()).collect::<Vec<_>>(),
            ["mcp_z_fast__ping"]
        );
        let statuses = |cache: &McpDiscoveryCache| {
            server_readiness(cache, session, &servers, &[])
                .servers
                .into_iter()
                .map(|server| (server.name, server.status))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            statuses(&cache),
            vec![
                ("a_slow".into(), crate::host::McpDiscoveryStatus::Pending),
                ("z_fast".into(), crate::host::McpDiscoveryStatus::Ready),
            ]
        );
        gate.add_permits(8);
        tokio::time::timeout(Duration::from_secs(2), async {
            while statuses(&cache)
                .iter()
                .any(|(_, status)| *status != crate::host::McpDiscoveryStatus::Ready)
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let second = discover_tool_definitions(&cache, client, session, &servers).await;
        assert_eq!(
            second.iter().map(|tool| tool.name()).collect::<Vec<_>>(),
            ["mcp_a_slow__ping", "mcp_z_fast__ping"]
        );
    }

    #[tokio::test]
    async fn rejected_http_discovery_stays_paused_until_reload_or_new_credentials() {
        use std::sync::atomic::{AtomicU16, AtomicUsize, Ordering};
        for status in [401, 403] {
            let egress = Arc::new(DiscoveryEgress {
                requests: AtomicUsize::new(0),
                status: AtomicU16::new(status),
                slow_gate: None,
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

    #[tokio::test]
    async fn readiness_reports_reason_probe_time_and_probe_lifts_the_auth_hold() {
        use std::sync::atomic::{AtomicU16, AtomicUsize, Ordering};
        let egress = Arc::new(DiscoveryEgress {
            requests: AtomicUsize::new(0),
            status: AtomicU16::new(401),
            slow_gate: None,
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
        servers.insert(
            "later".into(),
            ScopedMcpServer {
                url: "https://93.184.216.34/later".into(),
                deferred: true,
                ..Default::default()
            },
        );
        servers.insert(
            "bad__name".into(),
            ScopedMcpServer {
                url: "https://93.184.216.34/bad".into(),
                ..Default::default()
            },
        );
        servers.insert(
            "quiet".into(),
            ScopedMcpServer {
                url: "https://93.184.216.34/quiet".into(),
                tool_discovery: false,
                ..Default::default()
            },
        );
        let deferred = vec!["later".to_string()];

        let cold = server_readiness(&cache, session, &servers, &deferred);
        assert_eq!(
            cold.servers
                .iter()
                .map(|server| server.name.as_str())
                .collect::<Vec<_>>(),
            ["bad__name", "later", "private"],
            "one entry per discovering server; tool_discovery=false is excluded"
        );
        let bad = cold.server("bad__name").unwrap();
        assert_eq!(bad.status, crate::host::McpDiscoveryStatus::Failed);
        assert!(
            bad.reason
                .as_deref()
                .unwrap()
                .contains("not a valid tool prefix")
        );
        let later = cold.server("later").unwrap();
        assert!(later.deferred);
        assert_eq!(later.status, crate::host::McpDiscoveryStatus::Pending);
        let private = cold.server("private").unwrap();
        assert_eq!(private.status, crate::host::McpDiscoveryStatus::Pending);
        assert_eq!(
            (private.last_probe, private.reason.as_deref()),
            (None, None)
        );

        // A turn hits the authentication failure and holds it.
        let mut listed = servers.clone();
        listed.remove("later");
        assert!(
            discover_tool_definitions(&cache, client.clone(), session, &listed)
                .await
                .is_empty()
        );
        let held = server_readiness(&cache, session, &servers, &deferred);
        assert!(held.revision > cold.revision);
        let private = held.server("private").unwrap();
        assert_eq!(
            private.status,
            crate::host::McpDiscoveryStatus::AuthenticationRequired
        );
        assert!(private.reason.as_deref().unwrap().contains("HTTP 401"));
        let first_probe = private.last_probe.expect("completed attempt time");
        assert!(!private.probing);

        // Turns keep the hold; an explicit probe bypasses it.
        egress.status.store(200, Ordering::SeqCst);
        discover_tool_definitions(&cache, client.clone(), session, &listed).await;
        assert_eq!(
            server_readiness(&cache, session, &servers, &deferred)
                .server("private")
                .unwrap()
                .status,
            crate::host::McpDiscoveryStatus::AuthenticationRequired
        );
        probe_servers(
            &cache,
            client.clone(),
            session,
            &servers,
            &["private".into(), "later".into()],
        )
        .await
        .unwrap();
        let probed = server_readiness(&cache, session, &servers, &deferred);
        assert!(probed.revision > held.revision);
        let private = probed.server("private").unwrap();
        assert_eq!(private.status, crate::host::McpDiscoveryStatus::Ready);
        assert_eq!((private.tool_count, private.reason.as_deref()), (1, None));
        assert!(private.last_probe.unwrap() >= first_probe);
        let later = probed.server("later").unwrap();
        assert_eq!(
            (later.deferred, later.status),
            (true, crate::host::McpDiscoveryStatus::Ready),
            "an explicit probe connects a deferred server without revealing it"
        );
        assert_eq!(
            discover_tool_definitions(&cache, client.clone(), session, &listed)
                .await
                .iter()
                .map(|tool| tool.name())
                .collect::<Vec<_>>(),
            ["mcp_private__ping"]
        );

        // A transient probe failure records its reason and keeps the catalog.
        egress.status.store(500, Ordering::SeqCst);
        probe_servers(
            &cache,
            client.clone(),
            session,
            &servers,
            &["private".into()],
        )
        .await
        .unwrap();
        let failed = server_readiness(&cache, session, &servers, &deferred);
        let private = failed.server("private").unwrap();
        assert_eq!(private.status, crate::host::McpDiscoveryStatus::Failed);
        assert_eq!(private.tool_count, 1);
        assert!(private.reason.is_some());

        for rejected in ["missing", "quiet", "bad__name"] {
            let requests = egress.requests.load(Ordering::SeqCst);
            assert!(
                probe_servers(
                    &cache,
                    client.clone(),
                    session,
                    &servers,
                    &[rejected.into()]
                )
                .await
                .is_err(),
                "{rejected}"
            );
            assert_eq!(egress.requests.load(Ordering::SeqCst), requests);
        }
    }

    #[tokio::test]
    async fn probe_waiting_on_an_in_flight_attempt_reuses_its_result() {
        use std::sync::atomic::{AtomicU16, AtomicUsize, Ordering};
        use std::time::Duration;
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        let egress = Arc::new(DiscoveryEgress {
            requests: AtomicUsize::new(0),
            status: AtomicU16::new(200),
            slow_gate: Some(gate.clone()),
        });
        let client = Arc::new(McpClient::new(
            egress.clone(),
            Arc::new(crate::mcp::NoAuthProvider),
        ));
        let cache = Arc::new(McpDiscoveryCache::with_readiness_budget(Some(
            Duration::from_millis(10),
        )));
        // Requests one complete discovery makes, measured on an ungated path.
        let fast = servers_with(
            "fast",
            ScopedMcpServer {
                url: "https://93.184.216.34/fast".into(),
                ..Default::default()
            },
        );
        discover_tool_definitions(
            &Arc::new(McpDiscoveryCache::new()),
            client.clone(),
            Uuid::new_v4(),
            &fast,
        )
        .await;
        let per_discovery = egress.requests.swap(0, Ordering::SeqCst);
        assert!(per_discovery > 0);
        let session = Uuid::new_v4();
        let servers = servers_with(
            "slow",
            ScopedMcpServer {
                url: "https://93.184.216.34/slow".into(),
                ..Default::default()
            },
        );
        let mut watch = crate::host::McpReadinessWatch::new(cache.subscribe());
        let seen = watch.revision();
        // The budget expires; background discovery keeps running.
        assert!(
            discover_tool_definitions(&cache, client.clone(), session, &servers)
                .await
                .is_empty()
        );
        let connecting = server_readiness(&cache, session, &servers, &[]);
        assert!(connecting.servers[0].probing);
        assert_eq!(
            connecting.servers[0].status,
            crate::host::McpDiscoveryStatus::Pending
        );
        assert!(
            tokio::time::timeout(Duration::from_secs(1), watch.changed())
                .await
                .unwrap()
                .unwrap()
                > seen
        );
        let probe = tokio::spawn({
            let cache = cache.clone();
            let servers = servers.clone();
            async move { probe_servers(&cache, client, session, &servers, &["slow".into()]).await }
        });
        tokio::task::yield_now().await;
        // Exactly one discovery's worth: a second upstream attempt would block.
        gate.add_permits(per_discovery);
        tokio::time::timeout(Duration::from_secs(2), probe)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let ready = server_readiness(&cache, session, &servers, &[]);
        assert_eq!(
            ready.servers[0].status,
            crate::host::McpDiscoveryStatus::Ready
        );
        assert!(!ready.servers[0].probing);
        assert_eq!(egress.requests.load(Ordering::SeqCst), per_discovery);
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

    #[tokio::test]
    async fn current_owner_context_updates_cached_tools_and_deferred_placeholders() {
        use std::sync::atomic::{AtomicU16, AtomicUsize, Ordering};
        for deferred in [false, true] {
            let egress = Arc::new(DiscoveryEgress {
                requests: AtomicUsize::new(0),
                status: AtomicU16::new(200),
                slow_gate: None,
            });
            let client = Arc::new(McpClient::new(
                egress.clone(),
                Arc::new(crate::mcp::NoAuthProvider),
            ));
            let cache = Arc::new(McpDiscoveryCache::new());
            let storage = crate::host::in_memory::InMemorySessionStorageStore::new();
            let session_id = crate::typed_id::SessionId::new();
            let mut servers = servers_with(
                "gateway",
                ScopedMcpServer {
                    url: "https://93.184.216.34/mcp".into(),
                    deferred,
                    description: Some("Geothermal telemetry".into()),
                    capability_context: Some(crate::mcp_server::McpCapabilityContext {
                        id: "retail".into(),
                        name: "Retail operations".into(),
                        description: "Cobalt warehouse".into(),
                    }),
                    ..Default::default()
                },
            );
            let definitions = discover_turn_tool_definitions(
                &cache,
                client.clone(),
                session_id,
                &servers,
                &storage,
            )
            .await;
            assert_eq!(definitions.len(), 1);
            assert_eq!(
                definitions[0].hints().capability_description.as_deref(),
                Some("Cobalt warehouse")
            );
            let index =
                everruns_contracts::runtime::tool_catalog::ToolSearchIndex::new(&definitions);
            for query in ["cobalt", "geothermal"] {
                let hits = index.search(query, 8, |_| true);
                assert_eq!(hits.len(), 1, "{query}");
                assert!(hits[0].server_match);
                assert_eq!(hits[0].source.as_ref().unwrap().id, "gateway");
            }
            assert_eq!(
                definitions[0].hints().metadata.as_ref().unwrap()["mcp_server"]["description"],
                "Geothermal telemetry"
            );
            let requests = egress.requests.load(Ordering::SeqCst);
            servers
                .get_mut("gateway")
                .unwrap()
                .capability_context
                .as_mut()
                .unwrap()
                .description = "Fresh invoice reconciliation".into();
            let updated = discover_turn_tool_definitions(
                &cache,
                client.clone(),
                session_id,
                &servers,
                &storage,
            )
            .await;
            let index = everruns_contracts::runtime::tool_catalog::ToolSearchIndex::new(&updated);
            assert!(index.search("cobalt", 8, |_| true).is_empty());
            assert_eq!(index.search("reconciliation", 8, |_| true).len(), 1);
            servers.get_mut("gateway").unwrap().capability_context = None;
            let unowned =
                discover_turn_tool_definitions(&cache, client, session_id, &servers, &storage)
                    .await;
            assert!(
                everruns_contracts::runtime::tool_catalog::ToolSearchIndex::new(&unowned)
                    .search("reconciliation", 8, |_| true)
                    .is_empty()
            );
            assert_eq!(
                egress.requests.load(Ordering::SeqCst),
                requests,
                "owner metadata must not invalidate transport discovery cache"
            );
            assert_eq!(requests == 0, deferred);
        }
    }
}

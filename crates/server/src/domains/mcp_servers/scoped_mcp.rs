// Scoped MCP helpers.
//
// Decision: harness/agent/session-scoped remote MCP servers are merged with
// last-wins semantics by logical name, then resolved ahead of org-scoped MCP
// servers. Tool discovery is live (no persisted cache) to keep this feature
// narrowly scoped and avoid mutating config rows during runtime.

use crate::kernel_imports::{
    Capability, EgressService, McpProtocolMode, McpServerActsAs, McpServerAuthMode,
    McpServerTransportType, ScopedMcpServer, ScopedMcpServers,
    everruns_provider::tool_types::ToolDefinition, everruns_provider::typed_id::SessionId,
    everruns_provider::url_validation::validate_safe_url, merge_scoped_mcp_servers,
    resolve_runtime_capabilities,
};
use anyhow::{Result, anyhow};
use everruns_core::capabilities::{CapabilityRegistry, collect_capability_mcp_servers};
use everruns_core::connection_services::UserConnectionResolver;
use everruns_core::mcp_server::sanitize_mcp_server_name;
use everruns_mcp::McpCapability;
use everruns_platform::{Agent, Harness, Session};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use uuid::Uuid;

use crate::domains::mcp_servers::McpServerResolved;
use crate::domains::mcp_servers::service::{McpServerService, fetch_mcp_tools};
use crate::storage::StorageBackend;

pub fn merge_effective_scoped_mcp_servers(
    harness: &Harness,
    agent: Option<&Agent>,
    session: &Session,
) -> ScopedMcpServers {
    let mut layers = vec![&harness.mcp_servers];
    if let Some(agent) = agent {
        layers.push(&agent.mcp_servers);
    }
    layers.push(&session.mcp_servers);

    let merged = merge_scoped_mcp_server_layers(layers);
    strip_untrusted_oauth_from_scoped_mcp_servers(&merged)
}

/// Sanitize explicit (user-controlled) scoped MCP servers so they cannot
/// request OAuth connection tokens for runtime tool discovery.
///
/// Capability-contributed servers are trusted (built-in code) and bypass this
/// step entirely — see `merge_effective_scoped_mcp_servers_with_capabilities`.
///
/// Behavior is intentionally narrow: only entries that were explicitly
/// configured as `auth_mode = OAuth` are altered. We clear `oauth_provider_id`
/// (so no token resolution happens) and switch `auth_mode` back to `None`. We
/// also disable `tool_discovery` on those entries — without a token, an
/// authenticated tools/list would 401, so attempting it would just spam logs
/// and slow scoped-MCP wiring.
fn strip_untrusted_oauth_from_scoped_mcp_servers(servers: &ScopedMcpServers) -> ScopedMcpServers {
    servers
        .iter()
        .map(|(name, server)| {
            let mut server = server.clone();
            if matches!(server.auth_mode, McpServerAuthMode::OAuth) {
                server.auth_mode = McpServerAuthMode::None;
                server.oauth_provider_id = None;
                server.tool_discovery = false;
            }
            (name.clone(), server)
        })
        .collect()
}
pub fn merge_effective_scoped_mcp_servers_with_capabilities(
    harness: &Harness,
    agent: Option<&Agent>,
    session: &Session,
    capability_registry: &CapabilityRegistry,
) -> ScopedMcpServers {
    let explicit = merge_effective_scoped_mcp_servers(harness, agent, session);
    // Status-agnostic projection: scoped-MCP wiring historically saw the
    // stored records regardless of lifecycle status (EVE-877, EVE-881).
    let agent_definition = agent.map(|a| a.definition());
    let harness_definition = harness.definition();
    // EVE-882: capability resolution consumes the portable execution view.
    let execution_session = session.execution_session();
    let resolved = resolve_runtime_capabilities(
        &harness_definition,
        agent_definition.as_ref(),
        &execution_session,
        capability_registry,
    );
    let contributed =
        collect_capability_mcp_servers(&resolved.resolved_capability_configs, capability_registry);

    merge_scoped_mcp_servers(&contributed, &explicit)
}

pub fn merge_scoped_mcp_server_layers<'a, I>(layers: I) -> ScopedMcpServers
where
    I: IntoIterator<Item = &'a ScopedMcpServers>,
{
    let mut merged = ScopedMcpServers::default();
    for layer in layers {
        merged = merge_scoped_mcp_servers(&merged, layer);
    }
    merged
}

pub fn validate_merged_scoped_mcp_servers<'a, I>(layers: I) -> Result<ScopedMcpServers>
where
    I: IntoIterator<Item = &'a ScopedMcpServers>,
{
    let merged = merge_scoped_mcp_server_layers(layers);
    validate_scoped_mcp_servers(&merged)?;
    Ok(merged)
}

pub async fn resolve_scoped_mcp_server(
    mcp_server_service: &McpServerService,
    org_id: i64,
    harness: &Harness,
    agent: Option<&Agent>,
    session: &Session,
    server_prefix: &str,
) -> Result<Option<McpServerResolved>> {
    let effective = merge_effective_scoped_mcp_servers(harness, agent, session);
    let matched = effective.into_iter().find(|(name, _)| {
        everruns_core::mcp_server::is_valid_mcp_server_name(name)
            && sanitize_mcp_server_name(name) == server_prefix
    });
    resolve_matched_scoped_mcp_server(mcp_server_service, org_id, session.id.uuid(), matched).await
}

pub async fn resolve_scoped_mcp_server_with_capabilities(
    mcp_server_service: &McpServerService,
    org_id: i64,
    harness: &Harness,
    agent: Option<&Agent>,
    session: &Session,
    server_prefix: &str,
    capability_registry: &CapabilityRegistry,
) -> Result<Option<McpServerResolved>> {
    let effective = merge_effective_scoped_mcp_servers_with_capabilities(
        harness,
        agent,
        session,
        capability_registry,
    );
    let matched = effective.into_iter().find(|(name, _)| {
        everruns_core::mcp_server::is_valid_mcp_server_name(name)
            && sanitize_mcp_server_name(name) == server_prefix
    });
    resolve_matched_scoped_mcp_server(mcp_server_service, org_id, session.id.uuid(), matched).await
}

async fn resolve_matched_scoped_mcp_server(
    mcp_server_service: &McpServerService,
    org_id: i64,
    session_id: Uuid,
    matched: Option<(String, ScopedMcpServer)>,
) -> Result<Option<McpServerResolved>> {
    let Some((name, server)) = matched else {
        return Ok(None);
    };
    if let Some(preset) = &server.preset {
        let preset_name = preset.catalog_name();
        let mut resolved = mcp_server_service
            .resolve_transport_by_name(&everruns_core::Caller::internal(org_id), preset_name)
            .await?
            .ok_or_else(|| {
                anyhow!("Catalog MCP server preset '{preset_name}' is missing or not active")
            })?;
        resolved.id = scoped_mcp_server_uuid(session_id, &name);
        resolved.name = name;
        resolved.acts_as = server.acts_as;
        return Ok(Some(resolved));
    }

    Ok(Some(McpServerResolved {
        id: scoped_mcp_server_uuid(session_id, &name),
        name,
        url: server.url,
        auth_mode: server.auth_mode,
        protocol_mode: server.protocol_mode,
        oauth_provider_id: server.oauth_provider_id,
        acts_as: server.acts_as,
        api_key: None,
        headers: server.headers,
    }))
}

pub async fn materialize_scoped_mcp_servers(
    db: &StorageBackend,
    org_id: i64,
    servers: &ScopedMcpServers,
) -> Result<ScopedMcpServers> {
    let mut materialized = ScopedMcpServers::new();
    for (name, server) in servers {
        let Some(preset) = &server.preset else {
            materialized.insert(name.clone(), server.clone());
            continue;
        };
        let preset_name = preset.catalog_name();
        let row = db
            .get_mcp_server_by_name(org_id, preset_name)
            .await?
            .filter(|row| row.status == "active")
            .ok_or_else(|| {
                anyhow!("Catalog MCP server preset '{preset_name}' is missing or not active")
            })?;
        let settings = McpServerService::settings_from_row(&row);
        materialized.insert(
            name.clone(),
            ScopedMcpServer {
                transport_type: McpServerTransportType::from(row.transport_type.as_str()),
                url: row.url,
                headers: serde_json::from_value(row.headers).unwrap_or_default(),
                protocol_mode: settings.protocol_mode,
                acts_as: server.acts_as,
                ..Default::default()
            },
        );
    }
    Ok(materialized)
}

pub async fn build_materialized_scoped_mcp_tool_definitions(
    db: &StorageBackend,
    org_id: i64,
    servers: &ScopedMcpServers,
    session_id: Option<SessionId>,
    connection_resolver: Option<&Arc<dyn UserConnectionResolver>>,
    egress_service: &dyn EgressService,
) -> Result<Vec<ToolDefinition>> {
    let materialized = materialize_scoped_mcp_servers(db, org_id, servers).await?;
    build_scoped_mcp_tool_definitions(
        &materialized,
        session_id,
        connection_resolver,
        egress_service,
    )
    .await
}
pub async fn build_scoped_mcp_tool_definitions(
    servers: &ScopedMcpServers,
    session_id: Option<SessionId>,
    connection_resolver: Option<&Arc<dyn UserConnectionResolver>>,
    egress_service: &dyn EgressService,
) -> Result<Vec<ToolDefinition>> {
    let mut definitions = Vec::new();

    for (name, server) in servers {
        if !server.tool_discovery {
            tracing::debug!(
                server_name = %name,
                "Skipping scoped MCP tool discovery by server config"
            );
            continue;
        }

        let bearer_token =
            match resolve_scoped_mcp_discovery_token(name, server, session_id, connection_resolver)
                .await
            {
                Ok(bearer_token) => bearer_token,
                Err(error) => {
                    tracing::warn!(
                        server_name = %name,
                        error = %error,
                        "Failed to resolve scoped MCP discovery token, skipping server"
                    );
                    continue;
                }
            };
        let has_authorization_header = has_authorization_header(&server.headers);
        if server.auth_mode == McpServerAuthMode::OAuth
            && !has_authorization_header
            && bearer_token.is_none()
        {
            tracing::debug!(
                server_name = %name,
                "Skipping scoped MCP tool discovery because no user connection token is available"
            );
            continue;
        }

        let tools = match fetch_mcp_tools(
            egress_service,
            &server.url,
            bearer_token.as_deref(),
            &server.headers,
        )
        .await
        {
            Ok(tools) => tools,
            Err(error) => {
                tracing::warn!(
                    server_name = %name,
                    error = %error,
                    "Failed to discover scoped MCP tools, skipping server"
                );
                continue;
            }
        };
        let capability_id = session_id
            .map(|id| scoped_mcp_server_uuid(id.uuid(), name))
            .unwrap_or_else(Uuid::nil);
        let capability = McpCapability::new(capability_id, name.clone(), None, tools);
        definitions.extend(capability.tool_definitions());
    }

    Ok(definitions)
}

async fn resolve_scoped_mcp_discovery_token(
    server_name: &str,
    server: &everruns_core::ScopedMcpServer,
    session_id: Option<SessionId>,
    connection_resolver: Option<&Arc<dyn UserConnectionResolver>>,
) -> Result<Option<String>> {
    if server.auth_mode != McpServerAuthMode::OAuth || has_authorization_header(&server.headers) {
        return Ok(None);
    }

    let Some(provider) = server.oauth_provider_id.as_deref() else {
        tracing::debug!(
            server_name,
            "Skipping scoped MCP discovery token lookup because oauth_provider_id is missing"
        );
        return Ok(None);
    };
    let Some(session_id) = session_id else {
        tracing::debug!(
            server_name,
            provider,
            "Skipping scoped MCP discovery token lookup because session_id is unavailable"
        );
        return Ok(None);
    };
    let Some(resolver) = connection_resolver else {
        tracing::debug!(
            server_name,
            provider,
            "Skipping scoped MCP discovery token lookup because connection resolver is unavailable"
        );
        return Ok(None);
    };

    resolver
        .get_connection_token(session_id, provider)
        .await
        .map_err(|error| anyhow!("Failed to resolve scoped MCP discovery token: {error}"))
}

fn has_authorization_header(headers: &HashMap<String, String>) -> bool {
    headers
        .keys()
        .any(|header_name| header_name.eq_ignore_ascii_case("Authorization"))
}

pub fn validate_scoped_mcp_servers(servers: &ScopedMcpServers) -> Result<()> {
    let mut sanitized = HashSet::new();

    for (name, server) in servers {
        if name.trim().is_empty() {
            return Err(anyhow!("Scoped MCP server name cannot be empty"));
        }
        if server.preset.is_some() {
            validate_catalog_reference_shape(name, server)?;
        } else {
            if server.acts_as != McpServerActsAs::None {
                return Err(anyhow!(
                    "Scoped MCP server '{name}' with actsAs '{}' requires a catalog preset for OAuth",
                    server.acts_as
                ));
            }
            // Local-process (stdio) transport is hard-off in the hosted product;
            // it is only available to single-tenant runtime/CLI hosts
            // (knowledge/integrations/runtime-mcp.md D2). Reject it here so it can never be
            // configured on an organization's harness/agent/session.
            if server.transport_type.is_local() {
                return Err(anyhow!(
                    "Scoped MCP server '{name}' uses an unsupported transport: \
                     stdio MCP servers are not allowed in this deployment"
                ));
            }
            validate_safe_url(&server.url)
                .map_err(|e| anyhow!("Invalid scoped MCP server URL for '{name}': {e}"))?;
        }
        let prefix = sanitize_mcp_server_name(name);
        if !everruns_core::mcp_server::is_valid_mcp_server_name(name) {
            return Err(anyhow!(
                "Scoped MCP server name '{name}' is invalid after sanitization: \
                 consecutive or trailing underscores are reserved for MCP tool prefix delimiters"
            ));
        }
        if !sanitized.insert(prefix) {
            return Err(anyhow!(
                "Scoped MCP server names must be unique after sanitization"
            ));
        }
    }

    Ok(())
}

pub fn validate_capability_mcp_servers(servers: &ScopedMcpServers) -> Result<()> {
    for (name, server) in servers {
        if server.preset.is_some() {
            return Err(anyhow!(
                "Capability-contributed MCP server '{name}' cannot use a catalog preset"
            ));
        }
        if server.acts_as != McpServerActsAs::None {
            return Err(anyhow!(
                "Capability-contributed MCP server '{name}' cannot set actsAs"
            ));
        }
    }
    validate_scoped_mcp_servers(servers)
}
fn validate_catalog_reference_shape(name: &str, server: &ScopedMcpServer) -> Result<()> {
    let conflicting_field = if !server.url.is_empty() {
        Some("url")
    } else if !server.headers.is_empty() {
        Some("headers")
    } else if server.command.is_some() {
        Some("command")
    } else if !server.args.is_empty() {
        Some("args")
    } else if !server.env.is_empty() {
        Some("env")
    } else if server.auth_mode != McpServerAuthMode::None {
        Some("auth_mode")
    } else if server.protocol_mode != McpProtocolMode::Auto {
        Some("protocol_mode")
    } else if server.oauth_provider_id.is_some() {
        Some("oauth_provider_id")
    } else if !server.tool_discovery {
        Some("tool_discovery")
    } else {
        None
    };
    if let Some(field) = conflicting_field {
        return Err(anyhow!(
            "Scoped MCP server '{name}' catalog preset reference cannot be combined with inline field '{field}'"
        ));
    }
    Ok(())
}
pub async fn validate_scoped_mcp_servers_for_org(
    db: &StorageBackend,
    org_id: i64,
    servers: &ScopedMcpServers,
) -> Result<()> {
    validate_scoped_mcp_servers(servers)?;
    for (name, server) in servers {
        let Some(preset) = &server.preset else {
            continue;
        };
        let preset_name = preset.catalog_name();
        let row = db
            .get_mcp_server_by_name(org_id, preset_name)
            .await?
            .ok_or_else(|| {
                anyhow!(
                    "Scoped MCP server '{name}' references missing catalog preset '{preset_name}'"
                )
            })?;
        if row.status != "active" {
            return Err(anyhow!(
                "Scoped MCP server '{name}' references catalog preset '{preset_name}' with non-live status '{}'",
                row.status
            ));
        }
        if server.acts_as != McpServerActsAs::None {
            let settings = McpServerService::settings_from_row(&row);
            if settings.auth_mode != McpServerAuthMode::OAuth || settings.oauth.is_none() {
                return Err(anyhow!(
                    "Scoped MCP server '{name}' with actsAs '{}' requires catalog preset '{preset_name}' to have OAuth configuration",
                    server.acts_as
                ));
            }
        }
    }
    Ok(())
}

pub async fn validate_merged_scoped_mcp_servers_for_org<'a, I>(
    db: &StorageBackend,
    org_id: i64,
    layers: I,
) -> Result<ScopedMcpServers>
where
    I: IntoIterator<Item = &'a ScopedMcpServers>,
{
    let merged = merge_scoped_mcp_server_layers(layers);
    validate_scoped_mcp_servers_for_org(db, org_id, &merged).await?;
    Ok(merged)
}

fn scoped_mcp_server_uuid(session_id: Uuid, server_name: &str) -> Uuid {
    Uuid::new_v5(&session_id, server_name.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel_imports::{HarnessId, ScopedMcpServer, SessionId};
    use crate::storage::models::{CreateMcpServerRow, UpdateMcpServer};
    use chrono::Utc;
    use everruns_platform::{Agent, AgentStatus, generate_agent_public_id};
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct MutableConnectionResolver {
        token: tokio::sync::RwLock<Option<String>>,
    }

    #[async_trait::async_trait]
    impl UserConnectionResolver for MutableConnectionResolver {
        async fn get_connection_token(
            &self,
            _session_id: SessionId,
            _provider: &str,
        ) -> everruns_provider::error::Result<Option<String>> {
            Ok(self.token.read().await.clone())
        }
    }

    #[derive(Default)]
    struct CountingConnectionResolver {
        calls: AtomicUsize,
    }

    #[async_trait::async_trait]
    impl UserConnectionResolver for CountingConnectionResolver {
        async fn get_connection_token(
            &self,
            _session_id: SessionId,
            _provider: &str,
        ) -> everruns_provider::error::Result<Option<String>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(Some("legacy-token".to_string()))
        }
    }

    #[derive(Default)]
    struct CatalogPreviewEgress {
        calls: AtomicUsize,
    }

    #[async_trait::async_trait]
    impl EgressService for CatalogPreviewEgress {
        async fn send(
            &self,
            request: everruns_core::EgressRequest,
        ) -> everruns_core::EgressResult<everruns_core::EgressResponse> {
            assert!(
                request
                    .headers
                    .keys()
                    .all(|name| !name.eq_ignore_ascii_case("Authorization"))
            );
            let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
            match body["method"].as_str().unwrap_or_default() {
                "initialize" => Ok(everruns_core::EgressResponse {
                    status: 200,
                    headers: std::collections::BTreeMap::from([(
                        "Mcp-Session-Id".to_string(),
                        "preview-session".to_string(),
                    )]),
                    body: serde_json::to_vec(&serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": 0,
                        "result": {
                            "protocolVersion": "2025-06-18",
                            "capabilities": {}
                        }
                    }))
                    .unwrap(),
                }),
                "notifications/initialized" => Ok(everruns_core::EgressResponse {
                    status: 202,
                    headers: Default::default(),
                    body: Vec::new(),
                }),
                "tools/list" => {
                    self.calls.fetch_add(1, Ordering::SeqCst);
                    Ok(everruns_core::EgressResponse {
                        status: 200,
                        headers: Default::default(),
                        body: serde_json::to_vec(&serde_json::json!({
                            "jsonrpc": "2.0",
                            "id": 1,
                            "result": {
                                "tools": [{
                                    "name": "echo",
                                    "description": "Echo a message",
                                    "inputSchema": {"type": "object"}
                                }]
                            }
                        }))
                        .unwrap(),
                    })
                }
                method => panic!("unexpected MCP preview method: {method}"),
            }
        }

        async fn send_stream(
            &self,
            _request: everruns_core::EgressRequest,
        ) -> everruns_core::EgressResult<everruns_core::EgressStreamResponse> {
            panic!("MCP discovery should not use streaming egress")
        }
    }

    fn scoped_server(url: &str) -> ScopedMcpServer {
        ScopedMcpServer {
            url: url.to_string(),
            ..Default::default()
        }
    }
    fn oauth_scoped_server(url: &str, provider: &str) -> ScopedMcpServer {
        ScopedMcpServer {
            url: url.to_string(),
            auth_mode: McpServerAuthMode::OAuth,
            oauth_provider_id: Some(provider.to_string()),
            ..Default::default()
        }
    }
    fn catalog_server(preset: &str, acts_as: McpServerActsAs) -> ScopedMcpServer {
        ScopedMcpServer {
            preset: Some(format!("catalog:{preset}").parse().unwrap()),
            acts_as,
            ..Default::default()
        }
    }
    async fn seed_catalog_server(
        db: &StorageBackend,
        name: &str,
        oauth: bool,
    ) -> everruns_provider::typed_id::McpServerId {
        let settings = crate::domains::mcp_servers::service::McpServerSettings {
            auth_mode: if oauth {
                McpServerAuthMode::OAuth
            } else {
                McpServerAuthMode::None
            },
            protocol_mode: McpProtocolMode::V2025June,
            oauth: oauth
                .then_some(crate::domains::mcp_servers::service::McpServerOAuthSettings::default()),
        };
        db.create_mcp_server(
            everruns_core::DEFAULT_ORG_ID,
            CreateMcpServerRow {
                name: name.to_string(),
                description: None,
                url: "http://8.8.8.8/mcp".to_string(),
                transport_type: "http".to_string(),
                api_key_encrypted: None,
                headers: Some(serde_json::json!({"X-Catalog":"value"})),
                settings: Some(serde_json::to_value(settings).unwrap()),
            },
        )
        .await
        .unwrap()
        .id
    }

    #[test]
    fn detects_authorization_header_case_insensitively() {
        let mut headers = HashMap::new();
        assert!(!has_authorization_header(&headers));

        headers.insert("authorization".to_string(), "Bearer token".to_string());
        assert!(has_authorization_header(&headers));
    }

    #[tokio::test]
    async fn newly_connected_token_is_visible_on_next_turn_in_same_session() {
        let session_id = SessionId::new();
        let server = oauth_scoped_server("https://mcp.resend.com/mcp", "mcp_oauth_resend");
        let resolver = Arc::new(MutableConnectionResolver {
            token: tokio::sync::RwLock::new(None),
        });
        let resolver_trait: Arc<dyn UserConnectionResolver> = resolver.clone();

        assert!(
            resolve_scoped_mcp_discovery_token(
                "resend",
                &server,
                Some(session_id),
                Some(&resolver_trait),
            )
            .await
            .unwrap()
            .is_none()
        );

        *resolver.token.write().await = Some("fresh-oauth-token".to_string());

        assert_eq!(
            resolve_scoped_mcp_discovery_token(
                "resend",
                &server,
                Some(session_id),
                Some(&resolver_trait),
            )
            .await
            .unwrap()
            .as_deref(),
            Some("fresh-oauth-token")
        );
    }

    fn test_harness() -> Harness {
        Harness {
            id: HarnessId::new(),
            name: "test-harness".to_string(),
            display_name: None,
            icon: None,
            description: None,
            intro_markdown: None,
            short_description: None,
            starters: Vec::new(),
            system_prompt: Some("harness".to_string()),
            parent_harness_id: None,
            default_model_id: None,
            tags: vec![],
            capabilities: vec![],
            initial_files: vec![],
            network_access: None,
            parallel_tool_calls: None,
            mcp_servers: Default::default(),
            embedder_metadata: Default::default(),
            is_built_in: false,
            status: everruns_platform::HarnessStatus::Active,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            archived_at: None,
            deleted_at: None,
        }
    }

    fn test_agent() -> Agent {
        let public_id = generate_agent_public_id();
        Agent {
            public_id,
            internal_id: public_id.uuid(),
            name: "test-agent".to_string(),
            display_name: None,
            description: None,
            intro_markdown: None,
            short_description: None,
            starters: Vec::new(),
            system_prompt: "agent".to_string(),
            default_model_id: None,
            harness_id: everruns_provider::typed_id::HarnessId::from_uuid(uuid::Uuid::nil()),
            default_version_id: None,
            forked_from_agent_id: None,
            forked_from_version_id: None,
            root_agent_id: None,
            tags: vec![],
            capabilities: vec![],
            initial_files: vec![],
            network_access: None,
            max_iterations: None,
            parallel_tool_calls: None,
            tools: vec![],
            mcp_servers: Default::default(),
            status: AgentStatus::Active,
            exposures_suspended: false,
            exposed: false,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            archived_at: None,
            deleted_at: None,
            usage: None,
        }
    }

    fn test_session(
        harness_id: HarnessId,
        agent_id: everruns_provider::typed_id::AgentId,
    ) -> Session {
        let session_id = SessionId::new();
        Session {
            source: Default::default(),
            activity: Default::default(),
            run_summary: None,
            id: session_id,
            // Default 1:1 session<->workspace: workspace.id mirrors the session id.
            workspace_id: everruns_provider::typed_id::WorkspaceId::from_uuid(session_id.uuid()),
            organization_id: everruns_core::DEFAULT_ORG_PUBLIC_ID.to_string(),
            harness_id,
            agent_id: Some(agent_id),
            agent_version_id: None,
            agent_identity_id: None,
            owner_principal_id: everruns_provider::typed_id::PrincipalId::from_seed(1),
            resolved_owner_user_id: None,
            owner: None,
            effective_owner: None,
            title: None,
            goal: None,
            locale: None,
            preview: None,
            output_preview: None,
            tags: vec![],
            model_id: None,
            capabilities: vec![],
            tools: vec![],
            mcp_servers: Default::default(),
            system_prompt: None,
            initial_files: vec![],
            hints: None,
            network_access: None,
            max_iterations: None,
            parallel_tool_calls: None,
            status: everruns_platform::SessionStatus::Started,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            started_at: None,
            finished_at: None,
            usage: None,
            is_pinned: None,
            archived_at: None,
            active_schedule_count: None,
            event_count: None,
            task_count: None,
            file_count: None,
            features: vec![],
            parent_session_id: None,
            forked_from_session_id: None,
            forked_from_sequence: None,
            blueprint_id: None,
            blueprint_config: None,
        }
    }

    #[test]
    fn merge_effective_scoped_mcp_servers_strips_explicit_oauth_fields() {
        let mut harness = test_harness();
        harness.mcp_servers.insert(
            "docs".to_string(),
            oauth_scoped_server("https://harness.example.com/mcp", "github"),
        );

        let agent = test_agent();
        let session = test_session(harness.id, agent.public_id);

        let merged = merge_effective_scoped_mcp_servers(&harness, Some(&agent), &session);
        let docs = merged.get("docs").expect("server exists");

        assert_eq!(docs.auth_mode, McpServerAuthMode::None);
        assert!(docs.oauth_provider_id.is_none());
        assert!(
            !docs.tool_discovery,
            "tool discovery must be disabled on stripped explicit OAuth servers — without a token, an authenticated tools/list would 401",
        );
    }

    #[test]
    fn merge_effective_scoped_mcp_servers_strips_explicit_only_when_oauth() {
        // Non-OAuth explicit entries (e.g. `auth_mode = None`) must pass through
        // unchanged. The sanitizer is narrow on purpose — see the doc comment on
        // strip_untrusted_oauth_from_scoped_mcp_servers.
        let mut harness = test_harness();
        harness.mcp_servers.insert(
            "docs".to_string(),
            scoped_server("https://harness.example.com/mcp"),
        );
        let agent = test_agent();
        let session = test_session(harness.id, agent.public_id);

        let merged = merge_effective_scoped_mcp_servers(&harness, Some(&agent), &session);
        let docs = merged.get("docs").expect("server exists");

        assert_eq!(docs.auth_mode, McpServerAuthMode::None);
        assert!(docs.oauth_provider_id.is_none());
        assert!(docs.tool_discovery);
    }

    #[test]
    fn merge_with_capabilities_preserves_capability_oauth_strips_explicit() {
        use everruns_capability::CapabilityRef as AgentCapabilityConfig;
        use everruns_core::capabilities::{Capability, CapabilityRegistry, RiskLevel};

        struct OAuthMcpCapability;

        impl Capability for OAuthMcpCapability {
            fn id(&self) -> &str {
                "oauth_mcp_test"
            }

            fn name(&self) -> &str {
                "OAuth MCP Test"
            }

            fn description(&self) -> &str {
                "Capability that contributes a scoped MCP server with OAuth"
            }

            fn risk_level(&self) -> RiskLevel {
                RiskLevel::Low
            }

            fn mcp_servers(&self) -> ScopedMcpServers {
                let mut servers = ScopedMcpServers::default();
                servers.insert(
                    "trusted_docs".to_string(),
                    oauth_scoped_server("https://capability.example.com/mcp", "github"),
                );
                servers
            }
        }

        let mut registry = CapabilityRegistry::new();
        registry.register(OAuthMcpCapability);

        let mut harness = test_harness();
        harness
            .capabilities
            .push(AgentCapabilityConfig::new("oauth_mcp_test"));
        // Explicit OAuth entry on a different name — must be sanitized.
        harness.mcp_servers.insert(
            "user_docs".to_string(),
            oauth_scoped_server("https://harness.example.com/mcp", "github"),
        );

        let agent = test_agent();
        let session = test_session(harness.id, agent.public_id);

        let merged = merge_effective_scoped_mcp_servers_with_capabilities(
            &harness,
            Some(&agent),
            &session,
            &registry,
        );

        let trusted = merged.get("trusted_docs").expect("contributed server");
        assert_eq!(
            trusted.auth_mode,
            McpServerAuthMode::OAuth,
            "capability-contributed OAuth must be preserved"
        );
        assert_eq!(trusted.oauth_provider_id.as_deref(), Some("github"));
        assert!(trusted.tool_discovery);

        let user = merged.get("user_docs").expect("explicit server");
        assert_eq!(
            user.auth_mode,
            McpServerAuthMode::None,
            "explicit OAuth must be stripped"
        );
        assert!(user.oauth_provider_id.is_none());
        assert!(!user.tool_discovery);
    }

    #[test]
    fn merge_effective_scoped_mcp_servers_prefers_more_specific_layers() {
        let mut harness = test_harness();
        harness.mcp_servers.insert(
            "docs".to_string(),
            scoped_server("https://harness.example.com/mcp"),
        );

        let mut agent = test_agent();
        agent.mcp_servers.insert(
            "docs".to_string(),
            scoped_server("https://agent.example.com/mcp"),
        );
        agent.mcp_servers.insert(
            "search".to_string(),
            scoped_server("https://agent-search.example.com/mcp"),
        );

        let mut session = test_session(harness.id, agent.public_id);
        session.mcp_servers.insert(
            "docs".to_string(),
            scoped_server("https://session.example.com/mcp"),
        );

        let merged = merge_effective_scoped_mcp_servers(&harness, Some(&agent), &session);

        assert_eq!(merged.len(), 2);
        assert_eq!(
            merged.get("docs").map(|server| server.url.as_str()),
            Some("https://session.example.com/mcp")
        );
        assert_eq!(
            merged.get("search").map(|server| server.url.as_str()),
            Some("https://agent-search.example.com/mcp")
        );
    }

    #[test]
    fn validate_scoped_mcp_servers_rejects_stdio_transport() {
        let mut servers = ScopedMcpServers::default();
        servers.insert(
            "fs".to_string(),
            ScopedMcpServer {
                transport_type: everruns_core::McpServerTransportType::Stdio,
                command: Some("mcp-server-filesystem".to_string()),
                ..Default::default()
            },
        );

        let error = validate_scoped_mcp_servers(&servers).unwrap_err();
        assert!(
            error.to_string().contains("stdio"),
            "expected stdio rejection, got: {error}"
        );
    }

    #[test]
    fn validate_scoped_mcp_servers_rejects_inline_identity_and_preset_fields() {
        for acts_as in [McpServerActsAs::Service, McpServerActsAs::User] {
            let servers = ScopedMcpServers::from([(
                "docs".into(),
                ScopedMcpServer {
                    url: "https://docs.example.com/mcp".into(),
                    acts_as,
                    ..Default::default()
                },
            )]);
            let error = validate_scoped_mcp_servers(&servers).unwrap_err();
            assert!(error.to_string().contains("requires a catalog preset"));
        }

        for (field, server) in [
            (
                "url",
                ScopedMcpServer {
                    url: "https://docs.example.com/mcp".into(),
                    ..catalog_server("linear", McpServerActsAs::None)
                },
            ),
            (
                "headers",
                ScopedMcpServer {
                    headers: HashMap::from([("X-Test".into(), "value".into())]),
                    ..catalog_server("linear", McpServerActsAs::None)
                },
            ),
        ] {
            let servers = ScopedMcpServers::from([("docs".into(), server)]);
            let error = validate_scoped_mcp_servers(&servers).unwrap_err();
            assert!(error.to_string().contains(field), "{error}");
        }
    }

    #[tokio::test]
    async fn catalog_validation_requires_live_existing_oauth_presets() {
        let db = StorageBackend::in_memory();
        let org_id = everruns_core::DEFAULT_ORG_ID;
        let missing = ScopedMcpServers::from([(
            "docs".into(),
            catalog_server("missing", McpServerActsAs::None),
        )]);
        let error = validate_scoped_mcp_servers_for_org(&db, org_id, &missing)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("missing"), "{error}");

        let plain_id = seed_catalog_server(&db, "plain", false).await;
        let needs_oauth = ScopedMcpServers::from([(
            "docs".into(),
            catalog_server("plain", McpServerActsAs::Service),
        )]);
        let error = validate_scoped_mcp_servers_for_org(&db, org_id, &needs_oauth)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("OAuth"), "{error}");
        db.update_mcp_server(
            org_id,
            plain_id.uuid(),
            UpdateMcpServer {
                status: Some("disabled".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let disabled = ScopedMcpServers::from([(
            "docs".into(),
            catalog_server("plain", McpServerActsAs::None),
        )]);
        let error = validate_scoped_mcp_servers_for_org(&db, org_id, &disabled)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("disabled"), "{error}");

        db.update_mcp_server(
            org_id,
            plain_id.uuid(),
            UpdateMcpServer {
                status: Some("archived".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let archived = ScopedMcpServers::from([(
            "docs".into(),
            catalog_server("plain", McpServerActsAs::None),
        )]);
        let error = validate_scoped_mcp_servers_for_org(&db, org_id, &archived)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("plain"), "{error}");

        let deleted_id = seed_catalog_server(&db, "deleted", false).await;
        db.update_mcp_server(
            org_id,
            deleted_id.uuid(),
            UpdateMcpServer {
                status: Some("deleted".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let deleted = ScopedMcpServers::from([(
            "docs".into(),
            catalog_server("deleted", McpServerActsAs::None),
        )]);
        let error = validate_scoped_mcp_servers_for_org(&db, org_id, &deleted)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("deleted"), "{error}");
    }

    #[tokio::test]
    async fn two_logical_names_can_resolve_the_same_catalog_preset() {
        let db = Arc::new(StorageBackend::in_memory());
        seed_catalog_server(&db, "linear", true).await;
        let service = McpServerService::new(db, None);
        let harness = test_harness();
        let agent = test_agent();
        let mut session = test_session(harness.id, agent.public_id);
        session.mcp_servers = ScopedMcpServers::from([
            (
                "issues".into(),
                catalog_server("linear", McpServerActsAs::Service),
            ),
            (
                "projects".into(),
                catalog_server("linear", McpServerActsAs::User),
            ),
        ]);

        for (prefix, acts_as) in [
            ("issues", McpServerActsAs::Service),
            ("projects", McpServerActsAs::User),
        ] {
            let resolved = resolve_scoped_mcp_server(
                &service,
                everruns_core::DEFAULT_ORG_ID,
                &harness,
                Some(&agent),
                &session,
                prefix,
            )
            .await
            .unwrap()
            .unwrap();
            assert_eq!(resolved.name, prefix);
            assert_eq!(resolved.url, "http://8.8.8.8/mcp");
            assert_eq!(resolved.auth_mode, McpServerAuthMode::None);
            assert_eq!(resolved.protocol_mode, McpProtocolMode::V2025June);
            assert!(resolved.oauth_provider_id.is_none());
            assert!(resolved.api_key.is_none());
            assert_eq!(resolved.acts_as, acts_as);
            assert_eq!(resolved.headers.get("X-Catalog"), Some(&"value".into()));
        }
    }
    #[tokio::test]
    async fn catalog_preview_discovery_never_uses_legacy_connection_tokens() {
        let db = StorageBackend::in_memory();
        seed_catalog_server(&db, "linear", true).await;
        let resolver = Arc::new(CountingConnectionResolver::default());
        let resolver_trait: Arc<dyn UserConnectionResolver> = resolver.clone();
        let egress = CatalogPreviewEgress::default();

        for acts_as in [
            McpServerActsAs::None,
            McpServerActsAs::Service,
            McpServerActsAs::User,
        ] {
            let servers =
                ScopedMcpServers::from([("docs".to_string(), catalog_server("linear", acts_as))]);
            let materialized =
                materialize_scoped_mcp_servers(&db, everruns_core::DEFAULT_ORG_ID, &servers)
                    .await
                    .unwrap();
            let docs = materialized.get("docs").unwrap();
            assert_eq!(docs.auth_mode, McpServerAuthMode::None);
            assert!(docs.oauth_provider_id.is_none());
            assert_eq!(docs.acts_as, acts_as);
            let discovered = fetch_mcp_tools(&egress, &docs.url, None, &docs.headers)
                .await
                .unwrap();
            assert_eq!(discovered.len(), 1);

            let tools = build_materialized_scoped_mcp_tool_definitions(
                &db,
                everruns_core::DEFAULT_ORG_ID,
                &servers,
                Some(SessionId::new()),
                Some(&resolver_trait),
                &egress,
            )
            .await
            .unwrap();
            assert_eq!(tools.len(), 1, "catalog preview must expose one MCP tool");
            assert!(tools[0].name().contains("echo"));
        }

        assert_eq!(resolver.calls.load(Ordering::SeqCst), 0);
        assert_eq!(egress.calls.load(Ordering::SeqCst), 6);
    }

    #[test]
    fn capability_mcp_validation_rejects_catalog_presets_and_execution_identity() {
        let preset = ScopedMcpServers::from([(
            "docs".to_string(),
            catalog_server("linear", McpServerActsAs::None),
        )]);
        let error = validate_capability_mcp_servers(&preset).unwrap_err();
        assert!(error.to_string().contains("cannot use a catalog preset"));

        let identity = ScopedMcpServers::from([(
            "docs".to_string(),
            ScopedMcpServer {
                url: "https://docs.example.com/mcp".to_string(),
                acts_as: McpServerActsAs::Service,
                ..Default::default()
            },
        )]);
        let error = validate_capability_mcp_servers(&identity).unwrap_err();
        assert!(error.to_string().contains("cannot set actsAs"));
    }

    #[test]
    fn validate_scoped_mcp_servers_rejects_duplicate_sanitized_names() {
        let mut servers = ScopedMcpServers::default();
        servers.insert(
            "Docs API".to_string(),
            scoped_server("https://one.example.com/mcp"),
        );
        servers.insert(
            "docs-api".to_string(),
            scoped_server("https://two.example.com/mcp"),
        );

        let error = validate_scoped_mcp_servers(&servers).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("must be unique after sanitization")
        );
    }

    #[test]
    fn validate_scoped_mcp_servers_rejects_reserved_delimiter_after_sanitization() {
        for name in ["admin__foo", "admin..foo", "admin_", "admin-", "_"] {
            let servers = ScopedMcpServers::from([(
                name.into(),
                scoped_server("https://one.example.com/mcp"),
            )]);
            let error = validate_scoped_mcp_servers(&servers).unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("reserved for MCP tool prefix delimiters"),
                "{name}: {error}"
            );
        }
        let servers = ScopedMcpServers::from([(
            "admin_api".into(),
            scoped_server("https://one.example.com/mcp"),
        )]);
        validate_scoped_mcp_servers(&servers).unwrap();
    }

    #[test]
    fn validate_merged_scoped_mcp_servers_rejects_cross_layer_duplicates() {
        let mut harness_servers = ScopedMcpServers::default();
        harness_servers.insert(
            "Docs API".to_string(),
            scoped_server("https://one.example.com/mcp"),
        );

        let mut session_servers = ScopedMcpServers::default();
        session_servers.insert(
            "docs-api".to_string(),
            scoped_server("https://two.example.com/mcp"),
        );

        let error =
            validate_merged_scoped_mcp_servers([&harness_servers, &session_servers]).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("must be unique after sanitization")
        );
    }

    #[test]
    fn scoped_mcp_server_uuid_is_stable_and_namespaced_by_session() {
        let session_a = SessionId::new().uuid();
        let session_b = SessionId::new().uuid();

        assert_eq!(
            scoped_mcp_server_uuid(session_a, "docs"),
            scoped_mcp_server_uuid(session_a, "docs")
        );
        assert_ne!(
            scoped_mcp_server_uuid(session_a, "docs"),
            scoped_mcp_server_uuid(session_b, "docs")
        );
    }
}

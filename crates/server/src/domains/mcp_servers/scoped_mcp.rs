// Scoped MCP helpers.
//
// Decision: harness/agent/session-scoped remote MCP servers are merged with
// last-wins semantics by logical name, then resolved ahead of org-scoped MCP
// servers. Tool discovery is live (no persisted cache) to keep this feature
// narrowly scoped and avoid mutating config rows during runtime.

use anyhow::{Result, anyhow};
use everruns_core::capabilities::{CapabilityRegistry, collect_capability_mcp_servers};
use everruns_core::mcp_server::sanitize_mcp_server_name;
use everruns_core::traits::UserConnectionResolver;
use everruns_core::{
    Agent, Capability, Harness, McpCapability, McpServerAuthMode, ScopedMcpServers, Session,
    SessionId, ToolDefinition, merge_scoped_mcp_servers, resolve_runtime_capabilities,
    validate_safe_url,
};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use uuid::Uuid;

use crate::domains::mcp_servers::McpServerResolved;
use crate::domains::mcp_servers::service::fetch_mcp_tools;

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
    merge_scoped_mcp_server_layers(layers)
}

pub fn merge_effective_scoped_mcp_servers_with_capabilities(
    harness: &Harness,
    agent: Option<&Agent>,
    session: &Session,
    capability_registry: &CapabilityRegistry,
) -> ScopedMcpServers {
    let explicit = merge_effective_scoped_mcp_servers(harness, agent, session);
    let resolved = resolve_runtime_capabilities(
        std::slice::from_ref(harness),
        agent,
        session,
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

pub fn resolve_scoped_mcp_server(
    harness: &Harness,
    agent: Option<&Agent>,
    session: &Session,
    server_prefix: &str,
) -> Option<McpServerResolved> {
    let effective = merge_effective_scoped_mcp_servers(harness, agent, session);
    effective.into_iter().find_map(|(name, server)| {
        (sanitize_mcp_server_name(&name) == server_prefix).then(|| McpServerResolved {
            id: scoped_mcp_server_uuid(session.id.uuid(), &name),
            name,
            url: server.url,
            auth_mode: server.auth_mode,
            oauth_provider_id: server.oauth_provider_id,
            api_key: None,
            headers: server.headers,
        })
    })
}

pub fn resolve_scoped_mcp_server_with_capabilities(
    harness: &Harness,
    agent: Option<&Agent>,
    session: &Session,
    server_prefix: &str,
    capability_registry: &CapabilityRegistry,
) -> Option<McpServerResolved> {
    let effective = merge_effective_scoped_mcp_servers_with_capabilities(
        harness,
        agent,
        session,
        capability_registry,
    );
    effective.into_iter().find_map(|(name, server)| {
        (sanitize_mcp_server_name(&name) == server_prefix).then(|| McpServerResolved {
            id: scoped_mcp_server_uuid(session.id.uuid(), &name),
            name,
            url: server.url,
            auth_mode: server.auth_mode,
            oauth_provider_id: server.oauth_provider_id,
            api_key: None,
            headers: server.headers,
        })
    })
}

pub async fn build_scoped_mcp_tool_definitions(
    servers: &ScopedMcpServers,
    session_id: Option<SessionId>,
    connection_resolver: Option<&Arc<dyn UserConnectionResolver>>,
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

        let tools =
            match fetch_mcp_tools(&server.url, bearer_token.as_deref(), &server.headers).await {
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
        let capability = McpCapability::new(Uuid::nil(), name.clone(), None, tools);
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
        validate_safe_url(&server.url)
            .map_err(|e| anyhow!("Invalid scoped MCP server URL for '{name}': {e}"))?;
        let prefix = sanitize_mcp_server_name(name);
        if prefix.contains("__") {
            return Err(anyhow!(
                "Scoped MCP server name '{name}' is invalid after sanitization: \
                 consecutive underscores are reserved for MCP tool prefix delimiters"
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

fn scoped_mcp_server_uuid(session_id: Uuid, server_name: &str) -> Uuid {
    Uuid::new_v5(&session_id, server_name.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use everruns_core::{
        Agent, AgentStatus, Harness, HarnessId, HarnessStatus, ScopedMcpServer, Session, SessionId,
        SessionStatus, generate_agent_public_id,
    };

    fn scoped_server(url: &str) -> ScopedMcpServer {
        ScopedMcpServer {
            transport_type: everruns_core::McpServerTransportType::Http,
            url: url.to_string(),
            headers: Default::default(),
            auth_mode: McpServerAuthMode::None,
            oauth_provider_id: None,
            tool_discovery: true,
        }
    }

    #[test]
    fn detects_authorization_header_case_insensitively() {
        let mut headers = HashMap::new();
        assert!(!has_authorization_header(&headers));

        headers.insert("authorization".to_string(), "Bearer token".to_string());
        assert!(has_authorization_header(&headers));
    }

    fn test_harness() -> Harness {
        Harness {
            id: HarnessId::new(),
            name: "test-harness".to_string(),
            display_name: None,
            description: None,
            system_prompt: "harness".to_string(),
            parent_harness_id: None,
            default_model_id: None,
            tags: vec![],
            capabilities: vec![],
            initial_files: vec![],
            network_access: None,
            mcp_servers: Default::default(),
            is_built_in: false,
            status: HarnessStatus::Active,
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
            system_prompt: "agent".to_string(),
            default_model_id: None,
            tags: vec![],
            capabilities: vec![],
            initial_files: vec![],
            network_access: None,
            max_iterations: None,
            tools: vec![],
            mcp_servers: Default::default(),
            status: AgentStatus::Active,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            archived_at: None,
            deleted_at: None,
            usage: None,
        }
    }

    fn test_session(harness_id: HarnessId, agent_id: everruns_core::AgentId) -> Session {
        Session {
            id: SessionId::new(),
            organization_id: everruns_core::DEFAULT_ORG_PUBLIC_ID.to_string(),
            harness_id,
            agent_id: Some(agent_id),
            agent_identity_id: None,
            owner_principal_id: everruns_core::PrincipalId::from_seed(1),
            resolved_owner_user_id: None,
            owner: None,
            effective_owner: None,
            title: None,
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
            status: SessionStatus::Started,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            started_at: None,
            finished_at: None,
            usage: None,
            is_pinned: None,
            active_schedule_count: None,
            features: vec![],
            parent_session_id: None,
            subagent_name: None,
            subagent_task: None,
            subagent_status: None,
            blueprint_id: None,
            blueprint_config: None,
        }
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
        let mut servers = ScopedMcpServers::default();
        servers.insert(
            "admin__foo".to_string(),
            scoped_server("https://one.example.com/mcp"),
        );

        let error = validate_scoped_mcp_servers(&servers).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("reserved for MCP tool prefix delimiters")
        );
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

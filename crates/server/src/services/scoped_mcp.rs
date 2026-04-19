// Scoped MCP helpers.
//
// Decision: harness/agent/session-scoped remote MCP servers are merged with
// last-wins semantics by logical name, then resolved ahead of org-scoped MCP
// servers. Tool discovery is live (no persisted cache) to keep this feature
// narrowly scoped and avoid mutating config rows during runtime.

use anyhow::{Result, anyhow};
use everruns_core::mcp_server::sanitize_mcp_server_name;
use everruns_core::{
    Agent, Capability, Harness, McpCapability, McpServerAuthMode, ScopedMcpServers, Session,
    ToolDefinition, merge_scoped_mcp_servers, validate_safe_url,
};
use std::collections::HashSet;
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
            auth_mode: McpServerAuthMode::None,
            oauth_provider_id: None,
            api_key: None,
            headers: server.headers,
        })
    })
}

pub async fn build_scoped_mcp_tool_definitions(
    servers: &ScopedMcpServers,
) -> Result<Vec<ToolDefinition>> {
    let mut definitions = Vec::new();

    for (name, server) in servers {
        let tools = fetch_mcp_tools(&server.url, None, &server.headers).await?;
        let capability = McpCapability::new(Uuid::nil(), name.clone(), None, tools);
        definitions.extend(capability.tool_definitions());
    }

    Ok(definitions)
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
        }
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

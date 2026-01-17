// Capability service - business logic for capabilities
//
// Uses CapabilityRegistry from everruns-core as the single source of truth
// for capability definitions.
//
// MCP servers are integrated as "virtual capabilities" that appear alongside
// built-in capabilities. Their tools are prefixed with "mcp_{server_name}_".
//
// Note: Agent-specific capability management is handled by AgentService.

use crate::services::mcp_server::McpServerService;
use crate::storage::{EncryptionService, StorageBackend};
use anyhow::Result;
use everruns_core::capabilities::{Capability, CapabilityRegistry};
use everruns_core::{
    CapabilityId, CapabilityInfo, CapabilityStatus, McpCapability, McpToolDefinition,
    mcp_capability_id,
};
use std::sync::Arc;

pub struct CapabilityService {
    db: Arc<StorageBackend>,
    registry: CapabilityRegistry,
    mcp_service: McpServerService,
}

impl CapabilityService {
    pub fn new(db: Arc<StorageBackend>, encryption: Option<Arc<EncryptionService>>) -> Self {
        Self {
            db: db.clone(),
            registry: CapabilityRegistry::with_builtins(),
            mcp_service: McpServerService::new(db, encryption),
        }
    }

    /// List all available capabilities including MCP servers (public info only)
    pub async fn list_all(&self) -> Result<Vec<CapabilityInfo>> {
        // Get built-in capabilities
        let mut capabilities: Vec<CapabilityInfo> = self
            .registry
            .list()
            .into_iter()
            .map(|cap| CapabilityInfo::from_core(cap.as_ref()))
            .collect();

        // Get MCP server capabilities
        let mcp_servers = self.mcp_service.list_active_with_tools().await?;
        for server_with_tools in mcp_servers {
            let mcp_cap = McpCapability::new(
                server_with_tools.server.id,
                server_with_tools.server.name.clone(),
                server_with_tools.server.description.clone(),
                server_with_tools.cached_tools.clone(),
            );

            // Create CapabilityInfo from MCP capability
            let tool_count = server_with_tools.cached_tools.len();
            let description = server_with_tools
                .server
                .description
                .clone()
                .unwrap_or_else(|| format!("MCP Server with {} tool(s)", tool_count));

            capabilities.push(CapabilityInfo {
                id: CapabilityId::new(mcp_capability_id(server_with_tools.server.id)),
                name: server_with_tools.server.name.clone(),
                description,
                status: CapabilityStatus::Available,
                icon: Some("mcp".to_string()), // Official MCP logo
                category: Some("MCP Servers".to_string()),
                system_prompt: None,
                tool_definitions: mcp_cap.tool_definitions(),
                is_mcp: true,
            });
        }

        Ok(capabilities)
    }

    /// Get a specific capability by ID
    pub async fn get(&self, id: &CapabilityId) -> Result<Option<CapabilityInfo>> {
        // Check if it's an MCP capability
        if let Some(server_id) = id.mcp_server_id() {
            let tools = self.mcp_service.get_tools(server_id, false).await?;
            let server = self.mcp_service.get(server_id).await?;

            if let Some(server) = server {
                let mcp_cap = McpCapability::new(
                    server.id,
                    server.name.clone(),
                    server.description.clone(),
                    tools.clone(),
                );

                let tool_count = tools.len();
                let description = server
                    .description
                    .clone()
                    .unwrap_or_else(|| format!("MCP Server with {} tool(s)", tool_count));

                return Ok(Some(CapabilityInfo {
                    id: CapabilityId::new(mcp_capability_id(server.id)),
                    name: server.name,
                    description,
                    status: CapabilityStatus::Available,
                    icon: Some("mcp".to_string()), // Official MCP logo
                    category: Some("MCP Servers".to_string()),
                    system_prompt: None,
                    tool_definitions: mcp_cap.tool_definitions(),
                    is_mcp: true,
                }));
            }
            return Ok(None);
        }

        // Regular capability
        Ok(self
            .registry
            .get(id.as_str())
            .map(|cap| CapabilityInfo::from_core(cap.as_ref())))
    }

    /// Get MCP tools for a specific MCP server capability
    #[allow(dead_code)]
    pub async fn get_mcp_tools(
        &self,
        server_id: uuid::Uuid,
        force_refresh: bool,
    ) -> Result<Vec<McpToolDefinition>> {
        self.mcp_service.get_tools(server_id, force_refresh).await
    }

    /// Refresh tools for an MCP server
    #[allow(dead_code)]
    pub async fn refresh_mcp_tools(&self, server_id: uuid::Uuid) -> Result<Vec<McpToolDefinition>> {
        self.mcp_service.refresh_tools(server_id).await
    }

    /// Get the built-in capability registry
    #[allow(dead_code)]
    pub fn registry(&self) -> &CapabilityRegistry {
        &self.registry
    }

    /// Get the database storage backend
    #[allow(dead_code)]
    pub fn db(&self) -> &Arc<StorageBackend> {
        &self.db
    }

    /// Preview the final agent shape by computing the merged system prompt and tools.
    ///
    /// This collects contributions from all specified capabilities:
    /// - System prompt additions are prepended to the base prompt
    /// - Tool definitions are collected from all capabilities
    ///
    /// # Arguments
    ///
    /// * `base_system_prompt` - The agent's base system prompt
    /// * `capability_configs` - The capabilities to apply with their configs
    ///
    /// # Returns
    ///
    /// A tuple of (final_system_prompt, tool_definitions)
    pub async fn preview(
        &self,
        base_system_prompt: &str,
        capability_configs: &[everruns_core::AgentCapabilityConfig],
    ) -> Result<(String, Vec<everruns_core::ToolDefinition>)> {
        use everruns_core::capabilities::collect_capabilities;

        let mut system_prompt_parts: Vec<String> = Vec::new();
        let mut tool_definitions: Vec<everruns_core::ToolDefinition> = Vec::new();

        // Separate built-in capabilities from MCP capabilities
        let mut builtin_cap_ids: Vec<String> = Vec::new();
        let mut mcp_cap_ids: Vec<uuid::Uuid> = Vec::new();

        for cap_config in capability_configs {
            let cap_ref = &cap_config.capability_ref;
            if let Some(server_id) = cap_ref.mcp_server_id() {
                mcp_cap_ids.push(server_id);
            } else {
                builtin_cap_ids.push(cap_ref.to_string());
            }
        }

        // Collect from built-in capabilities
        let collected = collect_capabilities(&builtin_cap_ids, &self.registry);
        if let Some(prefix) = collected.system_prompt_prefix() {
            system_prompt_parts.push(prefix);
        }
        tool_definitions.extend(collected.tool_definitions);

        // Collect from MCP capabilities
        for server_id in mcp_cap_ids {
            let tools = self.mcp_service.get_tools(server_id, false).await?;
            let server = self.mcp_service.get(server_id).await?;

            if let Some(server) = server {
                let mcp_cap = McpCapability::new(
                    server.id,
                    server.name.clone(),
                    server.description.clone(),
                    tools,
                );
                tool_definitions.extend(mcp_cap.tool_definitions());
            }
        }

        // Build final system prompt
        let final_system_prompt = if system_prompt_parts.is_empty() {
            base_system_prompt.to_string()
        } else {
            format!(
                "{}\n\n{}",
                system_prompt_parts.join("\n\n"),
                base_system_prompt
            )
        };

        Ok((final_system_prompt, tool_definitions))
    }
}

// Capability service - business logic for capabilities
//
// Uses CapabilityRegistry from everruns-core as the single source of truth
// for capability definitions.
//
// MCP servers are integrated as "virtual capabilities" that appear alongside
// built-in capabilities. Their tools are prefixed with "mcp_{server_name}_".
//
// Skills from the database registry are integrated as virtual capabilities,
// following the same pattern as MCP servers.
//
// Note: Agent-specific capability management is handled by AgentService.

use crate::services::mcp_server::McpServerService;
use crate::services::skill::SkillService;
use crate::storage::{EncryptionService, StorageBackend};
use anyhow::Result;
use everruns_core::capabilities::{Capability, CapabilityRegistry};
use everruns_core::{
    CapabilityId, CapabilityInfo, CapabilityStatus, McpCapability, McpToolDefinition,
    SkillCapability, SkillInstructions, SkillMeta, SkillSource, mcp_capability_id,
    skill_capability_id,
};
use std::sync::Arc;

pub struct CapabilityService {
    db: Arc<StorageBackend>,
    registry: CapabilityRegistry,
    mcp_service: McpServerService,
    skill_service: Arc<SkillService>,
}

impl CapabilityService {
    pub fn new(db: Arc<StorageBackend>, encryption: Option<Arc<EncryptionService>>) -> Self {
        Self {
            db: db.clone(),
            registry: CapabilityRegistry::with_builtins(),
            mcp_service: McpServerService::new(db.clone(), encryption),
            skill_service: Arc::new(SkillService::new(db)),
        }
    }

    /// List all available capabilities including MCP servers and skills (public info only)
    pub async fn list_all(&self, org_id: i64) -> Result<Vec<CapabilityInfo>> {
        // Get built-in capabilities
        let mut capabilities: Vec<CapabilityInfo> = self
            .registry
            .list()
            .into_iter()
            .map(|cap| CapabilityInfo::from_core(cap.as_ref()))
            .collect();

        // Get MCP server capabilities
        let mcp_servers = self.mcp_service.list_active_with_tools(org_id).await?;
        for server_with_tools in mcp_servers {
            let mcp_cap = McpCapability::new(
                server_with_tools.server.id.uuid(),
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
                id: CapabilityId::new(mcp_capability_id(server_with_tools.server.id.uuid())),
                name: server_with_tools.server.name.clone(),
                description,
                status: CapabilityStatus::Available,
                icon: Some("mcp".to_string()), // Official MCP logo
                category: Some("MCP Servers".to_string()),
                system_prompt: None,
                tool_definitions: mcp_cap.tool_definitions(),
                is_mcp: true,
                is_skill: false,
                dependencies: vec![], // MCP capabilities have no dependencies
            });
        }

        // Get skill capabilities from the registry
        let skills = self.skill_service.list(org_id).await?;
        for skill in &skills {
            if skill.status != everruns_core::SkillStatus::Active {
                continue;
            }

            let skill_cap = SkillCapability::from_registry(
                skill.id.uuid(),
                skill.name.clone(),
                skill.description.clone(),
                String::new(), // Instructions not needed for listing
                vec![],
            );

            capabilities.push(CapabilityInfo {
                id: CapabilityId::new(skill_capability_id(skill.id.uuid())),
                name: skill.name.clone(),
                description: skill.description.clone(),
                status: CapabilityStatus::Available,
                icon: Some("wand".to_string()),
                category: Some("Skills".to_string()),
                system_prompt: skill_cap.system_prompt_preview(),
                tool_definitions: skill_cap.tool_definitions(),
                is_mcp: false,
                is_skill: true,
                dependencies: vec!["session_file_system".to_string()],
            });
        }

        Ok(capabilities)
    }

    /// Get a specific capability by ID
    ///
    /// For MCP capabilities, uses cached tools only (no external refresh).
    /// This ensures viewing a capability doesn't fail if the MCP server is unreachable.
    /// Use the refresh tools endpoint to explicitly update cached tools.
    pub async fn get(&self, org_id: i64, id: &CapabilityId) -> Result<Option<CapabilityInfo>> {
        // Check if it's an MCP capability
        if let Some(server_id) = id.mcp_server_id() {
            // Use cached tools only - no external refresh on read
            let tools = self.mcp_service.get_cached_tools(org_id, server_id).await;
            let server = self.mcp_service.get(org_id, server_id).await?;

            if let Some(server) = server {
                let mcp_cap = McpCapability::new(
                    server.id.uuid(),
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
                    id: CapabilityId::new(mcp_capability_id(server.id.uuid())),
                    name: server.name,
                    description,
                    status: CapabilityStatus::Available,
                    icon: Some("mcp".to_string()), // Official MCP logo
                    category: Some("MCP Servers".to_string()),
                    system_prompt: None,
                    tool_definitions: mcp_cap.tool_definitions(),
                    is_mcp: true,
                    is_skill: false,
                    dependencies: vec![], // MCP capabilities have no dependencies
                }));
            }
            return Ok(None);
        }

        // Check if it's a skill capability
        if let Some(skill_uuid) = id.skill_id() {
            let skill = self.skill_service.get(org_id, skill_uuid).await?;
            if let Some(skill) = skill {
                let content = self.skill_service.get_content(org_id, skill_uuid).await?;

                let files: Vec<(String, String)> = content
                    .as_ref()
                    .map(|c| {
                        c.files
                            .iter()
                            .map(|f| (f.path.clone(), f.content.clone()))
                            .collect()
                    })
                    .unwrap_or_default();

                let skill_cap = SkillCapability::from_registry(
                    skill.id.uuid(),
                    skill.name.clone(),
                    skill.description.clone(),
                    content.map(|c| c.skill_md).unwrap_or_default(),
                    files,
                );

                return Ok(Some(CapabilityInfo {
                    id: CapabilityId::new(skill_capability_id(skill.id.uuid())),
                    name: skill.name.clone(),
                    description: skill.description.clone(),
                    status: CapabilityStatus::Available,
                    icon: Some("wand".to_string()),
                    category: Some("Skills".to_string()),
                    system_prompt: skill_cap.system_prompt_preview(),
                    tool_definitions: skill_cap.tool_definitions(),
                    is_mcp: false,
                    is_skill: true,
                    dependencies: vec!["session_file_system".to_string()],
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
        org_id: i64,
        server_id: uuid::Uuid,
        force_refresh: bool,
    ) -> Result<Vec<McpToolDefinition>> {
        self.mcp_service
            .get_tools(org_id, server_id, force_refresh)
            .await
    }

    /// Refresh tools for an MCP server
    #[allow(dead_code)]
    pub async fn refresh_mcp_tools(
        &self,
        org_id: i64,
        server_id: uuid::Uuid,
    ) -> Result<Vec<McpToolDefinition>> {
        self.mcp_service.refresh_tools(org_id, server_id).await
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
    /// This collects contributions from all specified capabilities and their dependencies:
    /// - System prompt additions are prepended to the base prompt
    /// - Tool definitions are collected from all capabilities
    /// - Dependencies are automatically resolved (dependencies before dependents)
    ///
    /// For MCP capabilities, uses cached tools (even if stale) to avoid blocking
    /// on external MCP server calls during preview.
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
        org_id: i64,
        base_system_prompt: &str,
        capability_configs: &[everruns_core::AgentCapabilityConfig],
    ) -> Result<(String, Vec<everruns_core::ToolDefinition>)> {
        use everruns_core::capabilities::{
            SystemPromptContext, collect_capabilities, resolve_dependencies,
        };

        let mut system_prompt_parts: Vec<String> = Vec::new();
        let mut tool_definitions: Vec<everruns_core::ToolDefinition> = Vec::new();

        // Separate built-in capabilities from MCP and skill capabilities
        let mut builtin_cap_ids: Vec<String> = Vec::new();
        let mut mcp_cap_ids: Vec<uuid::Uuid> = Vec::new();
        let mut skill_cap_ids: Vec<uuid::Uuid> = Vec::new();

        for cap_config in capability_configs {
            let cap_ref = &cap_config.capability_ref;
            if let Some(server_id) = cap_ref.mcp_server_id() {
                mcp_cap_ids.push(server_id);
            } else if let Some(skill_id) = cap_ref.skill_id() {
                skill_cap_ids.push(skill_id);
            } else {
                builtin_cap_ids.push(cap_ref.to_string());
            }
        }

        // Resolve dependencies for built-in capabilities
        let resolved = resolve_dependencies(&builtin_cap_ids, &self.registry)
            .map_err(|e| anyhow::anyhow!("Failed to resolve capability dependencies: {}", e))?;

        // Collect from resolved capabilities (includes dependencies in correct order)
        // Preview has no session context, so dynamic capabilities (agent_instructions) return None
        let ctx = SystemPromptContext::without_file_store(everruns_core::SessionId::new());
        let collected = collect_capabilities(&resolved.resolved_ids, &self.registry, &ctx).await;
        if let Some(prefix) = collected.system_prompt_prefix() {
            system_prompt_parts.push(prefix);
        }
        tool_definitions.extend(collected.tool_definitions);

        // Collect from MCP capabilities using cached tools only (no refresh)
        // Note: MCP capabilities don't have dependencies
        for server_id in mcp_cap_ids {
            let tools = self.mcp_service.get_cached_tools(org_id, server_id).await;
            let server = self.mcp_service.get(org_id, server_id).await?;

            if let Some(server) = server {
                let mcp_cap = McpCapability::new(
                    server.id.uuid(),
                    server.name.clone(),
                    server.description.clone(),
                    tools,
                );
                tool_definitions.extend(mcp_cap.tool_definitions());
            }
        }

        // Collect from skill capabilities
        let mut skill_metas: Vec<SkillMeta> = Vec::new();
        for skill_uuid in &skill_cap_ids {
            if let Some(skill) = self.skill_service.get(org_id, *skill_uuid).await? {
                skill_metas.push(SkillMeta {
                    name: skill.name,
                    description: skill.description,
                    source: SkillSource::Registry {
                        skill_id: skill_uuid.to_string(),
                    },
                });
            }
        }

        if !skill_metas.is_empty() {
            let skill_cap = SkillCapability::from_discovered(skill_metas);

            // Load instructions for each skill
            for skill_uuid in &skill_cap_ids {
                if let Some(content) = self.skill_service.get_content(org_id, *skill_uuid).await?
                    && let Ok(parsed) = everruns_core::parse_skill_md(&content.skill_md)
                {
                    let files: Vec<(String, String)> = content
                        .files
                        .into_iter()
                        .map(|f| (f.path, f.content))
                        .collect();
                    skill_cap.register_instructions(
                        &parsed.name,
                        SkillInstructions {
                            instructions: parsed.instructions,
                            files,
                        },
                    );
                }
            }

            if let Some(prompt) = skill_cap.system_prompt_addition() {
                system_prompt_parts.push(format!(
                    "<capability id=\"skills\">\n{}\n</capability>",
                    prompt
                ));
            }
            tool_definitions.extend(skill_cap.tool_definitions());
        }

        // Build final system prompt (XML-wrapped when capabilities contribute prompts)
        let final_system_prompt = if system_prompt_parts.is_empty() {
            base_system_prompt.to_string()
        } else {
            format!(
                "{}\n\n<system-prompt>\n{}\n</system-prompt>",
                system_prompt_parts.join("\n\n"),
                base_system_prompt
            )
        };

        Ok((final_system_prompt, tool_definitions))
    }
}

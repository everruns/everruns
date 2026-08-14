// Capability service - business logic for capabilities
//
// Uses CapabilityRegistry from everruns-core as the single source of truth
// for capability definitions.
//
// MCP servers are integrated as "virtual capabilities" that appear alongside
// built-in capabilities. Their tools are prefixed with "mcp_{server_name}_".
//
// Skills from the database registry are integrated as AttachSkillCapability
// instances that mount skill files into the VFS. The built-in SkillsCapability
// discovers them at runtime.
//
// Note: Agent-specific capability management is handled by domains::agents commands.
//
// EVE-316: CapabilityService owns a Moka cache for skill listings (previously
// held by SkillService). Skills change infrequently but are fetched on every
// agent run (capability listing). Cache uses 5-min TTL, invalidated by
// `invalidate_skills_cache` which commands call after mutations.

use crate::domains::mcp_servers::McpServerService;
use crate::domains::skills::queries as skill_q;
use crate::storage::{EncryptionService, StorageBackend};
use anyhow::Result;
use everruns_capability::{CapabilityId, is_plugin_capability, parse_plugin_capability_id};
use everruns_core::capabilities::{Capability, CapabilityRegistry, SkillCapabilityIdExt};
use everruns_core::{
    Caller, CapabilityInfo, CapabilityStatus, DeclarativeCapabilityDefinition, RiskLevel, Skill,
    declarative_capability_info, is_declarative_capability, parse_declarative_capability_id,
    plugin_capability_info, skill_capability_id,
};
use everruns_mcp::{McpCapability, McpCapabilityIdExt, mcp_capability_id};
use moka::future::Cache;
use std::sync::Arc;
use std::time::Duration;

/// Cache TTL for skill listings per org
const SKILL_CACHE_TTL: Duration = Duration::from_secs(300); // 5 minutes

pub struct CapabilityService {
    registry: CapabilityRegistry,
    mcp_service: McpServerService,
    db: Arc<StorageBackend>,
    /// Cache: org_id -> Vec<Skill>
    skill_list_cache: Cache<i64, Arc<Vec<Skill>>>,
}

impl CapabilityService {
    pub fn new(db: Arc<StorageBackend>, encryption: Option<Arc<EncryptionService>>) -> Self {
        Self::with_registry(
            db,
            encryption,
            everruns_platform::capabilities::hosted_capability_registry(),
        )
    }

    pub fn with_registry(
        db: Arc<StorageBackend>,
        encryption: Option<Arc<EncryptionService>>,
        registry: CapabilityRegistry,
    ) -> Self {
        let skill_list_cache = Cache::builder()
            .time_to_live(SKILL_CACHE_TTL)
            .max_capacity(1_000)
            .build();
        Self {
            registry,
            mcp_service: McpServerService::new(db.clone(), encryption),
            db,
            skill_list_cache,
        }
    }

    /// Replace the embedded `McpServerService` so outbound MCP traffic flows
    /// through the platform's shared egress boundary (spec: `knowledge/operations/egress.md`).
    pub fn with_mcp_egress_service(
        mut self,
        egress_service: Arc<dyn everruns_core::EgressService>,
    ) -> Self {
        let db = self.db.clone();
        let encryption = self.mcp_service.encryption();
        self.mcp_service = McpServerService::with_egress_service(db, encryption, egress_service);
        self
    }

    pub fn registry(&self) -> &CapabilityRegistry {
        &self.registry
    }

    /// Egress service used by the embedded `McpServerService` for outbound
    /// HTTP to remote MCP servers. Surfaced so command handlers that need to
    /// discover scoped MCP tools can share the platform's egress boundary.
    pub fn egress_service(&self) -> Arc<dyn everruns_core::EgressService> {
        self.mcp_service.egress_service()
    }

    /// Invalidate the cached skill list for an org. Called by skill mutation
    /// commands after create/update/delete.
    pub async fn invalidate_skills_cache(&self, org_id: i64) {
        self.skill_list_cache.invalidate(&org_id).await;
    }

    /// Fetch non-archived skills for an org, using the cache where possible.
    ///
    /// Returns both `Active` and `Disabled` skills (everything not archived).
    /// Callers are expected to filter by `SkillStatus::Active` if they only
    /// want active entries — see `list_all` and `list_skill_commands_for_org`.
    async fn cached_skills(&self, org_id: i64) -> Result<Arc<Vec<Skill>>> {
        if let Some(cached) = self.skill_list_cache.get(&org_id).await {
            return Ok(cached);
        }
        let skills = skill_q::list_skills(&self.db, org_id, None, false).await?;
        let arc = Arc::new(skills);
        self.skill_list_cache.insert(org_id, arc.clone()).await;
        Ok(arc)
    }

    /// List all available capabilities including MCP servers and skills (public info only)
    pub async fn list_all(&self, org_id: i64) -> Result<Vec<CapabilityInfo>> {
        let internal_caller = Caller::internal(org_id);
        // Get built-in capabilities
        let mut capabilities: Vec<CapabilityInfo> = self
            .registry
            .list()
            .into_iter()
            .map(|cap| CapabilityInfo::from_core(cap.as_ref()))
            .collect();

        // Get MCP server capabilities
        let mcp_servers = self
            .mcp_service
            .list_active_with_tools(&internal_caller)
            .await?;
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
                is_guardrail: false,
                dependencies: vec![], // MCP capabilities have no dependencies
                features: vec![],
                config_schema: None,
                config_ui_schema: None,
                risk_level: RiskLevel::Low,
                agent_count: 0,
                harness_count: 0,
                docs_slug: None,
                localizations: Default::default(),
            });
        }

        // Get skill capabilities from the registry (mount-only, no prompt/tools)
        let skills = self.cached_skills(org_id).await?;
        for skill in skills.iter() {
            if skill.status != everruns_core::SkillStatus::Active {
                continue;
            }

            capabilities.push(CapabilityInfo {
                id: CapabilityId::new(skill_capability_id(skill.id.uuid())),
                name: skill.name.clone(),
                description: skill.description.clone(),
                status: CapabilityStatus::Available,
                icon: Some("wand".to_string()),
                category: Some("Skills".to_string()),
                system_prompt: None,      // SkillsCapability handles prompt
                tool_definitions: vec![], // SkillsCapability handles tools
                is_mcp: false,
                is_skill: true,
                is_guardrail: false,
                dependencies: vec!["session_file_system".to_string()],
                features: vec![],
                config_schema: None,
                config_ui_schema: None,
                risk_level: RiskLevel::Low,
                agent_count: 0,
                harness_count: 0,
                docs_slug: None,
                localizations: Default::default(),
            });
        }

        let declarative_rows = self
            .db
            .list_declarative_capabilities(org_id, None, false)
            .await?;
        for row in declarative_rows {
            if row.status != "active" {
                continue;
            }
            let mut definition: DeclarativeCapabilityDefinition =
                serde_json::from_value(row.definition.clone()).unwrap_or_default();
            definition.name = row.name.clone();
            definition.display_name = row.display_name.clone();
            definition.description = row.description.clone();
            definition.status = CapabilityStatus::Available;
            let mut info = declarative_capability_info(&row.name, definition);
            info.agent_count = 0;
            info.harness_count = 0;
            capabilities.push(info);
        }

        // Add active installed plugin capabilities.
        let plugin_rows = self.db.list_active_plugin_installs(org_id).await?;
        for row in plugin_rows {
            let mut definition: everruns_core::DeclarativeCapabilityDefinition =
                serde_json::from_value(row.definition.clone()).unwrap_or_default();
            // Override display_name from manifest when the definition doesn't declare one.
            if definition.display_name.is_none() {
                definition.display_name = row
                    .manifest
                    .get("displayName")
                    .or_else(|| row.manifest.get("display_name"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
            }
            // Override description from manifest when definition has an empty value.
            if definition.description.is_empty()
                && let Some(desc) = row.manifest.get("description").and_then(|v| v.as_str())
            {
                definition.description = desc.to_string();
            }
            definition.status = CapabilityStatus::Available;
            let mut info = plugin_capability_info(&row.public_id, definition);
            info.agent_count = 0;
            info.harness_count = 0;
            capabilities.push(info);
        }

        // Populate agent/harness reference counts in one pair of grouped queries.
        let (agent_counts, harness_counts) = tokio::try_join!(
            self.db.count_agent_capability_references(org_id),
            self.db.count_harness_capability_references(org_id),
        )?;
        for cap in capabilities.iter_mut() {
            let id = cap.id.as_str();
            cap.agent_count = agent_counts.get(id).copied().unwrap_or(0);
            cap.harness_count = harness_counts.get(id).copied().unwrap_or(0);
        }

        Ok(capabilities)
    }

    /// Get a specific capability by ID
    ///
    /// For MCP capabilities, uses cached tools only (no external refresh).
    /// This ensures viewing a capability doesn't fail if the MCP server is unreachable.
    /// Use the refresh tools endpoint to explicitly update cached tools.
    pub async fn get(&self, org_id: i64, id: &CapabilityId) -> Result<Option<CapabilityInfo>> {
        let info = self.get_inner(org_id, id).await?;
        if let Some(mut info) = info {
            let key = info.id.as_str();
            let (agent_count, harness_count) = tokio::try_join!(
                self.db.count_agents_for_capability(org_id, key),
                self.db.count_harnesses_for_capability(org_id, key),
            )?;
            info.agent_count = agent_count;
            info.harness_count = harness_count;
            return Ok(Some(info));
        }
        Ok(None)
    }

    async fn get_inner(&self, org_id: i64, id: &CapabilityId) -> Result<Option<CapabilityInfo>> {
        // Check if it's an MCP capability
        if let Some(server_id) = id.mcp_server_id() {
            let internal_caller = Caller::internal(org_id);
            // Use cached tools only - no external refresh on read
            let tools = self
                .mcp_service
                .get_cached_tools(&internal_caller, server_id)
                .await?;
            let server = self.mcp_service.get(&internal_caller, server_id).await?;

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
                    is_guardrail: false,
                    dependencies: vec![], // MCP capabilities have no dependencies
                    features: vec![],
                    config_schema: None,
                    config_ui_schema: None,
                    risk_level: RiskLevel::Low,
                    agent_count: 0,
                    harness_count: 0,
                    docs_slug: None,
                    localizations: Default::default(),
                }));
            }
            return Ok(None);
        }

        // Check if it's a skill capability (mount-only, no prompt/tools)
        if let Some(skill_uuid) = id.skill_id() {
            let skill = skill_q::get_skill(&self.db, org_id, skill_uuid).await?;
            if let Some(skill) = skill {
                return Ok(Some(CapabilityInfo {
                    id: CapabilityId::new(skill_capability_id(skill.id.uuid())),
                    name: skill.name.clone(),
                    description: skill.description.clone(),
                    status: CapabilityStatus::Available,
                    icon: Some("wand".to_string()),
                    category: Some("Skills".to_string()),
                    system_prompt: None,      // SkillsCapability handles prompt
                    tool_definitions: vec![], // SkillsCapability handles tools
                    is_mcp: false,
                    is_skill: true,
                    is_guardrail: false,
                    dependencies: vec!["session_file_system".to_string()],
                    features: vec![],
                    config_schema: None,
                    config_ui_schema: None,
                    risk_level: RiskLevel::Low,
                    agent_count: 0,
                    harness_count: 0,
                    docs_slug: None,
                    localizations: Default::default(),
                }));
            }
            return Ok(None);
        }

        if is_declarative_capability(id.as_str()) {
            let Some(name) = parse_declarative_capability_id(id.as_str()) else {
                return Ok(None);
            };
            if let Some(row) = self
                .db
                .get_declarative_capability_by_name(org_id, name)
                .await?
            {
                if row.status == "deleted" {
                    return Ok(None);
                }
                let mut definition: DeclarativeCapabilityDefinition =
                    serde_json::from_value(row.definition.clone()).unwrap_or_default();
                definition.name = row.name;
                definition.display_name = row.display_name;
                definition.description = row.description;
                definition.status = if row.status == "active" {
                    CapabilityStatus::Available
                } else {
                    CapabilityStatus::Deprecated
                };
                return Ok(Some(declarative_capability_info(name, definition)));
            }
            return Ok(None);
        }

        // Plugin capability — look up by stable installation public ID.
        if is_plugin_capability(id.as_str()) {
            let Some(plugin_public_id) = parse_plugin_capability_id(id.as_str()) else {
                return Ok(None);
            };
            if let Some(row) = self
                .db
                .get_plugin_install_by_public_id(org_id, plugin_public_id)
                .await?
            {
                if row.status != "active" {
                    return Ok(None);
                }
                let mut definition: DeclarativeCapabilityDefinition =
                    serde_json::from_value(row.definition.clone()).unwrap_or_default();
                if definition.display_name.is_none() {
                    definition.display_name = row
                        .manifest
                        .get("displayName")
                        .or_else(|| row.manifest.get("display_name"))
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());
                }
                if definition.description.is_empty()
                    && let Some(desc) = row.manifest.get("description").and_then(|v| v.as_str())
                {
                    definition.description = desc.to_string();
                }
                definition.status = CapabilityStatus::Available;
                return Ok(Some(plugin_capability_info(plugin_public_id, definition)));
            }
            return Ok(None);
        }

        // Regular capability
        Ok(self
            .registry
            .get(id.as_str())
            .map(|cap| CapabilityInfo::from_core(cap.as_ref())))
    }

    /// Return the IDs of any high-risk capabilities in the given list.
    ///
    /// Looks up each capability in the built-in registry.  MCP and skill
    /// capabilities are always Low risk (user-configured), so only built-in
    /// capabilities can be High.
    pub fn high_risk_ids(&self, cap_refs: &[&str]) -> Vec<String> {
        cap_refs
            .iter()
            .filter(|id| {
                self.registry
                    .get(id)
                    .is_some_and(|c| c.risk_level() == RiskLevel::High)
            })
            .map(|id| id.to_string())
            .collect()
    }

    /// Return high-risk built-in capability IDs reachable from the given refs.
    ///
    /// Declarative refs are expanded using their persisted `dependencies`.
    pub async fn high_risk_ids_for_org(
        &self,
        org_id: i64,
        cap_refs: &[&str],
    ) -> Result<Vec<String>> {
        let mut refs: Vec<String> = cap_refs.iter().map(|id| (*id).to_string()).collect();
        for cap_ref in cap_refs {
            if !is_declarative_capability(cap_ref) {
                continue;
            }
            let Some(name) = parse_declarative_capability_id(cap_ref) else {
                continue;
            };
            let Some(row) = self
                .db
                .get_declarative_capability_by_name(org_id, name)
                .await?
            else {
                continue;
            };
            // Fail closed: a broken declarative definition must NOT silently skip
            // dependency expansion, since this function gates admin-only high-risk
            // capability assignment. Propagate the parse error to the caller.
            let definition: DeclarativeCapabilityDefinition =
                serde_json::from_value(row.definition).map_err(|error| {
                    anyhow::anyhow!(
                        "declarative capability {cap_ref} has malformed definition: {error}"
                    )
                })?;
            refs.extend(definition.dependencies);
        }

        refs.sort();
        refs.dedup();
        Ok(self.high_risk_ids(&refs.iter().map(String::as_str).collect::<Vec<_>>()))
    }

    /// List system commands from all registered capabilities.
    ///
    /// System commands execute directly without the LLM (e.g., /clear, /status).
    pub fn list_system_commands(&self) -> Vec<everruns_core::command::CommandDescriptor> {
        self.registry
            .list()
            .iter()
            .flat_map(|cap| cap.commands())
            .collect()
    }

    /// List invocable skill commands from the org's skills registry.
    ///
    /// Returns skills where user_invocable is true, formatted as commands.
    pub async fn list_skill_commands(
        &self,
        org_id: i64,
    ) -> Result<Vec<everruns_core::command::CommandDescriptor>> {
        let skills = self.cached_skills(org_id).await?;
        let commands = skills
            .iter()
            .filter(|s| s.status == everruns_core::SkillStatus::Active && s.user_invocable)
            .cloned()
            .map(|s| everruns_core::command::CommandDescriptor {
                name: s.name,
                description: s.description,
                source: everruns_core::command::CommandSource::Skill,
                args: vec![],
            })
            .collect();
        Ok(commands)
    }

    /// Preview the final agent shape by computing the merged system prompt and tools.
    ///
    /// This collects contributions from all specified capabilities and their dependencies:
    /// - System prompt additions are appended after the base prompt
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
        capability_configs: &[everruns_capability::CapabilityRef],
    ) -> Result<(String, Vec<everruns_provider::tool_types::ToolDefinition>)> {
        use everruns_core::capabilities::{
            SystemPromptContext, collect_capabilities_with_configs, resolve_capability_configs,
        };

        let mut system_prompt_parts: Vec<String> = Vec::new();
        let mut tool_definitions: Vec<everruns_provider::tool_types::ToolDefinition> = Vec::new();

        // Plugin and declarative definitions are deliberately not persisted in
        // agent config. Resolve their current, exact definitions before the
        // shared collector evaluates dependencies and contributions.
        let capability_configs =
            crate::domains::capabilities::queries::hydrate_declarative_capability_configs(
                self.db.as_ref(),
                org_id,
                capability_configs.to_vec(),
            )
            .await?;

        // Separate built-in capabilities from MCP capabilities
        // Skill capabilities (skill:{uuid}) are mount-only; skip in preview.
        let mut mcp_cap_ids: Vec<uuid::Uuid> = Vec::new();
        let mut builtin_cap_configs: Vec<everruns_capability::CapabilityRef> = Vec::new();

        for cap_config in &capability_configs {
            let cap_ref = &cap_config.typed_id();
            if let Some(server_id) = cap_ref.mcp_server_id() {
                mcp_cap_ids.push(server_id);
            } else if cap_ref.skill_id().is_some() {
                // AttachSkillCapability: mount-only, no preview contribution
            } else {
                builtin_cap_configs.push(cap_config.clone());
            }
        }

        // Resolve dependencies for built-in capabilities
        let resolved = resolve_capability_configs(&builtin_cap_configs, &self.registry)
            .map_err(|e| anyhow::anyhow!("Failed to resolve capability dependencies: {}", e))?;

        // Collect from resolved capabilities (includes dependencies in correct order)
        // Preview has no session context, so dynamic capabilities (agent_instructions) return None
        let ctx =
            SystemPromptContext::without_file_store(everruns_provider::typed_id::SessionId::new());
        let collected = collect_capabilities_with_configs(&resolved, &self.registry, &ctx).await;
        if let Some(prefix) = collected.system_prompt_prefix() {
            system_prompt_parts.push(prefix);
        }
        tool_definitions.extend(collected.tool_definitions);
        tool_definitions.extend(
            crate::domains::mcp_servers::scoped_mcp::build_scoped_mcp_tool_definitions(
                &collected.mcp_servers,
                None,
                None,
                self.mcp_service.egress_service().as_ref(),
            )
            .await?,
        );

        // Collect from MCP capabilities using cached tools only (no refresh)
        // Note: MCP capabilities don't have dependencies
        let internal_caller = Caller::internal(org_id);
        for server_id in mcp_cap_ids {
            let tools = self
                .mcp_service
                .get_cached_tools(&internal_caller, server_id)
                .await?;
            let server = self.mcp_service.get(&internal_caller, server_id).await?;

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

        // Skill capabilities are mount-only (AttachSkillCapability).
        // At runtime, SkillsCapability discovers mounted skills from VFS.
        // In preview mode (no VFS), skills appear via the static SkillsCapability
        // prompt which is already collected above as a built-in capability.
        // No additional prompt or tool injection needed here.

        // Build final system prompt (XML-wrapped when capabilities contribute prompts)
        let additions = (!system_prompt_parts.is_empty()).then(|| system_prompt_parts.join("\n\n"));
        let final_system_prompt = everruns_core::capabilities::compose_system_prompt(
            base_system_prompt,
            additions.as_deref(),
        );

        Ok((final_system_prompt, tool_definitions))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::memory::InMemoryDatabase;
    use everruns_provider::typed_id::SkillId;

    fn make_service() -> CapabilityService {
        let db = Arc::new(StorageBackend::InMemory(Arc::new(InMemoryDatabase::new())));
        CapabilityService::with_registry(
            db,
            None,
            everruns_platform::capabilities::hosted_capability_registry(),
        )
    }

    async fn insert_skill(svc: &CapabilityService, org_id: i64, name: &str) {
        let input = crate::storage::models::CreateSkillRow {
            public_id: SkillId::new().to_string(),
            name: name.to_string(),
            description: format!("{name} description"),
            license: None,
            compatibility: None,
            metadata: serde_json::json!({}),
            allowed_tools: None,
            instructions: format!("Instructions for {name}"),
            source_type: "markdown".to_string(),
            archive_data: None,
            version: "1.0.0".to_string(),
        };
        svc.db.create_skill(org_id, input).await.unwrap();
    }

    async fn is_cached(svc: &CapabilityService, org_id: i64) -> bool {
        svc.skill_list_cache.get(&org_id).await.is_some()
    }

    #[tokio::test]
    async fn test_cache_miss_populates_cache() {
        let svc = make_service();
        insert_skill(&svc, 1, "cache-test").await;

        assert!(!is_cached(&svc, 1).await);
        let skills = svc.cached_skills(1).await.unwrap();
        assert_eq!(skills.len(), 1);
        assert!(is_cached(&svc, 1).await);
    }

    #[tokio::test]
    async fn test_cache_hit_returns_cached_skills() {
        let svc = make_service();
        insert_skill(&svc, 1, "cached-skill").await;

        let first = svc.cached_skills(1).await.unwrap();
        assert_eq!(first.len(), 1);

        // Insert a second skill directly; cache should hide it until invalidated.
        insert_skill(&svc, 1, "sneaky-skill").await;
        let second = svc.cached_skills(1).await.unwrap();
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].name, "cached-skill");
    }

    #[tokio::test]
    async fn test_invalidate_skills_cache_clears_entry() {
        let svc = make_service();
        insert_skill(&svc, 1, "one").await;
        svc.cached_skills(1).await.unwrap();
        assert!(is_cached(&svc, 1).await);

        svc.invalidate_skills_cache(1).await;
        assert!(!is_cached(&svc, 1).await);

        // After invalidation, a newly inserted skill is visible.
        insert_skill(&svc, 1, "two").await;
        let skills = svc.cached_skills(1).await.unwrap();
        assert_eq!(skills.len(), 2);
    }

    #[tokio::test]
    async fn test_cache_independent_per_org() {
        let svc = make_service();
        insert_skill(&svc, 1, "org-a").await;
        insert_skill(&svc, 2, "org-b").await;

        let a = svc.cached_skills(1).await.unwrap();
        let b = svc.cached_skills(2).await.unwrap();
        assert_eq!(a.len(), 1);
        assert_eq!(b.len(), 1);
        assert_eq!(a[0].name, "org-a");
        assert_eq!(b[0].name, "org-b");

        // Invalidating org 1 does not affect org 2.
        svc.invalidate_skills_cache(1).await;
        assert!(!is_cached(&svc, 1).await);
        assert!(is_cached(&svc, 2).await);
    }

    // Regression tests for fix(capabilities): restore high-risk levels for
    // bash/fetch (#1500). `high_risk_ids` is the admin-gating primitive used
    // by `api::agents::require_admin_for_high_risk`, the agent domain command
    // `check_high_risk_caps`, and the session service's high-risk guard. If a
    // refactor again downgrades `bashkit_shell` or `web_fetch` the admin gate
    // would silently stop firing (TM-AGENT-005).
    mod high_risk_ids_tests {
        use super::*;

        #[test]
        fn flags_bashkit_shell_as_high_risk() {
            let svc = make_service();
            let high = svc.high_risk_ids(&["bashkit_shell"]);
            assert_eq!(high, vec!["bashkit_shell".to_string()]);
        }

        #[test]
        fn flags_legacy_virtual_bash_alias_as_high_risk() {
            // Persisted agent configs may still carry the pre-rename
            // `virtual_bash` ID; the alias must stay admin-gated.
            let svc = make_service();
            let high = svc.high_risk_ids(&["virtual_bash"]);
            assert_eq!(high, vec!["virtual_bash".to_string()]);
        }

        #[test]
        fn flags_web_fetch_as_high_risk() {
            let svc = make_service();
            let high = svc.high_risk_ids(&["web_fetch"]);
            assert_eq!(high, vec!["web_fetch".to_string()]);
        }

        #[test]
        fn flags_both_bash_and_fetch_in_mixed_input() {
            let svc = make_service();
            let high = svc.high_risk_ids(&["current_time", "bashkit_shell", "web_fetch"]);
            assert!(high.contains(&"bashkit_shell".to_string()));
            assert!(high.contains(&"web_fetch".to_string()));
            assert!(!high.contains(&"current_time".to_string()));
            assert_eq!(high.len(), 2);
        }

        #[test]
        fn does_not_flag_low_risk_capability() {
            let svc = make_service();
            let high = svc.high_risk_ids(&["current_time"]);
            assert!(
                high.is_empty(),
                "default/low-risk capabilities must not be flagged as high-risk"
            );
        }

        #[test]
        fn does_not_flag_unknown_capability_refs() {
            let svc = make_service();
            // MCP and skill capability refs are resolved externally and are always
            // Low risk; unknown refs must not be pulled into the high-risk set.
            let high = svc.high_risk_ids(&["mcp:example/tool", "skill:local"]);
            assert!(
                high.is_empty(),
                "non-builtin refs must not appear as high-risk"
            );
        }

        #[test]
        fn empty_input_returns_empty() {
            let svc = make_service();
            assert!(svc.high_risk_ids(&[]).is_empty());
        }

        #[tokio::test]
        async fn flags_declarative_dependencies_as_high_risk() {
            use crate::storage::models::CreateDeclarativeCapabilityRow;
            let svc = make_service();
            svc.db
                .create_declarative_capability(
                    1,
                    CreateDeclarativeCapabilityRow {
                        public_id: everruns_provider::typed_id::DeclarativeCapabilityId::new()
                            .to_string(),
                        name: "hidden_fetch".to_string(),
                        display_name: Some("Hidden Fetch".to_string()),
                        description: "test".to_string(),
                        definition: serde_json::json!({
                            "name": "hidden_fetch",
                            "description": "test",
                            "dependencies": ["web_fetch"],
                        }),
                    },
                )
                .await
                .unwrap();

            let high = svc
                .high_risk_ids_for_org(1, &["declarative:hidden_fetch"])
                .await
                .unwrap();
            assert_eq!(high, vec!["web_fetch".to_string()]);
        }
    }
}

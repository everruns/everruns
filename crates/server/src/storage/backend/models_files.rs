//! Models, agent capabilities, session files, git refs, MCP servers, and plugin installs.

use super::*;

impl StorageBackend {
    /// Create a model with a specific ID (for seeding)
    /// Returns None if model already exists (idempotent)
    pub async fn create_model_with_id(
        &self,
        org_id: i64,
        id: Uuid,
        input: CreateModelRow,
    ) -> Result<Option<ModelRow>> {
        dispatch!(self, create_model_with_id, org_id, id, input)
    }

    pub async fn get_model(&self, org_id: i64, id: Uuid) -> Result<Option<ModelRow>> {
        dispatch!(self, get_model, org_id, id)
    }

    pub async fn get_model_for_mutation(&self, org_id: i64, id: Uuid) -> Result<Option<ModelRow>> {
        dispatch!(self, get_model_for_mutation, org_id, id)
    }

    pub async fn get_model_with_provider(
        &self,
        org_id: i64,
        id: Uuid,
    ) -> Result<Option<ModelWithProviderRow>> {
        dispatch!(self, get_model_with_provider, org_id, id)
    }

    pub async fn list_models_for_provider(
        &self,
        org_id: i64,
        provider_id: Uuid,
    ) -> Result<Vec<ModelRow>> {
        dispatch!(self, list_models_for_provider, org_id, provider_id)
    }

    pub async fn list_all_models(&self, org_id: i64) -> Result<Vec<ModelWithProviderRow>> {
        dispatch!(self, list_all_models, org_id)
    }

    pub async fn update_model(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdateModel,
    ) -> Result<Option<ModelRow>> {
        dispatch!(self, update_model, org_id, id, input)
    }

    pub async fn delete_model(&self, org_id: i64, id: Uuid) -> Result<bool> {
        dispatch!(self, delete_model, org_id, id)
    }

    pub async fn get_model_by_model_id(
        &self,
        org_id: i64,
        model_id: &str,
    ) -> Result<Option<ModelWithProviderRow>> {
        dispatch!(self, get_model_by_model_id, org_id, model_id)
    }

    // ============================================
    // Agent Capabilities
    // ============================================

    pub async fn get_agent_capabilities(&self, agent_id: Uuid) -> Result<Vec<AgentCapabilityRow>> {
        #[cfg(test)]
        self.record_session_list_lookup().await;
        dispatch!(self, get_agent_capabilities, agent_id)
    }

    pub async fn get_agent_capabilities_by_agent_ids(
        &self,
        org_id: i64,
        agent_ids: &[AgentId],
    ) -> Result<Vec<AgentCapabilityRow>> {
        #[cfg(test)]
        self.record_session_list_lookup().await;
        dispatch!(self, get_agent_capabilities_by_agent_ids, org_id, agent_ids)
    }

    pub async fn set_agent_capabilities(
        &self,
        agent_id: Uuid,
        capabilities: Vec<(String, i32, serde_json::Value)>,
    ) -> Result<Vec<AgentCapabilityRow>> {
        dispatch!(self, set_agent_capabilities, agent_id, capabilities)
    }

    pub async fn add_agent_capability(
        &self,
        input: CreateAgentCapabilityRow,
    ) -> Result<AgentCapabilityRow> {
        dispatch!(self, add_agent_capability, input)
    }

    pub async fn remove_agent_capability(
        &self,
        agent_id: Uuid,
        capability_id: &str,
    ) -> Result<bool> {
        dispatch!(self, remove_agent_capability, agent_id, capability_id)
    }

    // ============================================
    // Harness Capabilities
    // ============================================

    pub async fn get_harness_capabilities(
        &self,
        harness_id: Uuid,
    ) -> Result<Vec<HarnessCapabilityRow>> {
        #[cfg(test)]
        self.record_session_list_lookup().await;
        dispatch!(self, get_harness_capabilities, harness_id)
    }

    pub async fn get_harness_capabilities_by_harness_ids(
        &self,
        org_id: i64,
        harness_ids: &[HarnessId],
    ) -> Result<Vec<HarnessCapabilityRow>> {
        #[cfg(test)]
        self.record_session_list_lookup().await;
        dispatch!(
            self,
            get_harness_capabilities_by_harness_ids,
            org_id,
            harness_ids
        )
    }

    pub async fn set_harness_capabilities(
        &self,
        harness_id: Uuid,
        capabilities: Vec<(String, i32, serde_json::Value)>,
    ) -> Result<Vec<HarnessCapabilityRow>> {
        dispatch!(self, set_harness_capabilities, harness_id, capabilities)
    }

    /// Count active agents per capability_id within an org.
    pub async fn count_agent_capability_references(
        &self,
        org_id: i64,
    ) -> Result<std::collections::HashMap<String, u64>> {
        dispatch!(self, count_agent_capability_references, org_id)
    }

    /// Count active harnesses per capability_id within an org.
    pub async fn count_harness_capability_references(
        &self,
        org_id: i64,
    ) -> Result<std::collections::HashMap<String, u64>> {
        dispatch!(self, count_harness_capability_references, org_id)
    }

    /// Count active agents referencing a single capability_id within an org.
    pub async fn count_agents_for_capability(
        &self,
        org_id: i64,
        capability_id: &str,
    ) -> Result<u64> {
        dispatch!(self, count_agents_for_capability, org_id, capability_id)
    }

    /// Count active harnesses referencing a single capability_id within an org.
    pub async fn count_harnesses_for_capability(
        &self,
        org_id: i64,
        capability_id: &str,
    ) -> Result<u64> {
        dispatch!(self, count_harnesses_for_capability, org_id, capability_id)
    }

    // ============================================
    // Session Files
    // ============================================

    pub async fn create_session_file(&self, input: CreateSessionFileRow) -> Result<SessionFileRow> {
        dispatch!(self, create_session_file, input)
    }

    pub async fn get_session_file(
        &self,
        session_id: Uuid,
        path: &str,
    ) -> Result<Option<SessionFileRow>> {
        dispatch!(self, get_session_file, session_id, path)
    }

    pub async fn get_session_file_info(
        &self,
        session_id: Uuid,
        path: &str,
    ) -> Result<Option<SessionFileInfoRow>> {
        dispatch!(self, get_session_file_info, session_id, path)
    }

    pub async fn list_session_files(
        &self,
        session_id: Uuid,
        parent_path: &str,
    ) -> Result<Vec<SessionFileInfoRow>> {
        dispatch!(self, list_session_files, session_id, parent_path)
    }

    pub async fn list_all_session_files(
        &self,
        session_id: Uuid,
    ) -> Result<Vec<SessionFileInfoRow>> {
        dispatch!(self, list_all_session_files, session_id)
    }

    pub async fn update_session_file(
        &self,
        session_id: Uuid,
        path: &str,
        input: UpdateSessionFile,
    ) -> Result<Option<SessionFileRow>> {
        dispatch!(self, update_session_file, session_id, path, input)
    }

    pub async fn update_session_file_if_content_matches(
        &self,
        session_id: Uuid,
        path: &str,
        expected_content: Vec<u8>,
        input: UpdateSessionFile,
    ) -> Result<Option<SessionFileRow>> {
        dispatch!(
            self,
            update_session_file_if_content_matches,
            session_id,
            path,
            expected_content,
            input
        )
    }

    pub async fn delete_session_file(&self, session_id: Uuid, path: &str) -> Result<bool> {
        dispatch!(self, delete_session_file, session_id, path)
    }

    pub async fn delete_session_file_recursive(&self, session_id: Uuid, path: &str) -> Result<u64> {
        dispatch!(self, delete_session_file_recursive, session_id, path)
    }

    pub async fn move_session_file(
        &self,
        session_id: Uuid,
        source_path: &str,
        dest_path: &str,
    ) -> Result<Option<SessionFileRow>> {
        dispatch!(self, move_session_file, session_id, source_path, dest_path)
    }

    pub async fn copy_session_file(
        &self,
        session_id: Uuid,
        source_path: &str,
        dest_path: &str,
    ) -> Result<Option<SessionFileRow>> {
        dispatch!(self, copy_session_file, session_id, source_path, dest_path)
    }

    pub async fn grep_session_files(
        &self,
        session_id: Uuid,
        pattern: &str,
        path_prefix: Option<&str>,
        excluded_path_prefix: Option<&str>,
        max_file_bytes: i64,
    ) -> Result<Vec<SessionFileInfoRow>> {
        dispatch!(
            self,
            grep_session_files,
            session_id,
            pattern,
            path_prefix,
            excluded_path_prefix,
            max_file_bytes
        )
    }

    pub async fn session_file_exists(&self, session_id: Uuid, path: &str) -> Result<bool> {
        dispatch!(self, session_file_exists, session_id, path)
    }

    pub async fn has_readonly_session_files(&self, session_id: Uuid, path: &str) -> Result<bool> {
        dispatch!(self, has_readonly_session_files, session_id, path)
    }

    pub async fn session_directory_has_children(
        &self,
        session_id: Uuid,
        path: &str,
    ) -> Result<bool> {
        dispatch!(self, session_directory_has_children, session_id, path)
    }

    /// Sum of size_bytes for all non-directory files in a session.
    pub async fn total_session_file_bytes(&self, session_id: Uuid) -> Result<i64> {
        dispatch!(self, total_session_file_bytes, session_id)
    }

    /// Load all non-directory files with content for a session (single query, for git commit).
    pub async fn load_all_session_files_with_content(
        &self,
        session_id: Uuid,
    ) -> Result<Vec<SessionFileRow>> {
        dispatch!(self, load_all_session_files_with_content, session_id)
    }

    // ============================================
    // Session Git Objects
    // ============================================

    pub async fn write_git_object(&self, input: CreateSessionGitObject) -> Result<()> {
        dispatch!(self, write_git_object, input)
    }

    pub async fn write_git_objects_batch(
        &self,
        objects: Vec<CreateSessionGitObject>,
    ) -> Result<()> {
        dispatch!(self, write_git_objects_batch, objects)
    }

    pub async fn read_git_object(
        &self,
        session_id: Uuid,
        oid: &[u8],
    ) -> Result<Option<SessionGitObjectRow>> {
        dispatch!(self, read_git_object, session_id, oid)
    }

    /// Load all git objects for a session in a single query (avoids N+1).
    pub async fn load_all_git_objects(&self, session_id: Uuid) -> Result<Vec<SessionGitObjectRow>> {
        dispatch!(self, load_all_git_objects, session_id)
    }

    pub async fn git_object_exists(&self, session_id: Uuid, oid: &[u8]) -> Result<bool> {
        dispatch!(self, git_object_exists, session_id, oid)
    }

    pub async fn read_git_object_header(
        &self,
        session_id: Uuid,
        oid: &[u8],
    ) -> Result<Option<(i16, i64)>> {
        dispatch!(self, read_git_object_header, session_id, oid)
    }

    pub async fn list_git_object_oids(&self, session_id: Uuid) -> Result<Vec<Vec<u8>>> {
        dispatch!(self, list_git_object_oids, session_id)
    }

    pub async fn fork_git_objects(
        &self,
        source_session_id: Uuid,
        target_session_id: Uuid,
    ) -> Result<u64> {
        dispatch!(self, fork_git_objects, source_session_id, target_session_id)
    }

    // ============================================
    // Session Git Refs
    // ============================================

    pub async fn write_git_ref(&self, input: CreateSessionGitRef) -> Result<()> {
        dispatch!(self, write_git_ref, input)
    }

    pub async fn read_git_ref(
        &self,
        session_id: Uuid,
        name: &str,
    ) -> Result<Option<SessionGitRefRow>> {
        dispatch!(self, read_git_ref, session_id, name)
    }

    pub async fn delete_git_ref(&self, session_id: Uuid, name: &str) -> Result<bool> {
        dispatch!(self, delete_git_ref, session_id, name)
    }

    pub async fn list_git_refs(&self, session_id: Uuid) -> Result<Vec<SessionGitRefRow>> {
        dispatch!(self, list_git_refs, session_id)
    }

    pub async fn fork_git_refs(
        &self,
        source_session_id: Uuid,
        target_session_id: Uuid,
    ) -> Result<u64> {
        dispatch!(self, fork_git_refs, source_session_id, target_session_id)
    }

    // ============================================
    // MCP Servers
    // ============================================

    pub async fn create_mcp_server(
        &self,
        org_id: i64,
        input: CreateMcpServerRow,
    ) -> Result<McpServerRow> {
        dispatch!(self, create_mcp_server, org_id, input)
    }

    pub async fn create_mcp_server_with_id(
        &self,
        org_id: i64,
        id: Uuid,
        input: CreateMcpServerRow,
    ) -> Result<Option<McpServerRow>> {
        dispatch!(self, create_mcp_server_with_id, org_id, id, input)
    }

    pub async fn get_mcp_server(&self, org_id: i64, id: Uuid) -> Result<Option<McpServerRow>> {
        dispatch!(self, get_mcp_server, org_id, id)
    }

    /// Batch fetch multiple MCP servers by IDs in a single query.
    pub async fn get_mcp_servers_batch(
        &self,
        org_id: i64,
        ids: &[Uuid],
    ) -> Result<Vec<McpServerRow>> {
        dispatch!(self, get_mcp_servers_batch, org_id, ids)
    }

    pub async fn get_mcp_server_by_name(
        &self,
        org_id: i64,
        name: &str,
    ) -> Result<Option<McpServerRow>> {
        dispatch!(self, get_mcp_server_by_name, org_id, name)
    }

    pub async fn list_mcp_servers(
        &self,
        org_id: i64,
        search: Option<&str>,
        include_archived: bool,
    ) -> Result<Vec<McpServerRow>> {
        dispatch!(self, list_mcp_servers, org_id, search, include_archived)
    }

    pub async fn list_active_mcp_servers(&self, org_id: i64) -> Result<Vec<McpServerRow>> {
        dispatch!(self, list_active_mcp_servers, org_id)
    }

    pub async fn update_mcp_server(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdateMcpServer,
    ) -> Result<Option<McpServerRow>> {
        dispatch!(self, update_mcp_server, org_id, id, input)
    }

    pub async fn update_mcp_server_tools(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdateMcpServerTools,
    ) -> Result<Option<McpServerRow>> {
        dispatch!(self, update_mcp_server_tools, org_id, id, input)
    }

    pub async fn get_mcp_service_tool_cache(
        &self,
        org_id: i64,
        mcp_server_id: Uuid,
        agent_id: Uuid,
        cache_scope: &str,
        credential_hash: &str,
    ) -> Result<Option<McpServiceToolCacheRow>> {
        dispatch!(
            self,
            get_mcp_service_tool_cache,
            org_id,
            mcp_server_id,
            agent_id,
            cache_scope,
            credential_hash
        )
    }

    pub async fn upsert_mcp_service_tool_cache(
        &self,
        input: UpsertMcpServiceToolCache,
    ) -> Result<McpServiceToolCacheRow> {
        dispatch!(self, upsert_mcp_service_tool_cache, input)
    }

    pub async fn delete_mcp_service_tool_caches(
        &self,
        org_id: i64,
        mcp_server_id: Uuid,
        agent_id: Uuid,
    ) -> Result<u64> {
        dispatch!(
            self,
            delete_mcp_service_tool_caches,
            org_id,
            mcp_server_id,
            agent_id
        )
    }

    pub async fn delete_mcp_server(&self, org_id: i64, id: Uuid) -> Result<bool> {
        dispatch!(self, delete_mcp_server, org_id, id)
    }

    pub async fn destroy_mcp_server(&self, org_id: i64, id: Uuid) -> Result<bool> {
        dispatch!(self, destroy_mcp_server, org_id, id)
    }

    // ============================================
    // Skills
    // ============================================

    pub async fn create_skill(&self, org_id: i64, input: CreateSkillRow) -> Result<SkillRow> {
        dispatch!(self, create_skill, org_id, input)
    }

    pub async fn get_skill(&self, org_id: i64, id: Uuid) -> Result<Option<SkillRow>> {
        dispatch!(self, get_skill, org_id, id)
    }

    pub async fn get_skill_by_name(&self, org_id: i64, name: &str) -> Result<Option<SkillRow>> {
        dispatch!(self, get_skill_by_name, org_id, name)
    }

    pub async fn list_skills(
        &self,
        org_id: i64,
        search: Option<&str>,
        include_archived: bool,
    ) -> Result<Vec<SkillRow>> {
        dispatch!(self, list_skills, org_id, search, include_archived)
    }

    pub async fn list_non_deleted_skill_ids(&self, org_id: i64) -> Result<Vec<Uuid>> {
        dispatch!(self, list_non_deleted_skill_ids, org_id)
    }

    pub async fn update_skill(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdateSkill,
    ) -> Result<Option<SkillRow>> {
        dispatch!(self, update_skill, org_id, id, input)
    }

    pub async fn delete_skill(&self, org_id: i64, id: Uuid) -> Result<bool> {
        dispatch!(self, delete_skill, org_id, id)
    }

    pub async fn destroy_skill(&self, org_id: i64, id: Uuid) -> Result<bool> {
        dispatch!(self, destroy_skill, org_id, id)
    }

    // ============================================
    // Declarative Capabilities
    // ============================================

    pub async fn create_declarative_capability(
        &self,
        org_id: i64,
        input: CreateDeclarativeCapabilityRow,
    ) -> Result<DeclarativeCapabilityRow> {
        dispatch!(self, create_declarative_capability, org_id, input)
    }

    pub async fn get_declarative_capability(
        &self,
        org_id: i64,
        id: Uuid,
    ) -> Result<Option<DeclarativeCapabilityRow>> {
        dispatch!(self, get_declarative_capability, org_id, id)
    }

    pub async fn get_declarative_capability_by_name(
        &self,
        org_id: i64,
        name: &str,
    ) -> Result<Option<DeclarativeCapabilityRow>> {
        dispatch!(self, get_declarative_capability_by_name, org_id, name)
    }

    pub async fn get_declarative_capability_by_public_id(
        &self,
        org_id: i64,
        public_id: &str,
    ) -> Result<Option<DeclarativeCapabilityRow>> {
        dispatch!(
            self,
            get_declarative_capability_by_public_id,
            org_id,
            public_id
        )
    }

    pub async fn list_declarative_capabilities(
        &self,
        org_id: i64,
        search: Option<&str>,
        include_archived: bool,
    ) -> Result<Vec<DeclarativeCapabilityRow>> {
        dispatch!(
            self,
            list_declarative_capabilities,
            org_id,
            search,
            include_archived
        )
    }

    pub async fn update_declarative_capability(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdateDeclarativeCapability,
    ) -> Result<Option<DeclarativeCapabilityRow>> {
        dispatch!(self, update_declarative_capability, org_id, id, input)
    }

    pub async fn delete_declarative_capability(&self, org_id: i64, id: Uuid) -> Result<bool> {
        dispatch!(self, delete_declarative_capability, org_id, id)
    }

    pub async fn destroy_declarative_capability(&self, org_id: i64, id: Uuid) -> Result<bool> {
        dispatch!(self, destroy_declarative_capability, org_id, id)
    }

    // ============================================
    // Plugin Marketplaces
    // ============================================

    pub async fn create_plugin_marketplace(
        &self,
        org_id: i64,
        input: CreatePluginMarketplaceRow,
    ) -> Result<PluginMarketplaceRow> {
        dispatch!(self, create_plugin_marketplace, org_id, input)
    }

    pub async fn get_plugin_marketplace(
        &self,
        org_id: i64,
        id: Uuid,
    ) -> Result<Option<PluginMarketplaceRow>> {
        dispatch!(self, get_plugin_marketplace, org_id, id)
    }

    pub async fn get_plugin_marketplace_by_public_id(
        &self,
        org_id: i64,
        public_id: &str,
    ) -> Result<Option<PluginMarketplaceRow>> {
        dispatch!(self, get_plugin_marketplace_by_public_id, org_id, public_id)
    }

    pub async fn list_plugin_marketplaces(
        &self,
        org_id: i64,
        search: Option<&str>,
    ) -> Result<Vec<PluginMarketplaceRow>> {
        dispatch!(self, list_plugin_marketplaces, org_id, search)
    }

    pub async fn update_plugin_marketplace(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdatePluginMarketplace,
    ) -> Result<Option<PluginMarketplaceRow>> {
        dispatch!(self, update_plugin_marketplace, org_id, id, input)
    }

    pub async fn delete_plugin_marketplace(&self, org_id: i64, id: Uuid) -> Result<bool> {
        dispatch!(self, delete_plugin_marketplace, org_id, id)
    }

    // ============================================
    // Plugin Installs
    // ============================================

    pub async fn create_plugin_install(
        &self,
        org_id: i64,
        input: CreatePluginInstallRow,
    ) -> Result<PluginInstallRow> {
        dispatch!(self, create_plugin_install, org_id, input)
    }

    pub async fn get_plugin_install(
        &self,
        org_id: i64,
        id: Uuid,
    ) -> Result<Option<PluginInstallRow>> {
        dispatch!(self, get_plugin_install, org_id, id)
    }

    pub async fn get_plugin_install_by_public_id(
        &self,
        org_id: i64,
        public_id: &str,
    ) -> Result<Option<PluginInstallRow>> {
        dispatch!(self, get_plugin_install_by_public_id, org_id, public_id)
    }

    pub async fn get_plugin_install_by_name(
        &self,
        org_id: i64,
        name: &str,
    ) -> Result<Option<PluginInstallRow>> {
        dispatch!(self, get_plugin_install_by_name, org_id, name)
    }

    pub async fn list_plugin_installs(
        &self,
        org_id: i64,
        search: Option<&str>,
    ) -> Result<Vec<PluginInstallRow>> {
        dispatch!(self, list_plugin_installs, org_id, search)
    }

    pub async fn list_active_plugin_installs(&self, org_id: i64) -> Result<Vec<PluginInstallRow>> {
        dispatch!(self, list_active_plugin_installs, org_id)
    }

    pub async fn update_plugin_install(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdatePluginInstall,
    ) -> Result<Option<PluginInstallRow>> {
        dispatch!(self, update_plugin_install, org_id, id, input)
    }
}

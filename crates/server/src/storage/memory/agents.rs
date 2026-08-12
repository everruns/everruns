// In-memory storage: Agents, Agent Capabilities

use super::super::models::*;
use super::InMemoryDatabase;
use super::matches_search_tokens;
use anyhow::Result;
use everruns_core::{AgentId, AgentIdentityId};
use std::collections::HashMap;
use uuid::Uuid;

impl InMemoryDatabase {
    // ============================================
    // Agents
    // ============================================

    pub async fn create_agent(&self, org_id: i64, input: CreateAgentRow) -> Result<AgentRow> {
        let now = Self::now();
        let id = AgentId::new();
        let row = AgentRow {
            id,
            public_id: input.public_id,
            org_id,
            name: input.name,
            display_name: input.display_name,
            description: input.description,
            system_prompt: input.system_prompt,
            default_model_id: input.default_model_id,
            harness_id: input.harness_id,
            harness_source: "explicit".to_string(),
            agent_identity_id: None,
            default_version_id: None,
            forked_from_agent_id: None,
            forked_from_version_id: None,
            root_agent_id: None,
            tags: input.tags,
            initial_files: input.initial_files,
            tools: input.tools,
            mcp_servers: input.mcp_servers,
            network_access: input.network_access,
            max_iterations: input.max_iterations,
            parallel_tool_calls: input.parallel_tool_calls,
            status: "active".to_string(),
            is_built_in: input.is_built_in,
            created_at: now,
            updated_at: now,
            archived_at: None,
            deleted_at: None,
            total_input_tokens: 0,
            total_output_tokens: 0,
            total_cache_read_tokens: 0,
            total_cache_creation_tokens: 0,
            total_actual_cost_usd: 0.0,
            total_estimated_cost_usd: 0.0,
            total_cost_usd: 0.0,
        };
        self.agents.write().insert(id, row.clone());
        Ok(row)
    }

    /// Create agent with a specific ID, idempotent (returns None if exists)
    /// Create or update agent with a specific ID (for seeding).
    /// Returns Some(row) if created or updated, None if unchanged.
    pub async fn create_agent_with_id(
        &self,
        org_id: i64,
        id: AgentId,
        input: CreateAgentRow,
    ) -> Result<Option<AgentRow>> {
        let mut agents = self.agents.write();
        let now = Self::now();

        if let Some(existing) = agents.get(&id) {
            // Check if seed-controlled fields differ
            if existing.name == input.name
                && existing.display_name == input.display_name
                && existing.description == input.description
                && existing.system_prompt == input.system_prompt
                && existing.harness_id == input.harness_id
                && existing.tags == input.tags
                && existing.initial_files == input.initial_files
                && existing.tools == input.tools
                && existing.mcp_servers == input.mcp_servers
            {
                return Ok(None); // Unchanged
            }
            // Update changed fields
            let row = AgentRow {
                name: input.name,
                display_name: input.display_name,
                description: input.description,
                system_prompt: input.system_prompt,
                harness_id: input.harness_id,
                tags: input.tags,
                initial_files: input.initial_files,
                tools: input.tools,
                mcp_servers: input.mcp_servers,
                updated_at: now,
                ..existing.clone()
            };
            agents.insert(id, row.clone());
            return Ok(Some(row));
        }

        let row = AgentRow {
            id,
            public_id: input.public_id,
            org_id,
            name: input.name,
            display_name: input.display_name,
            description: input.description,
            system_prompt: input.system_prompt,
            default_model_id: input.default_model_id,
            harness_id: input.harness_id,
            harness_source: "explicit".to_string(),
            agent_identity_id: None,
            default_version_id: None,
            forked_from_agent_id: None,
            forked_from_version_id: None,
            root_agent_id: None,
            tags: input.tags,
            initial_files: input.initial_files,
            tools: input.tools,
            mcp_servers: input.mcp_servers,
            network_access: input.network_access,
            max_iterations: input.max_iterations,
            parallel_tool_calls: input.parallel_tool_calls,
            status: "active".to_string(),
            is_built_in: input.is_built_in,
            created_at: now,
            updated_at: now,
            archived_at: None,
            deleted_at: None,
            total_input_tokens: 0,
            total_output_tokens: 0,
            total_cache_read_tokens: 0,
            total_cache_creation_tokens: 0,
            total_actual_cost_usd: 0.0,
            total_estimated_cost_usd: 0.0,
            total_cost_usd: 0.0,
        };
        agents.insert(id, row.clone());
        Ok(Some(row))
    }

    pub async fn get_agent(&self, org_id: i64, id: AgentId) -> Result<Option<AgentRow>> {
        Ok(self
            .agents
            .read()
            .get(&id)
            .filter(|a| a.org_id == org_id)
            .cloned())
    }

    pub async fn get_agents_by_ids(&self, org_id: i64, ids: &[AgentId]) -> Result<Vec<AgentRow>> {
        Ok(self
            .agents
            .read()
            .values()
            .filter(|row| row.org_id == org_id && ids.contains(&row.id))
            .cloned()
            .collect())
    }

    /// Look up the owning org for an agent by its public_id (cross-org resolver).
    pub async fn get_agent_organization_id(&self, public_id: &str) -> Result<Option<i64>> {
        Ok(self
            .agents
            .read()
            .values()
            .find(|a| a.public_id == public_id)
            .map(|a| a.org_id))
    }

    pub async fn get_agent_by_public_id(
        &self,
        org_id: i64,
        public_id: &str,
    ) -> Result<Option<AgentRow>> {
        Ok(self
            .agents
            .read()
            .values()
            .find(|a| a.org_id == org_id && a.public_id == public_id)
            .cloned())
    }

    /// Flag an agent as platform-supplied. Idempotent; used by org bootstrap.
    pub async fn mark_agent_built_in(&self, org_id: i64, id: AgentId) -> Result<()> {
        if let Some(agent) = self
            .agents
            .write()
            .values_mut()
            .find(|a| a.org_id == org_id && a.id == id)
        {
            agent.is_built_in = true;
        }
        Ok(())
    }

    /// Count agents against the per-org limit.
    /// Includes active and archived; excludes soft-deleted and built-in rows.
    pub async fn count_agents_for_org(&self, org_id: i64) -> Result<i64> {
        Ok(self
            .agents
            .read()
            .values()
            // Built-in agents are platform-supplied and do not consume the
            // org's quota — same rule as built-in harnesses.
            .filter(|a| a.org_id == org_id && a.status != "deleted" && !a.is_built_in)
            .count() as i64)
    }

    pub async fn list_agents(
        &self,
        org_id: i64,
        search: Option<&str>,
        include_archived: bool,
        pagination: crate::api::common::Pagination,
    ) -> Result<(Vec<AgentRow>, u32)> {
        let agents = self.agents.read();
        let mut result: Vec<_> = agents
            .values()
            .filter(|a| {
                a.org_id == org_id
                    && if include_archived {
                        a.status != "deleted"
                    } else {
                        a.status == "active"
                    }
            })
            .filter(|a| {
                matches_search_tokens(
                    search,
                    &[
                        a.display_name.as_deref().unwrap_or(""),
                        &a.name,
                        a.description.as_deref().unwrap_or(""),
                    ],
                )
            })
            .cloned()
            .collect();
        result.sort_by_key(|agent| std::cmp::Reverse(agent.created_at));

        let total = result.len() as u32;
        let offset = pagination.offset as usize;
        let limit = pagination.limit as usize;
        let paginated = result.into_iter().skip(offset).take(limit).collect();

        Ok((paginated, total))
    }

    pub async fn get_agent_by_name(&self, org_id: i64, name: &str) -> Result<Option<AgentRow>> {
        Ok(self
            .agents
            .read()
            .values()
            .find(|a| a.org_id == org_id && a.name == name && a.status != "deleted")
            .cloned())
    }

    pub async fn update_agent(
        &self,
        org_id: i64,
        id: AgentId,
        input: UpdateAgent,
    ) -> Result<Option<AgentRow>> {
        let mut agents = self.agents.write();
        if let Some(agent) = agents.get_mut(&id).filter(|a| a.org_id == org_id) {
            if let Some(name) = input.name {
                agent.name = name;
            }
            if let Some(display_name) = input.display_name {
                agent.display_name = Some(display_name);
            }
            if let Some(description) = input.description {
                agent.description = Some(description);
            }
            if let Some(system_prompt) = input.system_prompt {
                agent.system_prompt = system_prompt;
            }
            if let Some(default_model_id) = input.default_model_id {
                agent.default_model_id = Some(default_model_id);
            }
            if let Some(harness_id) = input.harness_id {
                agent.harness_id = harness_id;
            }
            if let Some(harness_source) = input.harness_source {
                agent.harness_source = harness_source;
            }
            if let Some(default_version_id) = input.default_version_id {
                agent.default_version_id = Some(default_version_id);
            }
            if let Some(forked_from_agent_id) = input.forked_from_agent_id {
                agent.forked_from_agent_id = Some(forked_from_agent_id);
            }
            if let Some(forked_from_version_id) = input.forked_from_version_id {
                agent.forked_from_version_id = Some(forked_from_version_id);
            }
            if let Some(root_agent_id) = input.root_agent_id {
                agent.root_agent_id = Some(root_agent_id);
            }
            if let Some(tags) = input.tags {
                agent.tags = tags;
            }
            if let Some(initial_files) = input.initial_files {
                agent.initial_files = initial_files;
            }
            if let Some(status) = input.status {
                agent.status = status;
            }
            if let Some(tools) = input.tools {
                agent.tools = tools;
            }
            if let Some(mcp_servers) = input.mcp_servers {
                agent.mcp_servers = mcp_servers;
            }
            if let Some(network_access) = input.network_access {
                agent.network_access = network_access;
            }
            if let Some(max_iterations) = input.max_iterations {
                agent.max_iterations = max_iterations;
            }
            if let Some(parallel_tool_calls) = input.parallel_tool_calls {
                agent.parallel_tool_calls = parallel_tool_calls;
            }
            agent.updated_at = Self::now();
            return Ok(Some(agent.clone()));
        }
        Ok(None)
    }

    /// Link an agent to its lazily-created identity, only when not already
    /// linked (EVE-758). Returns `true` when this call set the link, `false`
    /// when it was already linked — never overrides an existing link.
    pub async fn set_agent_identity_id(
        &self,
        org_id: i64,
        id: AgentId,
        agent_identity_id: AgentIdentityId,
    ) -> Result<bool> {
        let mut agents = self.agents.write();
        if let Some(agent) = agents.get_mut(&id).filter(|a| a.org_id == org_id) {
            if agent.agent_identity_id.is_some() {
                return Ok(false);
            }
            agent.agent_identity_id = Some(agent_identity_id);
            agent.updated_at = Self::now();
            return Ok(true);
        }
        Ok(false)
    }

    pub async fn has_agent_with_identity(
        &self,
        org_id: i64,
        agent_identity_id: AgentIdentityId,
    ) -> Result<bool> {
        Ok(self.agents.read().values().any(|agent| {
            agent.org_id == org_id
                && agent.agent_identity_id == Some(agent_identity_id)
                && agent.status != "deleted"
        }))
    }

    pub async fn delete_agent(&self, org_id: i64, id: AgentId) -> Result<bool> {
        let mut agents = self.agents.write();
        if let Some(agent) = agents.get_mut(&id) {
            if agent.org_id != org_id || agent.status != "active" {
                return Ok(false);
            }
            agent.status = "archived".to_string();
            agent.archived_at = Some(Self::now());
            agent.updated_at = Self::now();
            return Ok(true);
        }
        Ok(false)
    }

    pub async fn destroy_agent(&self, org_id: i64, id: AgentId) -> Result<bool> {
        let mut agents = self.agents.write();
        if let Some(agent) = agents.get_mut(&id) {
            if agent.org_id != org_id || agent.status != "archived" {
                return Ok(false);
            }
            agent.status = "deleted".to_string();
            agent.deleted_at = Some(Self::now());
            agent.updated_at = Self::now();
            return Ok(true);
        }
        Ok(false)
    }

    /// Upsert agent by public_id. Returns (row, was_created).
    pub async fn upsert_agent(
        &self,
        org_id: i64,
        input: CreateAgentRow,
    ) -> Result<(AgentRow, bool)> {
        let mut agents = self.agents.write();
        let existing_key = agents
            .iter()
            .find(|(_, a)| a.org_id == org_id && a.public_id == input.public_id)
            .map(|(k, _)| *k);

        if let Some(key) = existing_key {
            let agent = agents.get_mut(&key).unwrap();
            agent.name = input.name;
            agent.display_name = input.display_name;
            agent.description = input.description;
            agent.system_prompt = input.system_prompt;
            agent.default_model_id = input.default_model_id;
            agent.harness_id = input.harness_id;
            agent.tags = input.tags;
            agent.initial_files = input.initial_files;
            agent.tools = input.tools;
            agent.mcp_servers = input.mcp_servers;
            agent.max_iterations = input.max_iterations;
            agent.parallel_tool_calls = input.parallel_tool_calls;
            agent.status = "active".to_string();
            agent.updated_at = Self::now();
            Ok((agent.clone(), false))
        } else {
            let now = Self::now();
            let id = AgentId::new();
            let row = AgentRow {
                id,
                public_id: input.public_id,
                org_id,
                name: input.name,
                display_name: input.display_name,
                description: input.description,
                system_prompt: input.system_prompt,
                default_model_id: input.default_model_id,
                harness_id: input.harness_id,
                harness_source: "explicit".to_string(),
                agent_identity_id: None,
                default_version_id: None,
                forked_from_agent_id: None,
                forked_from_version_id: None,
                root_agent_id: None,
                tags: input.tags,
                initial_files: input.initial_files,
                tools: input.tools,
                mcp_servers: input.mcp_servers,
                network_access: input.network_access,
                max_iterations: input.max_iterations,
                parallel_tool_calls: input.parallel_tool_calls,
                status: "active".to_string(),
                is_built_in: input.is_built_in,
                created_at: now,
                updated_at: now,
                archived_at: None,
                deleted_at: None,
                total_input_tokens: 0,
                total_output_tokens: 0,
                total_cache_read_tokens: 0,
                total_cache_creation_tokens: 0,
                total_actual_cost_usd: 0.0,
                total_estimated_cost_usd: 0.0,
                total_cost_usd: 0.0,
            };
            agents.insert(id, row.clone());
            Ok((row, true))
        }
    }

    /// Upsert agent by name within org. Returns (row, was_created).
    pub async fn upsert_agent_by_name(
        &self,
        org_id: i64,
        input: CreateAgentRow,
    ) -> Result<(AgentRow, bool)> {
        let mut agents = self.agents.write();
        let existing_key = agents
            .iter()
            .find(|(_, a)| a.org_id == org_id && a.name == input.name && a.status != "deleted")
            .map(|(k, _)| *k);

        if let Some(key) = existing_key {
            let agent = agents.get_mut(&key).unwrap();
            agent.display_name = input.display_name;
            agent.description = input.description;
            agent.system_prompt = input.system_prompt;
            agent.default_model_id = input.default_model_id;
            agent.harness_id = input.harness_id;
            agent.tags = input.tags;
            agent.initial_files = input.initial_files;
            agent.tools = input.tools;
            agent.mcp_servers = input.mcp_servers;
            agent.network_access = input.network_access;
            agent.max_iterations = input.max_iterations;
            agent.parallel_tool_calls = input.parallel_tool_calls;
            agent.status = "active".to_string();
            agent.updated_at = Self::now();
            Ok((agent.clone(), false))
        } else {
            let now = Self::now();
            let id = AgentId::new();
            let row = AgentRow {
                id,
                public_id: input.public_id,
                org_id,
                name: input.name,
                display_name: input.display_name,
                description: input.description,
                system_prompt: input.system_prompt,
                default_model_id: input.default_model_id,
                harness_id: input.harness_id,
                harness_source: "explicit".to_string(),
                agent_identity_id: None,
                default_version_id: None,
                forked_from_agent_id: None,
                forked_from_version_id: None,
                root_agent_id: None,
                tags: input.tags,
                initial_files: input.initial_files,
                tools: input.tools,
                mcp_servers: input.mcp_servers,
                network_access: input.network_access,
                max_iterations: input.max_iterations,
                parallel_tool_calls: input.parallel_tool_calls,
                status: "active".to_string(),
                is_built_in: input.is_built_in,
                created_at: now,
                updated_at: now,
                archived_at: None,
                deleted_at: None,
                total_input_tokens: 0,
                total_output_tokens: 0,
                total_cache_read_tokens: 0,
                total_cache_creation_tokens: 0,
                total_actual_cost_usd: 0.0,
                total_estimated_cost_usd: 0.0,
                total_cost_usd: 0.0,
            };
            agents.insert(id, row.clone());
            Ok((row, true))
        }
    }

    /// Get agent public_id from internal UUID
    pub async fn get_agent_public_id(&self, org_id: i64, id: AgentId) -> Result<Option<String>> {
        Ok(self
            .agents
            .read()
            .get(&id)
            .filter(|a| a.org_id == org_id)
            .map(|a| a.public_id.clone()))
    }

    pub async fn create_agent_version(
        &self,
        input: CreateAgentVersionRow,
    ) -> Result<AgentVersionRow> {
        let row = AgentVersionRow {
            id: input.id,
            public_id: input.public_id,
            org_id: input.org_id,
            agent_id: input.agent_id,
            version_number: input.version_number,
            semver_major: input.semver_major,
            semver_minor: input.semver_minor,
            semver_patch: input.semver_patch,
            version: input.version,
            is_published: input.is_published,
            parent_version_id: input.parent_version_id,
            source_version_id: input.source_version_id,
            created_by_principal_id: input.created_by_principal_id,
            change_kind: input.change_kind,
            summary: input.summary,
            config_hash: input.config_hash,
            authored_config: input.authored_config,
            resolved_config: input.resolved_config,
            created_at: Self::now(),
        };
        self.agent_versions.write().insert(row.id, row.clone());
        Ok(row)
    }

    pub async fn list_agent_versions(
        &self,
        org_id: i64,
        agent_id: AgentId,
    ) -> Result<Vec<AgentVersionRow>> {
        let mut rows: Vec<_> = self
            .agent_versions
            .read()
            .values()
            .filter(|row| row.org_id == org_id && row.agent_id == agent_id)
            .cloned()
            .collect();
        rows.sort_by_key(|row| std::cmp::Reverse(row.version_number));
        Ok(rows)
    }

    pub async fn get_agent_version(
        &self,
        org_id: i64,
        id: everruns_core::AgentVersionId,
    ) -> Result<Option<AgentVersionRow>> {
        Ok(self
            .agent_versions
            .read()
            .get(&id)
            .filter(|row| row.org_id == org_id)
            .cloned())
    }

    pub async fn get_latest_agent_version(
        &self,
        org_id: i64,
        agent_id: AgentId,
    ) -> Result<Option<AgentVersionRow>> {
        Ok(self
            .list_agent_versions(org_id, agent_id)
            .await?
            .into_iter()
            .find(|row| row.is_published))
    }

    pub async fn get_latest_agent_snapshot(
        &self,
        org_id: i64,
        agent_id: AgentId,
    ) -> Result<Option<AgentVersionRow>> {
        Ok(self
            .list_agent_versions(org_id, agent_id)
            .await?
            .into_iter()
            .next())
    }

    pub async fn prune_agent_auto_snapshots(
        &self,
        org_id: i64,
        agent_id: AgentId,
        keep: i64,
    ) -> Result<u64> {
        let keep = keep.max(0) as usize;
        let mut rows: Vec<_> = self
            .agent_versions
            .read()
            .values()
            .filter(|row| {
                row.org_id == org_id
                    && row.agent_id == agent_id
                    && !row.is_published
                    && row.change_kind == "auto"
            })
            .cloned()
            .collect();
        rows.sort_by_key(|row| std::cmp::Reverse(row.version_number));

        let ids_to_remove: Vec<_> = rows.into_iter().skip(keep).map(|row| row.id).collect();
        let removed = ids_to_remove.len() as u64;
        let mut versions = self.agent_versions.write();
        for id in ids_to_remove {
            versions.remove(&id);
        }
        Ok(removed)
    }

    // ============================================
    // Agent Capabilities
    // ============================================

    pub async fn get_agent_capabilities(&self, agent_id: Uuid) -> Result<Vec<AgentCapabilityRow>> {
        let agent_id = AgentId::from_uuid(agent_id);
        let caps = self.agent_capabilities.read();
        let mut result: Vec<_> = caps
            .iter()
            .filter(|((aid, _), _)| *aid == agent_id)
            .map(|(_, c)| c.clone())
            .collect();
        result.sort_by_key(|c| c.position);
        Ok(result)
    }

    pub async fn get_agent_capabilities_by_agent_ids(
        &self,
        org_id: i64,
        agent_ids: &[AgentId],
    ) -> Result<Vec<AgentCapabilityRow>> {
        let agents = self.agents.read();
        let mut result: Vec<_> = self
            .agent_capabilities
            .read()
            .values()
            .filter(|row| {
                agent_ids.contains(&row.agent_id)
                    && agents
                        .get(&row.agent_id)
                        .is_some_and(|agent| agent.org_id == org_id)
            })
            .cloned()
            .collect();
        result.sort_by_key(|row| (row.agent_id.uuid(), row.position));
        Ok(result)
    }

    pub async fn set_agent_capabilities(
        &self,
        agent_id: Uuid,
        capabilities: Vec<(String, i32, serde_json::Value)>,
    ) -> Result<Vec<AgentCapabilityRow>> {
        let agent_id = AgentId::from_uuid(agent_id);
        let now = Self::now();
        let mut caps = self.agent_capabilities.write();

        // Remove existing capabilities for this agent
        let to_remove: Vec<_> = caps
            .keys()
            .filter(|(aid, _)| *aid == agent_id)
            .cloned()
            .collect();
        for key in to_remove {
            caps.remove(&key);
        }

        // Add new capabilities
        let mut result = Vec::new();
        for (capability_id, position, config) in capabilities.into_iter() {
            let row = AgentCapabilityRow {
                id: Uuid::now_v7(),
                agent_id,
                capability_id: capability_id.clone(),
                position,
                config,
                created_at: now,
            };
            caps.insert((agent_id, capability_id), row.clone());
            result.push(row);
        }

        Ok(result)
    }

    pub async fn add_agent_capability(
        &self,
        input: CreateAgentCapabilityRow,
    ) -> Result<AgentCapabilityRow> {
        let now = Self::now();
        let mut caps = self.agent_capabilities.write();

        let row = AgentCapabilityRow {
            id: Uuid::now_v7(),
            agent_id: input.agent_id,
            capability_id: input.capability_id.clone(),
            position: input.position,
            config: input.config,
            created_at: now,
        };
        caps.insert((input.agent_id, input.capability_id), row.clone());
        Ok(row)
    }

    pub async fn remove_agent_capability(
        &self,
        agent_id: Uuid,
        capability_id: &str,
    ) -> Result<bool> {
        let agent_id = AgentId::from_uuid(agent_id);
        Ok(self
            .agent_capabilities
            .write()
            .remove(&(agent_id, capability_id.to_string()))
            .is_some())
    }

    /// Count how many active agents reference each capability ID, scoped to an org.
    pub async fn count_agent_capability_references(
        &self,
        org_id: i64,
    ) -> Result<HashMap<String, u64>> {
        let agents = self.agents.read();
        let active: std::collections::HashSet<AgentId> = agents
            .values()
            .filter(|a| a.org_id == org_id && a.status == "active")
            .map(|a| a.id)
            .collect();

        let caps = self.agent_capabilities.read();
        let mut counts: HashMap<String, u64> = HashMap::new();
        for ((agent_id, cap_id), _) in caps.iter() {
            if active.contains(agent_id) {
                *counts.entry(cap_id.clone()).or_insert(0) += 1;
            }
        }
        Ok(counts)
    }

    /// Count how many active agents reference a single capability ID, scoped to an org.
    pub async fn count_agents_for_capability(
        &self,
        org_id: i64,
        capability_id: &str,
    ) -> Result<u64> {
        let agents = self.agents.read();
        let caps = self.agent_capabilities.read();
        let count = caps
            .iter()
            .filter(|((agent_id, cap_id), _)| {
                cap_id == capability_id
                    && agents
                        .get(agent_id)
                        .is_some_and(|a| a.org_id == org_id && a.status == "active")
            })
            .count();
        Ok(count as u64)
    }
}

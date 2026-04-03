// In-memory storage: Agents, Agent Capabilities

use super::super::models::*;
use super::InMemoryDatabase;
use super::matches_search_tokens;
use anyhow::Result;
use everruns_core::AgentId;
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
            description: input.description,
            system_prompt: input.system_prompt,
            default_model_id: input.default_model_id,
            tags: input.tags,
            initial_files: input.initial_files,
            tools: input.tools,
            network_access: input.network_access,
            max_iterations: input.max_iterations,
            status: "active".to_string(),
            created_at: now,
            updated_at: now,
            archived_at: None,
            deleted_at: None,
            total_input_tokens: 0,
            total_output_tokens: 0,
            total_cache_read_tokens: 0,
            total_cache_creation_tokens: 0,
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
                && existing.description == input.description
                && existing.system_prompt == input.system_prompt
                && existing.tags == input.tags
                && existing.initial_files == input.initial_files
                && existing.tools == input.tools
            {
                return Ok(None); // Unchanged
            }
            // Update changed fields
            let row = AgentRow {
                name: input.name,
                description: input.description,
                system_prompt: input.system_prompt,
                tags: input.tags,
                initial_files: input.initial_files,
                tools: input.tools,
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
            description: input.description,
            system_prompt: input.system_prompt,
            default_model_id: input.default_model_id,
            tags: input.tags,
            initial_files: input.initial_files,
            tools: input.tools,
            network_access: input.network_access,
            max_iterations: input.max_iterations,
            status: "active".to_string(),
            created_at: now,
            updated_at: now,
            archived_at: None,
            deleted_at: None,
            total_input_tokens: 0,
            total_output_tokens: 0,
            total_cache_read_tokens: 0,
            total_cache_creation_tokens: 0,
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
                matches_search_tokens(search, &[&a.name, a.description.as_deref().unwrap_or("")])
            })
            .cloned()
            .collect();
        result.sort_by(|a, b| b.created_at.cmp(&a.created_at));

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
            .find(|a| a.org_id == org_id && a.name == name && a.status == "active")
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
            if let Some(description) = input.description {
                agent.description = Some(description);
            }
            if let Some(system_prompt) = input.system_prompt {
                agent.system_prompt = system_prompt;
            }
            if let Some(default_model_id) = input.default_model_id {
                agent.default_model_id = Some(default_model_id);
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
            if let Some(network_access) = input.network_access {
                agent.network_access = network_access;
            }
            if let Some(max_iterations) = input.max_iterations {
                agent.max_iterations = max_iterations;
            }
            agent.updated_at = Self::now();
            return Ok(Some(agent.clone()));
        }
        Ok(None)
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
            agent.description = input.description;
            agent.system_prompt = input.system_prompt;
            agent.default_model_id = input.default_model_id;
            agent.tags = input.tags;
            agent.initial_files = input.initial_files;
            agent.tools = input.tools;
            agent.max_iterations = input.max_iterations;
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
                description: input.description,
                system_prompt: input.system_prompt,
                default_model_id: input.default_model_id,
                tags: input.tags,
                initial_files: input.initial_files,
                tools: input.tools,
                network_access: input.network_access,
                max_iterations: input.max_iterations,
                status: "active".to_string(),
                created_at: now,
                updated_at: now,
                archived_at: None,
                deleted_at: None,
                total_input_tokens: 0,
                total_output_tokens: 0,
                total_cache_read_tokens: 0,
                total_cache_creation_tokens: 0,
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
}

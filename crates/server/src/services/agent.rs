// Agent service for business logic (M2)
//
// Note: OTel instrumentation is handled via the event-listener pattern.
// Agent creation events are not yet implemented but would be handled
// by event listeners rather than direct spans.

use crate::errors::ResourceNotFoundError;
use crate::storage::{
    AgentRow, StorageBackend,
    models::{CreateAgentRow, UpdateAgent},
};
use anyhow::Result;
use everruns_core::{
    Agent, AgentCapabilityConfig, AgentId, AgentStatus, Caller, InitialFile, Permission, Policy,
    Rule, TokenUsage,
};
use everruns_macros::policy;
use std::sync::Arc;
use uuid::Uuid;

use crate::api::agents::{CreateAgentRequest, UpdateAgentRequest};

/// Policy: View agents (read-only).
pub const AGENT_VIEW: Policy = Policy {
    id: "agent.view",
    rules: &[Rule::UserHasPermission(Permission::OrgAgentsManage)],
};

/// Policy: Manage agents (create, update, copy, delete).
pub const AGENT_MANAGE: Policy = Policy {
    id: "agent.manage",
    rules: &[Rule::UserHasPermission(Permission::OrgAgentsManage)],
};

pub const AGENT_DANGEROUS: Policy = Policy {
    id: "agent.dangerous",
    rules: &[
        Rule::UserHasPermission(Permission::OrgAgentsManage),
        Rule::UserHasPermission(Permission::OrgAgentsDangerous),
    ],
};

pub struct AgentService {
    db: Arc<StorageBackend>,
}

fn ensure_file_system_capability(
    mut capabilities: Vec<AgentCapabilityConfig>,
    has_initial_files: bool,
) -> Vec<AgentCapabilityConfig> {
    if has_initial_files
        && !capabilities
            .iter()
            .any(|cap| cap.capability_id() == "session_file_system")
    {
        capabilities.insert(0, AgentCapabilityConfig::new("session_file_system"));
    }
    capabilities
}

impl AgentService {
    pub fn new(db: Arc<StorageBackend>) -> Self {
        Self { db }
    }

    #[policy(AGENT_MANAGE)]
    pub async fn create(
        &self,
        caller: &Caller,
        client_id: Option<AgentId>,
        req: CreateAgentRequest,
    ) -> Result<Agent> {
        let capabilities_to_store =
            ensure_file_system_capability(req.capabilities.clone(), !req.initial_files.is_empty());
        crate::services::capability_validation::validate_capability_refs(
            &self.db,
            caller.org_id,
            &capabilities_to_store,
        )
        .await?;
        let default_model_id = self
            .validate_default_model_id(caller.org_id, req.default_model_id)
            .await?;

        // When no client_id, generate internal UUID and derive public_id from it.
        // This keeps public_id == AgentId::from_uuid(internal_id), so session FKs
        // (which store the internal UUID) serialize to the same agent_<hex> string.
        // When client_id is supplied, public_id differs from internal_id.
        let (row, agent_id_uuid) = if let Some(client_id) = client_id {
            let input = CreateAgentRow {
                public_id: client_id.to_string(),
                name: req.name.clone(),
                description: req.description.clone(),
                system_prompt: req.system_prompt.clone(),
                default_model_id,
                tags: req.tags.clone(),
                initial_files: serde_json::to_value(&req.initial_files).unwrap_or_default(),
                tools: serde_json::to_value(&req.tools).unwrap_or_default(),
            };
            let row = self.db.create_agent(caller.org_id, input).await?;
            let uuid = row.id.uuid();
            (row, uuid)
        } else {
            let internal_uuid = Uuid::now_v7();
            let public_id = AgentId::from_uuid(internal_uuid);
            let input = CreateAgentRow {
                public_id: public_id.to_string(),
                name: req.name.clone(),
                description: req.description.clone(),
                system_prompt: req.system_prompt.clone(),
                default_model_id,
                tags: req.tags.clone(),
                initial_files: serde_json::to_value(&req.initial_files).unwrap_or_default(),
                tools: serde_json::to_value(&req.tools).unwrap_or_default(),
            };
            let row = self
                .db
                .create_agent_with_id(caller.org_id, AgentId::from_uuid(internal_uuid), input)
                .await?
                .ok_or_else(|| anyhow::anyhow!("Agent UUID collision"))?;
            (row, internal_uuid)
        };

        // Set capabilities if provided
        let capabilities = if !capabilities_to_store.is_empty() {
            let cap_tuples: Vec<(String, i32, serde_json::Value)> = capabilities_to_store
                .iter()
                .enumerate()
                .map(|(idx, cap)| {
                    (
                        cap.capability_ref.to_string(),
                        idx as i32,
                        cap.config.clone(),
                    )
                })
                .collect();
            self.db
                .set_agent_capabilities(agent_id_uuid, cap_tuples)
                .await?;
            capabilities_to_store
        } else {
            vec![]
        };

        Ok(Self::row_to_agent(row, capabilities))
    }

    #[policy(AGENT_VIEW)]
    pub async fn get(&self, caller: &Caller, id: Uuid) -> Result<Option<Agent>> {
        let row = self
            .db
            .get_agent(caller.org_id, AgentId::from_uuid(id))
            .await?;
        match row {
            Some(row) if row.status != "deleted" => {
                let capabilities = self.get_capabilities(id).await?;
                Ok(Some(Self::row_to_agent(row, capabilities)))
            }
            None => Ok(None),
            Some(_) => Ok(None),
        }
    }

    #[policy(AGENT_VIEW)]
    pub async fn get_by_public_id(
        &self,
        caller: &Caller,
        public_id: &str,
    ) -> Result<Option<Agent>> {
        let row = self
            .db
            .get_agent_by_public_id(caller.org_id, public_id)
            .await?;
        match row {
            Some(row) if row.status != "deleted" => {
                let capabilities = self.get_capabilities(row.id.uuid()).await?;
                Ok(Some(Self::row_to_agent(row, capabilities)))
            }
            None => Ok(None),
            Some(_) => Ok(None),
        }
    }

    #[policy(AGENT_VIEW)]
    pub async fn list(
        &self,
        caller: &Caller,
        search: Option<&str>,
        include_archived: bool,
        pagination: crate::api::common::Pagination,
    ) -> Result<(Vec<Agent>, u32)> {
        let (rows, total) = self
            .db
            .list_agents(caller.org_id, search, include_archived, pagination)
            .await?;

        // Fetch capabilities for each agent
        let mut agents = Vec::with_capacity(rows.len());
        for row in rows {
            let capabilities = self.get_capabilities(row.id.uuid()).await?;
            agents.push(Self::row_to_agent(row, capabilities));
        }

        Ok((agents, total))
    }

    #[policy(AGENT_MANAGE)]
    pub async fn update(
        &self,
        caller: &Caller,
        public_id: &str,
        req: UpdateAgentRequest,
    ) -> Result<Option<Agent>> {
        // Resolve public_id -> internal AgentId
        let row = self
            .db
            .get_agent_by_public_id(caller.org_id, public_id)
            .await?;
        let Some(existing) = row else {
            return Ok(None);
        };
        if existing.status != "active" {
            anyhow::bail!("Archived or deleted agents cannot be edited");
        }
        let internal_id = existing.id;
        let existing_initial_files: Vec<InitialFile> =
            serde_json::from_value(existing.initial_files.clone()).unwrap_or_default();
        let final_has_initial_files = req
            .initial_files
            .as_ref()
            .map(|files| !files.is_empty())
            .unwrap_or(!existing_initial_files.is_empty());

        let capabilities_override = match req.capabilities.clone() {
            Some(caps) => Some(ensure_file_system_capability(caps, final_has_initial_files)),
            None if final_has_initial_files => Some(ensure_file_system_capability(
                self.get_capabilities(internal_id.uuid()).await?,
                true,
            )),
            None => None,
        };
        if let Some(ref caps) = capabilities_override {
            crate::services::capability_validation::validate_capability_refs(
                &self.db,
                caller.org_id,
                caps,
            )
            .await?;
        }
        let default_model_id = self
            .validate_default_model_id(caller.org_id, req.default_model_id)
            .await?;

        let input = UpdateAgent {
            name: req.name,
            description: req.description,
            system_prompt: req.system_prompt,
            default_model_id,
            tags: req.tags,
            status: req.status.map(|s| s.to_string()),
            initial_files: req
                .initial_files
                .map(|files| serde_json::to_value(&files).unwrap_or_default()),
            tools: req
                .tools
                .map(|t| serde_json::to_value(&t).unwrap_or_default()),
        };
        let row = self
            .db
            .update_agent(caller.org_id, internal_id, input)
            .await?;

        match row {
            Some(row) => {
                // Update capabilities if provided or required by initial files
                let capabilities = if let Some(caps) = capabilities_override {
                    let cap_tuples: Vec<(String, i32, serde_json::Value)> = caps
                        .iter()
                        .enumerate()
                        .map(|(idx, cap)| {
                            (
                                cap.capability_ref.to_string(),
                                idx as i32,
                                cap.config.clone(),
                            )
                        })
                        .collect();
                    self.db
                        .set_agent_capabilities(internal_id.uuid(), cap_tuples)
                        .await?;
                    caps
                } else {
                    self.get_capabilities(internal_id.uuid()).await?
                };

                Ok(Some(Self::row_to_agent(row, capabilities)))
            }
            None => Ok(None),
        }
    }

    /// Copy an agent by public_id. Creates a new agent with "{name} (copy)" and
    /// duplicates description, system_prompt, default_model_id, tags, capabilities, tools.
    #[policy(AGENT_MANAGE)]
    pub async fn copy(&self, caller: &Caller, public_id: &str) -> Result<Option<Agent>> {
        let source = self.get_by_public_id(caller, public_id).await?;
        let Some(source) = source else {
            return Ok(None);
        };

        let req = CreateAgentRequest {
            id: None,
            name: format!("{} (copy)", source.name),
            description: source.description,
            system_prompt: source.system_prompt,
            default_model_id: source.default_model_id,
            tags: source.tags,
            capabilities: source.capabilities,
            initial_files: source.initial_files,
            tools: source.tools,
        };

        let agent = self.create(caller, None, req).await?;
        Ok(Some(agent))
    }

    #[policy(AGENT_MANAGE)]
    pub async fn delete(&self, caller: &Caller, public_id: &str) -> Result<bool> {
        // Resolve public_id -> internal AgentId
        let row = self
            .db
            .get_agent_by_public_id(caller.org_id, public_id)
            .await?;
        let Some(existing) = row else {
            return Ok(false);
        };
        self.db.delete_agent(caller.org_id, existing.id).await
    }

    #[policy(AGENT_DANGEROUS)]
    pub async fn destroy(&self, caller: &Caller, public_id: &str) -> Result<bool> {
        let row = self
            .db
            .get_agent_by_public_id(caller.org_id, public_id)
            .await?;
        let Some(existing) = row else {
            return Ok(false);
        };
        if existing.status != "archived" {
            anyhow::bail!("Agent must be archived before deletion");
        }
        self.db.destroy_agent(caller.org_id, existing.id).await
    }

    /// Upsert agent by public_id. Returns (agent, was_created).
    #[policy(AGENT_MANAGE)]
    pub async fn upsert(
        &self,
        caller: &Caller,
        public_id: &str,
        req: CreateAgentRequest,
    ) -> Result<(Agent, bool)> {
        let capabilities_to_store =
            ensure_file_system_capability(req.capabilities.clone(), !req.initial_files.is_empty());
        crate::services::capability_validation::validate_capability_refs(
            &self.db,
            caller.org_id,
            &capabilities_to_store,
        )
        .await?;
        let default_model_id = self
            .validate_default_model_id(caller.org_id, req.default_model_id)
            .await?;

        let input = CreateAgentRow {
            public_id: public_id.to_string(),
            name: req.name,
            description: req.description,
            system_prompt: req.system_prompt,
            default_model_id,
            tags: req.tags,
            initial_files: serde_json::to_value(&req.initial_files).unwrap_or_default(),
            tools: serde_json::to_value(&req.tools).unwrap_or_default(),
        };
        let (row, was_created) = self.db.upsert_agent(caller.org_id, input).await?;
        let agent_id_uuid = row.id.uuid();

        // Set capabilities if provided (replace existing on upsert)
        let capabilities = if !capabilities_to_store.is_empty() {
            let cap_tuples: Vec<(String, i32, serde_json::Value)> = capabilities_to_store
                .iter()
                .enumerate()
                .map(|(idx, cap)| {
                    (
                        cap.capability_ref.to_string(),
                        idx as i32,
                        cap.config.clone(),
                    )
                })
                .collect();
            self.db
                .set_agent_capabilities(agent_id_uuid, cap_tuples)
                .await?;
            capabilities_to_store
        } else if was_created {
            vec![]
        } else {
            // Existing agent, no capabilities in request -> keep existing
            self.get_capabilities(agent_id_uuid).await?
        };

        Ok((Self::row_to_agent(row, capabilities), was_created))
    }

    async fn get_capabilities(&self, agent_id: Uuid) -> Result<Vec<AgentCapabilityConfig>> {
        let rows = self.db.get_agent_capabilities(agent_id).await?;
        Ok(rows
            .into_iter()
            .map(|row| AgentCapabilityConfig::with_config(row.capability_id, row.config))
            .collect())
    }

    async fn validate_default_model_id(
        &self,
        org_id: i64,
        default_model_id: Option<everruns_core::ModelId>,
    ) -> Result<Option<everruns_core::ModelId>> {
        let Some(model_id) = default_model_id else {
            return Ok(None);
        };

        self.db
            .get_llm_model(org_id, model_id.uuid())
            .await?
            .ok_or_else(|| ResourceNotFoundError::new("Model"))?;

        Ok(Some(model_id))
    }

    fn row_to_agent(row: AgentRow, capabilities: Vec<AgentCapabilityConfig>) -> Agent {
        // Convert database usage columns to TokenUsage
        let usage = if row.total_input_tokens > 0 || row.total_output_tokens > 0 {
            Some(TokenUsage::with_cache(
                row.total_input_tokens as u32,
                row.total_output_tokens as u32,
                if row.total_cache_read_tokens > 0 {
                    Some(row.total_cache_read_tokens as u32)
                } else {
                    None
                },
                if row.total_cache_creation_tokens > 0 {
                    Some(row.total_cache_creation_tokens as u32)
                } else {
                    None
                },
            ))
        } else {
            None
        };

        // Parse public_id from the stored string
        let public_id: AgentId = row
            .public_id
            .parse()
            .unwrap_or_else(|_| AgentId::from_uuid(row.id.uuid()));

        Agent {
            public_id,
            internal_id: row.id.uuid(),
            name: row.name,
            description: row.description,
            system_prompt: row.system_prompt,
            default_model_id: row.default_model_id,
            tags: row.tags,
            capabilities,
            initial_files: serde_json::from_value::<Vec<InitialFile>>(row.initial_files)
                .unwrap_or_default(),
            tools: serde_json::from_value(row.tools).unwrap_or_default(),
            status: AgentStatus::from(row.status.as_str()),
            created_at: row.created_at,
            updated_at: row.updated_at,
            archived_at: row.archived_at,
            deleted_at: row.deleted_at,
            usage,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{CreateLlmModelRow, CreateLlmProviderRow, CreateOrganizationRow};
    use everruns_core::DEFAULT_ORG_ID;

    fn build_create_request(
        default_model_id: Option<everruns_core::ModelId>,
    ) -> CreateAgentRequest {
        CreateAgentRequest {
            id: None,
            name: "Test Agent".to_string(),
            description: None,
            system_prompt: "Test".to_string(),
            default_model_id,
            tags: vec![],
            capabilities: vec![],
            initial_files: vec![],
            tools: vec![],
        }
    }

    fn build_update_request(
        default_model_id: Option<everruns_core::ModelId>,
    ) -> UpdateAgentRequest {
        UpdateAgentRequest {
            name: None,
            description: None,
            system_prompt: None,
            default_model_id,
            tags: None,
            capabilities: None,
            initial_files: None,
            status: None,
            tools: None,
        }
    }

    async fn create_second_org(db: &StorageBackend) -> i64 {
        db.create_organization_with_id(
            2,
            CreateOrganizationRow {
                public_id: "org_2".to_string(),
                name: "Org 2".to_string(),
                created_by: None,
            },
        )
        .await
        .unwrap()
        .unwrap()
        .org_id
    }

    async fn create_model(
        db: &StorageBackend,
        org_id: i64,
        model_id: &str,
    ) -> everruns_core::ModelId {
        let provider = db
            .create_llm_provider(
                org_id,
                CreateLlmProviderRow {
                    name: format!("Provider {org_id}"),
                    provider_type: "openai".to_string(),
                    base_url: None,
                    api_key_encrypted: None,
                    settings: None,
                },
            )
            .await
            .unwrap();

        db.create_llm_model(
            org_id,
            CreateLlmModelRow {
                provider_id: provider.id,
                model_id: model_id.to_string(),
                display_name: model_id.to_string(),
                capabilities: vec![],
                installed: true,
                is_favorite: false,
                source: "manual".to_string(),
                provider_metadata: None,
            },
        )
        .await
        .unwrap()
        .id
    }

    #[tokio::test]
    async fn create_rejects_default_model_from_another_org() {
        let db = Arc::new(StorageBackend::in_memory());
        let service = AgentService::new(db.clone());
        let caller = Caller::internal(DEFAULT_ORG_ID);
        let other_org_id = create_second_org(&db).await;
        let other_model_id = create_model(&db, other_org_id, "cross-org-model").await;

        let err = service
            .create(&caller, None, build_create_request(Some(other_model_id)))
            .await
            .unwrap_err();

        assert_eq!(err.to_string(), "Model not found");
    }

    #[tokio::test]
    async fn update_rejects_default_model_from_another_org() {
        let db = Arc::new(StorageBackend::in_memory());
        let service = AgentService::new(db.clone());
        let caller = Caller::internal(DEFAULT_ORG_ID);
        let other_org_id = create_second_org(&db).await;
        let other_model_id = create_model(&db, other_org_id, "cross-org-model").await;

        let agent = service
            .create(&caller, None, build_create_request(None))
            .await
            .unwrap();

        let err = service
            .update(
                &caller,
                &agent.public_id.to_string(),
                build_update_request(Some(other_model_id)),
            )
            .await
            .unwrap_err();

        assert_eq!(err.to_string(), "Model not found");
    }

    #[tokio::test]
    async fn upsert_rejects_default_model_from_another_org() {
        let db = Arc::new(StorageBackend::in_memory());
        let service = AgentService::new(db.clone());
        let caller = Caller::internal(DEFAULT_ORG_ID);
        let other_org_id = create_second_org(&db).await;
        let other_model_id = create_model(&db, other_org_id, "cross-org-model").await;

        let err = service
            .upsert(
                &caller,
                "agent_00000000000000000000000000000099",
                build_create_request(Some(other_model_id)),
            )
            .await
            .unwrap_err();

        assert_eq!(err.to_string(), "Model not found");
    }

    #[tokio::test]
    async fn upsert_rejects_nonexistent_default_model() {
        let db = Arc::new(StorageBackend::in_memory());
        let service = AgentService::new(db.clone());
        let caller = Caller::internal(DEFAULT_ORG_ID);
        let fake_model_id = everruns_core::ModelId::from_uuid(uuid::Uuid::new_v4());

        let err = service
            .upsert(
                &caller,
                "agent_00000000000000000000000000000098",
                build_create_request(Some(fake_model_id)),
            )
            .await
            .unwrap_err();

        assert_eq!(err.to_string(), "Model not found");
    }

    #[tokio::test]
    async fn create_rejects_unknown_builtin_capability() {
        let db = Arc::new(StorageBackend::in_memory());
        let service = AgentService::new(db.clone());
        let caller = Caller::internal(DEFAULT_ORG_ID);

        let mut req = build_create_request(None);
        req.capabilities = vec![AgentCapabilityConfig::new("nonexistent_cap")];

        let err = service.create(&caller, None, req).await.unwrap_err();
        assert_eq!(err.to_string(), "Capability not found");
    }

    #[tokio::test]
    async fn create_rejects_nonexistent_mcp_ref() {
        let db = Arc::new(StorageBackend::in_memory());
        let service = AgentService::new(db.clone());
        let caller = Caller::internal(DEFAULT_ORG_ID);

        let mut req = build_create_request(None);
        req.capabilities = vec![AgentCapabilityConfig::new(format!(
            "mcp:{}",
            uuid::Uuid::new_v4()
        ))];

        let err = service.create(&caller, None, req).await.unwrap_err();
        assert_eq!(err.to_string(), "MCP server not found");
    }

    #[tokio::test]
    async fn create_rejects_nonexistent_skill_ref() {
        let db = Arc::new(StorageBackend::in_memory());
        let service = AgentService::new(db.clone());
        let caller = Caller::internal(DEFAULT_ORG_ID);

        let mut req = build_create_request(None);
        req.capabilities = vec![AgentCapabilityConfig::new(format!(
            "skill:{}",
            uuid::Uuid::new_v4()
        ))];

        let err = service.create(&caller, None, req).await.unwrap_err();
        assert_eq!(err.to_string(), "Skill not found");
    }

    #[tokio::test]
    async fn create_accepts_valid_builtin_capability() {
        let db = Arc::new(StorageBackend::in_memory());
        let service = AgentService::new(db.clone());
        let caller = Caller::internal(DEFAULT_ORG_ID);

        let mut req = build_create_request(None);
        req.capabilities = vec![AgentCapabilityConfig::new("current_time")];

        let agent = service.create(&caller, None, req).await.unwrap();
        assert_eq!(agent.capabilities[0].capability_id(), "current_time");
    }

    #[tokio::test]
    async fn update_rejects_unknown_capability() {
        let db = Arc::new(StorageBackend::in_memory());
        let service = AgentService::new(db.clone());
        let caller = Caller::internal(DEFAULT_ORG_ID);

        let agent = service
            .create(&caller, None, build_create_request(None))
            .await
            .unwrap();

        let mut req = build_update_request(None);
        req.capabilities = Some(vec![AgentCapabilityConfig::new("bogus_cap")]);

        let err = service
            .update(&caller, &agent.public_id.to_string(), req)
            .await
            .unwrap_err();

        assert_eq!(err.to_string(), "Capability not found");
    }

    #[tokio::test]
    async fn upsert_updates_initial_files() {
        let db = Arc::new(StorageBackend::in_memory());
        let service = AgentService::new(db.clone());
        let caller = Caller::internal(DEFAULT_ORG_ID);

        // Create agent with initial_files via upsert
        let public_id = "agent_00000000000000000000000000000050";
        let mut req = build_create_request(None);
        req.initial_files = vec![InitialFile {
            path: "/AGENTS.md".to_string(),
            content: "old content".to_string(),
            encoding: "utf-8".to_string(),
            is_readonly: false,
        }];

        let (agent, was_created) = service.upsert(&caller, public_id, req).await.unwrap();
        assert!(was_created);
        assert_eq!(agent.initial_files.len(), 1);
        assert_eq!(agent.initial_files[0].content, "old content");

        // Upsert again with updated initial_files
        let mut req2 = build_create_request(None);
        req2.initial_files = vec![InitialFile {
            path: "/AGENTS.md".to_string(),
            content: "new content".to_string(),
            encoding: "utf-8".to_string(),
            is_readonly: false,
        }];

        let (agent2, was_created2) = service.upsert(&caller, public_id, req2).await.unwrap();
        assert!(!was_created2);
        assert_eq!(agent2.initial_files.len(), 1);
        assert_eq!(agent2.initial_files[0].content, "new content");
    }
}

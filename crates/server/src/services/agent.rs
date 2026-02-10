// Agent service for business logic (M2)
//
// Note: OTel instrumentation is handled via the event-listener pattern.
// Agent creation events are not yet implemented but would be handled
// by event listeners rather than direct spans.

use crate::storage::{
    AgentRow, StorageBackend,
    models::{CreateAgentRow, UpdateAgent},
};
use anyhow::Result;
use everruns_core::{Agent, AgentCapabilityConfig, AgentId, AgentStatus, TokenUsage};
use std::sync::Arc;
use uuid::Uuid;

use crate::api::agents::{CreateAgentRequest, UpdateAgentRequest};

pub struct AgentService {
    db: Arc<StorageBackend>,
}

impl AgentService {
    pub fn new(db: Arc<StorageBackend>) -> Self {
        Self { db }
    }

    pub async fn create(
        &self,
        org_id: i64,
        client_id: Option<AgentId>,
        req: CreateAgentRequest,
    ) -> Result<Agent> {
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
                default_model_id: req.default_model_id,
                tags: req.tags.clone(),
                tools: serde_json::to_value(&req.tools).unwrap_or_default(),
            };
            let row = self.db.create_agent(org_id, input).await?;
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
                default_model_id: req.default_model_id,
                tags: req.tags.clone(),
                tools: serde_json::to_value(&req.tools).unwrap_or_default(),
            };
            let row = self
                .db
                .create_agent_with_id(org_id, AgentId::from_uuid(internal_uuid), input)
                .await?
                .ok_or_else(|| anyhow::anyhow!("Agent UUID collision"))?;
            (row, internal_uuid)
        };

        // Set capabilities if provided
        let capabilities = if !req.capabilities.is_empty() {
            let cap_tuples: Vec<(String, i32, serde_json::Value)> = req
                .capabilities
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
            req.capabilities
        } else {
            vec![]
        };

        Ok(Self::row_to_agent(row, capabilities))
    }

    pub async fn get(&self, org_id: i64, id: Uuid) -> Result<Option<Agent>> {
        let row = self.db.get_agent(org_id, AgentId::from_uuid(id)).await?;
        match row {
            Some(row) => {
                let capabilities = self.get_capabilities(id).await?;
                Ok(Some(Self::row_to_agent(row, capabilities)))
            }
            None => Ok(None),
        }
    }

    pub async fn get_by_public_id(&self, org_id: i64, public_id: &str) -> Result<Option<Agent>> {
        let row = self.db.get_agent_by_public_id(org_id, public_id).await?;
        match row {
            Some(row) => {
                let capabilities = self.get_capabilities(row.id.uuid()).await?;
                Ok(Some(Self::row_to_agent(row, capabilities)))
            }
            None => Ok(None),
        }
    }

    pub async fn list(&self, org_id: i64) -> Result<Vec<Agent>> {
        let rows = self.db.list_agents(org_id).await?;

        // Fetch capabilities for each agent
        let mut agents = Vec::with_capacity(rows.len());
        for row in rows {
            let capabilities = self.get_capabilities(row.id.uuid()).await?;
            agents.push(Self::row_to_agent(row, capabilities));
        }

        Ok(agents)
    }

    pub async fn update(
        &self,
        org_id: i64,
        public_id: &str,
        req: UpdateAgentRequest,
    ) -> Result<Option<Agent>> {
        // Resolve public_id -> internal AgentId
        let row = self.db.get_agent_by_public_id(org_id, public_id).await?;
        let Some(existing) = row else {
            return Ok(None);
        };
        let internal_id = existing.id;

        let input = UpdateAgent {
            name: req.name,
            description: req.description,
            system_prompt: req.system_prompt,
            default_model_id: req.default_model_id,
            tags: req.tags,
            status: req.status.map(|s| s.to_string()),
            tools: req
                .tools
                .map(|t| serde_json::to_value(&t).unwrap_or_default()),
        };
        let row = self.db.update_agent(org_id, internal_id, input).await?;

        match row {
            Some(row) => {
                // Update capabilities if provided
                let capabilities = if let Some(caps) = req.capabilities {
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

    pub async fn delete(&self, org_id: i64, public_id: &str) -> Result<bool> {
        // Resolve public_id -> internal AgentId
        let row = self.db.get_agent_by_public_id(org_id, public_id).await?;
        let Some(existing) = row else {
            return Ok(false);
        };
        self.db.delete_agent(org_id, existing.id).await
    }

    /// Upsert agent by public_id. Returns (agent, was_created).
    pub async fn upsert(
        &self,
        org_id: i64,
        public_id: &str,
        req: CreateAgentRequest,
    ) -> Result<(Agent, bool)> {
        let input = CreateAgentRow {
            public_id: public_id.to_string(),
            name: req.name,
            description: req.description,
            system_prompt: req.system_prompt,
            default_model_id: req.default_model_id,
            tags: req.tags,
            tools: serde_json::to_value(&req.tools).unwrap_or_default(),
        };
        let (row, was_created) = self.db.upsert_agent(org_id, input).await?;
        let agent_id_uuid = row.id.uuid();

        // Set capabilities if provided (replace existing on upsert)
        let capabilities = if !req.capabilities.is_empty() {
            let cap_tuples: Vec<(String, i32, serde_json::Value)> = req
                .capabilities
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
            req.capabilities
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
            tools: serde_json::from_value(row.tools).unwrap_or_default(),
            status: AgentStatus::from(row.status.as_str()),
            created_at: row.created_at,
            updated_at: row.updated_at,
            usage,
        }
    }
}

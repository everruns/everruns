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

    pub async fn create(&self, org_id: i64, req: CreateAgentRequest) -> Result<Agent> {
        // Note: OTel instrumentation is handled via event listeners.
        // Agent creation events would be handled by listeners rather than direct spans.
        let input = CreateAgentRow {
            name: req.name,
            description: req.description,
            system_prompt: req.system_prompt,
            default_model_id: req.default_model_id,
            tags: req.tags,
        };
        let row = self.db.create_agent(org_id, input).await?;
        let agent_id = row.id;

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
                .set_agent_capabilities(agent_id.uuid(), cap_tuples)
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
        id: Uuid,
        req: UpdateAgentRequest,
    ) -> Result<Option<Agent>> {
        let input = UpdateAgent {
            name: req.name,
            description: req.description,
            system_prompt: req.system_prompt,
            default_model_id: req.default_model_id,
            tags: req.tags,
            status: req.status.map(|s| s.to_string()),
        };
        let row = self
            .db
            .update_agent(org_id, AgentId::from_uuid(id), input)
            .await?;

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
                    self.db.set_agent_capabilities(id, cap_tuples).await?;
                    caps
                } else {
                    self.get_capabilities(id).await?
                };

                Ok(Some(Self::row_to_agent(row, capabilities)))
            }
            None => Ok(None),
        }
    }

    pub async fn delete(&self, org_id: i64, id: Uuid) -> Result<bool> {
        self.db.delete_agent(org_id, AgentId::from_uuid(id)).await
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

        Agent {
            id: row.id,
            name: row.name,
            description: row.description,
            system_prompt: row.system_prompt,
            default_model_id: row.default_model_id,
            tags: row.tags,
            capabilities,
            status: AgentStatus::from(row.status.as_str()),
            created_at: row.created_at,
            updated_at: row.updated_at,
            usage,
        }
    }
}

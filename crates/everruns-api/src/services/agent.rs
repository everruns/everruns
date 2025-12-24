// Agent service for business logic (M2)

use anyhow::Result;
use everruns_core::{Agent, AgentStatus};
use everruns_storage::{
    models::{CreateAgentRow, UpdateAgent},
    Database,
};
use std::sync::Arc;
use uuid::Uuid;

use crate::agents::{CreateAgentRequest, UpdateAgentRequest};

pub struct AgentService {
    db: Arc<Database>,
}

impl AgentService {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }

    pub async fn create(&self, req: CreateAgentRequest) -> Result<Agent> {
        let input = CreateAgentRow {
            name: req.name,
            description: req.description,
            system_prompt: req.system_prompt,
            default_model_id: req.default_model_id,
            tags: req.tags,
        };
        let row = self.db.create_agent(input).await?;
        Ok(Self::row_to_agent(row))
    }

    pub async fn get(&self, id: Uuid) -> Result<Option<Agent>> {
        let row = self.db.get_agent(id).await?;
        Ok(row.map(Self::row_to_agent))
    }

    pub async fn list(&self) -> Result<Vec<Agent>> {
        let rows = self.db.list_agents().await?;
        Ok(rows.into_iter().map(Self::row_to_agent).collect())
    }

    pub async fn update(&self, id: Uuid, req: UpdateAgentRequest) -> Result<Option<Agent>> {
        let input = UpdateAgent {
            name: req.name,
            description: req.description,
            system_prompt: req.system_prompt,
            default_model_id: req.default_model_id,
            tags: req.tags,
            status: req.status.map(|s| s.to_string()),
        };
        let row = self.db.update_agent(id, input).await?;
        Ok(row.map(Self::row_to_agent))
    }

    pub async fn delete(&self, id: Uuid) -> Result<bool> {
        self.db.delete_agent(id).await
    }

    fn row_to_agent(row: everruns_storage::AgentRow) -> Agent {
        Agent {
            id: row.id,
            name: row.name,
            description: row.description,
            system_prompt: row.system_prompt,
            default_model_id: row.default_model_id,
            tags: row.tags,
            status: AgentStatus::from(row.status.as_str()),
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

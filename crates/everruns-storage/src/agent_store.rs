// Database-backed AgentStore implementation
//
// This module implements the core AgentStore trait for retrieving
// agent configurations from the database.

use async_trait::async_trait;
use everruns_core::{
    agent::{Agent, AgentStatus},
    capability_types::CapabilityId,
    traits::AgentStore,
    AgentLoopError, Result,
};
use uuid::Uuid;

use crate::repositories::Database;

// ============================================================================
// DbAgentStore - Retrieves agents from the database
// ============================================================================

/// Database-backed agent store
///
/// Retrieves agent configurations from the database.
/// Used by ReasonAtom to load agent data during workflow execution.
#[derive(Clone)]
pub struct DbAgentStore {
    db: Database,
}

impl DbAgentStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }
}

#[async_trait]
impl AgentStore for DbAgentStore {
    async fn get_agent(&self, agent_id: Uuid) -> Result<Option<Agent>> {
        let agent_row = self
            .db
            .get_agent(agent_id)
            .await
            .map_err(|e| AgentLoopError::store(e.to_string()))?;

        match agent_row {
            Some(row) => {
                // Load capabilities for this agent
                let capability_rows = self
                    .db
                    .get_agent_capabilities(agent_id)
                    .await
                    .map_err(|e| AgentLoopError::store(e.to_string()))?;

                let capabilities: Vec<CapabilityId> = capability_rows
                    .into_iter()
                    .map(|c| CapabilityId::new(c.capability_id))
                    .collect();

                Ok(Some(Agent {
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
                }))
            }
            None => Ok(None),
        }
    }
}

// ============================================================================
// Factory functions
// ============================================================================

/// Create a database-backed agent store
pub fn create_db_agent_store(db: Database) -> DbAgentStore {
    DbAgentStore::new(db)
}

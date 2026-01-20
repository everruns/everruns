// Database-backed AgentStore implementation
//
// This module implements the core AgentStore trait for retrieving
// agent configurations from the database.

use async_trait::async_trait;
use everruns_core::{
    AgentCapabilityConfig, AgentLoopError, DEFAULT_ORG_ID, Result,
    agent::{Agent, AgentStatus},
    traits::AgentStore,
};
use uuid::Uuid;

use super::repositories::Database;

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
        // TODO: Get org_id from context after Phase 3
        let agent_row = self
            .db
            .get_agent(DEFAULT_ORG_ID, agent_id)
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

                let capabilities: Vec<AgentCapabilityConfig> = capability_rows
                    .into_iter()
                    .map(|c| AgentCapabilityConfig::with_config(c.capability_id, c.config))
                    .collect();

                Ok(Some(Agent {
                    id: row.id.into(),
                    name: row.name,
                    description: row.description,
                    system_prompt: row.system_prompt,
                    default_model_id: row.default_model_id.map(|id| id.into()),
                    tags: row.tags,
                    capabilities,
                    status: AgentStatus::from(row.status.as_str()),
                    created_at: row.created_at,
                    updated_at: row.updated_at,
                    usage: None, // Usage not tracked in AgentStore context
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

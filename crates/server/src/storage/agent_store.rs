// Database-backed AgentStore implementation
//
// This module implements the core AgentStore trait for retrieving
// agent configurations from the database.
//
// Decision: org_id is baked into the struct at construction time,
// matching the Grpc/Adapter store pattern. Callers must provide
// the correct org_id when creating the store.

use async_trait::async_trait;
use everruns_core::{
    AgentCapabilityConfig, AgentId, Result, StoreResultExt,
    agent::{Agent, AgentStatus},
    from_json,
    traits::AgentStore,
};

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
    org_id: i64,
}

impl DbAgentStore {
    pub fn new(db: Database, org_id: i64) -> Self {
        Self { db, org_id }
    }
}

#[async_trait]
impl AgentStore for DbAgentStore {
    async fn get_agent(&self, agent_id: AgentId) -> Result<Option<Agent>> {
        let agent_row = self.db.get_agent(self.org_id, agent_id).await.store_err()?;

        match agent_row {
            Some(row) => {
                // Load capabilities for this agent
                let capability_rows = self
                    .db
                    .get_agent_capabilities(agent_id.uuid())
                    .await
                    .store_err()?;

                let capabilities: Vec<AgentCapabilityConfig> = capability_rows
                    .into_iter()
                    .map(|c| AgentCapabilityConfig::with_config(c.capability_id, c.config))
                    .collect();

                Ok(Some(Agent {
                    public_id: row
                        .public_id
                        .parse()
                        .unwrap_or_else(|_| AgentId::from_uuid(row.id.uuid())),
                    internal_id: row.id.uuid(),
                    name: row.name,
                    description: row.description,
                    system_prompt: row.system_prompt,
                    default_model_id: row.default_model_id,
                    tags: row.tags,
                    capabilities,
                    initial_files: from_json(row.initial_files),
                    tools: from_json(row.tools),
                    status: AgentStatus::from(row.status.as_str()),
                    created_at: row.created_at,
                    updated_at: row.updated_at,
                    archived_at: row.archived_at,
                    deleted_at: row.deleted_at,
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

/// Create a database-backed agent store scoped to the given org
pub fn create_db_agent_store(db: Database, org_id: i64) -> DbAgentStore {
    DbAgentStore::new(db, org_id)
}

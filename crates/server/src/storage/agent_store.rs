// Database-backed AgentStore implementation
//
// This module implements the core AgentStore trait — the narrow agent-loading
// seam for turn execution (EVE-872, EVE-877). The stored platform record is
// hydrated here and projected into the portable execution definition;
// archived/deleted agents fail at this seam, before host execution.
//
// Decision: org_id is baked into the struct at construction time,
// matching the Grpc/Adapter store pattern. Callers must provide
// the correct org_id when creating the store.

use crate::max_iterations;
use async_trait::async_trait;
use everruns_core::{
    AgentCapabilityConfig, AgentDefinition, AgentId, DependencyBlocker, Result, StoreResultExt,
    execution_loading::AgentStore, from_json,
};
use everruns_platform::{Agent, AgentStatus};

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

    /// Hydrate the stored platform record (row + capabilities).
    async fn load_record(&self, agent_id: AgentId) -> Result<Option<Agent>> {
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
                    display_name: row.display_name,
                    description: row.description,
                    system_prompt: row.system_prompt,
                    default_model_id: row.default_model_id,
                    harness_id: row.harness_id,
                    default_version_id: row.default_version_id,
                    forked_from_agent_id: row.forked_from_agent_id,
                    forked_from_version_id: row.forked_from_version_id,
                    root_agent_id: row.root_agent_id,
                    tags: row.tags,
                    capabilities,
                    initial_files: from_json(row.initial_files),
                    mcp_servers: from_json(row.mcp_servers),
                    network_access: row
                        .network_access
                        .and_then(|v| serde_json::from_value(v).ok()),
                    max_iterations: max_iterations::from_db(row.max_iterations),
                    parallel_tool_calls: row.parallel_tool_calls,
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

#[async_trait]
impl AgentStore for DbAgentStore {
    async fn get_agent(&self, agent_id: AgentId) -> Result<Option<AgentDefinition>> {
        // Loading seam (EVE-877): archived/deleted records fail here, before
        // host execution.
        self.load_record(agent_id)
            .await?
            .map(|agent| agent.execution_definition())
            .transpose()
    }

    async fn get_agent_blocker(&self, agent_id: AgentId) -> Result<Option<DependencyBlocker>> {
        Ok(match self.load_record(agent_id).await? {
            Some(agent) => agent.dependency_blocker(),
            None => Some(DependencyBlocker::AgentDeleted),
        })
    }
}

// ============================================================================
// Factory functions
// ============================================================================

/// Create a database-backed agent store scoped to the given org
pub fn create_db_agent_store(db: Database, org_id: i64) -> DbAgentStore {
    DbAgentStore::new(db, org_id)
}

// Database-backed HarnessStore implementation
//
// Implements the core HarnessStore trait for retrieving
// harness configurations from the database.

use async_trait::async_trait;
use everruns_core::{
    AgentCapabilityConfig, AgentLoopError, DEFAULT_ORG_ID, HarnessId, Result,
    harness::{Harness, HarnessStatus},
    traits::HarnessStore,
};

use super::repositories::Database;

// ============================================================================
// DbHarnessStore - Retrieves harnesses from the database
// ============================================================================

/// Database-backed harness store
///
/// Retrieves harness configurations from the database.
/// Used by ReasonAtom to load harness data during workflow execution.
#[derive(Clone)]
pub struct DbHarnessStore {
    db: Database,
}

impl DbHarnessStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }
}

#[async_trait]
impl HarnessStore for DbHarnessStore {
    async fn get_harness(&self, harness_id: HarnessId) -> Result<Option<Harness>> {
        let harness_row = self
            .db
            .get_harness(DEFAULT_ORG_ID, harness_id)
            .await
            .map_err(|e| AgentLoopError::store(e.to_string()))?;

        match harness_row {
            Some(row) => {
                let capability_rows = self
                    .db
                    .get_harness_capabilities(harness_id.uuid())
                    .await
                    .map_err(|e| AgentLoopError::store(e.to_string()))?;

                let capabilities: Vec<AgentCapabilityConfig> = capability_rows
                    .into_iter()
                    .map(|c| AgentCapabilityConfig::with_config(c.capability_id, c.config))
                    .collect();

                Ok(Some(Harness {
                    id: row.id,
                    name: row.name,
                    description: row.description,
                    system_prompt: row.system_prompt,
                    default_model_id: row.default_model_id,
                    tags: row.tags,
                    capabilities,
                    status: HarnessStatus::from(row.status.as_str()),
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

/// Create a database-backed harness store
pub fn create_db_harness_store(db: Database) -> DbHarnessStore {
    DbHarnessStore::new(db)
}

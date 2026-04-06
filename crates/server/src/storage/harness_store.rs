// Database-backed HarnessStore implementation
//
// Implements the core HarnessStore trait for retrieving
// harness configurations from the database.
//
// Decision: org_id is baked into the struct at construction time,
// matching the Grpc/Adapter store pattern.

use async_trait::async_trait;
use everruns_core::{
    AgentCapabilityConfig, AgentLoopError, HarnessId, Result, StoreResultExt, from_json,
    harness::{Harness, HarnessStatus},
    merge_harness,
    traits::HarnessStore,
};
use std::collections::HashSet;

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
    org_id: i64,
}

impl DbHarnessStore {
    pub fn new(db: Database, org_id: i64) -> Self {
        Self { db, org_id }
    }
}

#[async_trait]
impl HarnessStore for DbHarnessStore {
    async fn get_harness(&self, harness_id: HarnessId) -> Result<Option<Harness>> {
        let mut visited = HashSet::new();
        let mut chain = Vec::new();
        let mut cursor = Some(harness_id);

        while let Some(current_harness_id) = cursor {
            if !visited.insert(current_harness_id) {
                return Err(AgentLoopError::store(
                    "Harness inheritance cycle detected".to_string(),
                ));
            }

            let Some(row) = self
                .db
                .get_harness(self.org_id, current_harness_id)
                .await
                .store_err()?
            else {
                if chain.is_empty() {
                    return Ok(None);
                }
                return Err(AgentLoopError::store(
                    "Parent harness not found".to_string(),
                ));
            };

            let capability_rows = self
                .db
                .get_harness_capabilities(current_harness_id.uuid())
                .await
                .store_err()?;

            let capabilities: Vec<AgentCapabilityConfig> = capability_rows
                .into_iter()
                .map(|c| AgentCapabilityConfig::with_config(c.capability_id, c.config))
                .collect();

            cursor = row.parent_harness_id;
            chain.push(Harness {
                id: row.id,
                name: row.name,
                display_name: row.display_name,
                description: row.description,
                system_prompt: row.system_prompt,
                parent_harness_id: row.parent_harness_id,
                default_model_id: row.default_model_id,
                tags: row.tags,
                capabilities,
                initial_files: from_json(row.initial_files),
                network_access: row
                    .network_access
                    .and_then(|v| serde_json::from_value(v).ok()),
                is_built_in: row.is_built_in,
                status: HarnessStatus::from(row.status.as_str()),
                created_at: row.created_at,
                updated_at: row.updated_at,
                archived_at: row.archived_at,
                deleted_at: row.deleted_at,
            });
        }

        let Some(mut effective) = chain.pop() else {
            return Ok(None);
        };
        while let Some(layer) = chain.pop() {
            effective = merge_harness(&effective, &layer);
        }

        Ok(Some(effective))
    }
}

// ============================================================================
// Factory functions
// ============================================================================

/// Create a database-backed harness store scoped to the given org
pub fn create_db_harness_store(db: Database, org_id: i64) -> DbHarnessStore {
    DbHarnessStore::new(db, org_id)
}

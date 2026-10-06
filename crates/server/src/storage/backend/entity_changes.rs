use anyhow::Result;

use super::StorageBackend;
use crate::storage::entity_changes::{EntityChangeQuery, EntityChangeRow, NewEntityChange};

impl StorageBackend {
    /// Appends one entity history row; see `crate::storage::entity_changes`.
    pub async fn record_entity_change(&self, change: NewEntityChange) -> Result<EntityChangeRow> {
        dispatch!(self, record_entity_change, change)
    }

    /// Lists entity history rows, newest first.
    pub async fn list_entity_changes(
        &self,
        query: &EntityChangeQuery,
    ) -> Result<Vec<EntityChangeRow>> {
        dispatch!(self, list_entity_changes, query)
    }
}

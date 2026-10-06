//! Entity history in memory, with the PostgreSQL ordering
//! (`storage::repositories::entity_changes`).

use anyhow::Result;

use super::InMemoryDatabase;
use crate::storage::entity_changes::{EntityChangeQuery, EntityChangeRow, NewEntityChange};

impl InMemoryDatabase {
    pub async fn record_entity_change(&self, change: NewEntityChange) -> Result<EntityChangeRow> {
        let row = EntityChangeRow::from_new(uuid::Uuid::now_v7(), chrono::Utc::now(), change);
        self.entity_changes.write().push(row.clone());
        Ok(row)
    }

    pub async fn list_entity_changes(
        &self,
        query: &EntityChangeQuery,
    ) -> Result<Vec<EntityChangeRow>> {
        let rows = self.entity_changes.read();
        // Appended in time order, so newest first is the reverse.
        Ok(rows
            .iter()
            .rev()
            .filter(|row| query.matches(row))
            .take(usize::try_from(query.limit).unwrap_or(0))
            .cloned()
            .collect())
    }
}

//! Entity history in memory, with the PostgreSQL ordering and revision rules
//! (`storage::repositories::entity_changes`).

use anyhow::Result;

use super::InMemoryDatabase;
use crate::storage::entity_changes::{
    EntityChangeQuery, EntityChangeRow, EntityRevisionKey, EntityRevisionRow,
    MAX_SNAPSHOTS_PER_ENTITY, NewEntityChange, StoredEntityChange,
};

fn same_entity(stored: &StoredEntityChange, org_id: i64, kind: &str, entity_ref: &str) -> bool {
    stored.row.org_id == org_id
        && stored.row.entity_kind == kind
        && stored.row.entity_ref == entity_ref
}

impl InMemoryDatabase {
    pub async fn record_entity_change(
        &self,
        mut change: NewEntityChange,
    ) -> Result<EntityChangeRow> {
        let mut rows = self.entity_changes.write();
        let (snapshot, hash) = (change.snapshot.take(), change.snapshot_hash.take());
        let mut row = EntityChangeRow::from_new(uuid::Uuid::now_v7(), chrono::Utc::now(), change);
        let latest = rows
            .iter()
            .rev()
            .filter(|s| same_entity(s, row.org_id, &row.entity_kind, &row.entity_ref))
            .find(|s| s.row.revision.is_some())
            .map(|s| (s.row.revision.unwrap_or(0), s.snapshot_hash.clone()));
        let stored = match (snapshot, hash) {
            (Some(snapshot), Some(hash))
                if latest
                    .as_ref()
                    .is_none_or(|(_, latest)| latest.as_ref() != Some(&hash)) =>
            {
                let revision = latest.map_or(0, |(revision, _)| revision) + 1;
                row.revision = Some(revision);
                for older in rows.iter_mut().filter(|s| {
                    same_entity(s, row.org_id, &row.entity_kind, &row.entity_ref)
                        && s.row
                            .revision
                            .is_some_and(|r| r <= revision - MAX_SNAPSHOTS_PER_ENTITY)
                }) {
                    older.snapshot = None;
                }
                StoredEntityChange {
                    row: row.clone(),
                    snapshot: Some(snapshot),
                    snapshot_hash: Some(hash),
                }
            }
            _ => StoredEntityChange {
                row: row.clone(),
                snapshot: None,
                snapshot_hash: None,
            },
        };
        rows.push(stored);
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
            .map(|stored| &stored.row)
            .filter(|row| query.matches(row))
            .take(usize::try_from(query.limit).unwrap_or(0))
            .cloned()
            .collect())
    }

    pub async fn get_entity_revision(
        &self,
        key: &EntityRevisionKey,
    ) -> Result<Option<EntityRevisionRow>> {
        let rows = self.entity_changes.read();
        Ok(rows
            .iter()
            .rev()
            .filter(|s| same_entity(s, key.org_id, &key.entity_kind, &key.entity_ref))
            .find(|s| match key.revision {
                Some(revision) => s.row.revision == Some(revision),
                None => s.row.revision.is_some(),
            })
            .map(|s| EntityRevisionRow {
                entry: s.row.clone(),
                snapshot: s.snapshot.clone(),
            }))
    }
}

//! Manager context in memory, with the PostgreSQL write semantics
//! (`storage::manager_context::plan_write`).

use anyhow::Result;

use super::InMemoryDatabase;
use crate::storage::manager_context::{
    ManagerContextEdit, ManagerContextKey, ManagerContextRow, ManagerContextWriteError, plan_write,
};

fn matches(row: &ManagerContextRow, key: &ManagerContextKey) -> bool {
    row.org_id == key.org_id
        && row.entity_kind == key.entity_kind
        && row.entity_ref == key.entity_ref
}

impl InMemoryDatabase {
    pub async fn get_manager_context(
        &self,
        key: &ManagerContextKey,
    ) -> Result<Option<ManagerContextRow>> {
        let rows = self.manager_context.read();
        Ok(rows.iter().find(|row| matches(row, key)).cloned())
    }

    pub async fn write_manager_context(
        &self,
        key: &ManagerContextKey,
        edit: &ManagerContextEdit,
        expected_revision: Option<i64>,
        updated_by_user_id: Option<uuid::Uuid>,
    ) -> Result<ManagerContextRow, ManagerContextWriteError> {
        let mut rows = self.manager_context.write();
        let index = rows.iter().position(|row| matches(row, key));
        let (content, revision) = plan_write(index.map(|i| &rows[i]), edit, expected_revision)?;
        let row = ManagerContextRow {
            org_id: key.org_id,
            entity_kind: key.entity_kind.clone(),
            entity_ref: key.entity_ref.clone(),
            content,
            revision,
            updated_by_user_id,
            updated_at: chrono::Utc::now(),
        };
        match index {
            Some(i) => rows[i] = row.clone(),
            None => rows.push(row.clone()),
        }
        Ok(row)
    }

    pub async fn delete_manager_context(&self, key: &ManagerContextKey) -> Result<()> {
        self.manager_context
            .write()
            .retain(|row| !matches(row, key));
        Ok(())
    }
}

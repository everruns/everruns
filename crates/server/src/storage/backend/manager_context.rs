use anyhow::Result;

use super::StorageBackend;
use crate::storage::manager_context::{
    ManagerContextEdit, ManagerContextKey, ManagerContextRow, ManagerContextWriteError,
};

impl StorageBackend {
    /// The manager context of one entity, if any was ever written.
    pub async fn get_manager_context(
        &self,
        key: &ManagerContextKey,
    ) -> Result<Option<ManagerContextRow>> {
        dispatch!(self, get_manager_context, key)
    }

    /// Applies `edit`, refusing it when `expected_revision` is stale; see
    /// `crate::storage::manager_context::plan_write`.
    pub async fn write_manager_context(
        &self,
        key: &ManagerContextKey,
        edit: &ManagerContextEdit,
        expected_revision: Option<i64>,
        updated_by_user_id: Option<uuid::Uuid>,
    ) -> std::result::Result<ManagerContextRow, ManagerContextWriteError> {
        dispatch!(
            self,
            write_manager_context,
            key,
            edit,
            expected_revision,
            updated_by_user_id
        )
    }

    /// Removes an entity's manager context (its entity was deleted).
    pub async fn delete_manager_context(&self, key: &ManagerContextKey) -> Result<()> {
        dispatch!(self, delete_manager_context, key)
    }
}

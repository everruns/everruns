use anyhow::Result;

use super::StorageBackend;
use crate::storage::manager_context::{ManagerContextKey, ManagerContextRow};

impl StorageBackend {
    /// As `get_manager_context`, locking the row (`FOR SHARE`) until the
    /// current transaction ends, so a change held to the revision read here
    /// commits before any context write that would move it.
    pub async fn get_manager_context_for_share(
        &self,
        key: &ManagerContextKey,
    ) -> Result<Option<ManagerContextRow>> {
        self.db.get_manager_context_locked(key, true).await
    }
}

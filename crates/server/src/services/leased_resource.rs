// Leased resource service.
//
// Provides read-side access to leased resources for session APIs and admin
// surfaces. The same generic leased-resource model can be reused by any
// capability or subsystem that creates provider-owned state requiring cleanup.

use anyhow::Result;
use everruns_core::{LeasedResource, SessionId};
use std::sync::Arc;

use crate::storage::backend::StorageBackend;
use crate::storage::leased_resource_row_to_domain;

/// Read-side service for leased resources.
pub struct LeasedResourceService {
    db: Arc<StorageBackend>,
}

impl LeasedResourceService {
    pub fn new(db: Arc<StorageBackend>) -> Self {
        Self { db }
    }

    /// List all leased resources for a session after validating org ownership.
    pub async fn list_for_session(
        &self,
        org_id: i64,
        session_id: SessionId,
    ) -> Result<Option<Vec<LeasedResource>>> {
        let Some(_session) = self.db.get_session(org_id, session_id).await? else {
            return Ok(None);
        };

        let rows = self.db.list_session_leased_resources(session_id).await?;
        rows.iter()
            .map(leased_resource_row_to_domain)
            .collect::<everruns_core::Result<Vec<_>>>()
            .map_err(Into::into)
            .map(Some)
    }
}

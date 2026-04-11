// Session resource service.
//
// Reads from the session resource registry for the API.

use anyhow::Result;
use everruns_core::{SessionId, SessionResourceEntry};
use std::sync::Arc;

use crate::storage::backend::StorageBackend;
use crate::storage::session_resource_store::DbSessionResourceRegistry;

/// Read-side service for session resources.
pub struct SessionResourceService {
    registry: DbSessionResourceRegistry,
    db: Arc<StorageBackend>,
}

impl SessionResourceService {
    pub fn new(db: Arc<StorageBackend>) -> Self {
        Self {
            registry: DbSessionResourceRegistry::new(db.clone()),
            db,
        }
    }

    /// List all resources for a session after validating org ownership.
    pub async fn list_for_session(
        &self,
        org_id: i64,
        session_id: SessionId,
    ) -> Result<Option<Vec<SessionResourceEntry>>> {
        let Some(_session) = self.db.get_session(org_id, session_id).await? else {
            return Ok(None);
        };

        use everruns_core::traits::SessionResourceRegistry;
        let entries = self.registry.list(session_id, None).await?;
        Ok(Some(entries))
    }
}

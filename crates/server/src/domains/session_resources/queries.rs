use crate::domains::common::CommandError;
use crate::storage::StorageBackend;
use crate::storage::session_resource_store::DbSessionResourceRegistry;
use everruns_core::session_services::SessionResourceRegistry;
use everruns_core::{SessionId, SessionResourceEntry};
use std::sync::Arc;

pub fn parse_session_id(input: &str) -> Result<SessionId, CommandError> {
    input
        .parse()
        .map_err(|e| CommandError::bad_request(format!("Invalid session ID: {e}")))
}

/// List all resources registered in a session, after validating org
/// ownership. Returns `None` if the session does not exist in `org_id`.
pub async fn list_for_session(
    db: &Arc<StorageBackend>,
    org_id: i64,
    session_id: SessionId,
) -> anyhow::Result<Option<Vec<SessionResourceEntry>>> {
    if db.get_session(org_id, session_id).await?.is_none() {
        return Ok(None);
    }
    let registry = DbSessionResourceRegistry::new(db.clone());
    let entries = registry.list(session_id, None).await?;
    Ok(Some(entries))
}

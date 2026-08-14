use crate::domains::common::CommandError;
use crate::domains::session_resources::queries::parse_session_id;
use crate::storage::StorageBackend;
use everruns_provider::typed_id::SessionId;
use std::sync::Arc;

pub async fn verify_session_ownership(
    db: &Arc<StorageBackend>,
    org_id: i64,
    session_id: SessionId,
) -> Result<(), CommandError> {
    db.get_session(org_id, session_id)
        .await
        .map_err(CommandError::internal)?
        .ok_or_else(|| CommandError::not_found("Session"))?;
    Ok(())
}

pub fn parse_owned_session_id(input: &str) -> Result<SessionId, CommandError> {
    parse_session_id(input)
}

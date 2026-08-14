use crate::domains::common::CommandError;
use crate::domains::session_resources::queries::parse_session_id;
use everruns_provider::typed_id::SessionId;
use std::sync::Arc;

pub fn parse_owned_session_id(input: &str) -> Result<SessionId, CommandError> {
    parse_session_id(input)
}

pub async fn verify_session_ownership(
    db: &Arc<crate::storage::StorageBackend>,
    org_id: i64,
    session_id: SessionId,
) -> Result<(), CommandError> {
    crate::domains::session_storage::queries::verify_session_ownership(db, org_id, session_id).await
}

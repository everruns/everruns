use crate::domains::common::{CommandError, Ctx};
use crate::storage::StorageBackend;
use crate::storage::session_task_store::DbSessionTaskRegistry;
use everruns_core::session_task::SessionTaskRegistry;
use everruns_core::{SessionId, SessionTask};
use std::sync::Arc;

pub fn parse_session_id(input: &str) -> Result<SessionId, CommandError> {
    input
        .parse()
        .map_err(|e| CommandError::bad_request(format!("Invalid session ID: {e}")))
}

/// Build the registry, attaching the ctx event service when present so API
/// mutations emit task.* events for live UIs.
pub fn registry_for_ctx(ctx: &Ctx) -> DbSessionTaskRegistry {
    let mut registry = DbSessionTaskRegistry::new(ctx.db.clone());
    if let Some(event_service) = &ctx.event_service {
        registry = registry.with_event_emitter(event_service.clone());
    }
    registry
}

/// Validate org ownership of the session. Returns false when the session
/// does not exist in `org_id`.
pub async fn session_in_org(
    db: &Arc<StorageBackend>,
    org_id: i64,
    session_id: SessionId,
) -> anyhow::Result<bool> {
    Ok(db.get_session(org_id, session_id).await?.is_some())
}

/// Look up one task after validating org ownership.
pub async fn get_task_in_org(
    ctx: &Ctx,
    org_id: i64,
    session_id: SessionId,
    task_id: &str,
) -> Result<Option<SessionTask>, CommandError> {
    if !session_in_org(&ctx.db, org_id, session_id)
        .await
        .map_err(crate::domains::common::classify_anyhow)?
    {
        return Ok(None);
    }
    registry_for_ctx(ctx)
        .get(session_id, task_id)
        .await
        .map_err(|e| CommandError::internal(anyhow::anyhow!(e)))
}

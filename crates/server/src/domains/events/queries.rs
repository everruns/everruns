use crate::domains::common::{CommandError, Ctx};
use crate::services::{EventService, SessionService};
use everruns_core::typed_id::SessionId;
use std::sync::Arc;

pub fn parse_session_id(input: &str) -> Result<SessionId, CommandError> {
    input
        .parse()
        .map_err(|e| CommandError::bad_request(format!("Invalid session ID: {e}")))
}

pub fn session_service(ctx: &Ctx) -> Result<&Arc<SessionService>, CommandError> {
    ctx.session_service
        .as_ref()
        .ok_or_else(|| CommandError::Internal(anyhow::anyhow!("Session service not configured")))
}

pub fn event_service(ctx: &Ctx) -> Result<&Arc<EventService>, CommandError> {
    ctx.event_service
        .as_ref()
        .ok_or_else(|| CommandError::Internal(anyhow::anyhow!("Event service not configured")))
}

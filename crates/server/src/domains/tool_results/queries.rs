use crate::domains::common::{CommandError, Ctx};
use everruns_core::host::TurnBackend;
use std::sync::Arc;

pub use crate::domains::sessions::queries::parse_session_id;

pub fn session_service(
    ctx: &Ctx,
) -> Result<&Arc<crate::domains::sessions::SessionService>, CommandError> {
    ctx.session_service
        .as_ref()
        .ok_or_else(|| CommandError::internal(anyhow::anyhow!("Session service not configured")))
}

pub fn event_service(ctx: &Ctx) -> Result<&Arc<crate::services::EventService>, CommandError> {
    ctx.event_service
        .as_ref()
        .ok_or_else(|| CommandError::internal(anyhow::anyhow!("Event service not configured")))
}

pub fn runner(ctx: &Ctx) -> Result<Arc<dyn TurnBackend>, CommandError> {
    ctx.runner
        .clone()
        .ok_or_else(|| CommandError::internal(anyhow::anyhow!("Agent runner not configured")))
}

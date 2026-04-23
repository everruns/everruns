use crate::domains::common::{CommandError, Ctx};
use crate::domains::messages::MessageService;
use crate::domains::sessions::SessionService;
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

pub fn message_service(ctx: &Ctx) -> Result<&Arc<MessageService>, CommandError> {
    ctx.message_service
        .as_ref()
        .ok_or_else(|| CommandError::Internal(anyhow::anyhow!("Message service not configured")))
}

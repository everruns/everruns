use crate::domains::common::{CommandError, Ctx};
use crate::domains::session_sandbox::SessionSandboxService;
use crate::domains::sessions::queries::get_session;
use everruns_core::ToolExecutionResult;
use everruns_provider::typed_id::SessionId;
use std::sync::Arc;

pub fn session_sandbox_service(ctx: &Ctx) -> Result<&Arc<SessionSandboxService>, CommandError> {
    ctx.session_sandbox_service.as_ref().ok_or_else(|| {
        CommandError::internal(anyhow::anyhow!("Session sandbox service not configured"))
    })
}

pub async fn verify_session_access(ctx: &Ctx, session_id: SessionId) -> Result<(), CommandError> {
    let _ = get_session(ctx, session_id, ctx.caller.user_id).await?;
    Ok(())
}

pub fn map_tool_error(err: ToolExecutionResult) -> CommandError {
    match err {
        ToolExecutionResult::ToolError(message) => CommandError::bad_request(message),
        ToolExecutionResult::InternalError(error) => {
            CommandError::internal(anyhow::anyhow!(error.message))
        }
        ToolExecutionResult::ConnectionRequired { provider } => {
            CommandError::unprocessable(format!("Connection required: {provider}"))
        }
        ToolExecutionResult::Success(_) | ToolExecutionResult::SuccessWithImages { .. } => {
            unreachable!("tool error mapper called with success result")
        }
    }
}

pub use crate::domains::sessions::queries::parse_session_id;

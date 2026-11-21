//! The permanent conversation is idempotent per organization and console user.
use super::{commands::CreateSession, platform_chat_starter as starter, queries as q};
use crate::domains::common::*;
use crate::domains::sessions::record::Session;
use serde::Deserialize;
use utoipa::ToSchema;
use uuid::Uuid;

async fn live_starter(ctx: &Ctx, user: Uuid) -> Result<Option<Session>, CommandError> {
    let Some(id) = ctx
        .db
        .get_platform_chat_starter_id(ctx.org_id(), user)
        .await?
    else {
        return Ok(None);
    };
    // A data-plane rollback can soft-delete the record between the two reads.
    // Look through that race instead of turning it into a permanent 404.
    Ok(q::session_service(ctx)?
        .get(&ctx.caller, id.uuid(), None)
        .await?)
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct EnsurePlatformChat {}
#[command(
    name = "ensure_platform_chat",
    category = "sessions",
    description = "Open the current user's permanent platform conversation",
    method = "POST",
    cli = CliRoute::new(&["sessions", "platform-chat"], "ensure").with_examples(&[CliExample::new("Open your permanent platform conversation, creating it on first use", "everruns sessions platform-chat ensure --reason 'Open the platform chat'")]),
    path = "/v1/sessions/platform-chat",
    policy = super::SESSION_MANAGE,
)]
impl Command for EnsurePlatformChat {
    type Output = Session;
    async fn execute(self, ctx: &Ctx) -> Result<Session, CommandError> {
        let user = ctx
            .caller
            .user_id
            .ok_or_else(|| CommandError::forbidden("Chat requires a console user"))?;
        if let Some(session) = live_starter(ctx, user).await? {
            return Ok(session);
        }
        let req = serde_json::from_value(serde_json::json!({"source":"chat", "agent_name":crate::platform_chat_agent::NAME, "title":"Chat", "tags":["chat", starter::PLATFORM_CHAT_STARTER_TAG]})).map_err(|e| CommandError::internal(e.into()))?;
        match CreateSession(req).run(ctx).await {
            Ok(session) => Ok(session),
            Err(error) if error.code.as_deref() == Some(crate::errors::ALREADY_EXISTS_CODE) => {
                live_starter(ctx, user).await?.ok_or(error)
            }
            Err(error) => Err(error),
        }
    }
}

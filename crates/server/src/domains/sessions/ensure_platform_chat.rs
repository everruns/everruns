//! The permanent conversation is idempotent per organization and console user.
use super::{commands::CreateSession, platform_chat_starter as starter, queries as q};
use crate::domains::common::*;
use crate::domains::sessions::record::Session;
use serde::Deserialize;
use utoipa::ToSchema;

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct EnsurePlatformChat {}
#[command(
    name = "ensure_platform_chat",
    category = "sessions",
    description = "Open the current user's permanent platform conversation",
    method = "POST",
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
        if let Some(id) = ctx
            .db
            .get_platform_chat_starter_id(ctx.org_id(), user)
            .await?
        {
            return q::get_session(ctx, id, None).await;
        }
        let req = serde_json::from_value(serde_json::json!({"source":"chat", "agent_name":crate::platform_chat_agent::NAME, "title":"Chat", "tags":["chat", starter::PLATFORM_CHAT_STARTER_TAG]})).map_err(|e| CommandError::internal(e.into()))?;
        match CreateSession(req).run(ctx).await {
            Ok(session) => Ok(session),
            Err(error) => {
                if let Some(id) = ctx
                    .db
                    .get_platform_chat_starter_id(ctx.org_id(), user)
                    .await?
                {
                    q::get_session(ctx, id, None).await
                } else {
                    Err(error)
                }
            }
        }
    }
}

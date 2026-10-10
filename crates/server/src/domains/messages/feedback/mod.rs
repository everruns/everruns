// Message feedback: a person's Good / Bad rating of a message in a session.
//
// Spec: knowledge/ui/chat-experience.md ("Message feedback").
//
// Decisions:
// - One rating per person and message. Rating again replaces it; a null
//   rating clears it, so the buttons toggle.
// - Rating takes SESSION_MANAGE, the same as sending a message: it is a
//   person's own action in the conversation. Reading your ratings takes
//   SESSION_VIEW. Only your own ratings are returned; Session Trace reads all
//   of them later.
// - The message must be a user or agent message of that session, so a rating
//   never points at nothing.

use super::queries as q;
use crate::domains::common::*;
use crate::storage::MessageFeedbackRow;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Longest comment kept with a rating.
const MAX_COMMENT_CHARS: usize = 4000;

/// A person's verdict on one message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum MessageRating {
    Good,
    Bad,
}

impl MessageRating {
    fn as_str(self) -> &'static str {
        match self {
            Self::Good => "good",
            Self::Bad => "bad",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "good" => Some(Self::Good),
            "bad" => Some(Self::Bad),
            _ => None,
        }
    }
}

/// The caller's rating of one message.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct MessageFeedback {
    /// Message the rating is for (`msg_...`).
    #[schema(example = "msg_01933b5a00007000800000000000001")]
    pub message_id: String,
    /// `good`, `bad`, or null when the rating was cleared.
    pub rating: Option<MessageRating>,
    /// Optional note kept with the rating.
    pub comment: Option<String>,
    /// When the rating was last set or cleared.
    pub updated_at: DateTime<Utc>,
}

impl From<MessageFeedbackRow> for MessageFeedback {
    fn from(row: MessageFeedbackRow) -> Self {
        Self {
            message_id: row.message_id,
            rating: MessageRating::parse(&row.rating),
            comment: row.comment,
            updated_at: row.updated_at,
        }
    }
}

fn require_user_id(ctx: &Ctx) -> Result<uuid::Uuid, CommandError> {
    ctx.caller
        .user_id
        .ok_or_else(|| CommandError::forbidden("Message feedback requires a signed-in user"))
}

/// The session, when the caller may see it.
async fn visible_session(ctx: &Ctx, session_id: &str) -> Result<uuid::Uuid, CommandError> {
    let session_id = q::parse_session_id(session_id)?;
    let session = q::session_service(ctx)?
        .get(&ctx.caller, session_id.uuid(), None)
        .await?
        .ok_or_else(|| CommandError::not_found("Session"))?;
    if !crate::domains::sessions::platform_chat_owner_matches_session(
        &ctx.db,
        &ctx.caller,
        &session,
    )
    .await?
    {
        return Err(CommandError::not_found("Session"));
    }
    Ok(session_id.uuid())
}

// ============================================================================
// SetMessageFeedback
// ============================================================================

/// Rate one message Good or Bad, or clear your rating.
#[derive(Debug, Deserialize, ToSchema, Serialize)]
pub struct SetMessageFeedback {
    /// Session's prefixed public identifier.
    pub session_id: String,
    /// Message to rate (`msg_...`), a user or agent message of the session.
    pub message_id: String,
    /// `good` or `bad`. Null clears your rating.
    #[serde(default)]
    pub rating: Option<MessageRating>,
    /// Optional note on what was good or bad.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
}

#[command(
    name = "set_message_feedback",
    category = "messages",
    description = "Rate one message in a session good or bad, with an optional comment. A null rating clears your rating.",
    method = "PUT",
    path = "/v1/sessions/{session_id}/messages/{message_id}/feedback",
    policy = crate::domains::sessions::SESSION_MANAGE,
    positional = "session_id",
    http = plain,
    responses((status = 404, description = "Session or message not found")),
    cli = CliRoute::new(&["sessions", "messages"], "rate").with_args(&[CliArg::new("session_id").at(1).long("session"), CliArg::new("message_id").at(2)]).with_examples(&[CliExample::new("Mark an agent reply as a bad answer", "everruns sessions messages rate session_01h9 msg_01h9 --rating bad --comment 'Ignored the failing test' --reason 'Flag a wrong answer'",)]),
)]
impl Command for SetMessageFeedback {
    type Output = MessageFeedback;

    async fn execute(self, ctx: &Ctx) -> Result<MessageFeedback, CommandError> {
        let user_id = require_user_id(ctx)?;
        let session_id = visible_session(ctx, &self.session_id).await?;
        if !ctx
            .db
            .session_has_message(session_id, &self.message_id)
            .await?
        {
            return Err(CommandError::not_found("Message"));
        }
        let comment = self
            .comment
            .map(|comment| comment.trim().to_string())
            .filter(|comment| !comment.is_empty());
        if comment
            .as_ref()
            .is_some_and(|comment| comment.chars().count() > MAX_COMMENT_CHARS)
        {
            return Err(CommandError::bad_request(format!(
                "Comment is longer than {MAX_COMMENT_CHARS} characters"
            )));
        }
        let Some(rating) = self.rating else {
            ctx.db
                .delete_message_feedback(ctx.org_id(), session_id, &self.message_id, user_id)
                .await?;
            return Ok(MessageFeedback {
                message_id: self.message_id,
                rating: None,
                comment: None,
                updated_at: Utc::now(),
            });
        };
        let row = ctx
            .db
            .upsert_message_feedback(
                ctx.org_id(),
                session_id,
                &self.message_id,
                user_id,
                rating.as_str(),
                comment.as_deref(),
            )
            .await?;
        Ok(row.into())
    }
}

// ============================================================================
// ListMessageFeedback
// ============================================================================

/// List your ratings of the messages in a session.
#[derive(Debug, Deserialize, ToSchema, Serialize)]
pub struct ListMessageFeedback {
    /// Session's prefixed public identifier.
    pub session_id: String,
}

#[command(
    name = "list_message_feedback",
    category = "messages",
    description = "List your good and bad ratings of the messages in a session.",
    method = "GET",
    path = "/v1/sessions/{session_id}/feedback",
    policy = crate::domains::sessions::SESSION_VIEW,
    positional = "session_id",
    http = plain,
    responses((status = 404, description = "Session not found")),
    cli = CliRoute::new(&["sessions", "messages"], "ratings").with_args(&[CliArg::new("session_id").at(1).long("session")]).with_examples(&[CliExample::new("See which replies you rated in a session", "everruns sessions messages ratings session_01h9",)]),
)]
impl Command for ListMessageFeedback {
    type Output = Vec<MessageFeedback>;

    async fn execute(self, ctx: &Ctx) -> Result<Vec<MessageFeedback>, CommandError> {
        let user_id = require_user_id(ctx)?;
        let session_id = visible_session(ctx, &self.session_id).await?;
        Ok(ctx
            .db
            .list_message_feedback(ctx.org_id(), session_id, user_id)
            .await?
            .into_iter()
            .map(MessageFeedback::from)
            .collect())
    }
}

#[cfg(test)]
mod tests;

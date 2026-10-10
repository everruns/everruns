// PostgreSQL repository: a person's Good / Bad rating of a session message.
//
// Spec: knowledge/ui/chat-experience.md ("Message feedback").

pub(super) mod rows;
use rows::*;

use super::Database;
use anyhow::Result;
use everruns_server_macros::sql;
use uuid::Uuid;

impl Database {
    /// Whether a user or agent message with this public id is in the session.
    pub async fn session_has_message(&self, session_id: Uuid, message_id: &str) -> Result<bool> {
        Ok(sqlx::query_scalar::<_, bool>(
            r#"
            SELECT EXISTS (
                SELECT 1 FROM events
                WHERE session_id = $1
                  AND event_type IN ('input.message', 'output.message.completed')
                  AND data->'message'->>'id' = $2
            )
            "#,
        )
        .bind(session_id)
        .bind(message_id)
        .fetch_one(&self.pool)
        .await?)
    }

    /// Save one person's rating of a message, replacing an earlier one.
    pub async fn upsert_message_feedback(
        &self,
        org_id: i64,
        session_id: Uuid,
        message_id: &str,
        user_id: Uuid,
        rating: &str,
        comment: Option<&str>,
    ) -> Result<MessageFeedbackRow> {
        Ok(sqlx::query_as::<_, MessageFeedbackRow>(sql!(
            r#"
            INSERT INTO message_feedback (org_id, session_id, message_id, user_id, rating, comment)
            VALUES ($1, $2, $3, $4, $5, $6)
            ON CONFLICT (session_id, message_id, user_id) DO UPDATE SET
                rating = EXCLUDED.rating,
                comment = EXCLUDED.comment
            RETURNING {MessageFeedbackRow}
            "#
        ))
        .bind(org_id)
        .bind(session_id)
        .bind(message_id)
        .bind(user_id)
        .bind(rating)
        .bind(comment)
        .fetch_one(&self.pool)
        .await?)
    }

    /// Clear one person's rating of a message.
    pub async fn delete_message_feedback(
        &self,
        org_id: i64,
        session_id: Uuid,
        message_id: &str,
        user_id: Uuid,
    ) -> Result<()> {
        sqlx::query(
            r#"
            DELETE FROM message_feedback
            WHERE org_id = $1 AND session_id = $2 AND message_id = $3 AND user_id = $4
            "#,
        )
        .bind(org_id)
        .bind(session_id)
        .bind(message_id)
        .bind(user_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// One person's ratings in a session, oldest first.
    pub async fn list_message_feedback(
        &self,
        org_id: i64,
        session_id: Uuid,
        user_id: Uuid,
    ) -> Result<Vec<MessageFeedbackRow>> {
        Ok(sqlx::query_as::<_, MessageFeedbackRow>(sql!(
            r#"
            SELECT {MessageFeedbackRow} FROM message_feedback
            WHERE org_id = $1 AND session_id = $2 AND user_id = $3
            ORDER BY created_at
            "#
        ))
        .bind(org_id)
        .bind(session_id)
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?)
    }
}

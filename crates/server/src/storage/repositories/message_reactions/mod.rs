// PostgreSQL reads behind message reactions.
//
// Reactions live in the session's event log (`message.reaction.added` /
// `.removed`); see knowledge/execution/message-reactions.md. These queries
// read them back and find which ones the agent was already told.

use super::Database;
use anyhow::Result;
use chrono::{DateTime, Utc};
use everruns_core::{MESSAGE_REACTION_ADDED, MESSAGE_REACTION_REMOVED};
use sqlx::FromRow;
use uuid::Uuid;

/// Most reaction events read for one session. A session with more is far
/// past any real conversation; the newest are the ones that matter.
const MAX_REACTION_EVENTS: i64 = 10_000;

/// One stored reaction event.
#[derive(Debug, Clone, FromRow)]
pub struct ReactionEventRow {
    pub sequence: i64,
    pub event_type: String,
    pub data: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

/// A user or agent message, as much as a reaction needs of it.
#[derive(Debug, Clone, FromRow)]
pub struct ReactedMessageRow {
    pub message_id: String,
    /// `input.message` (a user-role message) or `output.message.completed`
    /// (the agent's reply).
    pub event_type: String,
    pub message: serde_json::Value,
}

impl Database {
    /// Every reaction event of a session, oldest first.
    pub async fn list_reaction_events(&self, session_id: Uuid) -> Result<Vec<ReactionEventRow>> {
        let mut rows = sqlx::query_as::<_, ReactionEventRow>(
            r#"
            SELECT sequence::bigint AS sequence, event_type, data, created_at
            FROM events
            WHERE session_id = $1 AND event_type IN ($2, $3)
            ORDER BY sequence DESC
            LIMIT $4
            "#,
        )
        .bind(session_id)
        .bind(MESSAGE_REACTION_ADDED)
        .bind(MESSAGE_REACTION_REMOVED)
        .bind(MAX_REACTION_EVENTS)
        .fetch_all(&self.pool)
        .await?;
        rows.reverse();
        Ok(rows)
    }

    /// Last reaction event the agent was already told about: the newest
    /// `through_sequence` an input message's reaction note carries.
    pub async fn reactions_told_through(&self, session_id: Uuid) -> Result<Option<i64>> {
        Ok(sqlx::query_scalar::<_, Option<i64>>(
            r#"
            SELECT (data->'message'->'metadata'->'everruns_reactions'->>'through_sequence')::bigint
            FROM events
            WHERE session_id = $1
              AND event_type = 'input.message'
              AND data->'message'->'metadata' ? 'everruns_reactions'
            ORDER BY sequence DESC
            LIMIT 1
            "#,
        )
        .bind(session_id)
        .fetch_optional(&self.pool)
        .await?
        .flatten())
    }

    /// The user and agent messages of a session with these public ids.
    pub async fn reacted_messages(
        &self,
        session_id: Uuid,
        message_ids: &[String],
    ) -> Result<Vec<ReactedMessageRow>> {
        if message_ids.is_empty() {
            return Ok(Vec::new());
        }
        Ok(sqlx::query_as::<_, ReactedMessageRow>(
            r#"
            SELECT data->'message'->>'id' AS message_id, event_type, data->'message' AS message
            FROM events
            WHERE session_id = $1
              AND event_type IN ('input.message', 'output.message.completed')
              AND data->'message'->>'id' = ANY($2)
            "#,
        )
        .bind(session_id)
        .bind(message_ids)
        .fetch_all(&self.pool)
        .await?)
    }
}

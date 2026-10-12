// PostgreSQL repository: input-message lookups on the events table, used to
// trace a turn back to the input (and signed ingress) that started it.

use super::rows::EventRow;
use crate::storage::repositories::Database;
use anyhow::Result;
use everruns_contracts::typed_id::{MessageId, SessionId};
use everruns_server_macros::sql;

impl Database {
    /// Find the input event a client sent with `client_message_id`, so a retried
    /// send returns the stored message instead of starting a second turn.
    ///
    /// Decision: no dedicated table. The key rides in reserved message metadata
    /// (the Slack `slack_ts` dedup uses the same `data @>` shape), and the scan
    /// walks the session's input-message index newest first, bounded to the
    /// recent messages a retry can plausibly target.
    pub async fn find_input_message_by_client_id(
        &self,
        session_id: SessionId,
        client_message_id: &str,
    ) -> Result<Option<EventRow>> {
        let key = everruns_core::message::CLIENT_MESSAGE_ID_METADATA_KEY;
        let pattern = serde_json::json!({
            "message": { "metadata": { (key): client_message_id } }
        });
        Ok(sqlx::query_as::<_, EventRow>(sql!(
            r#"
            SELECT * FROM (
                SELECT {EventRow}
                FROM events
                WHERE session_id = $1
                  AND event_type = 'input.message'
                ORDER BY sequence DESC
                LIMIT 200
            ) recent
            WHERE data @> $2
            LIMIT 1
            "#
        ))
        .bind(session_id.uuid())
        .bind(&pattern)
        .fetch_optional(&self.pool)
        .await?)
    }

    /// Find the input event that contains an exact, session-scoped message ID.
    pub async fn find_input_message_event(
        &self,
        session_id: SessionId,
        message_id: MessageId,
    ) -> Result<Option<EventRow>> {
        Ok(sqlx::query_as::<_, EventRow>(sql!(
            r#"
            SELECT {EventRow}
            FROM events
            WHERE session_id = $1
              AND event_type = 'input.message'
              AND data->'message'->>'id' = $2
            LIMIT 1
            "#
        ))
        .bind(session_id.uuid())
        .bind(message_id.to_string())
        .fetch_optional(&self.pool)
        .await?)
    }

    /// Newest input message at or after `sequence`: the last message that
    /// steered the turn which started at `sequence`, or that input itself.
    pub async fn latest_input_message_event_since(
        &self,
        session_id: SessionId,
        sequence: i32,
    ) -> Result<Option<EventRow>> {
        Ok(sqlx::query_as::<_, EventRow>(sql!(
            r#"
            SELECT {EventRow}
            FROM events
            WHERE session_id = $1
              AND event_type = 'input.message'
              AND sequence >= $2
            ORDER BY sequence DESC
            LIMIT 1
            "#
        ))
        .bind(session_id.uuid())
        .bind(sequence)
        .fetch_optional(&self.pool)
        .await?)
    }

    /// Newest recent input messages whose message metadata names channel
    /// `endpoint_public_id`, newest first. Candidates only: message metadata is
    /// caller-controlled, so the caller still checks the server-authored
    /// provenance of each row before trusting it.
    pub async fn recent_input_message_events_for_channel(
        &self,
        session_id: SessionId,
        endpoint_public_id: &str,
        limit: i64,
    ) -> Result<Vec<EventRow>> {
        let pattern = serde_json::json!({
            "message": { "metadata": { "_app_channel_id": endpoint_public_id } }
        });
        Ok(sqlx::query_as::<_, EventRow>(sql!(
            r#"
            SELECT * FROM (
                SELECT {EventRow}
                FROM events
                WHERE session_id = $1
                  AND event_type = 'input.message'
                ORDER BY sequence DESC
                LIMIT 200
            ) recent
            WHERE data @> $2
            ORDER BY sequence DESC
            LIMIT $3
            "#
        ))
        .bind(session_id.uuid())
        .bind(&pattern)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?)
    }
}

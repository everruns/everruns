// PostgreSQL implementation of PartialStreamStore (EVE-532).

use async_trait::async_trait;
use everruns_core::error::AgentLoopError;
use everruns_core::typed_id::{MessageId, SessionId};
use everruns_core::{durability::PartialStreamState, durability::PartialStreamStore};
use sqlx::PgPool;

/// PostgreSQL-backed partial-stream store.
///
/// Detects whether the *latest* `output.message.started` for a turn has a
/// matching `output.message.completed/replaced` with a higher sequence number.
/// If not, returns the accumulated text from the most recent delta after that
/// start point.
///
/// Sequence-based detection is necessary for turns with multiple reasoning
/// iterations: a completed event from an earlier iteration must not mask an
/// in-flight partial stream from the current one.
#[derive(Clone)]
pub struct PgPartialStreamStore {
    pool: PgPool,
}

impl PgPartialStreamStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl PartialStreamStore for PgPartialStreamStore {
    async fn get_partial_stream(
        &self,
        session_id: SessionId,
        turn_id: &str,
    ) -> Result<Option<PartialStreamState>, AgentLoopError> {
        // Find the latest output.message.started and carry its message id into
        // recovery so the reconstructed lifecycle stays in one id scope.
        let started: Option<(i32, Option<String>)> = sqlx::query_as(
            r#"
            SELECT sequence, data->>'message_id'
            FROM events
            WHERE session_id = $1
              AND context->>'turn_id' = $2
              AND event_type = 'output.message.started'
            ORDER BY sequence DESC
            LIMIT 1
            "#,
        )
        .bind(session_id.uuid())
        .bind(turn_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AgentLoopError::tool(format!("partial_stream started check: {e}")))?;

        let Some((started_seq, raw_message_id)) = started else {
            return Ok(None);
        };
        // A worker upgraded while an old-format stream is in flight may find a
        // started event without message_id. Allocate an id for the recovery
        // lifecycle in that one legacy case; all new emissions persist the id.
        let legacy_stream = raw_message_id.is_none();
        let message_id = match raw_message_id {
            Some(raw) => MessageId::parse(&raw).map_err(|e| {
                AgentLoopError::tool(format!("partial_stream invalid message_id: {e}"))
            })?,
            None => MessageId::new(),
        };

        // Check if a completion/replacement with sequence > started_seq exists.
        // Using sequence ordering avoids false positives from earlier iterations.
        let completed = sqlx::query_scalar::<_, bool>(
            r#"
            SELECT EXISTS (
                SELECT 1 FROM events
                WHERE session_id = $1
                  AND context->>'turn_id' = $2
                  AND sequence > $3
                  AND event_type IN ('output.message.completed', 'output.message.replaced')
                  AND (
                    $5
                    OR (event_type = 'output.message.completed'
                        AND data->'message'->>'id' = $4)
                    OR (event_type = 'output.message.replaced'
                        AND data->>'message_id' = $4)
                  )
            )
            "#,
        )
        .bind(session_id.uuid())
        .bind(turn_id)
        .bind(started_seq)
        .bind(message_id.to_string())
        .bind(legacy_stream)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AgentLoopError::tool(format!("partial_stream completed check: {e}")))?;

        if completed {
            return Ok(None);
        }

        // Stream started but never completed: fetch the latest delta's accumulated text.
        // Scoped to sequence > started_seq so we only look at the current partial stream.
        // Returns empty string if no deltas were emitted before the worker died.
        let accumulated = sqlx::query_scalar::<_, Option<String>>(
            r#"
            SELECT data->>'accumulated'
            FROM events
            WHERE session_id = $1
              AND context->>'turn_id' = $2
              AND event_type = 'output.message.delta'
              AND sequence > $3
              AND ($5 OR data->>'message_id' = $4)
            ORDER BY sequence DESC
            LIMIT 1
            "#,
        )
        .bind(session_id.uuid())
        .bind(turn_id)
        .bind(started_seq)
        .bind(message_id.to_string())
        .bind(legacy_stream)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AgentLoopError::tool(format!("partial_stream delta fetch: {e}")))?
        .flatten()
        .unwrap_or_default();

        Ok(Some(PartialStreamState {
            message_id,
            accumulated,
        }))
    }
}

// PostgreSQL implementation of PartialStreamStore (EVE-532).

use async_trait::async_trait;
use everruns_core::error::AgentLoopError;
use everruns_core::traits::{PartialStreamState, PartialStreamStore};
use everruns_core::typed_id::SessionId;
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
        // Find the sequence of the latest output.message.started for this turn.
        let started_seq: Option<i32> = sqlx::query_scalar(
            r#"
            SELECT MAX(sequence)
            FROM events
            WHERE session_id = $1
              AND context->>'turn_id' = $2
              AND event_type = 'output.message.started'
            "#,
        )
        .bind(session_id.uuid())
        .bind(turn_id)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AgentLoopError::tool(format!("partial_stream started check: {e}")))?;

        let Some(started_seq) = started_seq else {
            return Ok(None);
        };

        // Check if a completion/replacement with sequence > started_seq exists.
        // Using sequence ordering avoids false positives from earlier iterations.
        let completed = sqlx::query_scalar::<_, bool>(
            r#"
            SELECT EXISTS (
                SELECT 1 FROM events
                WHERE session_id = $1
                  AND context->>'turn_id' = $2
                  AND event_type IN ('output.message.completed', 'output.message.replaced')
                  AND sequence > $3
            )
            "#,
        )
        .bind(session_id.uuid())
        .bind(turn_id)
        .bind(started_seq)
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
            ORDER BY sequence DESC
            LIMIT 1
            "#,
        )
        .bind(session_id.uuid())
        .bind(turn_id)
        .bind(started_seq)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AgentLoopError::tool(format!("partial_stream delta fetch: {e}")))?
        .flatten()
        .unwrap_or_default();

        Ok(Some(PartialStreamState { accumulated }))
    }
}

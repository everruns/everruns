// PostgreSQL implementation of PartialStreamStore (EVE-532).

use async_trait::async_trait;
use everruns_core::error::AgentLoopError;
use everruns_core::traits::{PartialStreamState, PartialStreamStore};
use everruns_core::typed_id::SessionId;
use sqlx::PgPool;

/// PostgreSQL-backed partial-stream store.
///
/// Queries the `events` table to detect whether `output.message.started` was
/// emitted for a turn without a matching `output.message.completed` or
/// `output.message.replaced`, and returns the `accumulated` text from the
/// most recent `output.message.delta` if so.
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
        // Check whether output.message.started exists for this turn.
        let started = sqlx::query_scalar::<_, bool>(
            r#"
            SELECT EXISTS (
                SELECT 1 FROM events
                WHERE session_id = $1
                  AND context->>'turn_id' = $2
                  AND event_type = 'output.message.started'
            )
            "#,
        )
        .bind(session_id.as_uuid())
        .bind(turn_id)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AgentLoopError::tool(format!("partial_stream started check: {e}")))?;

        if !started {
            return Ok(None);
        }

        // Check whether the stream was already completed or replaced.
        let completed = sqlx::query_scalar::<_, bool>(
            r#"
            SELECT EXISTS (
                SELECT 1 FROM events
                WHERE session_id = $1
                  AND context->>'turn_id' = $2
                  AND event_type IN ('output.message.completed', 'output.message.replaced')
            )
            "#,
        )
        .bind(session_id.as_uuid())
        .bind(turn_id)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AgentLoopError::tool(format!("partial_stream completed check: {e}")))?;

        if completed {
            return Ok(None);
        }

        // Stream started but never completed: fetch the latest delta's accumulated text.
        // Returns empty string if no deltas were emitted before the worker died.
        let accumulated = sqlx::query_scalar::<_, Option<String>>(
            r#"
            SELECT data->>'accumulated'
            FROM events
            WHERE session_id = $1
              AND context->>'turn_id' = $2
              AND event_type = 'output.message.delta'
            ORDER BY created_at DESC
            LIMIT 1
            "#,
        )
        .bind(session_id.as_uuid())
        .bind(turn_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AgentLoopError::tool(format!("partial_stream delta fetch: {e}")))?
        .flatten()
        .unwrap_or_default();

        Ok(Some(PartialStreamState { accumulated }))
    }
}

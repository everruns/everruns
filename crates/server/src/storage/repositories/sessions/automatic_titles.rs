//! Durable admission and ownership fences for advisory title generation.
use super::*;

#[derive(Debug, sqlx::FromRow)]
pub(crate) struct AutomaticTitleState {
    pub title: Option<String>,
    pub title_is_automatic: bool,
    pub auto_title_sequence: i64,
    pub auto_title_attempt_sequence: i64,
    pub auto_title_input_sequence: i64,
    pub auto_title_backoff: i16,
}

impl AutomaticTitleState {
    fn eligible(&self, sequence: i32, retitle: bool) -> bool {
        self.auto_title_sequence < i64::from(sequence)
            && self.auto_title_attempt_sequence < i64::from(sequence)
            && (self.title.is_none() || (self.title_is_automatic && retitle))
    }
}

impl Database {
    /// Project only bounded text in PostgreSQL: attachments, tool calls, and
    /// arbitrarily large message payloads must not be loaded by advisory work.
    pub(crate) async fn automatic_title_messages(
        &self,
        org_id: i64,
        session_id: SessionId,
        sequence: i32,
    ) -> Result<Vec<(String, Option<String>)>> {
        Ok(sqlx::query_as(
            "SELECT e.event_type,
                left(jsonb_path_query_first(e.data, '$.message.content[*] ? (@.type == \"text\").text') #>> '{}', 2000)
             FROM events e JOIN sessions s ON s.id = e.session_id
             WHERE s.org_id = $1 AND e.session_id = $2 AND e.sequence <= $3
               AND e.event_type IN ('input.message', 'output.message.completed')
             ORDER BY e.sequence DESC LIMIT 24",
        ).bind(org_id).bind(session_id).bind(sequence).fetch_all(&self.pool).await?)
    }

    pub(crate) async fn claim_automatic_title_check(
        &self,
        org_id: i64,
        session_id: SessionId,
        sequence: i32,
        retitle: bool,
    ) -> Result<Option<AutomaticTitleState>> {
        let mut tx = self.pool.begin().await?;
        let state = sqlx::query_as::<_, AutomaticTitleState>(
            "SELECT title, title_is_automatic, auto_title_sequence, auto_title_attempt_sequence,
                    auto_title_input_sequence, auto_title_backoff
             FROM sessions WHERE org_id = $1 AND id = $2 AND archived_at IS NULL
             AND (auto_title_checked_at IS NULL OR auto_title_checked_at <= now() - interval '5 minutes')
             FOR UPDATE",
        ).bind(org_id).bind(session_id).fetch_optional(&mut *tx).await?;
        let Some(state) = state.filter(|state| state.eligible(sequence, retitle)) else {
            tx.commit().await?;
            return Ok(None);
        };
        // Only user inputs count: autonomous continuations must not buy checks.
        // A partial index and LIMIT bound the scan even for very long sessions.
        let inputs = sqlx::query_scalar::<_, i32>(
            "SELECT sequence FROM events WHERE session_id = $1 AND event_type = 'input.message'
             AND sequence > $2 AND sequence <= $3 ORDER BY sequence DESC LIMIT 32",
        )
        .bind(session_id)
        .bind(state.auto_title_input_sequence)
        .bind(sequence)
        .fetch_all(&mut *tx)
        .await?;
        let required = if state.auto_title_attempt_sequence == 0 {
            1
        } else {
            4_usize << state.auto_title_backoff.clamp(0, 3)
        };
        if inputs.len() < required {
            tx.commit().await?;
            return Ok(None);
        }
        // Reserve before the paid call. Failures and process restarts retain
        // cooldown/backoff, and competing listeners cannot claim the same work.
        sqlx::query(
            "UPDATE sessions SET auto_title_attempt_sequence = $3,
             auto_title_input_sequence = $4, auto_title_checked_at = now(),
             auto_title_backoff = LEAST(auto_title_backoff + 1, 3)
             WHERE org_id = $1 AND id = $2",
        )
        .bind(org_id)
        .bind(session_id)
        .bind(i64::from(sequence))
        .bind(i64::from(inputs[0]))
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(Some(state))
    }

    pub(crate) async fn set_automatic_title(
        &self,
        org_id: i64,
        session_id: SessionId,
        title: Option<&str>,
        sequence: i32,
        retitle: bool,
    ) -> Result<Option<(Option<String>,)>> {
        let mut tx = self.pool.begin().await?;
        // Event inserts lock event_sequences before sessions. Follow that order
        // because the caller emits session.title.updated in the same transaction.
        // Manual command transactions can already hold the session row; NOWAIT
        // below drops advisory work instead of forming a lock-order deadlock.
        let exists = sqlx::query_scalar::<_, i32>(
            "SELECT 1 FROM event_sequences es JOIN sessions s ON s.id = es.session_id
             WHERE s.org_id = $1 AND s.id = $2 FOR UPDATE OF es",
        )
        .bind(org_id)
        .bind(session_id)
        .fetch_optional(&mut *tx)
        .await?;
        if exists.is_none() {
            tx.commit().await?;
            return Ok(None);
        }
        let previous = sqlx::query_as::<_, (Option<String>,)>(
            "WITH prior AS (
                SELECT id, title FROM sessions WHERE org_id = $1 AND id = $2
                AND archived_at IS NULL AND (title IS NULL OR (title_is_automatic AND $5))
                AND auto_title_sequence < $4 AND auto_title_attempt_sequence = $4 FOR UPDATE NOWAIT
             ) UPDATE sessions s SET title = COALESCE($3, s.title), title_is_automatic = TRUE,
               auto_title_sequence = $4,
               updated_at = CASE WHEN $3 IS NOT NULL AND $3 IS DISTINCT FROM s.title
                   THEN now() ELSE s.updated_at END,
               auto_title_backoff = CASE WHEN $3 IS NOT NULL AND $3 IS DISTINCT FROM s.title
                   THEN 0 ELSE s.auto_title_backoff END
             FROM prior WHERE s.id = prior.id RETURNING prior.title",
        )
        .bind(org_id)
        .bind(session_id)
        .bind(title)
        .bind(i64::from(sequence))
        .bind(retitle)
        .fetch_optional(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(previous)
    }

    #[cfg(test)]
    pub(crate) async fn automatic_title_state(
        &self,
        org_id: i64,
        session_id: SessionId,
    ) -> Result<AutomaticTitleState> {
        Ok(sqlx::query_as(
            "SELECT title, title_is_automatic, auto_title_sequence, auto_title_attempt_sequence,
             auto_title_input_sequence, auto_title_backoff FROM sessions WHERE org_id = $1 AND id = $2",
        ).bind(org_id).bind(session_id).fetch_one(&self.pool).await?)
    }
}

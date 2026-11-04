// PostgreSQL repository: tool calls a worker took off their turn
// (knowledge/execution/detached-tool-calls.md).
//
// Decisions:
// - A row says a call of the session's open turn is still running on its
//   holder. While any row of a session exists its actor is not leased (the
//   gate in `lease_session_actors`): the turn parked waiting for the call, and
//   steering input waits for it too, as it waits for any act.
// - Only the actor's live holder can detach a call (the insert checks its
//   lease fence), and only the row's holder can renew or complete it. A
//   holder renews all of its rows in one statement.
// - A row past its expiry is dead: renew does not revive it, appends under it
//   are refused, and the next lease call deletes it and makes the actor
//   runnable, so the fold settles the call as after any crash mid-tool.
// - Completing the last row of a session makes the actor runnable, in the
//   transaction that deletes it. The actor row is locked first, as the expiry
//   sweep and cancel do, so concurrent completions of one session agree on
//   which one was last.

use super::ttl_millis;
use crate::storage::Database;
use anyhow::Result;
use everruns_contracts::typed_id::SessionId;
use std::time::Duration;
use uuid::Uuid;

/// Most expired rows' sessions one lease call makes runnable again.
const EXPIRED_SWEEP_BATCH: i64 = 100;

impl Database {
    /// Record `tool_call_id` of `session_id`'s turn `turn_id` as running on
    /// `holder` for `ttl`. `false` when `holder` no longer holds the actor's
    /// lease `fence`: the call stays with its turn.
    pub async fn detach_tool_call(
        &self,
        session_id: SessionId,
        tool_call_id: &str,
        turn_id: Uuid,
        holder: &str,
        fence: i64,
        ttl: Duration,
    ) -> Result<bool> {
        let inserted = sqlx::query_scalar::<_, i32>(
            r#"
            INSERT INTO detached_tool_calls
                (session_id, tool_call_id, org_id, turn_id, holder, expires_at)
            SELECT a.session_id, $2, a.org_id, $3, $4,
                   NOW() + $6::float8 * INTERVAL '1 millisecond'
            FROM session_actors a
            WHERE a.session_id = $1 AND a.lease_holder = $4 AND a.lease_fence = $5
              AND a.lease_expires_at > NOW()
            ON CONFLICT (session_id, tool_call_id) DO UPDATE SET
                turn_id = EXCLUDED.turn_id,
                holder = EXCLUDED.holder,
                expires_at = EXCLUDED.expires_at,
                created_at = NOW()
            RETURNING 1
            "#,
        )
        .bind(session_id)
        .bind(tool_call_id)
        .bind(turn_id)
        .bind(holder)
        .bind(fence)
        .bind(ttl_millis(ttl))
        .fetch_optional(&self.pool)
        .await?;
        Ok(inserted.is_some())
    }

    /// Extend `holder`'s live rows among `calls` by `ttl`. Returns the calls
    /// still live; a call missing from the reply was cancelled, ended with
    /// its turn, or expired, and its holder stops it.
    pub async fn renew_detached_tool_calls(
        &self,
        holder: &str,
        ttl: Duration,
        calls: &[(SessionId, String)],
    ) -> Result<Vec<(SessionId, String)>> {
        if calls.is_empty() {
            return Ok(Vec::new());
        }
        let (sessions, ids): (Vec<Uuid>, Vec<String>) = calls
            .iter()
            .map(|(session, id)| (session.uuid(), id.clone()))
            .unzip();
        let rows = sqlx::query_as::<_, (Uuid, String)>(
            r#"
            UPDATE detached_tool_calls d SET
                expires_at = NOW() + $2::float8 * INTERVAL '1 millisecond'
            FROM UNNEST($3::uuid[], $4::text[]) AS k(session_id, tool_call_id)
            WHERE d.session_id = k.session_id AND d.tool_call_id = k.tool_call_id
              AND d.holder = $1 AND d.expires_at > NOW()
            RETURNING d.session_id, d.tool_call_id
            "#,
        )
        .bind(holder)
        .bind(ttl_millis(ttl))
        .bind(&sessions)
        .bind(&ids)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(session, id)| (SessionId::from_uuid(session), id))
            .collect())
    }

    /// The call finished and its `tool.completed` is stored: delete
    /// `holder`'s live row. `None` when it was not (cancelled, expired, or
    /// never detached); otherwise whether this was the session's last one,
    /// which made the actor runnable.
    pub async fn complete_detached_tool_call(
        &self,
        session_id: SessionId,
        tool_call_id: &str,
        holder: &str,
    ) -> Result<Option<bool>> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT 1 FROM session_actors WHERE session_id = $1 FOR UPDATE")
            .bind(session_id)
            .execute(&mut *tx)
            .await?;
        let deleted = sqlx::query_scalar::<_, i32>(
            "DELETE FROM detached_tool_calls \
             WHERE session_id = $1 AND tool_call_id = $2 AND holder = $3 \
               AND expires_at > NOW() \
             RETURNING 1",
        )
        .bind(session_id)
        .bind(tool_call_id)
        .bind(holder)
        .fetch_optional(&mut *tx)
        .await?;
        if deleted.is_none() {
            tx.rollback().await?;
            return Ok(None);
        }
        let remaining: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM detached_tool_calls WHERE session_id = $1)",
        )
        .bind(session_id)
        .fetch_one(&mut *tx)
        .await?;
        if !remaining {
            super::mark_session_actor_resumable(&mut *tx, session_id).await?;
        }
        tx.commit().await?;
        Ok(Some(!remaining))
    }

    /// Expire `holder`'s rows among `calls` now (it stopped them: shutdown,
    /// or their turn did not park), so the next lease call resumes their
    /// turns without waiting out a TTL.
    pub async fn expire_detached_tool_calls(
        &self,
        holder: &str,
        calls: &[(SessionId, String)],
    ) -> Result<()> {
        if calls.is_empty() {
            return Ok(());
        }
        let (sessions, ids): (Vec<Uuid>, Vec<String>) = calls
            .iter()
            .map(|(session, id)| (session.uuid(), id.clone()))
            .unzip();
        sqlx::query(
            r#"
            UPDATE detached_tool_calls d SET expires_at = NOW()
            FROM UNNEST($2::uuid[], $3::text[]) AS k(session_id, tool_call_id)
            WHERE d.session_id = k.session_id AND d.tool_call_id = k.tool_call_id
              AND d.holder = $1 AND d.expires_at > NOW()
            "#,
        )
        .bind(holder)
        .bind(&sessions)
        .bind(&ids)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Delete expired rows and make their actors runnable to resume the
    /// open turn from the log. Actor rows are locked first (see the header);
    /// one a lease or completion holds is swept by a later call.
    pub(super) async fn sweep_expired_detached_tool_calls(&self) -> Result<u64> {
        let swept = sqlx::query(
            r#"
            WITH locked AS (
                SELECT a.session_id
                FROM session_actors a
                WHERE a.session_id IN (
                    SELECT session_id FROM detached_tool_calls WHERE expires_at <= NOW()
                )
                ORDER BY a.session_id
                LIMIT $1
                FOR UPDATE OF a SKIP LOCKED
            ),
            gone AS (
                DELETE FROM detached_tool_calls d
                USING locked l
                WHERE d.session_id = l.session_id AND d.expires_at <= NOW()
                RETURNING d.session_id
            )
            UPDATE session_actors a SET
                runnable_since = COALESCE(a.runnable_since, NOW()),
                inbox_seq = GREATEST(
                    a.inbox_seq,
                    COALESCE((SELECT MAX(e.sequence) FROM events e
                              WHERE e.session_id = a.session_id), 0)
                )
            WHERE a.session_id IN (SELECT session_id FROM gone)
            "#,
        )
        .bind(EXPIRED_SWEEP_BATCH)
        .execute(&self.pool)
        .await?;
        Ok(swept.rows_affected())
    }

    /// The tool call ids of `session_id`'s rows, live or not yet swept.
    pub async fn list_detached_tool_calls(&self, session_id: SessionId) -> Result<Vec<String>> {
        Ok(sqlx::query_scalar(
            "SELECT tool_call_id FROM detached_tool_calls \
             WHERE session_id = $1 ORDER BY created_at, tool_call_id",
        )
        .bind(session_id)
        .fetch_all(&self.pool)
        .await?)
    }
}

#[cfg(test)]
mod tests;

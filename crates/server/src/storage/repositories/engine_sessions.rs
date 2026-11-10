// PostgreSQL repository: the Engine's session authority over the platform's
// session actors (data plane v2 step 4).
//
// Decisions:
// - One authority. A session that runs on the Engine keeps the platform's
//   event log as its journal and its `session_actors` row as its lease: the
//   Engine binding, suspension and timer are columns of that row, and the
//   Engine's receipts (turn assignments, outcomes) are rows of
//   `engine_session_records`. Every write here is fenced by the actor lease
//   in its own statement, as every journal append is, so a holder that lost
//   its lease writes nothing.
// - The binding names an opaque agent key and a revision, never a platform
//   id. Only `engine_agent_keys` maps a key back to an agent.
// - A session moves to its key's current revision only between turns, at
//   claim (`claim_engine_binding`). The log decides whether a turn is open:
//   its latest `turn.started` not followed by a terminal turn event. A turn
//   always finishes on the revision it started with.
// - A suspended session is never leased (`LEASE_ELIGIBLE`) until a person or
//   operator resumes it. A timer marks the actor runnable when it falls due
//   (`wake_due_engine_sessions`, run on every lease poll).

use anyhow::Result;
use chrono::{DateTime, Utc};
use everruns_contracts::typed_id::SessionId;
use serde_json::Value;

use super::Database;

/// A session's Engine binding and scheduling state.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct EngineBindingRow {
    pub agent_key: String,
    pub revision: String,
    /// The session runtime format the binding was created with.
    pub runtime: i32,
    pub suspended_reason: Option<String>,
    pub wake_at: Option<DateTime<Utc>>,
}

/// The lease the caller presents, checked in each write's statement.
macro_rules! held {
    () => {
        "session_id = $1 AND lease_holder = $2 AND lease_fence = $3 AND lease_expires_at > NOW()"
    };
}

/// The log has a turn open: its latest turn event is a `turn.started`.
macro_rules! in_turn {
    () => {
        "(SELECT e.event_type FROM events e
           WHERE e.session_id = session_actors.session_id
             AND e.event_type IN ('turn.started', 'turn.completed', 'turn.failed', 'turn.cancelled')
           ORDER BY e.sequence DESC
           LIMIT 1) IS NOT DISTINCT FROM 'turn.started'"
    };
}

/// Write a receipt under the lease: one statement, the lease row locked
/// (`FOR SHARE`) for the insert, so a lease taken over in between cannot
/// slip past the check.
macro_rules! commit_record {
    ($conflict:literal) => {
        concat!(
            "WITH lease AS (SELECT session_id FROM session_actors WHERE ",
            held!(),
            " FOR SHARE),
             written AS (
                 INSERT INTO engine_session_records (session_id, record_key, value)
                 SELECT session_id, $4, $5 FROM lease
                 ON CONFLICT (session_id, record_key) ",
            $conflict,
            " RETURNING 1)
             SELECT EXISTS (SELECT 1 FROM lease), EXISTS (SELECT 1 FROM written)"
        )
    };
}
const COMMIT_ABSENT: &str = commit_record!("DO NOTHING");
const COMMIT_REPLACE: &str =
    commit_record!("DO UPDATE SET value = excluded.value, updated_at = now()");

impl Database {
    /// `session_id`'s Engine binding, or `None` for a session that runs on
    /// the worker's own host.
    pub async fn engine_session_binding(
        &self,
        session_id: SessionId,
    ) -> Result<Option<EngineBindingRow>> {
        Ok(sqlx::query_as(
            "SELECT engine_agent_key AS agent_key, engine_revision AS revision,
                    COALESCE(engine_runtime, 0) AS runtime,
                    engine_suspended_reason AS suspended_reason, engine_wake_at AS wake_at
               FROM session_actors
              WHERE session_id = $1 AND engine_agent_key IS NOT NULL",
        )
        .bind(session_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    /// Bind `session_id` to `agent_key` at `revision`, under the caller's
    /// lease, when it is unbound and no turn is open in its log. A bound
    /// session keeps its binding. Returns the binding after the call, or
    /// `None` when the lease is lost or a turn is open on an unbound session
    /// (it binds at a later claim, between turns).
    pub async fn bind_engine_session(
        &self,
        session_id: SessionId,
        holder: &str,
        fence: i64,
        agent_key: &str,
        revision: &str,
        runtime: i32,
    ) -> Result<Option<EngineBindingRow>> {
        let bound: Option<EngineBindingRow> = sqlx::query_as(concat!(
            "UPDATE session_actors SET
                 engine_agent_key = COALESCE(engine_agent_key, $4),
                 engine_revision = COALESCE(engine_revision, $5),
                 engine_runtime = COALESCE(engine_runtime, $6)
              WHERE ",
            held!(),
            " AND (engine_agent_key IS NOT NULL OR NOT ",
            in_turn!(),
            ") RETURNING engine_agent_key AS agent_key, engine_revision AS revision,
                        engine_runtime AS runtime,
                        engine_suspended_reason AS suspended_reason, engine_wake_at AS wake_at"
        ))
        .bind(session_id)
        .bind(holder)
        .bind(fence)
        .bind(agent_key)
        .bind(revision)
        .bind(runtime)
        .fetch_optional(&self.pool)
        .await?;
        Ok(bound)
    }

    /// The binding a new claim runs, under the caller's lease: between turns
    /// the session first moves to its key's current revision. `None` when
    /// the lease is lost or the session is unbound.
    pub async fn claim_engine_binding(
        &self,
        session_id: SessionId,
        holder: &str,
        fence: i64,
    ) -> Result<Option<EngineBindingRow>> {
        Ok(sqlx::query_as(concat!(
            "UPDATE session_actors SET
                 engine_revision = CASE
                     WHEN ",
            in_turn!(),
            " THEN engine_revision
                     ELSE COALESCE((SELECT c.revision FROM engine_agent_current c
                                     WHERE c.agent_key = session_actors.engine_agent_key),
                                   engine_revision)
                 END,
                 -- A due timer is this claim's work.
                 engine_wake_at = CASE WHEN engine_wake_at <= NOW() THEN NULL
                                       ELSE engine_wake_at END
              WHERE ",
            held!(),
            " AND engine_agent_key IS NOT NULL
              RETURNING engine_agent_key AS agent_key, engine_revision AS revision,
                        COALESCE(engine_runtime, 0) AS runtime,
                        engine_suspended_reason AS suspended_reason, engine_wake_at AS wake_at"
        ))
        .bind(session_id)
        .bind(holder)
        .bind(fence)
        .fetch_optional(&self.pool)
        .await?)
    }

    /// One Engine receipt of `session_id`.
    pub async fn engine_session_record(
        &self,
        session_id: SessionId,
        key: &str,
    ) -> Result<Option<Value>> {
        Ok(sqlx::query_scalar(
            "SELECT value FROM engine_session_records WHERE session_id = $1 AND record_key = $2",
        )
        .bind(session_id)
        .bind(key)
        .fetch_optional(&self.pool)
        .await?)
    }

    /// Up to `limit` of `session_id`'s Engine receipts whose key starts with
    /// `prefix`, in key order.
    pub async fn engine_session_records(
        &self,
        session_id: SessionId,
        prefix: &str,
        limit: i64,
    ) -> Result<Vec<(String, Value)>> {
        Ok(sqlx::query_as(
            "SELECT record_key, value FROM engine_session_records
              WHERE session_id = $1 AND starts_with(record_key, $2)
              ORDER BY record_key
              LIMIT $3",
        )
        .bind(session_id)
        .bind(prefix)
        .bind(limit.max(0))
        .fetch_all(&self.pool)
        .await?)
    }

    /// Write an Engine receipt under the caller's lease. With `absent`, only
    /// a new key is written. `None` when the lease is lost (nothing written);
    /// otherwise whether the receipt was written.
    pub async fn commit_engine_session_record(
        &self,
        session_id: SessionId,
        holder: &str,
        fence: i64,
        key: &str,
        value: &Value,
        absent: bool,
    ) -> Result<Option<bool>> {
        let (held, written): (bool, bool) = sqlx::query_as(if absent {
            COMMIT_ABSENT
        } else {
            COMMIT_REPLACE
        })
        .bind(session_id)
        .bind(holder)
        .bind(fence)
        .bind(key)
        .bind(value)
        .fetch_one(&self.pool)
        .await?;
        Ok(held.then_some(written))
    }

    /// Suspend `session_id` under the caller's lease: it is not leased again
    /// until [`Self::resume_engine_session`]. Whether the lease was held.
    pub async fn suspend_engine_session(
        &self,
        session_id: SessionId,
        holder: &str,
        fence: i64,
        reason: &str,
    ) -> Result<bool> {
        let done = sqlx::query(concat!(
            "UPDATE session_actors SET engine_suspended_reason = $4 WHERE ",
            held!()
        ))
        .bind(session_id)
        .bind(holder)
        .bind(fence)
        .bind(reason)
        .execute(&self.pool)
        .await?;
        Ok(done.rows_affected() == 1)
    }

    /// Clear `session_id`'s suspension (a person or operator resumed it) and
    /// mark its actor runnable from the log's head. Whether it was suspended.
    pub async fn resume_engine_session(&self, session_id: SessionId) -> Result<bool> {
        let mut tx = self.pool.begin().await?;
        let cleared = sqlx::query(
            "UPDATE session_actors SET engine_suspended_reason = NULL
              WHERE session_id = $1 AND engine_suspended_reason IS NOT NULL",
        )
        .bind(session_id)
        .execute(&mut *tx)
        .await?
        .rows_affected()
            == 1;
        if cleared {
            super::session_actors::mark_session_actor_resumable(&mut *tx, session_id).await?;
        }
        tx.commit().await?;
        Ok(cleared)
    }

    /// Set (or clear) `session_id`'s timer under the caller's lease. Whether
    /// the lease was held.
    pub async fn schedule_engine_session(
        &self,
        session_id: SessionId,
        holder: &str,
        fence: i64,
        wake_at: Option<DateTime<Utc>>,
    ) -> Result<bool> {
        let done = sqlx::query(concat!(
            "UPDATE session_actors SET engine_wake_at = $4 WHERE ",
            held!()
        ))
        .bind(session_id)
        .bind(holder)
        .bind(fence)
        .bind(wake_at)
        .execute(&self.pool)
        .await?;
        Ok(done.rows_affected() == 1)
    }

    /// Mark up to `limit` actors whose Engine timer fell due runnable. The
    /// timer stays set until the claim that runs it clears it, so a lease
    /// that dies before running it leaves it due.
    pub async fn wake_due_engine_sessions(&self, limit: i64) -> Result<u64> {
        Ok(sqlx::query(
            "UPDATE session_actors SET runnable_since = NOW()
              WHERE session_id IN (
                  SELECT session_id FROM session_actors
                   WHERE engine_wake_at <= NOW()
                     AND engine_suspended_reason IS NULL
                     AND runnable_since IS NULL
                   ORDER BY engine_wake_at
                   LIMIT $1
                   FOR UPDATE SKIP LOCKED
              )",
        )
        .bind(limit.max(0))
        .execute(&self.pool)
        .await?
        .rows_affected())
    }
}

#[cfg(test)]
mod tests;

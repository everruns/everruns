// PostgreSQL repository: sessions as actors (actor-based design, step 6b).
//
// The runnable index and the lease protocol a worker uses to run a session's
// whole turn. Inert until the worker cuts over (step 6c).
//
// Decisions:
// - The session's event log is the only turn state. This table says which
//   sessions have work and who runs each one; resume folds the log.
// - `mark_session_actor_runnable` is a plain upsert on any executor, so the
//   caller runs it in the same transaction that persists the input. The input
//   and the runnable marker commit together or not at all; a worker can never
//   see one without the other.
// - Leasing locks candidate rows with `FOR UPDATE SKIP LOCKED`, so concurrent
//   lease calls take disjoint sets without waiting on each other. Every lease
//   raises the fence, so an expired holder can neither renew nor release the
//   lease that replaced it.
// - The per-org cap counts held leases at the start of the lease statement.
//   Two concurrent lease calls do not see each other's new leases, so the cap
//   is a fairness bound, not a quota: one org can exceed it by at most what the
//   concurrent callers take in that instant. Admission (`ORG_MAX_ACTIVE_TURNS`)
//   stays where the input is accepted.
// - The candidate scan reads at most `ACTOR_LEASE_SCAN_FACTOR` times the batch,
//   oldest first, then applies the cap. An org over its cap spends scan slots
//   without taking any, so a deep backlog from one org can delay others by at
//   most one scan window per lease call; widen the factor if that shows up.
// - Cancel: it applies to the turn running (or about to run) when it was
//   requested, so it lands only on an actor that is leased or runnable. A
//   lease or renew reports it and marks it reported; release clears a reported
//   cancel, and keeps an unreported one (it landed after the holder's last
//   renew) pending and the actor runnable, so the next holder sees it.
// - Release clears the runnable marker only when the holder consumed every
//   input up to `inbox_seq` and no unreported cancel is pending. An input that
//   lands while a turn finishes moves `inbox_seq` past what the holder
//   consumed, so the actor stays runnable and the next lease runs it.
// - The journal read starts at the session's latest `turn.started` (the turn
//   a fold resumes) or after a given sequence, and returns at most
//   `MAX_ACTOR_JOURNAL_LIMIT` events per call; the caller pages with
//   `has_more`. A turn's log is bounded in practice by its iteration cap, so
//   one page is the common case.

pub(super) mod rows;
use rows::*;

#[cfg(test)]
mod tests;

use super::Database;
use crate::storage::EventRow;
use anyhow::Result;
use everruns_contracts::typed_id::SessionId;
use everruns_server_macros::sql;
use std::time::Duration;

/// Held leases one org may have at once, unless the caller passes its own cap.
pub const DEFAULT_ACTOR_LEASES_PER_ORG: i64 = 32;
/// Most actors one lease call takes.
pub const MAX_ACTOR_LEASE_BATCH: i64 = 100;
/// Candidate rows a lease call scans per actor it may take; see the header.
const ACTOR_LEASE_SCAN_FACTOR: i64 = 8;
/// Upper bound on the candidate scan of one lease call.
const MAX_ACTOR_LEASE_SCAN: i64 = 1000;
/// Shortest and longest lease a caller may ask for.
pub const MIN_ACTOR_LEASE_TTL: Duration = Duration::from_secs(1);
pub const MAX_ACTOR_LEASE_TTL: Duration = Duration::from_secs(300);
/// Journal page size when the caller passes none.
pub const DEFAULT_ACTOR_JOURNAL_LIMIT: i64 = 500;
/// Largest journal page one read returns.
pub const MAX_ACTOR_JOURNAL_LIMIT: i64 = 2000;

fn ttl_millis(ttl: Duration) -> i64 {
    let ttl = ttl.clamp(MIN_ACTOR_LEASE_TTL, MAX_ACTOR_LEASE_TTL);
    i64::try_from(ttl.as_millis()).unwrap_or(i64::MAX)
}

/// Mark `session_id`'s actor runnable with input up to `inbox_seq`.
///
/// Runs on any executor so the caller can put it in the transaction that
/// persists the input. Idempotent: `inbox_seq` only moves forward, and an
/// actor that is already runnable keeps its place in line.
pub async fn mark_session_actor_runnable<'e, E>(
    executor: E,
    session_id: SessionId,
    org_id: i64,
    inbox_seq: i64,
) -> Result<()>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    sqlx::query(
        r#"
        INSERT INTO session_actors (session_id, org_id, inbox_seq, runnable_since)
        VALUES ($1, $2, $3, NOW())
        ON CONFLICT (session_id) DO UPDATE SET
            inbox_seq = GREATEST(session_actors.inbox_seq, EXCLUDED.inbox_seq),
            runnable_since = COALESCE(session_actors.runnable_since, NOW())
        "#,
    )
    .bind(session_id)
    .bind(org_id)
    .bind(inbox_seq)
    .execute(executor)
    .await?;
    Ok(())
}

impl Database {
    /// [`mark_session_actor_runnable`] on the request pool, which joins the
    /// current command transaction when one is open.
    pub async fn mark_session_actor_runnable(
        &self,
        session_id: SessionId,
        org_id: i64,
        inbox_seq: i64,
    ) -> Result<()> {
        mark_session_actor_runnable(&self.pool, session_id, org_id, inbox_seq).await
    }

    /// Lease up to `limit` runnable actors whose lease is free or expired, to
    /// `holder` for `ttl`, oldest first, taking at most `max_per_org` held
    /// leases per org (counting leases already held).
    pub async fn lease_session_actors(
        &self,
        holder: &str,
        limit: i64,
        ttl: Duration,
        max_per_org: i64,
    ) -> Result<Vec<LeasedActorRow>> {
        let limit = limit.clamp(0, MAX_ACTOR_LEASE_BATCH);
        if limit == 0 || max_per_org <= 0 {
            return Ok(Vec::new());
        }
        let scan = (limit * ACTOR_LEASE_SCAN_FACTOR).min(MAX_ACTOR_LEASE_SCAN);
        Ok(sqlx::query_as::<_, LeasedActorRow>(
            r#"
            WITH candidates AS (
                SELECT session_id, org_id, runnable_since
                FROM session_actors
                WHERE runnable_since IS NOT NULL
                  AND (lease_holder IS NULL OR lease_expires_at <= NOW())
                ORDER BY runnable_since, session_id
                LIMIT $2
                FOR UPDATE SKIP LOCKED
            ),
            held AS (
                SELECT org_id, COUNT(*) AS leased
                FROM session_actors
                WHERE lease_holder IS NOT NULL
                  AND lease_expires_at > NOW()
                  AND org_id IN (SELECT org_id FROM candidates)
                GROUP BY org_id
            ),
            ranked AS (
                SELECT c.session_id, c.runnable_since,
                       COALESCE(h.leased, 0)
                         + ROW_NUMBER() OVER (
                               PARTITION BY c.org_id
                               ORDER BY c.runnable_since, c.session_id
                           ) AS org_slot
                FROM candidates c
                LEFT JOIN held h ON h.org_id = c.org_id
            ),
            chosen AS (
                SELECT session_id
                FROM ranked
                WHERE org_slot <= $3
                ORDER BY runnable_since, session_id
                LIMIT $1
            ),
            leased AS (
                UPDATE session_actors a SET
                    lease_holder = $4,
                    lease_fence = a.lease_fence + 1,
                    lease_expires_at = NOW() + $5::float8 * INTERVAL '1 millisecond',
                    cancel_reported = (a.cancel_requested_at IS NOT NULL)
                FROM chosen
                WHERE a.session_id = chosen.session_id
                RETURNING a.session_id, a.org_id, a.lease_fence, a.inbox_seq,
                          a.cancel_requested_at, a.runnable_since
            )
            SELECT session_id, org_id, lease_fence AS fence, inbox_seq,
                   (cancel_requested_at IS NOT NULL) AS cancel_requested
            FROM leased
            ORDER BY runnable_since, session_id
            "#,
        )
        .bind(limit)
        .bind(scan)
        .bind(max_per_org)
        .bind(holder)
        .bind(ttl_millis(ttl))
        .fetch_all(&self.pool)
        .await?)
    }

    /// Extend `holder`'s lease on `session_id` by `ttl`. `None` when the lease
    /// is lost: another holder took it (the fence moved) or it was released.
    pub async fn renew_session_actor_lease(
        &self,
        session_id: SessionId,
        holder: &str,
        fence: i64,
        ttl: Duration,
    ) -> Result<Option<ActorRenewalRow>> {
        Ok(sqlx::query_as::<_, ActorRenewalRow>(
            r#"
            UPDATE session_actors SET
                lease_expires_at = NOW() + $4::float8 * INTERVAL '1 millisecond',
                cancel_reported = (cancel_requested_at IS NOT NULL)
            WHERE session_id = $1 AND lease_holder = $2 AND lease_fence = $3
            RETURNING inbox_seq, (cancel_requested_at IS NOT NULL) AS cancel_requested
            "#,
        )
        .bind(session_id)
        .bind(holder)
        .bind(fence)
        .bind(ttl_millis(ttl))
        .fetch_optional(&self.pool)
        .await?)
    }

    /// Give up `holder`'s lease on `session_id`, having consumed input up to
    /// `consumed_inbox_seq`. Returns `None` when the lease was already lost,
    /// otherwise whether the actor is still runnable (new input arrived, or a
    /// cancel the holder never saw is pending).
    pub async fn release_session_actor(
        &self,
        session_id: SessionId,
        holder: &str,
        fence: i64,
        consumed_inbox_seq: i64,
    ) -> Result<Option<bool>> {
        Ok(sqlx::query_scalar::<_, bool>(
            r#"
            UPDATE session_actors SET
                lease_holder = NULL,
                lease_expires_at = NULL,
                runnable_since = CASE
                    WHEN inbox_seq <= $4
                     AND (cancel_requested_at IS NULL OR cancel_reported)
                    THEN NULL
                    ELSE runnable_since
                END,
                cancel_requested_at = CASE
                    WHEN cancel_reported THEN NULL
                    ELSE cancel_requested_at
                END,
                cancel_reported = FALSE
            WHERE session_id = $1 AND lease_holder = $2 AND lease_fence = $3
            RETURNING runnable_since IS NOT NULL
            "#,
        )
        .bind(session_id)
        .bind(holder)
        .bind(fence)
        .bind(consumed_inbox_seq)
        .fetch_optional(&self.pool)
        .await?)
    }

    /// Ask the turn `session_id` is running, or about to run, to stop. `false`
    /// when the actor is idle (not leased, nothing runnable): there is no turn
    /// to cancel. A repeated request resets the report, so the holder hears it.
    pub async fn request_session_actor_cancel(&self, session_id: SessionId) -> Result<bool> {
        let updated = sqlx::query(
            r#"
            UPDATE session_actors SET
                cancel_requested_at = NOW(),
                cancel_reported = FALSE
            WHERE session_id = $1
              AND (runnable_since IS NOT NULL OR lease_holder IS NOT NULL)
            "#,
        )
        .bind(session_id)
        .execute(&self.pool)
        .await?;
        Ok(updated.rows_affected() > 0)
    }

    /// The actor row for `session_id`, if it ever had work.
    pub async fn get_session_actor(
        &self,
        session_id: SessionId,
    ) -> Result<Option<SessionActorRow>> {
        Ok(sqlx::query_as::<_, SessionActorRow>(sql!(
            "SELECT {SessionActorRow} FROM session_actors WHERE session_id = $1"
        ))
        .bind(session_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    /// The tail of `session_id`'s log an actor folds to resume: events after
    /// `after_sequence`, or from the latest `turn.started` (the whole log when
    /// no turn started yet). At most `limit` events, capped at
    /// [`MAX_ACTOR_JOURNAL_LIMIT`].
    pub async fn read_session_actor_journal(
        &self,
        session_id: SessionId,
        after_sequence: Option<i32>,
        limit: i64,
    ) -> Result<ActorJournalRows> {
        let limit = if limit <= 0 {
            DEFAULT_ACTOR_JOURNAL_LIMIT
        } else {
            limit.min(MAX_ACTOR_JOURNAL_LIMIT)
        };
        let mut events = sqlx::query_as::<_, EventRow>(sql!(
            r#"
            SELECT {EventRow}
            FROM events
            WHERE session_id = $1
              AND sequence >= CASE
                  WHEN $2::int IS NOT NULL THEN $2::int + 1
                  ELSE COALESCE((
                      SELECT sequence FROM events
                      WHERE session_id = $1 AND event_type = 'turn.started'
                      ORDER BY sequence DESC
                      LIMIT 1
                  ), 0)
              END
            ORDER BY sequence
            LIMIT $3
            "#
        ))
        .bind(session_id)
        .bind(after_sequence)
        .bind(limit + 1)
        .fetch_all(&self.pool)
        .await?;
        let has_more = events.len() as i64 > limit;
        events.truncate(limit as usize);
        let head_sequence = sqlx::query_scalar::<_, Option<i32>>(
            "SELECT MAX(sequence) FROM events WHERE session_id = $1",
        )
        .bind(session_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(ActorJournalRows {
            events,
            has_more,
            head_sequence,
        })
    }
}

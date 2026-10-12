// PostgreSQL repository: sessions as actors (actor-based design).
//
// The runnable index and the lease protocol a worker uses to run a session's
// whole turn (step 6b), with the inbox cursor and the poison guard the
// cutover added (step 6c, migration 20261012014120).
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
// - Cancel has one writer for its events. On an actor a live holder leases,
//   the request is recorded on the row and the holder ends the turn (a lease
//   or renew reports it and marks it reported; release clears a reported
//   cancel, and keeps an unreported one pending and the actor runnable, so the
//   next holder sees it). On an actor that is runnable but not held (or whose
//   holder's lease expired), nothing runs the turn, so the request withdraws
//   the actor instead: its pending input counts as consumed and the caller
//   records the cancel in the log itself. One statement decides which, under
//   the row lock, so a lease cannot slip in between.
// - Release clears the runnable marker only when the holder consumed every
//   input up to `inbox_seq` and no unreported cancel is pending. An input that
//   lands while a turn finishes moves `inbox_seq` past what the holder
//   consumed, so the actor stays runnable and the next lease runs it.
// - The inbox cursor (`consumed_seq`) moves only forward, on renew and
//   release. A mark records the input a fresh turn starts from
//   (`next_input_seq`) only when the previous one was already consumed, so
//   queued messages start turns in arrival order. A resolution mark (tool
//   results, an answer) points the inbox at the log head instead and leaves
//   the next fresh turn's input alone: it continues the open turn.
// - `attempts` counts leases since the last release; the worker seals a turn
//   whose holders keep dying instead of running it again (poison guard).
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
        INSERT INTO session_actors
            (session_id, org_id, inbox_seq, next_input_seq, runnable_since)
        VALUES ($1, $2, $3, $3, NOW())
        ON CONFLICT (session_id) DO UPDATE SET
            inbox_seq = GREATEST(session_actors.inbox_seq, EXCLUDED.inbox_seq),
            next_input_seq = CASE
                WHEN session_actors.next_input_seq <= session_actors.consumed_seq
                 AND EXCLUDED.next_input_seq > session_actors.consumed_seq
                THEN EXCLUDED.next_input_seq
                ELSE session_actors.next_input_seq
            END,
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

/// Mark `session_id`'s actor runnable to continue its open turn from input
/// already in the log (recorded tool results, an answer, a decision): the
/// inbox moves to the log's head, and the next fresh turn's input is left
/// alone. Looks the org up from the session; a missing session marks nothing.
pub async fn mark_session_actor_resumable<'e, E>(executor: E, session_id: SessionId) -> Result<()>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    sqlx::query(
        r#"
        INSERT INTO session_actors (session_id, org_id, inbox_seq, runnable_since)
        SELECT s.id, s.org_id,
               COALESCE((SELECT MAX(e.sequence) FROM events e WHERE e.session_id = s.id), 0),
               NOW()
        FROM sessions s
        WHERE s.id = $1
        ON CONFLICT (session_id) DO UPDATE SET
            inbox_seq = GREATEST(session_actors.inbox_seq, EXCLUDED.inbox_seq),
            runnable_since = COALESCE(session_actors.runnable_since, NOW())
        "#,
    )
    .bind(session_id)
    .execute(executor)
    .await?;
    Ok(())
}

/// What one pass of [`Database::convert_legacy_turn_tasks`] did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, sqlx::FromRow)]
pub struct LegacyTurnSweep {
    /// Old per-step turn tasks cancelled.
    pub retired_tasks: i64,
    /// Session actors marked runnable in their place.
    pub marked_actors: i64,
}

/// Activity types of the per-step turn workflow that session actors replaced.
pub const LEGACY_TURN_ACTIVITY_TYPES: [&str; 3] = ["process_input", "reason", "act"];

impl Database {
    /// TEMPORARY (actor-based design, step 6c): hand old-path turn work that
    /// appeared after the cutover migration ran to session actors. Step 7 deletes it
    /// together with the per-step turn workflow.
    ///
    /// During a rolling deploy a server or worker of the previous release can
    /// still create per-step turn tasks (`process_input`, `reason`, `act`,
    /// under a `turn_workflow`) after the cutover migration converted the
    /// in-flight ones: an old server starting a turn, or an old worker
    /// finishing a step and enqueueing the next. Nothing of this release runs
    /// them. This pass does what the cutover migration did, for at most `limit` of
    /// them: every pending one, and every claimed one whose claim went
    /// silent for `stale_claim` (its old worker died; a live one finishes its
    /// step and its next task is swept in turn). It marks the session's actor
    /// runnable with the inbox at the log's head and the cursor at the latest
    /// turn boundary, cancels the tasks and their `turn_workflow`, all in one
    /// statement, so it is idempotent and safe to run on every server.
    pub async fn convert_legacy_turn_tasks(
        &self,
        limit: i64,
        stale_claim: Duration,
    ) -> Result<LegacyTurnSweep> {
        let stale_ms = i64::try_from(stale_claim.as_millis()).unwrap_or(i64::MAX);
        Ok(sqlx::query_as::<_, LegacyTurnSweep>(
            r#"
            WITH legacy AS (
                SELECT t.id, t.workflow_id
                FROM durable_task_queue t
                WHERE t.activity_type = ANY($3)
                  AND (t.status = 'pending'
                       OR (t.status = 'claimed'
                           AND COALESCE(t.heartbeat_at, t.claimed_at, t.created_at)
                               < NOW() - $2::float8 * INTERVAL '1 millisecond'))
                ORDER BY t.created_at
                LIMIT $1
                FOR UPDATE SKIP LOCKED
            ),
            retired AS (
                UPDATE durable_task_queue q SET
                    status = 'cancelled',
                    last_error = 'superseded by session actors (cutover sweep)'
                FROM legacy
                WHERE q.id = legacy.id
                RETURNING q.id
            ),
            workflows AS (
                UPDATE durable_workflow_instances w SET
                    status = 'cancelled',
                    completed_at = NOW(),
                    updated_at = NOW()
                WHERE w.id IN (SELECT workflow_id FROM legacy)
                  AND w.workflow_type = 'turn_workflow'
                  AND w.status IN ('pending', 'running')
                RETURNING w.id
            ),
            turns AS (
                SELECT s.id AS session_id, s.org_id
                FROM sessions s
                WHERE s.id IN (SELECT workflow_id FROM legacy)
                ORDER BY s.id
            ),
            marked AS (
                INSERT INTO session_actors (
                    session_id, org_id, runnable_since, inbox_seq, consumed_seq, next_input_seq
                )
                SELECT
                    t.session_id,
                    t.org_id,
                    NOW(),
                    COALESCE((SELECT MAX(e.sequence) FROM events e
                              WHERE e.session_id = t.session_id), 0),
                    COALESCE((
                        SELECT MAX(e.sequence) FROM events e
                        WHERE e.session_id = t.session_id
                          AND e.event_type IN (
                              'turn.started', 'turn.completed', 'turn.failed',
                              'turn.cancelled', 'turn.sealed'
                          )
                    ), 0),
                    0
                FROM turns t
                ON CONFLICT (session_id) DO UPDATE SET
                    runnable_since = COALESCE(session_actors.runnable_since, NOW()),
                    inbox_seq = GREATEST(session_actors.inbox_seq, EXCLUDED.inbox_seq),
                    consumed_seq = GREATEST(session_actors.consumed_seq, EXCLUDED.consumed_seq)
                RETURNING session_id
            )
            SELECT
                (SELECT COUNT(*) FROM retired) AS retired_tasks,
                (SELECT COUNT(*) FROM marked) AS marked_actors
            "#,
        )
        .bind(limit.max(0))
        .bind(stale_ms)
        .bind(LEGACY_TURN_ACTIVITY_TYPES.map(String::from).to_vec())
        .fetch_one(&self.pool)
        .await?)
    }

    /// [`mark_session_actor_resumable`] on the request pool, which joins the
    /// current command transaction when one is open.
    pub async fn mark_session_actor_resumable(&self, session_id: SessionId) -> Result<()> {
        mark_session_actor_resumable(&self.pool, session_id).await
    }

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
                    cancel_reported = (a.cancel_requested_at IS NOT NULL),
                    attempts = a.attempts + 1
                FROM chosen
                WHERE a.session_id = chosen.session_id
                RETURNING a.session_id, a.org_id, a.lease_fence, a.inbox_seq,
                          a.consumed_seq, a.next_input_seq, a.attempts,
                          a.cancel_requested_at, a.runnable_since
            )
            SELECT session_id, org_id, lease_fence AS fence, inbox_seq,
                   consumed_seq, next_input_seq, attempts,
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

    /// Extend `holder`'s lease on `session_id` by `ttl`, recording that its
    /// turn consumed input up to `consumed_seq`. `None` when the lease is
    /// lost: another holder took it (the fence moved), it was released, or a
    /// cancel withdrew the actor after the lease expired.
    pub async fn renew_session_actor_lease(
        &self,
        session_id: SessionId,
        holder: &str,
        fence: i64,
        ttl: Duration,
        consumed_seq: i64,
    ) -> Result<Option<ActorRenewalRow>> {
        Ok(sqlx::query_as::<_, ActorRenewalRow>(
            r#"
            UPDATE session_actors SET
                lease_expires_at = NOW() + $4::float8 * INTERVAL '1 millisecond',
                cancel_reported = (cancel_requested_at IS NOT NULL),
                consumed_seq = GREATEST(consumed_seq, $5)
            WHERE session_id = $1 AND lease_holder = $2 AND lease_fence = $3
            RETURNING inbox_seq, (cancel_requested_at IS NOT NULL) AS cancel_requested
            "#,
        )
        .bind(session_id)
        .bind(holder)
        .bind(fence)
        .bind(ttl_millis(ttl))
        .bind(consumed_seq)
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
                consumed_seq = GREATEST(consumed_seq, $4),
                attempts = 0,
                runnable_since = CASE
                    WHEN inbox_seq <= GREATEST(consumed_seq, $4)
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

    /// Cancel the turn `session_id` is running or about to run; see the
    /// header for who records it. One statement under the row lock.
    pub async fn request_session_actor_cancel(&self, session_id: SessionId) -> Result<ActorCancel> {
        let leased = sqlx::query_scalar::<_, bool>(
            r#"
            WITH current AS (
                SELECT session_id,
                       (lease_holder IS NOT NULL AND lease_expires_at > NOW()) AS leased
                FROM session_actors
                WHERE session_id = $1
                  AND (runnable_since IS NOT NULL OR lease_holder IS NOT NULL)
                FOR UPDATE
            )
            UPDATE session_actors a SET
                cancel_requested_at = CASE WHEN c.leased THEN NOW() ELSE NULL END,
                cancel_reported = FALSE,
                runnable_since = CASE WHEN c.leased THEN a.runnable_since ELSE NULL END,
                consumed_seq = CASE
                    WHEN c.leased THEN a.consumed_seq
                    ELSE GREATEST(a.consumed_seq, a.inbox_seq)
                END,
                lease_holder = CASE WHEN c.leased THEN a.lease_holder ELSE NULL END,
                lease_expires_at = CASE WHEN c.leased THEN a.lease_expires_at ELSE NULL END,
                attempts = CASE WHEN c.leased THEN a.attempts ELSE 0 END
            FROM current c
            WHERE a.session_id = c.session_id
            RETURNING c.leased
            "#,
        )
        .bind(session_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(match leased {
            Some(true) => ActorCancel::Requested,
            Some(false) => ActorCancel::Withdrawn,
            None => ActorCancel::Idle,
        })
    }

    /// Actors a live holder leases right now, across every org.
    pub async fn count_leased_session_actors(&self) -> Result<i64> {
        Ok(sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM session_actors \
             WHERE lease_holder IS NOT NULL AND lease_expires_at > NOW()",
        )
        .fetch_one(&self.pool)
        .await?)
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

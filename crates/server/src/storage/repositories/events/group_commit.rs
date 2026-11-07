//! Group commit for fenced event appends.
//!
//! Decisions:
//! - A worker's event append (`create_events_fenced`, `create_events_detached`)
//!   used to be one transaction each: lock the session's sequence row, check
//!   the lease, insert, commit. Every commit waits for a WAL flush, and at 500
//!   parallel agents those flushes were the wall (production, 2026-10-29).
//!   Here appends wait on a queue, and one autocommitted statement stores
//!   everything queued: locks, fence checks, sequence reservation and insert
//!   in one round trip and one commit for many appends
//!   (`store_in_one_statement`). Each caller is answered after that commit,
//!   so an answered append is as durable as before.
//! - No timer. A lane commits as soon as its previous commit returns, taking
//!   whatever arrived meanwhile. Alone, an append costs what it did plus an
//!   in-process hand-off; under load it waits for at most the commit already
//!   in flight, and groups grow by themselves.
//! - A few lanes per server (`EVENT_GROUP_COMMIT_LANES`), each owning the
//!   sessions that hash to it, so commits overlap while one session's appends
//!   keep their order. Locks are taken in session id order in every lane, as
//!   in the single append, so lanes and server replicas cannot deadlock on
//!   them. Replicas need no coordination: each groups what it receives.
//! - Fences are checked together, and an append whose lease is gone is
//!   answered "lease lost" and left out; the others still commit.
//! - Any error the database reports (a bad payload, a deadlock) fails the
//!   group with nothing stored, and it then runs append by append, so the
//!   error reaches only the append that caused it. A connection lost
//!   mid-statement is not retried: its outcome is unknown, and each caller
//!   gets the error as a lone append's caller did.
//! - An append to a session without an `event_sequences` row yet (its first
//!   event) runs alone, after the group, in group order.
//! - Inside a command transaction the append keeps running on that
//!   transaction (`storage::transaction`), never on a lane.
//! - `EVENT_GROUP_COMMIT=off` turns grouping off (bench comparisons).

use std::hash::{Hash, Hasher};
use std::sync::{Arc, OnceLock};

use anyhow::Result;
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

use super::rows::{CreateEventRow, EventRow};
use super::{ActorLeaseFenceRejected, MAX_EVENT_BATCH};
use crate::storage::repositories::Database;
use everruns_server_macros::sql;
use everruns_contracts::typed_id::SessionId;

/// Appends one group may take.
const MAX_GROUP_APPENDS: usize = 64;
/// Events one group may take (an append larger than this runs alone).
const MAX_GROUP_EVENTS: usize = MAX_EVENT_BATCH;
const DEFAULT_LANES: usize = 4;
const LANE_CAPACITY: usize = 4096;

/// What an append's lease check is made against.
#[derive(Debug, Clone)]
pub(super) enum AppendCheck {
    /// The session actor lease `fence` held by `holder`.
    Lease { holder: String, fence: i64 },
    /// The detached tool call `tool_call_id` held by `holder`.
    Detached {
        holder: String,
        tool_call_id: String,
    },
}

impl AppendCheck {
    fn rejected(&self, session_id: Uuid) -> anyhow::Error {
        let (holder, fence) = match self {
            Self::Lease { holder, fence } => (holder.clone(), *fence),
            Self::Detached { holder, .. } => (holder.clone(), 0),
        };
        ActorLeaseFenceRejected {
            session_id: SessionId::from_uuid(session_id),
            holder,
            fence,
        }
        .into()
    }
}

struct Append {
    inputs: Vec<CreateEventRow>,
    check: AppendCheck,
    reply: oneshot::Sender<Result<Vec<EventRow>>>,
}

impl Append {
    fn sessions(&self) -> Vec<Uuid> {
        let mut sessions: Vec<Uuid> = self.inputs.iter().map(|r| r.session_id.uuid()).collect();
        sessions.sort();
        sessions.dedup();
        sessions
    }
}

/// The lanes of one database. Shared by every clone of it.
pub(crate) struct EventLogWriter {
    enabled: bool,
    lanes: Vec<OnceLock<mpsc::Sender<Append>>>,
}

impl EventLogWriter {
    pub(crate) fn from_env() -> Arc<Self> {
        let enabled = !std::env::var("EVENT_GROUP_COMMIT")
            .is_ok_and(|v| matches!(v.to_ascii_lowercase().as_str(), "off" | "0" | "false"));
        let lanes = std::env::var("EVENT_GROUP_COMMIT_LANES")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .filter(|n| *n > 0)
            .unwrap_or(DEFAULT_LANES);
        Arc::new(Self {
            enabled,
            lanes: (0..lanes).map(|_| OnceLock::new()).collect(),
        })
    }
}

impl Database {
    /// Store `inputs` under `check`, grouped with other appends when a lane
    /// can take it, alone otherwise.
    pub(super) async fn append_checked(
        &self,
        inputs: Vec<CreateEventRow>,
        check: AppendCheck,
    ) -> Result<Vec<EventRow>> {
        let Some(writer) = self.event_writer.as_ref() else {
            return self.append_alone(inputs, &check).await;
        };
        if !writer.enabled
            || inputs.is_empty()
            || inputs.len() > MAX_GROUP_EVENTS
            || crate::storage::transaction::in_transaction()
        {
            return self.append_alone(inputs, &check).await;
        }
        let lane = lane_of(inputs[0].session_id.uuid(), writer.lanes.len());
        let sender = writer.lanes[lane].get_or_init(|| {
            let (tx, rx) = mpsc::channel(LANE_CAPACITY);
            tokio::spawn(run_lane(self.without_writer(), rx));
            tx
        });
        let (reply, answer) = oneshot::channel();
        let append = Append {
            inputs,
            check,
            reply,
        };
        if let Err(mpsc::error::SendError(append)) = sender.send(append).await {
            return self.append_alone(append.inputs, &append.check).await;
        }
        answer
            .await
            .map_err(|_| anyhow::anyhow!("event log writer stopped before answering"))?
    }

    async fn append_alone(
        &self,
        inputs: Vec<CreateEventRow>,
        check: &AppendCheck,
    ) -> Result<Vec<EventRow>> {
        match check {
            AppendCheck::Lease { holder, fence } => {
                self.create_events_checked(inputs, holder, *fence, None)
                    .await
            }
            AppendCheck::Detached {
                holder,
                tool_call_id,
            } => {
                self.create_events_checked(inputs, holder, 0, Some(tool_call_id))
                    .await
            }
        }
    }
}

fn lane_of(session_id: Uuid, lanes: usize) -> usize {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    session_id.hash(&mut hasher);
    (hasher.finish() % lanes as u64) as usize
}

async fn run_lane(db: Database, mut queue: mpsc::Receiver<Append>) {
    while let Some(first) = queue.recv().await {
        let mut events = first.inputs.len();
        let mut group = vec![first];
        while group.len() < MAX_GROUP_APPENDS {
            match queue.try_recv() {
                Ok(next) if events + next.inputs.len() <= MAX_GROUP_EVENTS => {
                    events += next.inputs.len();
                    group.push(next);
                }
                Ok(next) => {
                    // Too big for this group: it starts the next one.
                    commit_group(&db, std::mem::take(&mut group)).await;
                    events = next.inputs.len();
                    group.push(next);
                }
                Err(_) => break,
            }
        }
        commit_group(&db, group).await;
    }
}

async fn commit_group(db: &Database, group: Vec<Append>) {
    if group.is_empty() {
        return;
    }
    match store_in_one_statement(db, &group).await {
        Ok(verdicts) => {
            // Appends to a session with no sequence row yet go alone, in
            // group order: the single append creates the row.
            let mut unsequenced = Vec::new();
            for (append, verdict) in group.into_iter().zip(verdicts) {
                match verdict {
                    Verdict::Stored(rows) => {
                        let _ = append.reply.send(Ok(rows));
                    }
                    Verdict::Lost(session) => {
                        let _ = append.reply.send(Err(append.check.rejected(session)));
                    }
                    Verdict::Unsequenced => unsequenced.push(append),
                }
            }
            for append in unsequenced {
                let result = db.append_alone(append.inputs, &append.check).await;
                let _ = append.reply.send(result);
            }
        }
        Err(StatementError::Unknown(error)) => {
            let message = format!("{error:#}");
            for append in group {
                let _ = append.reply.send(Err(anyhow::anyhow!(message.clone())));
            }
        }
        Err(StatementError::NotStored(error)) => {
            if group.len() > 1 {
                tracing::warn!(
                    appends = group.len(),
                    error = %format!("{error:#}"),
                    "grouped event append failed; storing each append alone"
                );
            }
            for append in group {
                let result = db.append_alone(append.inputs, &append.check).await;
                let _ = append.reply.send(result);
            }
        }
    }
}

/// What became of one append of a group.
enum Verdict {
    /// Stored; its rows, in input order.
    Stored(Vec<EventRow>),
    /// Refused: this session's lease (or detached call) is not the
    /// append's. Nothing of it was written.
    Lost(Uuid),
    /// A session it writes to has no `event_sequences` row yet; nothing of
    /// it was written.
    Unsequenced,
}

enum StatementError {
    /// The database refused the statement or it never ran: nothing landed,
    /// and every append may run again alone.
    NotStored(anyhow::Error),
    /// The connection failed mid-statement: whether it landed is unknown.
    Unknown(anyhow::Error),
}

/// The whole group as one autocommitted statement: lock the sessions'
/// sequence rows, check every append's lease, reserve sequences for the
/// accepted ones and insert their events.
///
/// Decision: one statement, not a transaction of four (lock, fence check,
/// insert, plus BEGIN and COMMIT). That was six statements per append in the
/// stress bench, the sixth a foreign-key check on `sessions`: the lock was a
/// no-op upsert of `event_sequences`, so the reservation updated a row
/// version this transaction had written, and PostgreSQL re-checks a foreign
/// key in that case. Here the lock is `FOR NO KEY UPDATE` and the row is
/// updated once.
///
/// Lock order stays the one every writer uses: a session's `event_sequences`
/// row before its actor row (or detached call row). The fence checks join
/// `locked`, so an actor row is locked only once its session's sequence row
/// already is; across sessions both go in session id order.
/// A session with no sequence row is left to the single append, which
/// creates it; a `FOR NO KEY UPDATE` cannot lock a row its snapshot does not
/// see.
async fn store_in_one_statement(
    db: &Database,
    group: &[Append],
) -> Result<Vec<Verdict>, StatementError> {
    let mut claim_idx = Vec::new();
    let mut claim_session = Vec::new();
    let mut claim_holder = Vec::new();
    let mut claim_fence: Vec<Option<i64>> = Vec::new();
    let mut claim_call: Vec<Option<String>> = Vec::new();
    for (index, append) in group.iter().enumerate() {
        for session in append.sessions() {
            claim_idx.push(index as i32);
            claim_session.push(session);
            match &append.check {
                AppendCheck::Lease { holder, fence } => {
                    claim_holder.push(holder.clone());
                    claim_fence.push(Some(*fence));
                    claim_call.push(None);
                }
                AppendCheck::Detached {
                    holder,
                    tool_call_id,
                } => {
                    claim_holder.push(holder.clone());
                    claim_fence.push(None);
                    claim_call.push(Some(tool_call_id.clone()));
                }
            }
        }
    }

    let mut qb = sqlx::QueryBuilder::<sqlx::Postgres>::new(
        "WITH input (ord, idx, session_id, event_type, ts, context, data, metadata, tags) AS (",
    );
    let inputs = group
        .iter()
        .enumerate()
        .flat_map(|(index, append)| append.inputs.iter().map(move |input| (index, input)));
    qb.push_values(inputs.enumerate(), |mut row, (ord, (index, input))| {
        row.push_bind(ord as i64)
            .push_unseparated("::BIGINT")
            .push_bind(index as i32)
            .push_unseparated("::INTEGER")
            .push_bind(input.session_id.uuid())
            .push_unseparated("::UUID")
            .push_bind(&input.event_type)
            .push_unseparated("::TEXT")
            .push_bind(input.ts)
            .push_unseparated("::TIMESTAMPTZ")
            .push_bind(&input.context)
            .push_unseparated("::JSONB")
            .push_bind(&input.data)
            .push_unseparated("::JSONB")
            .push_bind(&input.metadata)
            .push_unseparated("::JSONB")
            .push_bind(&input.tags)
            .push_unseparated("::TEXT[]");
    });
    qb.push("), claims AS (SELECT * FROM UNNEST(");
    qb.push_bind(&claim_idx);
    qb.push("::INTEGER[], ");
    qb.push_bind(&claim_session);
    qb.push("::UUID[], ");
    qb.push_bind(&claim_holder);
    qb.push("::TEXT[], ");
    qb.push_bind(&claim_fence);
    qb.push("::BIGINT[], ");
    qb.push_bind(&claim_call);
    qb.push("::TEXT[]) AS c(idx, session_id, holder, fence, tool_call_id))");
    qb.push(sql!(
        r#"
        , locked AS MATERIALIZED (
            SELECT es.session_id FROM event_sequences es
            WHERE es.session_id IN (SELECT session_id FROM claims)
            ORDER BY es.session_id
            FOR NO KEY UPDATE
        ),
        held_lease AS MATERIALIZED (
            SELECT c.idx, a.session_id
            FROM claims c
            JOIN locked l ON l.session_id = c.session_id
            JOIN session_actors a ON a.session_id = c.session_id
             AND c.fence IS NOT NULL
             AND a.lease_holder = c.holder AND a.lease_fence = c.fence
             AND a.lease_expires_at > NOW()
            ORDER BY a.session_id
            FOR SHARE OF a
        ),
        held_detached AS MATERIALIZED (
            SELECT c.idx, d.session_id
            FROM claims c
            JOIN locked l ON l.session_id = c.session_id
            JOIN detached_tool_calls d ON d.session_id = c.session_id
             AND c.tool_call_id IS NOT NULL
             AND d.holder = c.holder AND d.tool_call_id = c.tool_call_id
             AND d.expires_at > NOW()
            ORDER BY d.session_id
            FOR SHARE OF d
        ),
        held AS (
            SELECT idx, session_id FROM held_lease
            UNION
            SELECT idx, session_id FROM held_detached
        ),
        verdict AS MATERIALIZED (
            SELECT c.idx,
                   bool_and(l.session_id IS NOT NULL) AS sequenced,
                   bool_and(h.idx IS NOT NULL) AS held,
                   (min(c.session_id::TEXT) FILTER (WHERE h.idx IS NULL))::UUID AS lost
            FROM claims c
            LEFT JOIN locked l ON l.session_id = c.session_id
            LEFT JOIN held h ON h.idx = c.idx AND h.session_id = c.session_id
            GROUP BY c.idx
        ),
        keep AS (
            SELECT i.* FROM input i JOIN verdict v ON v.idx = i.idx
            WHERE v.sequenced AND v.held
        ),
        counts AS (
            SELECT session_id, COUNT(*)::INTEGER AS n FROM keep GROUP BY session_id
        ),
        reserved AS (
            UPDATE event_sequences es
            SET next_sequence = es.next_sequence + counts.n, updated_at = NOW()
            FROM counts
            WHERE es.session_id = counts.session_id
            RETURNING es.session_id, es.next_sequence
        ),
        planned AS MATERIALIZED (
            SELECT k.ord, k.idx, k.session_id,
                   r.next_sequence - c.n - 1
                       + (ROW_NUMBER() OVER (PARTITION BY k.session_id ORDER BY k.ord))::INTEGER
                       AS sequence,
                   k.event_type, k.ts, k.context, k.data, k.metadata, k.tags
            FROM keep k
            JOIN counts c USING (session_id)
            JOIN reserved r USING (session_id)
        ),
        ins AS (
            INSERT INTO events (session_id, sequence, event_type, ts, context, data, metadata, tags)
            SELECT session_id, sequence, event_type, ts, context, data, metadata, tags
            FROM planned ORDER BY ord
            RETURNING {EventRow}
        )
        SELECT v.idx AS append_idx, v.sequenced AS append_sequenced, v.lost AS append_lost,
               ins.*
        FROM verdict v
        LEFT JOIN planned p ON p.idx = v.idx
        LEFT JOIN ins ON ins.session_id = p.session_id AND ins.sequence = p.sequence
        ORDER BY v.idx, p.ord
        "#
    ));

    let rows = match qb.build().fetch_all(&db.pool).await {
        Ok(rows) => rows,
        Err(error) => {
            return Err(match error {
                sqlx::Error::Database(_) | sqlx::Error::PoolTimedOut | sqlx::Error::PoolClosed => {
                    StatementError::NotStored(error.into())
                }
                other => StatementError::Unknown(other.into()),
            });
        }
    };
    read_verdicts(group, rows).map_err(StatementError::Unknown)
}

/// One verdict per append from [`store_in_one_statement`]'s rows: one row
/// per stored event, and one bare row per append that stored nothing.
fn read_verdicts(group: &[Append], rows: Vec<sqlx::postgres::PgRow>) -> Result<Vec<Verdict>> {
    use sqlx::{FromRow, Row};
    let mut verdicts: Vec<Option<Verdict>> = (0..group.len()).map(|_| None).collect();
    for row in rows {
        let index: i32 = row.try_get("append_idx")?;
        let index = usize::try_from(index)?;
        anyhow::ensure!(index < group.len(), "verdict for an append not in the group");
        let sequenced: bool = row.try_get("append_sequenced")?;
        let lost: Option<Uuid> = row.try_get("append_lost")?;
        let id: Option<Uuid> = row.try_get("id")?;
        let verdict = &mut verdicts[index];
        match (id, sequenced, lost) {
            (Some(_), _, _) => {
                let event = EventRow::from_row(&row)?;
                match verdict {
                    Some(Verdict::Stored(rows)) => rows.push(event),
                    _ => *verdict = Some(Verdict::Stored(vec![event])),
                }
            }
            (None, false, _) => *verdict = Some(Verdict::Unsequenced),
            (None, true, Some(session)) => *verdict = Some(Verdict::Lost(session)),
            (None, true, None) => anyhow::bail!("an accepted append stored no events"),
        }
    }
    verdicts
        .into_iter()
        .zip(group)
        .map(|(verdict, append)| match verdict {
            Some(Verdict::Stored(rows)) => {
                anyhow::ensure!(
                    rows.len() == append.inputs.len(),
                    "stored {} events of an append of {}",
                    rows.len(),
                    append.inputs.len()
                );
                Ok(Verdict::Stored(rows))
            }
            Some(other) => Ok(other),
            None => anyhow::bail!("no verdict for an append"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::StorageBackend;
    use crate::storage::test_database::test_session_row;
    use std::time::Duration;

    const ORG: i64 = 2;
    const TTL: Duration = Duration::from_secs(30);

    /// A session whose actor `holder` leases; returns it and the fence. Its
    /// first event (`session.created`, sequence 1) is already stored, as
    /// intake does before any lease.
    async fn leased(db: &StorageBackend, holder: &str) -> (SessionId, i64) {
        let (id, fence) = leased_without_events(db, holder).await;
        db.database()
            .create_event(row(id, "session.created"))
            .await
            .unwrap();
        (id, fence)
    }

    /// [`leased`] before any event: the session has no sequence row yet.
    async fn leased_without_events(db: &StorageBackend, holder: &str) -> (SessionId, i64) {
        let id = db.create_session(test_session_row(ORG)).await.unwrap().id;
        db.mark_session_actor_runnable(id, ORG, 1).await.unwrap();
        let lease = db.lease_session_actors(holder, 1, TTL, 32).await.unwrap();
        assert_eq!(lease[0].session_id, id);
        (id, lease[0].fence)
    }

    fn row(session_id: SessionId, event_type: &str) -> CreateEventRow {
        CreateEventRow {
            session_id,
            event_type: event_type.to_string(),
            ts: chrono::Utc::now(),
            context: serde_json::json!({}),
            data: serde_json::json!({}),
            metadata: None,
            tags: None,
        }
    }

    fn append(
        inputs: Vec<CreateEventRow>,
        holder: &str,
        fence: i64,
    ) -> (Append, oneshot::Receiver<Result<Vec<EventRow>>>) {
        let (reply, answer) = oneshot::channel();
        (
            Append {
                inputs,
                check: AppendCheck::Lease {
                    holder: holder.to_string(),
                    fence,
                },
                reply,
            },
            answer,
        )
    }

    async fn types(db: &StorageBackend, session_id: SessionId) -> Vec<String> {
        sqlx::query_scalar("SELECT event_type FROM events WHERE session_id = $1 ORDER BY sequence")
            .bind(session_id)
            .fetch_all(db.database().pool())
            .await
            .unwrap()
    }

    fn rejected(result: &Result<Vec<EventRow>>) -> bool {
        result
            .as_ref()
            .err()
            .and_then(|e| e.downcast_ref::<ActorLeaseFenceRejected>())
            .is_some()
    }

    #[tokio::test]
    async fn a_group_stores_every_live_append_and_refuses_the_stale_one() {
        let db = StorageBackend::test_database();
        let (a, fa) = leased(&db, "w1").await;
        let (b, fb) = leased(&db, "w2").await;
        let (c, fc) = leased(&db, "w1").await;
        let (ga, ra) = append(
            vec![row(a, "turn.started"), row(a, "reason.started")],
            "w1",
            fa,
        );
        let (gb, rb) = append(vec![row(b, "turn.started")], "w2", fb);
        // Right holder, a fence it no longer holds.
        let (gc, rc) = append(vec![row(c, "turn.started")], "w1", fc + 1);
        // The same session again, after its first append in the group.
        let (ga2, ra2) = append(vec![row(a, "tool.completed")], "w1", fa);
        commit_group(db.database(), vec![ga, gb, gc, ga2]).await;

        let stored_a = ra.await.unwrap().unwrap();
        assert_eq!(
            stored_a.iter().map(|r| r.sequence).collect::<Vec<_>>(),
            vec![2, 3],
            "an append gets its own rows back, in order"
        );
        assert_eq!(rb.await.unwrap().unwrap().len(), 1);
        assert!(rejected(&rc.await.unwrap()));
        assert_eq!(ra2.await.unwrap().unwrap()[0].sequence, 4);
        assert_eq!(
            types(&db, a).await,
            [
                "session.created",
                "turn.started",
                "reason.started",
                "tool.completed"
            ]
        );
        assert_eq!(types(&db, b).await, ["session.created", "turn.started"]);
        assert_eq!(
            types(&db, c).await,
            ["session.created"],
            "the stale append wrote nothing"
        );
    }

    #[tokio::test]
    async fn a_failing_append_fails_alone_and_the_rest_of_its_group_lands() {
        let db = StorageBackend::test_database();
        let (a, fa) = leased(&db, "w1").await;
        let (b, fb) = leased(&db, "w1").await;
        let (ga, ra) = append(vec![row(a, "turn.started")], "w1", fa);
        // PostgreSQL refuses a NUL byte in text, so this insert fails after
        // its fence passed, failing the shared transaction.
        let (gb, rb) = append(vec![row(b, "bad\0type")], "w1", fb);
        commit_group(db.database(), vec![ga, gb]).await;

        assert_eq!(ra.await.unwrap().unwrap().len(), 1);
        let error = rb.await.unwrap().expect_err("the bad payload fails");
        assert!(!rejected(&Err(error)), "a storage error, not a lost lease");
        assert_eq!(types(&db, a).await, ["session.created", "turn.started"]);
        assert_eq!(types(&db, b).await, ["session.created"]);
    }

    #[tokio::test]
    async fn detached_appends_are_checked_against_their_call() {
        let db = StorageBackend::test_database();
        let (a, fa) = leased(&db, "w1").await;
        let (gl, rl) = append(vec![row(a, "turn.started")], "w1", fa);
        let (reply, rd) = oneshot::channel();
        let gd = Append {
            inputs: vec![row(a, "tool.completed")],
            check: AppendCheck::Detached {
                holder: "w1".into(),
                tool_call_id: "call-not-detached".into(),
            },
            reply,
        };
        commit_group(db.database(), vec![gl, gd]).await;
        assert_eq!(rl.await.unwrap().unwrap().len(), 1);
        assert!(
            rejected(&rd.await.unwrap()),
            "no detached call row holds it"
        );
        assert_eq!(types(&db, a).await, ["session.created", "turn.started"]);
    }

    #[tokio::test]
    async fn concurrent_appends_through_the_lanes_all_land_in_order() {
        let db = StorageBackend::test_database();
        let mut sessions = Vec::new();
        for _ in 0..12 {
            sessions.push(leased(&db, "w1").await);
        }
        let mut writes = tokio::task::JoinSet::new();
        for (id, fence) in sessions.clone() {
            let db = db.clone();
            writes.spawn(async move {
                for n in 0..3 {
                    db.create_events_fenced(vec![row(id, &format!("step.{n}"))], "w1", fence)
                        .await
                        .unwrap();
                }
            });
        }
        while let Some(done) = writes.join_next().await {
            done.unwrap();
        }
        for (id, _) in sessions {
            assert_eq!(
                types(&db, id).await,
                ["session.created", "step.0", "step.1", "step.2"]
            );
        }
    }

    #[tokio::test]
    async fn a_first_event_goes_alone_and_the_rest_of_its_group_still_lands() {
        let db = StorageBackend::test_database();
        let (fresh, ff) = leased_without_events(&db, "w1").await;
        let (old, fo) = leased(&db, "w1").await;
        let (gf, rf) = append(vec![row(fresh, "turn.started")], "w1", ff);
        let (go, ro) = append(vec![row(old, "turn.started")], "w1", fo);
        let (gf2, rf2) = append(vec![row(fresh, "reason.started")], "w1", ff);
        commit_group(db.database(), vec![gf, go, gf2]).await;

        assert_eq!(rf.await.unwrap().unwrap()[0].sequence, 1);
        assert_eq!(rf2.await.unwrap().unwrap()[0].sequence, 2);
        assert_eq!(ro.await.unwrap().unwrap()[0].sequence, 2);
        assert_eq!(types(&db, fresh).await, ["turn.started", "reason.started"]);

        // Without a lease, a first event is refused like any other.
        let (stale, fs) = leased_without_events(&db, "w1").await;
        let (gs, rs) = append(vec![row(stale, "turn.started")], "w1", fs + 1);
        commit_group(db.database(), vec![gs]).await;
        assert!(rejected(&rs.await.unwrap()));
        assert!(types(&db, stale).await.is_empty());
    }

    /// Fenced appends grouped in one statement, message intake (unfenced,
    /// sequence row then actor row) and lease renewals, all on the same
    /// sessions at once: nothing deadlocks, and every session's sequences stay
    /// gap-free.
    #[tokio::test]
    async fn grouped_appends_intake_and_renewals_do_not_deadlock() {
        let db = StorageBackend::test_database();
        let mut sessions = Vec::new();
        for _ in 0..8 {
            sessions.push(leased(&db, "w1").await);
        }
        let mut work = tokio::task::JoinSet::new();
        for (id, fence) in sessions.clone() {
            let fenced = db.clone();
            work.spawn(async move {
                for n in 0..5 {
                    fenced
                        .create_events_fenced(vec![row(id, &format!("step.{n}"))], "w1", fence)
                        .await
                        .unwrap();
                }
            });
            let intake = db.clone();
            work.spawn(async move {
                for _ in 0..5 {
                    intake
                        .database()
                        .create_event(row(id, "input.message"))
                        .await
                        .unwrap();
                    intake.mark_session_actor_runnable(id, ORG, 1).await.unwrap();
                }
            });
        }
        let renewals = db.clone();
        let leases: Vec<crate::storage::ActorLeaseRenewalItem> = sessions
            .iter()
            .map(|(id, fence)| crate::storage::ActorLeaseRenewalItem {
                session_id: *id,
                fence: *fence,
                consumed_seq: 0,
            })
            .collect();
        work.spawn(async move {
            for _ in 0..5 {
                renewals
                    .renew_session_actor_leases("w1", TTL, &leases)
                    .await
                    .unwrap();
            }
        });
        while let Some(done) = work.join_next().await {
            done.unwrap();
        }
        for (id, _) in sessions {
            let sequences: Vec<i32> =
                sqlx::query_scalar("SELECT sequence FROM events WHERE session_id = $1 ORDER BY sequence")
                    .bind(id)
                    .fetch_all(db.database().pool())
                    .await
                    .unwrap();
            assert_eq!(sequences, (1..=11).collect::<Vec<i32>>());
        }
    }
}

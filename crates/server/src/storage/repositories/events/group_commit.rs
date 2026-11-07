//! Group commit for fenced event appends.
//!
//! Decisions:
//! - A worker's event append (`create_events_fenced`, `create_events_detached`)
//!   used to be one transaction each: lock the session's sequence row, check
//!   the lease, insert, commit. Every commit waits for a WAL flush, and at 500
//!   parallel agents those flushes were the wall (production, 2026-10-29).
//!   Here appends wait on a queue, and one transaction stores everything
//!   queued: one lock statement, one fence check, one insert, one commit for
//!   many appends. Each caller is answered after that commit, so an answered
//!   append is as durable as before.
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
//! - Any other failure before the commit (a bad payload, a deadlock) fails
//!   the group, which then runs append by append, so the error reaches only
//!   the append that caused it. A failed commit is not retried: its outcome is
//!   unknown, and each caller gets the error as a lone append's caller did.
//! - Inside a command transaction the append keeps running on that
//!   transaction (`storage::transaction`), never on a lane.
//! - `EVENT_GROUP_COMMIT=off` turns grouping off (bench comparisons).

use std::hash::{Hash, Hasher};
use std::sync::{Arc, OnceLock};

use anyhow::Result;
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

use super::rows::{CreateEventRow, EventRow};
use super::{ActorLeaseFenceRejected, MAX_EVENT_BATCH, batch_insert_query, order_batch_rows};
use crate::storage::repositories::Database;
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
    if group.len() == 1 {
        let append = group.into_iter().next().expect("one append");
        let result = db.append_alone(append.inputs, &append.check).await;
        let _ = append.reply.send(result);
        return;
    }
    match try_group(db, &group).await {
        Ok(GroupOutcome::Stored { results }) => {
            for (append, result) in group.into_iter().zip(results) {
                let _ = append.reply.send(result);
            }
        }
        Ok(GroupOutcome::CommitFailed(error)) => {
            let message = format!("{error:#}");
            for append in group {
                let _ = append.reply.send(Err(anyhow::anyhow!(message.clone())));
            }
        }
        Err(error) => {
            tracing::warn!(
                appends = group.len(),
                error = %format!("{error:#}"),
                "grouped event append failed; storing each append alone"
            );
            for append in group {
                let result = db.append_alone(append.inputs, &append.check).await;
                let _ = append.reply.send(result);
            }
        }
    }
}

enum GroupOutcome {
    /// One result per append, in group order.
    Stored { results: Vec<Result<Vec<EventRow>>> },
    /// The commit itself failed; whether it landed is unknown.
    CommitFailed(anyhow::Error),
}

/// One transaction for the whole group. `Err` means nothing was committed
/// and every append may run again alone.
async fn try_group(db: &Database, group: &[Append]) -> Result<GroupOutcome> {
    let per_append: Vec<Vec<Uuid>> = group.iter().map(Append::sessions).collect();
    let mut sessions: Vec<Uuid> = per_append.iter().flatten().copied().collect();
    sessions.sort();
    sessions.dedup();

    let mut tx = db.pool.begin().await?;
    // The single append's create-or-lock, for every session of the group, in
    // session id order (see `Database::create_events_checked`).
    sqlx::query(
        "INSERT INTO event_sequences (session_id, next_sequence, updated_at) \
         SELECT id, 1, NOW() FROM UNNEST($1::uuid[]) AS id ORDER BY id \
         ON CONFLICT (session_id) DO UPDATE SET next_sequence = event_sequences.next_sequence",
    )
    .bind(&sessions)
    .execute(&mut *tx)
    .await?;

    // Every (append, session) pair's claim, checked in one statement per kind.
    let mut lease_idx = Vec::new();
    let mut lease_session = Vec::new();
    let mut lease_holder = Vec::new();
    let mut lease_fence = Vec::new();
    let mut detached_idx = Vec::new();
    let mut detached_session = Vec::new();
    let mut detached_holder = Vec::new();
    let mut detached_call = Vec::new();
    for (index, (append, sessions)) in group.iter().zip(&per_append).enumerate() {
        for session in sessions {
            match &append.check {
                AppendCheck::Lease { holder, fence } => {
                    lease_idx.push(index as i32);
                    lease_session.push(*session);
                    lease_holder.push(holder.clone());
                    lease_fence.push(*fence);
                }
                AppendCheck::Detached {
                    holder,
                    tool_call_id,
                } => {
                    detached_idx.push(index as i32);
                    detached_session.push(*session);
                    detached_holder.push(holder.clone());
                    detached_call.push(tool_call_id.clone());
                }
            }
        }
    }
    let mut held: Vec<(i32, Uuid)> = Vec::new();
    if !lease_idx.is_empty() {
        held.extend(
            sqlx::query_as::<_, (i32, Uuid)>(
                "SELECT c.idx, a.session_id \
                 FROM UNNEST($1::int[], $2::uuid[], $3::text[], $4::bigint[]) \
                      AS c(idx, session_id, holder, fence) \
                 JOIN session_actors a ON a.session_id = c.session_id \
                  AND a.lease_holder = c.holder AND a.lease_fence = c.fence \
                  AND a.lease_expires_at > NOW() \
                 ORDER BY a.session_id FOR SHARE OF a",
            )
            .bind(&lease_idx)
            .bind(&lease_session)
            .bind(&lease_holder)
            .bind(&lease_fence)
            .fetch_all(&mut *tx)
            .await?,
        );
    }
    if !detached_idx.is_empty() {
        held.extend(
            sqlx::query_as::<_, (i32, Uuid)>(
                "SELECT c.idx, d.session_id \
                 FROM UNNEST($1::int[], $2::uuid[], $3::text[], $4::text[]) \
                      AS c(idx, session_id, holder, tool_call_id) \
                 JOIN detached_tool_calls d ON d.session_id = c.session_id \
                  AND d.holder = c.holder AND d.tool_call_id = c.tool_call_id \
                  AND d.expires_at > NOW() \
                 ORDER BY d.session_id FOR SHARE OF d",
            )
            .bind(&detached_idx)
            .bind(&detached_session)
            .bind(&detached_holder)
            .bind(&detached_call)
            .fetch_all(&mut *tx)
            .await?,
        );
    }

    // Accepted appends' events, in group order, as one insert.
    let mut verdicts: Vec<Option<Uuid>> = Vec::with_capacity(group.len());
    let mut accepted: Vec<CreateEventRow> = Vec::new();
    for (index, sessions) in per_append.iter().enumerate() {
        let lost = sessions
            .iter()
            .find(|session| !held.contains(&(index as i32, **session)))
            .copied();
        if lost.is_none() {
            accepted.extend(group[index].inputs.iter().cloned());
        }
        verdicts.push(lost);
    }
    let mut stored = Vec::with_capacity(accepted.len());
    for chunk in accepted.chunks(MAX_EVENT_BATCH) {
        let rows = batch_insert_query(chunk)
            .build_query_as::<EventRow>()
            .fetch_all(&mut *tx)
            .await?;
        stored.extend(order_batch_rows(chunk, rows)?);
    }
    if let Err(error) = tx.commit().await {
        return Ok(GroupOutcome::CommitFailed(error.into()));
    }
    let mut rows = stored.into_iter();
    let results = group
        .iter()
        .zip(verdicts)
        .map(|(append, lost)| match lost {
            Some(session) => Err(append.check.rejected(session)),
            None => Ok(rows.by_ref().take(append.inputs.len()).collect()),
        })
        .collect();
    Ok(GroupOutcome::Stored { results })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::StorageBackend;
    use crate::storage::test_database::test_session_row;
    use std::time::Duration;

    const ORG: i64 = 2;
    const TTL: Duration = Duration::from_secs(30);

    /// A session whose actor `holder` leases; returns it and the fence.
    async fn leased(db: &StorageBackend, holder: &str) -> (SessionId, i64) {
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
            vec![1, 2],
            "an append gets its own rows back, in order"
        );
        assert_eq!(rb.await.unwrap().unwrap().len(), 1);
        assert!(rejected(&rc.await.unwrap()));
        assert_eq!(ra2.await.unwrap().unwrap()[0].sequence, 3);
        assert_eq!(
            types(&db, a).await,
            ["turn.started", "reason.started", "tool.completed"]
        );
        assert_eq!(types(&db, b).await, ["turn.started"]);
        assert!(
            types(&db, c).await.is_empty(),
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
        assert_eq!(types(&db, a).await, ["turn.started"]);
        assert!(types(&db, b).await.is_empty());
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
        assert_eq!(types(&db, a).await, ["turn.started"]);
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
            assert_eq!(types(&db, id).await, ["step.0", "step.1", "step.2"]);
        }
    }
}

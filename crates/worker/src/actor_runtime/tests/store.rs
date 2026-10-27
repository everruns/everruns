//! An in-memory actor index over an in-process runtime's log, with the
//! semantics of the control plane's `session_actors` repository (see
//! crates/server/src/storage/repositories/session_actors). The repository's
//! own tests pin those semantics against PostgreSQL; this one lets the actor
//! runtime run against them without a database.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use everruns_contracts::error::{AgentLoopError, Result};
use everruns_contracts::typed_id::SessionId;

use crate::actor_lease_store::{
    ActorJournal, ActorLeaseStore, ActorRelease, ActorRenewal, DetachedCallKey, LeasedActor,
    SessionActorStore,
};
use crate::core::TURN_STARTED;
use crate::host::InProcessRuntime;
use everruns_contracts::typed_id::TurnId;

#[derive(Debug, Clone, Default)]
pub(super) struct Row {
    pub org_id: i64,
    pub inbox_seq: i64,
    pub consumed_seq: i64,
    pub next_input_seq: i64,
    pub attempts: u32,
    /// Order in the runnable line.
    pub runnable: Option<u64>,
    pub holder: Option<String>,
    pub fence: u64,
    pub expires: Option<Instant>,
    pub cancel_requested: bool,
    /// The inbox position the pending cancel targets.
    pub cancel_inbox_seq: i64,
    pub cancel_reported: bool,
}

impl Row {
    fn leased(&self) -> bool {
        self.holder.is_some() && self.expires.is_some_and(|at| at > Instant::now())
    }

    fn cancel(&self) -> Option<i64> {
        self.cancel_requested.then_some(self.cancel_inbox_seq)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Cancel {
    Requested,
    Withdrawn,
    Idle,
}

pub(super) struct MemoryActors {
    runtime: InProcessRuntime,
    rows: Mutex<HashMap<SessionId, Row>>,
    line: Mutex<u64>,
    renews: AtomicUsize,
    /// Renews still to fail with a store error, as during a control-plane
    /// outage.
    failing_renews: AtomicUsize,
    /// Journal reads still to fail with a store error (a pool timeout).
    failing_reads: AtomicUsize,
    /// Detached tool call records: holder and expiry.
    detached: Mutex<HashMap<DetachedCallKey, (String, Instant)>>,
}

impl MemoryActors {
    pub(super) fn new(runtime: InProcessRuntime) -> Self {
        Self {
            runtime,
            rows: Mutex::new(HashMap::new()),
            line: Mutex::new(0),
            renews: AtomicUsize::new(0),
            failing_renews: AtomicUsize::new(0),
            failing_reads: AtomicUsize::new(0),
            detached: Mutex::new(HashMap::new()),
        }
    }

    /// The tool call ids of `session_id`'s detached records.
    pub(super) fn detached(&self, session_id: SessionId) -> Vec<String> {
        let mut ids: Vec<_> = self
            .detached
            .lock()
            .unwrap()
            .keys()
            .filter(|key| key.session_id == session_id)
            .map(|key| key.tool_call_id.clone())
            .collect();
        ids.sort();
        ids
    }

    fn next_in_line(&self) -> u64 {
        let mut line = self.line.lock().unwrap();
        *line += 1;
        *line
    }

    /// Fail the next `count` renews with a store error.
    pub(super) fn fail_renews(&self, count: usize) {
        self.failing_renews.store(count, Ordering::SeqCst);
    }

    /// Fail the next `count` journal reads with a store error.
    pub(super) fn fail_reads(&self, count: usize) {
        self.failing_reads.store(count, Ordering::SeqCst);
    }

    /// Renews that reached the index, by any holder.
    pub(super) fn renews(&self) -> usize {
        self.renews.load(Ordering::SeqCst)
    }

    pub(super) fn row(&self, session_id: SessionId) -> Row {
        self.rows
            .lock()
            .unwrap()
            .get(&session_id)
            .cloned()
            .unwrap_or_default()
    }

    /// Intake: input at `sequence` was stored.
    pub(super) fn mark_runnable(&self, session_id: SessionId, org_id: i64, sequence: i64) {
        let order = self.next_in_line();
        let mut rows = self.rows.lock().unwrap();
        let row = rows.entry(session_id).or_insert_with(|| Row {
            org_id,
            ..Row::default()
        });
        row.inbox_seq = row.inbox_seq.max(sequence);
        if row.next_input_seq <= row.consumed_seq && sequence > row.consumed_seq {
            row.next_input_seq = sequence;
        }
        row.runnable.get_or_insert(order);
    }

    /// A resolution was stored: the inbox points at the log's head.
    pub(super) async fn mark_resumable(&self, session_id: SessionId, org_id: i64) {
        let head = self
            .runtime
            .events()
            .await
            .unwrap()
            .iter()
            .filter_map(|event| event.sequence)
            .max()
            .map_or(0, i64::from);
        let order = self.next_in_line();
        let mut rows = self.rows.lock().unwrap();
        let row = rows.entry(session_id).or_insert_with(|| Row {
            org_id,
            ..Row::default()
        });
        row.inbox_seq = row.inbox_seq.max(head);
        row.runnable.get_or_insert(order);
    }

    pub(super) fn cancel(&self, session_id: SessionId) -> Cancel {
        self.detached
            .lock()
            .unwrap()
            .retain(|key, _| key.session_id != session_id);
        let mut rows = self.rows.lock().unwrap();
        let Some(row) = rows
            .get_mut(&session_id)
            .filter(|row| row.runnable.is_some() || row.holder.is_some())
        else {
            return Cancel::Idle;
        };
        if row.leased() {
            row.cancel_requested = true;
            row.cancel_inbox_seq = row.inbox_seq;
            row.cancel_reported = false;
            Cancel::Requested
        } else {
            row.runnable = None;
            row.consumed_seq = row.consumed_seq.max(row.inbox_seq);
            row.holder = None;
            row.expires = None;
            row.attempts = 0;
            row.cancel_requested = false;
            row.cancel_reported = false;
            Cancel::Withdrawn
        }
    }

    /// A renew committed the cursor up to `sequence` before the holder died.
    pub(super) fn commit(&self, session_id: SessionId, sequence: i64) {
        if let Some(row) = self.rows.lock().unwrap().get_mut(&session_id) {
            row.consumed_seq = row.consumed_seq.max(sequence);
        }
    }

    /// The session is deleted: its actor row goes with it (FK cascade).
    pub(super) fn delete(&self, session_id: SessionId) {
        self.rows.lock().unwrap().remove(&session_id);
    }

    /// The holder's lease runs out now (its worker died).
    pub(super) fn expire(&self, session_id: SessionId) {
        if let Some(row) = self.rows.lock().unwrap().get_mut(&session_id) {
            row.expires = Some(Instant::now() - Duration::from_millis(1));
        }
    }
}

#[async_trait]
impl ActorLeaseStore for MemoryActors {
    async fn lease_actors(
        &self,
        holder: &str,
        limit: u32,
        ttl: Duration,
        _max_leased_per_org: Option<u32>,
    ) -> Result<Vec<LeasedActor>> {
        // Expired detached records make their actors runnable; live ones keep
        // their actors from being leased.
        let expired: Vec<SessionId> = {
            let mut detached = self.detached.lock().unwrap();
            let now = Instant::now();
            let expired = detached
                .iter()
                .filter(|(_, (_, expires))| *expires <= now)
                .map(|(key, _)| key.session_id)
                .collect();
            detached.retain(|_, (_, expires)| *expires > now);
            expired
        };
        for session_id in expired {
            self.mark_resumable(session_id, 0).await;
        }
        let busy: Vec<SessionId> = self
            .detached
            .lock()
            .unwrap()
            .keys()
            .map(|key| key.session_id)
            .collect();
        let mut rows = self.rows.lock().unwrap();
        let mut candidates: Vec<(u64, SessionId)> = rows
            .iter()
            .filter(|(id, row)| row.runnable.is_some() && !row.leased() && !busy.contains(id))
            .map(|(id, row)| (row.runnable.unwrap_or_default(), *id))
            .collect();
        candidates.sort_by_key(|(order, _)| *order);
        let mut leased = Vec::new();
        for (_, session_id) in candidates.into_iter().take(limit as usize) {
            let row = rows.get_mut(&session_id).unwrap();
            row.holder = Some(holder.to_string());
            row.fence += 1;
            row.expires = Some(Instant::now() + ttl);
            row.cancel_reported = row.cancel_requested;
            row.attempts += 1;
            leased.push(LeasedActor {
                session_id,
                org_id: row.org_id,
                fence: row.fence,
                inbox_seq: row.inbox_seq,
                cancel_inbox_seq: row.cancel(),
                consumed_seq: row.consumed_seq,
                next_input_seq: row.next_input_seq,
                attempts: row.attempts,
            });
        }
        Ok(leased)
    }

    async fn detach_tool_call(
        &self,
        call: &DetachedCallKey,
        _turn_id: TurnId,
        holder: &str,
        fence: u64,
        ttl: Duration,
    ) -> Result<bool> {
        let held = self
            .rows
            .lock()
            .unwrap()
            .get(&call.session_id)
            .is_some_and(|row| {
                row.leased() && row.holder.as_deref() == Some(holder) && row.fence == fence
            });
        if held {
            self.detached
                .lock()
                .unwrap()
                .insert(call.clone(), (holder.to_string(), Instant::now() + ttl));
        }
        Ok(held)
    }

    async fn renew_detached_tool_calls(
        &self,
        holder: &str,
        ttl: Duration,
        calls: &[DetachedCallKey],
    ) -> Result<Vec<DetachedCallKey>> {
        let mut detached = self.detached.lock().unwrap();
        let now = Instant::now();
        Ok(calls
            .iter()
            .filter(|call| match detached.get_mut(*call) {
                Some((owner, expires)) if owner == holder && *expires > now => {
                    *expires = now + ttl;
                    true
                }
                _ => false,
            })
            .cloned()
            .collect())
    }

    async fn complete_detached_tool_call(
        &self,
        call: &DetachedCallKey,
        holder: &str,
    ) -> Result<Option<bool>> {
        let last = {
            let mut detached = self.detached.lock().unwrap();
            match detached.get(call) {
                Some((owner, expires)) if owner == holder && *expires > Instant::now() => {}
                _ => return Ok(None),
            }
            detached.remove(call);
            !detached.keys().any(|key| key.session_id == call.session_id)
        };
        if last {
            self.mark_resumable(call.session_id, 0).await;
        }
        Ok(Some(last))
    }

    async fn expire_detached_tool_calls(
        &self,
        holder: &str,
        calls: &[DetachedCallKey],
    ) -> Result<()> {
        let mut detached = self.detached.lock().unwrap();
        for call in calls {
            if let Some((owner, expires)) = detached.get_mut(call)
                && owner == holder
            {
                *expires = Instant::now();
            }
        }
        Ok(())
    }
}

#[async_trait]
impl SessionActorStore for MemoryActors {
    async fn renew_actor_lease(
        &self,
        session_id: SessionId,
        holder: &str,
        fence: u64,
        ttl: Duration,
        consumed_seq: i64,
    ) -> Result<Option<ActorRenewal>> {
        self.renews.fetch_add(1, Ordering::SeqCst);
        if self
            .failing_renews
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |left| {
                left.checked_sub(1)
            })
            .is_ok()
        {
            return Err(AgentLoopError::store("control plane unavailable"));
        }
        let mut rows = self.rows.lock().unwrap();
        // No row at all: the session was deleted (EVE-1235).
        let Some(row) = rows.get_mut(&session_id) else {
            return Err(AgentLoopError::session_not_found(session_id));
        };
        if row.holder.as_deref() != Some(holder) || row.fence != fence {
            return Ok(None);
        }
        row.expires = Some(Instant::now() + ttl);
        row.cancel_reported = row.cancel_requested;
        row.consumed_seq = row.consumed_seq.max(consumed_seq);
        Ok(Some(ActorRenewal {
            inbox_seq: row.inbox_seq,
            cancel_inbox_seq: row.cancel(),
        }))
    }

    async fn release_actor(
        &self,
        session_id: SessionId,
        holder: &str,
        fence: u64,
        consumed_inbox_seq: i64,
    ) -> Result<ActorRelease> {
        let mut rows = self.rows.lock().unwrap();
        let Some(row) = rows
            .get_mut(&session_id)
            .filter(|row| row.holder.as_deref() == Some(holder) && row.fence == fence)
        else {
            return Ok(ActorRelease::LeaseLost);
        };
        row.holder = None;
        row.expires = None;
        row.consumed_seq = row.consumed_seq.max(consumed_inbox_seq);
        row.attempts = 0;
        if row.inbox_seq <= row.consumed_seq && (!row.cancel_requested || row.cancel_reported) {
            row.runnable = None;
        }
        if row.cancel_reported {
            row.cancel_requested = false;
        }
        row.cancel_reported = false;
        Ok(ActorRelease::Released {
            still_runnable: row.runnable.is_some(),
        })
    }

    async fn read_actor_journal(
        &self,
        _session_id: SessionId,
        after_sequence: Option<i32>,
        limit: u32,
    ) -> Result<ActorJournal> {
        if self
            .failing_reads
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |left| {
                left.checked_sub(1)
            })
            .is_ok()
        {
            return Err(AgentLoopError::store(
                "Failed to read actor journal: pool timed out while waiting for an open connection",
            ));
        }
        let events = self.runtime.events().await?;
        let head_sequence = events.iter().filter_map(|event| event.sequence).max();
        let from = match after_sequence {
            Some(after) => after + 1,
            None => events
                .iter()
                .filter(|event| event.event_type == TURN_STARTED)
                .filter_map(|event| event.sequence)
                .max()
                .unwrap_or(0),
        };
        let limit = if limit == 0 { 500 } else { limit as usize };
        let mut tail: Vec<_> = events
            .into_iter()
            .filter(|event| event.sequence.is_some_and(|sequence| sequence >= from))
            .collect();
        let has_more = tail.len() > limit;
        tail.truncate(limit);
        Ok(ActorJournal {
            events: tail,
            has_more,
            head_sequence,
        })
    }
}

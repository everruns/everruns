//! An in-memory actor index over an in-process runtime's log, with the
//! semantics of the control plane's `session_actors` repository (see
//! crates/server/src/storage/repositories/session_actors). The repository's
//! own tests pin those semantics against PostgreSQL; this one lets the actor
//! runtime run against them without a database.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use everruns_contracts::error::Result;
use everruns_contracts::typed_id::SessionId;

use crate::actor_lease_store::{
    ActorJournal, ActorLeaseStore, ActorRelease, ActorRenewal, LeasedActor,
};
use crate::core::TURN_STARTED;
use crate::host::InProcessRuntime;

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
    pub cancel_reported: bool,
}

impl Row {
    fn leased(&self) -> bool {
        self.holder.is_some() && self.expires.is_some_and(|at| at > Instant::now())
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
}

impl MemoryActors {
    pub(super) fn new(runtime: InProcessRuntime) -> Self {
        Self {
            runtime,
            rows: Mutex::new(HashMap::new()),
            line: Mutex::new(0),
        }
    }

    fn next_in_line(&self) -> u64 {
        let mut line = self.line.lock().unwrap();
        *line += 1;
        *line
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
        let mut rows = self.rows.lock().unwrap();
        let Some(row) = rows
            .get_mut(&session_id)
            .filter(|row| row.runnable.is_some() || row.holder.is_some())
        else {
            return Cancel::Idle;
        };
        if row.leased() {
            row.cancel_requested = true;
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
        let mut rows = self.rows.lock().unwrap();
        let mut candidates: Vec<(u64, SessionId)> = rows
            .iter()
            .filter(|(_, row)| row.runnable.is_some() && !row.leased())
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
                cancel_requested: row.cancel_requested,
                consumed_seq: row.consumed_seq,
                next_input_seq: row.next_input_seq,
                attempts: row.attempts,
            });
        }
        Ok(leased)
    }

    async fn renew_actor_lease(
        &self,
        session_id: SessionId,
        holder: &str,
        fence: u64,
        ttl: Duration,
        consumed_seq: i64,
    ) -> Result<Option<ActorRenewal>> {
        let mut rows = self.rows.lock().unwrap();
        let Some(row) = rows
            .get_mut(&session_id)
            .filter(|row| row.holder.as_deref() == Some(holder) && row.fence == fence)
        else {
            return Ok(None);
        };
        row.expires = Some(Instant::now() + ttl);
        row.cancel_reported = row.cancel_requested;
        row.consumed_seq = row.consumed_seq.max(consumed_seq);
        Ok(Some(ActorRenewal {
            inbox_seq: row.inbox_seq,
            cancel_requested: row.cancel_requested,
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

//! Raft state machine storage: [`CellState`] in memory, persisted as
//! snapshots.
//!
//! Decision: the state machine is not written on apply. It is rebuilt on
//! start from the latest snapshot plus the WAL after it, so the only fsync on
//! the hot path is the WAL's.
//!
//! Decision: a snapshot holds no history. Building one first flushes each
//! session's events not yet in the object store as a new segment (only
//! sessions that changed write anything), then records per-session metadata
//! and segment lists. Snapshots stay small whatever the history size, a
//! restarted or newly installed replica holds every session unloaded and
//! pages histories in on demand, and each snapshot is also copied to the
//! object store.

use std::collections::HashMap;
use std::fs;
use std::io::{self, Cursor, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use openraft::storage::{RaftStateMachine, Snapshot};
use openraft::{
    EntryPayload, OptionalSend, RaftSnapshotBuilder, SnapshotMeta, StorageIOError, StoredMembership,
};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, watch};

use crate::state::{CellId, CellState, Change, Response, SessionState};
use crate::store::{SegRef, Store};
use crate::types::{Entry, LogId, NodeId, StorageError, TypeConfig};
use everruns_contracts::typed_id::SessionId;
use everruns_core::events::Event;
use futures::stream::{self, StreamExt};

/// The cell state shared between the Raft state machine and the node's
/// executor, scheduler and API.
pub struct Shared {
    pub state: RwLock<CellState>,
    /// Every applied change, for the node's executor and scheduler loops.
    pub changes: broadcast::Sender<Change>,
    /// Per-session changes, for API waiters and streams.
    ///
    /// Decision: a waiter hears only its own session. With one cell-wide
    /// channel every waiter woke (and cloned) on every event in the cell,
    /// so cost grew with sessions times events.
    watchers: parking_lot::Mutex<HashMap<SessionId, broadcast::Sender<Change>>>,
    /// Index of the last applied entry, for read-your-writes waits.
    pub applied: watch::Sender<u64>,
    /// Session history segments and snapshot copies.
    pub store: Store,
    pub cell: CellId,
    /// When each session last changed, for unloading the coldest first.
    touched: parking_lot::Mutex<HashMap<SessionId, Instant>>,
    /// Running estimate of an event's size in memory, in bytes.
    event_bytes: AtomicU64,
    /// How long the last snapshot took to build and write, in ms.
    pub snapshot_ms: AtomicU64,
    /// Called with every applied change (the node's Engine hosting hears
    /// commits through it).
    pub on_change: std::sync::OnceLock<ChangeHook>,
}

/// A listener for every applied change.
pub type ChangeHook = Box<dyn Fn(&Change) + Send + Sync>;

impl Shared {
    pub fn new(store: Store, cell: CellId) -> Arc<Self> {
        Arc::new(Self {
            state: RwLock::new(CellState::default()),
            changes: broadcast::channel(65_536).0,
            watchers: Default::default(),
            applied: watch::channel(0).0,
            store,
            cell,
            touched: Default::default(),
            event_bytes: AtomicU64::new(4096),
            snapshot_ms: AtomicU64::new(0),
            on_change: std::sync::OnceLock::new(),
        })
    }

    /// The session's events after sequence `after`, paging unloaded ones in
    /// from the object store. `None` when the session is not in this cell.
    pub async fn read_events(
        &self,
        session: SessionId,
        after: i32,
    ) -> Option<anyhow::Result<Vec<Event>>> {
        let (offloaded, segments, resident) = {
            let state = self.state.read();
            let s = state.sessions.get(&session)?;
            (
                s.offloaded,
                s.segments.clone(),
                s.resident_after(after).to_vec(),
            )
        };
        if after >= offloaded {
            return Some(Ok(resident));
        }
        Some(
            self.store
                .load(session, &segments, after + 1, offloaded)
                .await
                .map(|mut events| {
                    events.extend(resident);
                    events
                }),
        )
    }

    /// What an executor needs of the session's log, among sequences
    /// `from..=to` (`to`: the end when `None`): the context events
    /// ([`is_context`](crate::store::is_context)) before `full_from`, and
    /// every event from `full_from` on, paging stored ones in as needed.
    ///
    /// Decision: a turn's model input is projected from messages only, and
    /// resuming a turn folds the events since its `turn.started`. So the
    /// rest of earlier turns (model calls, step and tool lifecycle, about
    /// four in five events) is never read by a turn, and an executor does
    /// not load it: a long session costs a turn its messages, not its log.
    pub async fn context(
        &self,
        session: SessionId,
        from: i32,
        to: Option<i32>,
        full_from: i32,
    ) -> Option<anyhow::Result<Vec<Event>>> {
        let from = from.max(1);
        let to = to.unwrap_or(i32::MAX);
        let keep = |e: &Event| {
            e.sequence.is_some_and(|s| {
                s >= from && s <= to && (s >= full_from || crate::store::is_context(&e.event_type))
            })
        };
        let (offloaded, segments, resident) = {
            let state = self.state.read();
            let s = state.sessions.get(&session)?;
            let resident: Vec<Event> = s
                .resident_after(from - 1)
                .iter()
                .take_while(|e| e.sequence.is_some_and(|s| s <= to))
                .filter(|e| keep(e))
                .cloned()
                .collect();
            (s.offloaded, s.segments.clone(), resident)
        };
        if offloaded < from {
            return Some(Ok(resident));
        }
        let stored_to = offloaded.min(to);
        let result = async {
            let mut events = self
                .store
                .load_context(session, &segments, from, stored_to.min(full_from - 1))
                .await?;
            let full = from.max(full_from);
            if full <= stored_to {
                events.extend(self.store.load(session, &segments, full, stored_to).await?);
            }
            events.extend(resident);
            Ok(events)
        }
        .await;
        Some(result)
    }

    /// The session's state with its events after `after` (paged in as
    /// needed). With `stub`, the events are left in the object store: only
    /// the unflushed tail travels, with the segment list (a move hands the
    /// session over this way, without copying its history).
    pub async fn image(
        &self,
        session: SessionId,
        after: i32,
        stub: bool,
    ) -> Option<anyhow::Result<SessionState>> {
        let mut image = {
            let state = self.state.read();
            let s = state.sessions.get(&session)?;
            if stub {
                return Some(Ok(move_image(s)));
            }
            self::stub(s)
        };
        match self.read_events(session, after).await? {
            Ok(events) => {
                image.offloaded = after;
                image.events = events;
                Some(Ok(image))
            }
            Err(error) => Some(Err(error)),
        }
    }

    /// Drop an idle session's events from memory once the object store
    /// holds them all (the leader stores them; see [`flush`](Self::flush)).
    /// Returns the events freed.
    pub fn unload(&self, session: SessionId) -> usize {
        let mut state = self.state.write();
        let Some(s) = state.sessions.get_mut(&session) else {
            return 0;
        };
        let last = s.last_sequence().unwrap_or(0);
        if !s.idle() || s.events.is_empty() || s.flushed() < last {
            return 0;
        }
        let freed = s.events.len();
        s.events = Vec::new();
        s.offloaded = last;
        freed
    }

    /// Leader only: store up to `limit` sessions' events that are not in
    /// the object store yet, and return the segments to record with
    /// `Stored`. A session qualifies once idle, or once its unstored tail
    /// reaches [`FLUSH_BUSY_AT`] events; one with [`COMPACT_AT`] segments has
    /// a run of them rewritten as one (see
    /// [`compaction_range`](crate::state::SessionState::compaction_range)).
    /// The flag marks a compaction.
    pub async fn flush(&self, limit: usize) -> Vec<(SessionId, SegRef, bool)> {
        // Per session: a compaction of stored segments `(from, to)`, or a
        // flush of the unstored tail.
        let jobs: Vec<(SessionId, Option<(i32, i32)>)> = {
            let state = self.state.read();
            state
                .sessions
                .iter()
                .filter_map(|(id, s)| {
                    let last = s.last_sequence().unwrap_or(0);
                    let tail = last - s.flushed();
                    let compact = (s.segments.len() >= COMPACT_AT)
                        .then(|| s.compaction_range())
                        .flatten();
                    (compact.is_some() || (tail > 0 && (s.idle() || tail >= FLUSH_BUSY_AT)))
                        .then_some((*id, compact))
                })
                .take(limit)
                .collect()
        };
        stream::iter(jobs)
            .map(|(id, range)| async move {
                let compact = range.is_some();
                let read =
                    match range {
                        Some((from, to)) => {
                            let segments = {
                                let state = self.state.read();
                                state.sessions.get(&id)?.segments.clone()
                            };
                            Some(self.store.load(id, &segments, from, to).await.map(
                                |mut events| {
                                    events.retain(|e| {
                                        e.sequence.is_some_and(|seq| seq >= from && seq <= to)
                                    });
                                    events
                                },
                            ))
                        }
                        None => {
                            let flushed = self.state.read().sessions.get(&id)?.flushed();
                            self.read_events(id, flushed).await
                        }
                    };
                let events = match read? {
                    Ok(events) if !events.is_empty() => events,
                    Ok(_) => return None,
                    Err(error) => {
                        tracing::warn!(session = %id, %error, "reading events to store failed");
                        return None;
                    }
                };
                match self.put_segment(id, &events).await {
                    Ok(seg) => Some((id, seg, compact)),
                    Err(error) => {
                        tracing::warn!(session = %id, %error, "segment write failed");
                        None
                    }
                }
            })
            .buffer_unordered(64)
            .filter_map(|x| async move { x })
            .collect()
            .await
    }

    async fn put_segment(&self, session: SessionId, events: &[Event]) -> anyhow::Result<SegRef> {
        let seg = self.store.put_segment(session, events).await?;
        // Weigh the in-memory size estimate by what was just written.
        let bytes = serde_json::to_vec(events).map(|b| b.len()).unwrap_or(0) as u64;
        let per = bytes * HEAP_PER_JSON_BYTE / events.len().max(1) as u64;
        let old = self.event_bytes.load(Ordering::Relaxed);
        self.event_bytes
            .store((old * 7 + per) / 8, Ordering::Relaxed);
        Ok(seg)
    }

    /// Estimated bytes of resident events, and their count.
    pub fn resident(&self) -> (u64, usize) {
        let events: usize = self
            .state
            .read()
            .sessions
            .values()
            .map(|s| s.events.len())
            .sum();
        (
            events as u64 * self.event_bytes.load(Ordering::Relaxed),
            events,
        )
    }

    /// Drop the stored part of a session's resident events that its next
    /// turn will not read from memory: everything before the messages after
    /// its newest checkpoint (before its current turn when it has none).
    /// Works on busy sessions too. Returns the events freed.
    ///
    /// Decision: unloading only idle sessions left a session that is never
    /// idle for long (a busy chat) holding its whole history: in a run of
    /// 40 sessions × 5,000 turns, resident events grew past 900k per node
    /// against a 256 MB budget.
    pub fn trim(&self, session: SessionId) -> usize {
        let mut state = self.state.write();
        let Some(s) = state.sessions.get_mut(&session) else {
            return 0;
        };
        let keep = if s.checkpoints.is_empty() {
            s.turn_from()
        } else {
            s.context_from()
        };
        let cut = s.flushed().min(keep - 1);
        if cut <= s.offloaded {
            return 0;
        }
        let freed = ((cut - s.offloaded) as usize).min(s.events.len());
        s.events.drain(..freed);
        s.offloaded = cut;
        freed
    }

    /// Resident sessions, most resident events first.
    pub fn largest_resident(&self) -> Vec<(SessionId, usize)> {
        let state = self.state.read();
        let mut sessions: Vec<_> = state
            .sessions
            .iter()
            .filter(|(_, s)| !s.events.is_empty())
            .map(|(id, s)| (*id, s.events.len()))
            .collect();
        sessions.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
        sessions
    }

    /// Idle sessions holding events in memory, least recently changed first.
    pub fn unload_candidates(&self) -> Vec<SessionId> {
        let touched = self.touched.lock();
        let state = self.state.read();
        let mut idle: Vec<(Option<Instant>, SessionId)> = state
            .sessions
            .iter()
            .filter(|(_, s)| s.idle() && !s.events.is_empty())
            .map(|(id, _)| (touched.get(id).copied(), *id))
            .collect();
        idle.sort_by_key(|(at, _)| *at);
        idle.into_iter().map(|(_, id)| id).collect()
    }

    /// Sessions idle and resident for longer than `age`.
    pub fn idle_longer_than(&self, age: std::time::Duration) -> Vec<SessionId> {
        let touched = self.touched.lock();
        let state = self.state.read();
        state
            .sessions
            .iter()
            .filter(|&(id, s)| {
                s.idle()
                    && !s.events.is_empty()
                    && touched.get(id).is_none_or(|at| at.elapsed() > age)
            })
            .map(|(id, _)| *id)
            .collect()
    }

    /// Changes to one session from now on.
    pub fn watch(&self, session: SessionId) -> broadcast::Receiver<Change> {
        self.watchers
            .lock()
            .entry(session)
            // Small: a listener that lags replays from the log. Each slot is
            // allocated up front, for every watched session.
            .or_insert_with(|| broadcast::channel(64).0)
            .subscribe()
    }

    /// Drop channels of sessions nobody watches any more.
    pub fn sweep_watchers(&self) {
        self.watchers.lock().retain(|_, tx| tx.receiver_count() > 0);
    }

    /// Announce an applied change.
    pub fn publish(&self, change: Change) {
        let session = change.session();
        self.touched.lock().insert(session, Instant::now());
        let mut watchers = self.watchers.lock();
        if let Some(tx) = watchers.get(&session)
            && tx.send(change.clone()).is_err()
        {
            watchers.remove(&session); // nobody listens any more
        }
        drop(watchers);
        if let Some(hook) = self.on_change.get() {
            hook(&change);
        }
        let _ = self.changes.send(change);
    }

    /// Wait until this replica has applied `index`.
    pub async fn wait_applied(&self, index: u64) {
        let mut rx = self.applied.subscribe();
        let _ = rx.wait_for(|applied| *applied >= index).await;
    }
}

/// An archived snapshot copy (`Store::put_snapshot`) as the cell state it
/// holds.
pub fn decode_archived(bytes: &[u8]) -> anyhow::Result<CellState> {
    let stored: StoredSnapshot = serde_json::from_slice(bytes)?;
    let mut state: CellState = serde_json::from_slice(&stored.data)?;
    state.resolve_definitions();
    Ok(state)
}

#[derive(Serialize, Deserialize)]
struct StoredSnapshot {
    meta: SnapshotMeta<NodeId, openraft::BasicNode>,
    /// The cell state as JSON, kept as text (a byte array would be written
    /// as a list of numbers, three times the size).
    #[serde(with = "as_text")]
    data: Vec<u8>,
}

mod as_text {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(data: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(std::str::from_utf8(data).map_err(serde::ser::Error::custom)?)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        Ok(String::deserialize(d)?.into_bytes())
    }
}

/// The Raft-facing state machine.
#[derive(Clone)]
pub struct StateMachine {
    shared: Arc<Shared>,
    dir: PathBuf,
    snapshot_seq: Arc<std::sync::atomic::AtomicU64>,
}

/// Heap bytes a parsed event takes per byte of its JSON.
///
/// Decision: the memory budget is in real bytes, so the resident estimate
/// scales the events' JSON size by what parsing them costs. Measured on the
/// llmsim mix: 2.6 to 3.4 (strings, maps and `Value` trees carry headers and
/// slack the JSON does not).
const HEAP_PER_JSON_BYTE: u64 = 3;

/// Compact a session's trailing segments once it has this many.
const COMPACT_AT: usize = 16;

/// Store a running turn's events once this many are not stored yet.
const FLUSH_BUSY_AT: i32 = 64;

/// What a move carries of a session: its metadata and the events the
/// object store doesn't hold yet (history stays in the store).
pub fn move_image(s: &SessionState) -> SessionState {
    let mut image = stub(s);
    image.events = s.resident_after(s.flushed()).to_vec();
    image
}

/// A session's metadata without its resident events.
fn stub(s: &SessionState) -> SessionState {
    SessionState {
        agent: s.agent.clone(),
        definition: s.definition.clone(),
        definition_ref: None,
        created_ms: s.created_ms,
        offloaded: s.flushed(),
        segments: s.segments.clone(),
        events: Vec::new(),
        authority: s.authority.clone(),
        placement: s.placement,
        tenant: s.tenant.clone(),
        generation: s.generation,
        moving_to: s.moving_to,
        turn_start: s.turn_start,
        checkpoints: s.checkpoints.clone(),
    }
}

fn sm_err(error: impl std::error::Error + 'static) -> StorageError {
    StorageIOError::write_state_machine(openraft::AnyError::new(&error)).into()
}

impl StateMachine {
    /// Open the state machine, restoring the latest stored snapshot.
    pub fn open(dir: impl Into<PathBuf>, shared: Arc<Shared>) -> io::Result<Self> {
        let dir = dir.into();
        fs::create_dir_all(&dir)?;
        let machine = Self {
            shared,
            dir,
            snapshot_seq: Default::default(),
        };
        if let Some(stored) = machine.read_current()? {
            let mut state: CellState =
                serde_json::from_slice(&stored.data).map_err(io::Error::other)?;
            state.resolve_definitions();
            let applied = state.last_applied.map(|id| id.index).unwrap_or(0);
            *machine.shared.state.write() = state;
            machine.shared.applied.send_replace(applied);
        }
        Ok(machine)
    }

    fn current_path(&self) -> PathBuf {
        self.dir.join("current.json")
    }

    fn read_current(&self) -> io::Result<Option<StoredSnapshot>> {
        match fs::read(self.current_path()) {
            Ok(bytes) => Ok(Some(
                serde_json::from_slice(&bytes).map_err(io::Error::other)?,
            )),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }

    fn write_current(&self, stored: &StoredSnapshot) -> io::Result<()> {
        let bytes = serde_json::to_vec(stored)?;
        let tmp = self.dir.join("current.json.tmp");
        let mut file = fs::File::create(&tmp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::rename(&tmp, self.current_path())?;
        fs::File::open(&self.dir)?.sync_all()?;
        Ok(())
    }

    /// Keep a copy of the snapshot in the object store; best effort.
    fn archive(&self, stored: &StoredSnapshot) {
        let Ok(bytes) = serde_json::to_vec(stored) else {
            return;
        };
        let store = self.shared.store.clone();
        let (cell, id) = (self.shared.cell, stored.meta.snapshot_id.clone());
        tokio::spawn(async move {
            if let Err(error) = store.put_snapshot(cell, &id, bytes).await {
                tracing::warn!(%error, "snapshot copy to the object store failed");
            }
        });
    }

    /// The cell state as a snapshot holds it: metadata and segment lists,
    /// plus each session's events that are not stored yet, inline.
    ///
    /// Decision: a snapshot writes nothing to the object store. Only the
    /// leader writes segments (as `Stored` commands), so every replica's
    /// snapshot of the same index is the same, and the cell keeps one copy
    /// of the history instead of one per replica.
    fn snapshot_state(&self) -> CellState {
        let state = self.shared.state.read();
        CellState {
            last_applied: state.last_applied,
            membership: state.membership.clone(),
            sessions: state
                .sessions
                .iter()
                .map(|(id, s)| {
                    let mut image = stub(s);
                    image.events = s.resident_after(s.flushed()).to_vec();
                    (*id, image)
                })
                .collect(),
            forwarded: state.forwarded.clone(),
            directory: state.directory.clone(),
            cells: state.cells.clone(),
            draining: state.draining.clone(),
            retired: state.retired.clone(),
            shards: state.shards.clone(),
            sealed: state.sealed.clone(),
            parts: state.parts.clone(),
            definitions: Default::default(),
            allowances: state.allowances.clone(),
        }
    }
}

impl RaftSnapshotBuilder<TypeConfig> for StateMachine {
    async fn build_snapshot(&mut self) -> Result<Snapshot<TypeConfig>, StorageError> {
        let started = Instant::now();
        let mut state = self.snapshot_state();
        state.share_definitions();
        let data = serde_json::to_vec(&state).map_err(sm_err)?;
        let (last_applied, membership) = (state.last_applied, state.membership.clone());
        drop(state);
        let seq = self
            .snapshot_seq
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let snapshot_id = match last_applied {
            Some(id) => format!("{}-{}-{seq}", id.leader_id, id.index),
            None => format!("empty-{seq}"),
        };
        let meta = SnapshotMeta {
            last_log_id: last_applied,
            last_membership: membership,
            snapshot_id,
        };
        let stored = StoredSnapshot {
            meta: meta.clone(),
            data: data.clone(),
        };
        let dir = self.clone();
        let stored =
            tokio::task::spawn_blocking(move || dir.write_current(&stored).map(|()| stored))
                .await
                .map_err(sm_err)?
                .map_err(sm_err)?;
        self.archive(&stored);
        self.shared
            .snapshot_ms
            .store(started.elapsed().as_millis() as u64, Ordering::Relaxed);
        Ok(Snapshot {
            meta,
            snapshot: Box::new(Cursor::new(data)),
        })
    }
}

impl RaftStateMachine<TypeConfig> for StateMachine {
    type SnapshotBuilder = Self;

    async fn applied_state(
        &mut self,
    ) -> Result<(Option<LogId>, StoredMembership<NodeId, openraft::BasicNode>), StorageError> {
        let state = self.shared.state.read();
        Ok((state.last_applied, state.membership.clone()))
    }

    async fn apply<I>(&mut self, entries: I) -> Result<Vec<Response>, StorageError>
    where
        I: IntoIterator<Item = Entry> + OptionalSend,
        I::IntoIter: OptionalSend,
    {
        let mut responses = Vec::new();
        let mut changes = Vec::new();
        let mut last = None;
        let t = std::time::Instant::now();
        {
            let lock = std::time::Instant::now();
            let mut state = self.shared.state.write();
            crate::prof::add("apply_lock", lock, 1);
            for entry in entries {
                state.last_applied = Some(entry.log_id);
                last = Some(entry.log_id.index);
                match entry.payload {
                    EntryPayload::Blank => responses.push(Response::Ok),
                    EntryPayload::Normal(command) => {
                        responses.push(state.apply(command, &mut changes));
                    }
                    EntryPayload::Membership(membership) => {
                        state.membership = StoredMembership::new(Some(entry.log_id), membership);
                        responses.push(Response::Ok);
                    }
                }
            }
        }
        crate::prof::add("apply", t, 1);
        let t = std::time::Instant::now();
        for change in changes {
            self.shared.publish(change);
        }
        crate::prof::add("publish", t, 1);
        if let Some(last) = last {
            self.shared.applied.send_replace(last);
        }
        Ok(responses)
    }

    async fn get_snapshot_builder(&mut self) -> Self::SnapshotBuilder {
        self.clone()
    }

    async fn begin_receiving_snapshot(&mut self) -> Result<Box<Cursor<Vec<u8>>>, StorageError> {
        Ok(Box::new(Cursor::new(Vec::new())))
    }

    async fn install_snapshot(
        &mut self,
        meta: &SnapshotMeta<NodeId, openraft::BasicNode>,
        snapshot: Box<Cursor<Vec<u8>>>,
    ) -> Result<(), StorageError> {
        let data = snapshot.into_inner();
        let mut state: CellState = serde_json::from_slice(&data).map_err(sm_err)?;
        state.resolve_definitions();
        let applied = state.last_applied.map(|id| id.index).unwrap_or(0);
        let sessions: Vec<_> = state.sessions.keys().copied().collect();
        *self.shared.state.write() = state;
        self.shared.applied.send_replace(applied);
        for session in sessions {
            self.shared.publish(Change::Session(session));
        }
        let stored = StoredSnapshot {
            meta: meta.clone(),
            data,
        };
        let dir = self.clone();
        tokio::task::spawn_blocking(move || dir.write_current(&stored))
            .await
            .map_err(sm_err)?
            .map_err(sm_err)?;
        Ok(())
    }

    async fn get_current_snapshot(&mut self) -> Result<Option<Snapshot<TypeConfig>>, StorageError> {
        let stored = self.read_current().map_err(sm_err)?;
        Ok(stored.map(|stored| Snapshot {
            meta: stored.meta,
            snapshot: Box::new(Cursor::new(stored.data)),
        }))
    }
}

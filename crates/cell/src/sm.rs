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
use std::time::{Duration, Instant};

use openraft::storage::{RaftStateMachine, Snapshot};
use openraft::{
    EntryPayload, OptionalSend, RaftSnapshotBuilder, SnapshotMeta, StorageIOError, StoredMembership,
};
use parking_lot::RwLock;
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
    /// Turns this cell finished in the last minute, as this replica applied
    /// them (`cell_slo_turn_p99_seconds`).
    pub turn_window: crate::prom::Window,
    /// Session history segments and snapshot copies.
    pub store: Store,
    pub cell: CellId,
    /// When each session last changed, for unloading the coldest first.
    touched: parking_lot::Mutex<HashMap<SessionId, Instant>>,
    /// The entry index of each session's latest change (at least: the
    /// index being applied when it changed), for executors' watches.
    changed: parking_lot::Mutex<HashMap<SessionId, u64>>,
    /// Running estimate of an event's size in memory, in bytes.
    event_bytes: AtomicU64,
    /// How long the last snapshot took to build and write, in ms.
    pub snapshot_ms: AtomicU64,
    /// When this replica last built a snapshot (or started).
    last_snapshot: parking_lot::Mutex<Instant>,
    /// Whether this replica leads its cell, as the ship loop last saw it:
    /// only the leader copies its snapshots to the object store.
    pub leading: std::sync::atomic::AtomicBool,
    /// A snapshot copy to the object store is in flight.
    archiving: Arc<std::sync::atomic::AtomicBool>,
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
            turn_window: Default::default(),
            store,
            cell,
            touched: Default::default(),
            changed: Default::default(),
            event_bytes: AtomicU64::new(4096),
            snapshot_ms: AtomicU64::new(0),
            last_snapshot: parking_lot::Mutex::new(Instant::now()),
            leading: Default::default(),
            archiving: Default::default(),
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
        let branch = self
            .state
            .read()
            .sessions
            .get(&session)
            .map_or(0, |s| s.branch);
        let seg = self.store.put_segment(session, events, branch).await?;
        // Weigh the in-memory size estimate by what was just written.
        let bytes = serde_json::to_vec(events).map(|b| b.len()).unwrap_or(0) as u64;
        let per = bytes * HEAP_PER_JSON_BYTE / events.len().max(1) as u64;
        let old = self.event_bytes.load(Ordering::Relaxed);
        self.event_bytes
            .store((old * 7 + per) / 8, Ordering::Relaxed);
        Ok(seg)
    }

    /// How long since this replica last built a snapshot (or started).
    pub fn last_snapshot_age(&self) -> std::time::Duration {
        self.last_snapshot.lock().elapsed()
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

    /// Drop every resident event the object store already holds: an idle
    /// session's whole history (`unload`), a busy one's before what its
    /// next turn reads (`trim`). Returns the events freed. Readers page
    /// them back as they do after the budget unloads them.
    pub fn shed_stored(&self) -> usize {
        let sessions: Vec<SessionId> = self.state.read().sessions.keys().copied().collect();
        sessions
            .into_iter()
            .map(|id| match self.unload(id) {
                0 => self.trim(id),
                freed => freed,
            })
            .sum()
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
        let at = *self.applied.borrow() + 1;
        self.changed.lock().insert(session, at);
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

    /// The index `session` last changed at (0: not since this node
    /// started).
    pub fn changed_at(&self, session: SessionId) -> u64 {
        self.changed.lock().get(&session).copied().unwrap_or(0)
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
    let (_, at) = split_snapshot(bytes)?;
    let mut state: CellState = serde_json::from_slice(&bytes[at..])?;
    state.resolve_definitions();
    Ok(state)
}

pub(crate) type Meta = SnapshotMeta<NodeId, openraft::BasicNode>;

/// The header of a stored snapshot (the node's file and the archived copy):
/// the metadata as one JSON line. The cell state's JSON follows as it is.
///
/// Decision: no envelope around the state. Wrapping it in a JSON document
/// escaped every quote (a 45% larger file) and serializing the envelope
/// copied the state twice more. With 40 000 sessions a cell state is about
/// 120 MB, and those copies, made every few seconds, are what pushed nodes
/// out of memory in the 75 000-session run.
fn snapshot_header(meta: &Meta) -> io::Result<Vec<u8>> {
    let mut header = serde_json::to_vec(meta)?;
    header.push(b'\n');
    Ok(header)
}

/// A stored snapshot's metadata and where its state starts. Compact JSON
/// has no raw newline, so the first one ends the header.
pub(crate) fn split_snapshot(bytes: &[u8]) -> io::Result<(Meta, usize)> {
    let end = bytes
        .iter()
        .position(|b| *b == b'\n')
        .ok_or_else(|| io::Error::other("snapshot without a header"))?;
    Ok((serde_json::from_slice(&bytes[..end])?, end + 1))
}

/// Write `data` (a cell state's JSON) as the current snapshot in `dir`, a
/// node's snapshot directory: what a node opens on start.
pub(crate) fn write_snapshot_file(
    dir: &std::path::Path,
    meta: &Meta,
    data: &[u8],
) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    let tmp = dir.join("current.snap.tmp");
    let mut file = fs::File::create(&tmp)?;
    file.write_all(&snapshot_header(meta)?)?;
    file.write_all(data)?;
    file.sync_all()?;
    fs::rename(&tmp, dir.join("current.snap"))?;
    fs::File::open(dir)?.sync_all()?;
    Ok(())
}

/// The header and body of an archived snapshot of `state`, as
/// `put_snapshot` stores it.
pub(crate) fn archived_parts(meta: &Meta, data: bytes::Bytes) -> io::Result<[bytes::Bytes; 2]> {
    Ok([bytes::Bytes::from(snapshot_header(meta)?), data])
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

/// Bytes of events a node may keep resident: its memory budget less what
/// the rest of the process holds (`rss` minus the `resident` estimate).
///
/// Decision: the budget caps the whole process, not only the events. The
/// estimate covers parsed events, while a node also holds every session's
/// metadata and receipts, the Raft log, snapshot copies and the allocator's
/// slack. Counting events alone, the 2026-11-15 EKS run sat at four times its
/// 3 GB budget and the kernel killed nodes at their 12 GiB limit. A floor of
/// an eighth keeps the node serving when the rest alone passes the budget:
/// a node that unloads everything still answers from the object store.
pub(crate) fn event_allowance(budget: u64, rss: u64, resident: u64) -> u64 {
    budget
        .saturating_sub(rss.saturating_sub(resident))
        .max(budget / 8)
}

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
        files: s.files.clone(),
        branch: s.branch,
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
        if let Some((_, data)) = machine.read_current()? {
            let mut state: CellState = serde_json::from_slice(&data).map_err(io::Error::other)?;
            state.resolve_definitions();
            let applied = state.last_applied.map(|id| id.index).unwrap_or(0);
            *machine.shared.state.write() = state;
            machine.shared.applied.send_replace(applied);
        }
        Ok(machine)
    }

    fn current_path(&self) -> PathBuf {
        self.dir.join("current.snap")
    }

    /// The current snapshot's metadata and state. The state is the file's
    /// own buffer with the header cut off, not a copy.
    fn read_current(&self) -> io::Result<Option<(Meta, Vec<u8>)>> {
        match fs::read(self.current_path()) {
            Ok(mut bytes) => {
                let (meta, at) = split_snapshot(&bytes)?;
                bytes.drain(..at);
                Ok(Some((meta, bytes)))
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }

    fn write_current(&self, meta: &Meta, data: &[u8]) -> io::Result<()> {
        write_snapshot_file(&self.dir, meta, data)
    }

    /// Keep a copy of the snapshot in the object store; best effort. It
    /// shares `data` with the caller instead of copying it.
    ///
    /// Decision: the leader's copy only, one at a time. Every replica's
    /// snapshot of a cell is the same state, and restore takes the latest
    /// copy whoever wrote it, so three copies bought nothing and tripled the
    /// upload. In EKS run 43827766384 all nine nodes copied a whole cell
    /// state every few minutes, through a NAT whose burst ran out, and once
    /// S3 slowed every spawned copy held its snapshot in memory until it
    /// finished or failed. A copy that finds one in flight skips: the next
    /// snapshot is newer anyway.
    fn archive(&self, meta: &Meta, data: bytes::Bytes, inc: u64, clock_ms: i64) {
        if !self.shared.leading.load(Ordering::Relaxed)
            || self.shared.archiving.swap(true, Ordering::AcqRel)
        {
            return;
        }
        let in_flight = self.shared.archiving.clone();
        let Ok(header) = snapshot_header(meta) else {
            in_flight.store(false, Ordering::Release);
            return;
        };
        let store = self.shared.store.clone();
        let (cell, id) = (self.shared.cell, meta.snapshot_id.clone());
        let index = meta.last_log_id.map_or(0, |id| id.index);
        tokio::spawn(async move {
            let parts = [bytes::Bytes::from(header), data];
            if let Err(error) = store
                .put_snapshot(cell, &id, inc, index, clock_ms, parts)
                .await
            {
                tracing::warn!(%error, "snapshot copy to the object store failed");
            }
            in_flight.store(false, Ordering::Release);
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
            incarnation: state.incarnation,
            clock_ms: state.clock_ms,
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
            orgs: state.orgs.clone(),
            feed: state.feed.clone(),
            turns_finished: Vec::new(),
        }
    }
}

impl RaftSnapshotBuilder<TypeConfig> for StateMachine {
    async fn build_snapshot(&mut self) -> Result<Snapshot<TypeConfig>, StorageError> {
        let started = Instant::now();
        let mut state = self.snapshot_state();
        state.share_definitions();
        let data = bytes::Bytes::from(serde_json::to_vec(&state).map_err(sm_err)?);
        let (last_applied, membership) = (state.last_applied, state.membership.clone());
        let (inc, clock_ms) = (state.incarnation, state.clock_ms);
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
        let (dir, written, file_data) = (self.clone(), meta.clone(), data.clone());
        tokio::task::spawn_blocking(move || dir.write_current(&written, &file_data))
            .await
            .map_err(sm_err)?
            .map_err(sm_err)?;
        self.archive(&meta, data, inc, clock_ms);
        self.shared
            .snapshot_ms
            .store(started.elapsed().as_millis() as u64, Ordering::Relaxed);
        *self.shared.last_snapshot.lock() = Instant::now();
        // openraft keeps only the metadata of a snapshot it built; a peer
        // gets the state through `get_current_snapshot`, which reads the
        // file. Handing the state back here would be one more copy held.
        Ok(Snapshot {
            meta,
            snapshot: Box::new(Cursor::new(Vec::new())),
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
            for ms in std::mem::take(&mut state.turns_finished) {
                self.shared.turn_window.observe(Duration::from_millis(ms));
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
        // Decision: shed what the object store holds before parsing the
        // new state next to the old one. A node that rejoins after a kill
        // holds the state it restored and replayed when its leader's
        // snapshot arrives; on EKS (run 43747035567) that was 6 GB at 14 000
        // sessions, and parsing the 25 000-session state beside it ran the
        // node out of its 12 GiB five times in a row. Shedding is what the
        // memory budget does under pressure, so readers page events back as
        // usual. The old state is dropped after the swap, outside the lock.
        let freed = self.shared.shed_stored();
        let mut state: CellState = serde_json::from_slice(&data).map_err(sm_err)?;
        state.resolve_definitions();
        let applied = state.last_applied.map(|id| id.index).unwrap_or(0);
        let sessions: Vec<_> = state.sessions.keys().copied().collect();
        tracing::info!(
            bytes = data.len(),
            sessions = sessions.len(),
            applied,
            freed,
            "installing a snapshot"
        );
        let old = std::mem::replace(&mut *self.shared.state.write(), state);
        drop(old);
        self.shared.applied.send_replace(applied);
        for session in sessions {
            self.shared.publish(Change::Session(session));
        }
        let (dir, meta) = (self.clone(), meta.clone());
        tokio::task::spawn_blocking(move || dir.write_current(&meta, &data))
            .await
            .map_err(sm_err)?
            .map_err(sm_err)?;
        Ok(())
    }

    async fn get_current_snapshot(&mut self) -> Result<Option<Snapshot<TypeConfig>>, StorageError> {
        let stored = self.read_current().map_err(sm_err)?;
        Ok(stored.map(|(meta, data)| Snapshot {
            meta,
            snapshot: Box::new(Cursor::new(data)),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::event_allowance;

    const GB: u64 = 1 << 30;

    /// Before a snapshot install: everything the store holds goes, an
    /// unstored tail stays (no other copy exists), and a busy session keeps
    /// what its next turn reads.
    #[test]
    fn shedding_drops_only_stored_events() {
        use crate::state::{CellState, SessionState};
        use crate::store::{SegRef, Store};
        use everruns_contracts::typed_id::SessionId;
        use everruns_core::events::{EventContext, SessionActivatedData};

        fn session(id: SessionId, events: i32, stored: i32) -> SessionState {
            let events: Vec<_> = (1..=events)
                .map(|seq| {
                    let mut event = everruns_core::events::Event::new(
                        id,
                        EventContext::default(),
                        SessionActivatedData {
                            turn_id: everruns_contracts::typed_id::TurnId::new(),
                            input_message_id: everruns_contracts::typed_id::MessageId::new(),
                        },
                    );
                    event.sequence = Some(seq);
                    event
                })
                .collect();
            let mut s = SessionState {
                events,
                ..Default::default()
            };
            if stored > 0 {
                s.segments.push(SegRef {
                    from: 1,
                    to: stored,
                    branch: 0,
                });
            }
            s
        }
        let shared = super::Shared::new(Store::open("memory://").unwrap(), 0);
        let (stored, tail) = (SessionId::new(), SessionId::new());
        {
            let mut state: parking_lot::RwLockWriteGuard<'_, CellState> = shared.state.write();
            state.sessions.insert(stored, session(stored, 5, 5));
            state.sessions.insert(tail, session(tail, 5, 3));
        }
        // All of the first; the stored 1..=3 of the second.
        assert_eq!(shared.shed_stored(), 5 + 3);
        let state = shared.state.read();
        assert!(state.sessions[&stored].events.is_empty());
        assert_eq!(state.sessions[&stored].last_sequence(), Some(5));
        let kept: Vec<_> = state.sessions[&tail]
            .events
            .iter()
            .map(|e| e.sequence)
            .collect();
        assert_eq!(kept, [Some(4), Some(5)], "the unstored tail stays");
    }

    /// Only the leader copies its snapshot to the object store, and a copy
    /// still in flight makes the next one skip.
    #[tokio::test]
    async fn only_the_leader_copies_its_snapshot_once_at_a_time() {
        use openraft::RaftSnapshotBuilder;
        use std::sync::atomic::Ordering;

        let store = crate::store::Store::open("memory://").unwrap();
        let shared = super::Shared::new(store.clone(), 0);
        let dir = tempfile::tempdir().unwrap();
        let mut machine = super::StateMachine::open(dir.path(), shared.clone()).unwrap();
        let copies = || async { store.snapshot_copies(0).await.unwrap().len() };

        machine.build_snapshot().await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert_eq!(copies().await, 0, "a follower keeps its snapshot local");

        shared.leading.store(true, Ordering::Relaxed);
        shared.archiving.store(true, Ordering::Relaxed);
        machine.build_snapshot().await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert_eq!(copies().await, 0, "one copy in flight at a time");

        shared.archiving.store(false, Ordering::Relaxed);
        machine.build_snapshot().await.unwrap();
        for _ in 0..100 {
            if copies().await == 1 && !shared.archiving.load(Ordering::Relaxed) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert_eq!(copies().await, 1, "the leader copies it");
        assert!(
            !shared.archiving.load(Ordering::Relaxed),
            "and is free again"
        );
    }

    #[test]
    fn the_budget_counts_what_the_process_holds_besides_events() {
        // Unknown size (not Linux): events get the whole budget.
        assert_eq!(event_allowance(8 * GB, 0, 0), 8 * GB);
        // 3 GB of other state leaves 5 GB for events, however many are in.
        assert_eq!(event_allowance(8 * GB, 4 * GB, GB), 5 * GB);
        assert_eq!(event_allowance(8 * GB, 7 * GB, 4 * GB), 5 * GB);
        // The rest alone past the budget: an eighth, never nothing.
        assert_eq!(event_allowance(8 * GB, 12 * GB, GB), GB);
    }
}

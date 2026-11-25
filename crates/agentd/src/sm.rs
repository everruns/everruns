//! Raft state machine storage: [`ShardState`] in memory, persisted as
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

use crate::state::{Change, Response, SessionState, ShardId, ShardState};
use crate::store::{SegRef, Store};
use crate::types::{Entry, LogId, NodeId, StorageError, TypeConfig};
use everruns_contracts::typed_id::SessionId;
use everruns_core::events::Event;
use futures::stream::{self, StreamExt};

/// The shard state shared between the Raft state machine and the node's
/// executor, scheduler and API.
pub struct Shared {
    pub state: RwLock<ShardState>,
    /// Every applied session change (never an event: `publish`), for the
    /// node's scheduler.
    pub changes: broadcast::Sender<Change>,
    /// Per-session changes, for API waiters and streams.
    ///
    /// Decision: a waiter hears only its own session. With one shard-wide
    /// channel every waiter woke (and cloned) on every event in the shard,
    /// so cost grew with sessions times events.
    watchers: parking_lot::Mutex<HashMap<SessionId, broadcast::Sender<Change>>>,
    /// Index of the last applied entry, for read-your-writes waits.
    pub applied: watch::Sender<u64>,
    /// Turns this shard finished in the last minute, as this replica applied
    /// them (`agentd_shard_slo_turn_p99_seconds`).
    pub turn_window: crate::prom::Window,
    /// Session history segments and snapshot copies.
    pub store: Store,
    pub shard: ShardId,
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
    /// Whether this replica leads its shard, as the ship loop last saw it
    /// (`set_leading`): only the leader copies its snapshots to the object
    /// store.
    leading: std::sync::atomic::AtomicBool,
    /// Snapshot copies on their way to the object store.
    archiver: Arc<Archiver>,
    /// This replica's snapshot directory, once its state machine opened.
    snapshot_dir: std::sync::OnceLock<PathBuf>,
    /// Called with every applied change (the node's Engine hosting hears
    /// commits through it).
    pub on_change: std::sync::OnceLock<ChangeHook>,
}

/// A listener for every applied change.
pub type ChangeHook = Box<dyn Fn(&Change) + Send + Sync>;

impl Shared {
    pub fn new(store: Store, shard: ShardId) -> Arc<Self> {
        Arc::new(Self {
            state: RwLock::new(ShardState::default()),
            changes: broadcast::channel(65_536).0,
            watchers: Default::default(),
            applied: watch::channel(0).0,
            turn_window: Default::default(),
            store,
            shard,
            touched: Default::default(),
            changed: Default::default(),
            event_bytes: AtomicU64::new(4096),
            snapshot_ms: AtomicU64::new(0),
            last_snapshot: parking_lot::Mutex::new(Instant::now()),
            leading: Default::default(),
            archiver: Default::default(),
            snapshot_dir: Default::default(),
            on_change: std::sync::OnceLock::new(),
        })
    }

    /// The session's events after sequence `after`, paging unloaded ones in
    /// from the object store. `None` when the session is not in this shard.
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

    /// Put back the payloads `events` of `session` hold by reference
    /// (`payload.rs`), for readers that hand events out. A session that
    /// never held one is not scanned.
    pub async fn resolved(&self, session: SessionId, events: &mut [Event]) -> anyhow::Result<()> {
        if self.holds_blobs(session) {
            crate::payload::resolve(&self.store, events).await?;
        }
        Ok(())
    }

    /// [`resolved`](Self::resolved) for shared events.
    pub async fn resolved_shared(
        &self,
        session: SessionId,
        events: &mut [Arc<Event>],
    ) -> anyhow::Result<()> {
        if self.holds_blobs(session) {
            crate::payload::resolve_shared(&self.store, events).await?;
        }
        Ok(())
    }

    fn holds_blobs(&self, session: SessionId) -> bool {
        self.state
            .read()
            .sessions
            .get(&session)
            .is_some_and(|s| s.blobs)
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
    /// `Stored`. A session qualifies once idle for `idle_for`, or once its
    /// unstored tail reaches [`FLUSH_BUSY_AT`] events; one with
    /// [`COMPACT_AT`] segments has a run of them rewritten as one (see
    /// [`compaction_range`](crate::state::SessionState::compaction_range)).
    /// The flag marks a compaction.
    ///
    /// Decision: an idle session waits [`FLUSH_IDLE_AFTER`] before its tail
    /// is stored, not one pass of the loop. A conversation's next message
    /// usually comes within a minute, and storing after every turn cost two
    /// object writes a turn (the 75 000-session run's largest share of
    /// requests). The events are in the Raft log and the snapshots until
    /// then, so waiting loses nothing; the node passes zero when it needs
    /// the memory back (`Node::store_events`).
    pub async fn flush(
        &self,
        limit: usize,
        idle_for: std::time::Duration,
    ) -> Vec<(SessionId, SegRef, bool)> {
        // Per session: a compaction of stored segments `(from, to)`, or a
        // flush of the unstored tail.
        let jobs: Vec<(SessionId, Option<(i32, i32)>)> = {
            let touched = self.touched.lock();
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
                    let quiet =
                        s.idle() && touched.get(id).is_none_or(|at| at.elapsed() >= idle_for);
                    (compact.is_some() || (tail > 0 && (quiet || tail >= FLUSH_BUSY_AT)))
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

    /// Estimated bytes of resident events in memory, and their count.
    ///
    /// Decision: an event in memory takes about half again its encoded
    /// size (parsed values, maps, string headers). Counting encoded bytes
    /// put the journal at 115 MB on a node whose heap profile held 170 MB
    /// of it (2026-11-23), so the budget let it grow past what it allows.
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
        // Decision: only session changes go on the shard-wide channel. A
        // broadcast channel keeps every value in its ring until the slot is
        // written over, read or not: with event changes on it, each node
        // held its last 65,536 events alive, past their unload and outside
        // the memory ledger (75k verifier C3). Its one reader, the
        // scheduler, wants sessions; readers of a session's events watch
        // that session (`watch`, a small ring dropped with its last reader).
        if matches!(change, Change::Session(_)) {
            let _ = self.changes.send(change);
        }
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

/// An archived snapshot copy (`Store::put_snapshot`) as the shard state it
/// holds.
pub fn decode_archived(bytes: &[u8]) -> anyhow::Result<ShardState> {
    let (_, at) = split_snapshot(bytes)?;
    let mut state: ShardState = serde_json::from_slice(&bytes[at..])?;
    state.resolve_definitions();
    Ok(state)
}

pub(crate) type Meta = SnapshotMeta<NodeId, openraft::BasicNode>;

/// The header of a stored snapshot (the node's file and the archived copy):
/// the metadata as one JSON line. The shard state's JSON follows as it is.
///
/// Decision: no envelope around the state. Wrapping it in a JSON document
/// escaped every quote (a 45% larger file) and serializing the envelope
/// copied the state twice more. With 40 000 sessions a shard state is about
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

/// Write `data` (a shard state's JSON) as the current snapshot in `dir`, a
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

/// The current snapshot in `dir`, a node's snapshot directory: its
/// metadata and state. The state is the file's own buffer with the header
/// cut off, not a copy.
pub(crate) fn read_snapshot_file(dir: &std::path::Path) -> io::Result<Option<(Meta, Vec<u8>)>> {
    match fs::read(dir.join("current.snap")) {
        Ok(mut bytes) => {
            let (meta, at) = split_snapshot(&bytes)?;
            bytes.drain(..at);
            Ok(Some((meta, bytes)))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

/// One snapshot copy waiting for the object store.
#[derive(Clone)]
struct Copy {
    id: String,
    inc: u64,
    index: u64,
    clock_ms: i64,
    parts: [bytes::Bytes; 2],
}

/// Snapshot copies to the object store: one in flight, the newest waiting.
#[derive(Default)]
struct Archiver {
    busy: std::sync::atomic::AtomicBool,
    next: parking_lot::Mutex<Option<Copy>>,
}

/// The fields of a shard state an archived copy is named by.
#[derive(serde::Deserialize)]
struct Stamp {
    #[serde(default)]
    incarnation: u64,
    #[serde(default)]
    clock_ms: i64,
}

impl Shared {
    /// The ship loop's view of whether this replica leads. A replica that
    /// starts leading copies its current snapshot: the log it ships starts
    /// after that snapshot when the snapshot purged entries it had not
    /// shipped (`ship.rs`), and restore needs a copy at or past that gap.
    pub fn set_leading(self: &Arc<Self>, leading: bool) {
        if !leading {
            self.leading.store(false, Ordering::Relaxed);
            return;
        }
        if self.leading.swap(true, Ordering::Relaxed) {
            return;
        }
        let Some(dir) = self.snapshot_dir.get().cloned() else {
            return;
        };
        let shared = self.clone();
        tokio::spawn(async move {
            let read = tokio::task::spawn_blocking(move || {
                let Some((meta, data)) = read_snapshot_file(&dir)? else {
                    return Ok(None);
                };
                let stamp: Stamp = serde_json::from_slice(&data)?;
                io::Result::Ok(Some((meta, data, stamp)))
            })
            .await;
            match read {
                Ok(Ok(Some((meta, data, stamp)))) => {
                    shared.archive(&meta, data.into(), stamp.incarnation, stamp.clock_ms)
                }
                Ok(Ok(None)) => {}
                Ok(Err(error)) => {
                    tracing::warn!(%error, "reading the snapshot to copy failed")
                }
                Err(error) => tracing::warn!(%error, "reading the snapshot to copy failed"),
            }
        });
    }

    /// Copy a snapshot to the object store; best effort, and only while
    /// this replica leads. It shares `data` with the caller instead of
    /// copying it.
    ///
    /// Decision: the leader's copies only, one upload at a time, and the
    /// newest snapshot waiting replaces an older one. Every replica's
    /// snapshot of a shard is the same state and restore takes the newest
    /// copy whoever wrote it, so three copies bought nothing and tripled
    /// the upload. In EKS run 43827766384 all nine nodes uploaded a whole
    /// shard state every few minutes through a NAT whose burst ran out, and
    /// once S3 slowed every spawned upload held its snapshot in memory
    /// until it finished or failed. Now a node holds at most two.
    ///
    /// Decision: a failed upload is tried again (backing off to 10 s, the shipping cadence) until
    /// it lands or a newer snapshot replaces it. While the object store is
    /// down the shard still snapshots and purges its log, and the shipper
    /// can then only ship from past the purge (`ship.rs`); restore bridges
    /// that gap with an archived snapshot at or past it. A copy given up
    /// on left the shipped log with a hole no snapshot covered, and the
    /// shard could not be rebuilt (10-minute store outage on the Mac mini,
    /// 2026-11-24: "shipped log ... has a hole: 565 to 1025").
    pub(crate) fn archive(&self, meta: &Meta, data: bytes::Bytes, inc: u64, clock_ms: i64) {
        if !self.leading.load(Ordering::Relaxed) {
            return;
        }
        let Ok(header) = snapshot_header(meta) else {
            return;
        };
        *self.archiver.next.lock() = Some(Copy {
            id: meta.snapshot_id.clone(),
            inc,
            index: meta.last_log_id.map_or(0, |id| id.index),
            clock_ms,
            parts: [bytes::Bytes::from(header), data],
        });
        if self.archiver.busy.swap(true, Ordering::AcqRel) {
            return;
        }
        let (archiver, store, shard) = (self.archiver.clone(), self.store.clone(), self.shard);
        tokio::spawn(async move {
            let mut backoff = std::time::Duration::from_secs(1);
            loop {
                let next = archiver.next.lock().take();
                let Some(copy) = next else {
                    archiver.busy.store(false, Ordering::Release);
                    // A copy queued after the take and before the store
                    // went unseen: go on unless another upload took it.
                    if archiver.next.lock().is_none() || archiver.busy.swap(true, Ordering::AcqRel)
                    {
                        break;
                    }
                    continue;
                };
                if let Err(error) = store
                    .put_snapshot(
                        shard,
                        &copy.id,
                        copy.inc,
                        copy.index,
                        copy.clock_ms,
                        copy.parts.clone(),
                    )
                    .await
                {
                    tracing::warn!(%error, index = copy.index, "snapshot copy to the object store failed; trying again");
                    {
                        let mut next = archiver.next.lock();
                        if next.is_none() {
                            *next = Some(copy);
                        }
                    }
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(std::time::Duration::from_secs(10));
                } else {
                    backoff = std::time::Duration::from_secs(1);
                }
            }
        });
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
/// slack the JSON does not); 2.78 over 18,753 events of a shard soak's store
/// (Mac mini, 2026-11-25).
///
/// Decision: the resident estimate is events times this, once. It also
/// multiplied by 1.5, so it counted events at 1.6 times their measured heap
/// and the memory ledger's remainder (C3) read low by the difference. The
/// estimate only splits the budget: the allowance is the budget less the
/// rest of the process (`event_allowance`), so counting events lower makes
/// a node unload sooner, never later.
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

/// Store an idle session's events once it has been quiet this long.
pub const FLUSH_IDLE_AFTER: std::time::Duration = std::time::Duration::from_secs(60);

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
        importing: s.importing,
        turn_start: s.turn_start,
        checkpoints: s.checkpoints.clone(),
        files: s.files.clone(),
        platform: s.platform.clone(),
        sqldb: s.sqldb.clone(),
        branch: s.branch,
        labels: s.labels.clone(),
        blobs: s.blobs,
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
        let _ = shared.snapshot_dir.set(dir.clone());
        let machine = Self {
            shared,
            dir,
            snapshot_seq: Default::default(),
        };
        if let Some((_, data)) = machine.read_current()? {
            let mut state: ShardState = serde_json::from_slice(&data).map_err(io::Error::other)?;
            state.resolve_definitions();
            let applied = state.last_applied.map(|id| id.index).unwrap_or(0);
            *machine.shared.state.write() = state;
            machine.shared.applied.send_replace(applied);
        }
        Ok(machine)
    }

    /// The current snapshot's metadata and state. The state is the file's
    /// own buffer with the header cut off, not a copy.
    fn read_current(&self) -> io::Result<Option<(Meta, Vec<u8>)>> {
        read_snapshot_file(&self.dir)
    }

    fn write_current(&self, meta: &Meta, data: &[u8]) -> io::Result<()> {
        write_snapshot_file(&self.dir, meta, data)
    }

    /// [`write_current`](Self::write_current) of `state`, encoded as it is
    /// written rather than into memory first.
    fn write_current_from(&self, meta: &Meta, state: &ShardState) -> io::Result<()> {
        fs::create_dir_all(&self.dir)?;
        let tmp = self.dir.join("current.snap.tmp");
        let mut file = io::BufWriter::with_capacity(1 << 20, fs::File::create(&tmp)?);
        file.write_all(&snapshot_header(meta)?)?;
        serde_json::to_writer(&mut file, state)?;
        let file = file.into_inner().map_err(|e| e.into_error())?;
        file.sync_all()?;
        fs::rename(&tmp, self.dir.join("current.snap"))?;
        fs::File::open(&self.dir)?.sync_all()?;
        Ok(())
    }

    /// The shard state as a snapshot holds it: metadata and segment lists,
    /// plus each session's events that are not stored yet, inline.
    ///
    /// Decision: a snapshot writes nothing to the object store. Only the
    /// leader writes segments (as `Stored` commands), so every replica's
    /// snapshot of the same index is the same, and the shard keeps one copy
    /// of the history instead of one per replica.
    fn snapshot_state(&self) -> ShardState {
        let state = self.shared.state.read();
        ShardState {
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
            shards: state.shards.clone(),
            draining: state.draining.clone(),
            retired: state.retired.clone(),
            importing: state.importing.clone(),
            slots: state.slots.clone(),
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
        let (last_applied, membership) = (state.last_applied, state.membership.clone());
        let (inc, clock_ms) = (state.incarnation, state.clock_ms);
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
        // Decision: the copy goes straight to the file, and the copy is
        // dropped before anything reads the file back. Serializing into
        // memory held the live state, its copy and the encoded copy at
        // once: 13.2 GB at 25 000 sessions per shard (run 44396952768),
        // and past a 14 GiB node with Phase 2's longer unstored tails
        // (run 45077134741), on all three nodes together.
        let (dir, written) = (self.clone(), meta.clone());
        tokio::task::spawn_blocking(move || {
            let result = dir.write_current_from(&written, &state);
            drop(state);
            result
        })
        .await
        .map_err(sm_err)?
        .map_err(sm_err)?;
        // Only the leader copies its snapshot to the object store; it reads
        // the file back once the copy is gone.
        if self.shared.leading.load(Ordering::Relaxed)
            && let Some((_, data)) = self.read_current().map_err(sm_err)?
        {
            self.shared
                .archive(&meta, bytes::Bytes::from(data), inc, clock_ms);
        }
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
        let mut state: ShardState = serde_json::from_slice(&data).map_err(sm_err)?;
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

/// The snapshot interval of `node`: `every` for node 1, a tenth more for
/// each later node, so replicas of one shard build their snapshots apart.
pub(crate) fn staggered(every: u64, node: crate::types::NodeId) -> u64 {
    every + every / 10 * node.saturating_sub(1).min(9)
}

/// Whether a node takes a snapshot now to purge its Raft log: `held` bytes
/// of entries after the last snapshot passed a sixteenth of the memory
/// `budget` (staggered by node), and it is ten times as long as the last
/// snapshot took since it was taken (at least five seconds).
///
/// Decision: a sixteenth, not a quarter. The log costs about two and a half
/// times its entries' bytes in resident memory (decoded copies, replication
/// buffers), and the entry-count trigger alone left 300 to 500 seconds
/// between snapshots under load. In the 75 000-agent gate (EKS run
/// 45179434646) five nodes ran out of memory holding 1.0 to 1.6 GB of log
/// against a quarter-budget trigger of 2.25 GB that never fired; a snapshot
/// there dropped the node's memory by 3.5 GB for 1.4 GB of log.
pub(crate) fn log_wants_snapshot(
    held: u64,
    budget: u64,
    node: crate::types::NodeId,
    since_last: std::time::Duration,
    last_took: std::time::Duration,
) -> bool {
    let wait = (last_took * 10).max(std::time::Duration::from_secs(5));
    held > staggered(budget / 16, node) && since_last > wait
}

#[cfg(test)]
mod tests {
    use super::{event_allowance, log_wants_snapshot, staggered};
    use std::time::Duration;

    #[test]
    fn a_log_past_a_sixteenth_of_the_budget_takes_a_snapshot() {
        let budget = 9 * GB;
        let s = Duration::from_secs;
        // The gate's dying nodes held 1.0 to 1.6 GB of log.
        assert!(log_wants_snapshot(GB, budget, 1, s(60), s(3)));
        // Below a sixteenth (576 MiB on 9 GiB) the entry count decides.
        assert!(!log_wants_snapshot(500 << 20, budget, 1, s(60), s(3)));
        // Later replicas wait a tenth more each, so a shard's nodes do not
        // all build at once.
        assert!(log_wants_snapshot(600 << 20, budget, 1, s(60), s(3)));
        assert!(!log_wants_snapshot(600 << 20, budget, 2, s(60), s(3)));
        // Not sooner than ten times the last snapshot took, at least 5 s.
        assert!(!log_wants_snapshot(GB, budget, 1, s(20), s(3)));
        assert!(!log_wants_snapshot(GB, budget, 1, s(4), Duration::ZERO));
        assert!(log_wants_snapshot(GB, budget, 1, s(6), Duration::ZERO));
    }

    const GB: u64 = 1 << 30;

    #[test]
    fn replicas_of_one_shard_snapshot_apart() {
        assert_eq!(staggered(20_000, 1), 20_000);
        assert_eq!(staggered(20_000, 2), 22_000);
        assert_eq!(staggered(20_000, 3), 24_000);
        // Large ids stay within twice the interval.
        assert_eq!(staggered(20_000, 1_000), 38_000);
    }

    /// An applied event is not kept alive by the shard-wide change channel:
    /// once its session lets it go, its memory is free (C3). Session
    /// changes still reach the scheduler.
    #[test]
    fn the_shard_wide_channel_holds_no_events() {
        use crate::state::Change;
        use crate::store::Store;
        use everruns_contracts::typed_id::SessionId;
        use everruns_core::events::{Event, EventContext, SessionActivatedData};
        let shared = super::Shared::new(Store::open("memory://").unwrap(), 0);
        let mut scheduler = shared.changes.subscribe();
        let session = SessionId::new();
        let event = std::sync::Arc::new(Event::new(
            session,
            EventContext::default(),
            SessionActivatedData {
                turn_id: everruns_contracts::typed_id::TurnId::new(),
                input_message_id: everruns_contracts::typed_id::MessageId::new(),
            },
        ));
        shared.publish(Change::Event(session, event.clone()));
        assert_eq!(
            std::sync::Arc::strong_count(&event),
            1,
            "nothing else holds the event"
        );
        shared.publish(Change::Session(session));
        assert!(matches!(scheduler.try_recv(), Ok(Change::Session(s)) if s == session));
        assert!(scheduler.try_recv().is_err());
    }

    /// Before a snapshot install: everything the store holds goes, an
    /// unstored tail stays (no other copy exists), and a busy session keeps
    /// what its next turn reads.
    #[test]
    fn shedding_drops_only_stored_events() {
        use crate::state::{SessionState, ShardState};
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
            let mut state: parking_lot::RwLockWriteGuard<'_, ShardState> = shared.state.write();
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

    /// Only the leader copies its snapshots to the object store: the
    /// current one when it starts leading, then each it builds, one upload
    /// at a time with only the newest waiting.
    #[tokio::test]
    async fn only_the_leader_copies_its_snapshots_one_at_a_time() {
        use openraft::RaftSnapshotBuilder;
        use std::sync::atomic::Ordering;
        use std::time::Duration;

        let store = crate::store::Store::open("memory://").unwrap();
        let shared = super::Shared::new(store.clone(), 0);
        let dir = tempfile::tempdir().unwrap();
        let mut machine = super::StateMachine::open(dir.path(), shared.clone()).unwrap();
        let copies = || async {
            let mut ids: Vec<String> = store
                .snapshot_copies(0)
                .await
                .unwrap()
                .into_iter()
                .map(|c| format!("{c:?}"))
                .collect();
            ids.sort();
            ids
        };
        let settle = || async {
            for _ in 0..200 {
                if !shared.archiver.busy.load(Ordering::Acquire) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        };

        machine.build_snapshot().await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(
            copies().await.is_empty(),
            "a follower keeps its snapshot local"
        );

        shared.set_leading(true);
        for _ in 0..200 {
            if copies().await.len() == 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        settle().await;
        assert_eq!(
            copies().await.len(),
            1,
            "a new leader copies its current snapshot"
        );

        // An upload in flight: later snapshots wait, the newest replacing
        // the older.
        shared.archiver.busy.store(true, Ordering::Release);
        let second = machine.build_snapshot().await.unwrap().meta.snapshot_id;
        let third = machine.build_snapshot().await.unwrap().meta.snapshot_id;
        let waiting = shared.archiver.next.lock().as_ref().map(|c| c.id.clone());
        assert_eq!(waiting, Some(third), "only the newest waits, not {second}");
        assert_eq!(
            copies().await.len(),
            1,
            "nothing uploads while one is in flight"
        );

        shared.archiver.busy.store(false, Ordering::Release);
        machine.build_snapshot().await.unwrap();
        settle().await;
        assert_eq!(copies().await.len(), 2, "the leader copies what it builds");
        assert!(shared.archiver.next.lock().is_none());

        shared.set_leading(false);
        machine.build_snapshot().await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(copies().await.len(), 2, "no copies once it stops leading");
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

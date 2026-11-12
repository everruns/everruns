//! The write-ahead log: Raft's log, in segment files, with group commit.
//!
//! Records are `len: u32 | crc32: u32 | json(entry)`, appended to the active
//! segment `wal/<first-index>.seg`. A segment rolls at [`SEGMENT_BYTES`].
//! `append` only buffers; a flusher task fsyncs and then acknowledges every
//! append that reached the file before the fsync started, so one fsync covers
//! all sessions' steps that arrived meanwhile (group commit). Raft counts an
//! entry once a quorum has acknowledged it, which is when the cell acknowledges
//! the client.
//!
//! Entries since the last purge are also held in memory: replication and
//! recovery read them, and purging (after a snapshot) bounds them.
//!
//! Decisions (the storage proof, `wal_tests.rs`, found the first two):
//! - A rewrite (truncation) commits by a rename. It writes the whole log to
//!   `rewrite.tmp`, syncs it, renames it to `rewrite.ready` (the commit
//!   point), then replaces the segments. Open finishes a ready rewrite and
//!   drops a tmp one. It used to delete the segments before renaming the
//!   rewrite in, so a crash between the two left an empty log: every entry
//!   the node had acknowledged, gone.
//! - Open refuses a log with a hole. Only the active segment can end torn
//!   (a roll syncs the old one first), and its tail is cut so new records
//!   follow intact ones. A tear anywhere else is corruption, and loading
//!   the segments after it would hand Raft a log with a gap, which it
//!   cannot reason about; the node stops instead and is rebuilt from its
//!   peers.

use std::collections::BTreeMap;
use std::fmt::Debug;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufWriter, Read, Write};
use std::ops::RangeBounds;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use openraft::storage::{LogFlushed, LogState, RaftLogStorage};
use openraft::{OptionalSend, RaftLogReader, StorageIOError};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tokio::sync::Notify;

use crate::types::{Entry, LogId, StorageError, TypeConfig, Vote};

/// Segment size at which a new segment starts.
pub const SEGMENT_BYTES: u64 = 64 * 1024 * 1024;

/// Counters for the benchmark: how many fsyncs covered how many appends.
#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize)]
pub struct WalStats {
    pub fsyncs: u64,
    pub entries: u64,
    pub appends: u64,
}

struct Segment {
    first: u64,
    path: PathBuf,
}

struct Inner {
    dir: PathBuf,
    entries: BTreeMap<u64, Entry>,
    /// Encoded size of each entry held in memory, and their sum: the
    /// node snapshots (and so purges) when the log held in memory grows.
    sizes: BTreeMap<u64, u64>,
    held: u64,
    segments: Vec<Segment>,
    writer: BufWriter<File>,
    written: u64,
    purged: Option<LogId>,
    vote: Option<Vote>,
    committed: Option<LogId>,
    pending: Vec<Ack>,
    /// Size at which the active segment rolls ([`SEGMENT_BYTES`]).
    segment_bytes: u64,
    stats: WalStats,
}

/// Raft log storage over segment files. Cheap to clone; clones share state.
#[derive(Clone)]
pub struct Wal {
    inner: Arc<Mutex<Inner>>,
    flush: Arc<Notify>,
}

#[derive(Serialize, Deserialize, Default)]
struct Meta {
    purged: Option<LogId>,
    vote: Option<Vote>,
    committed: Option<LogId>,
}

fn io_err(error: impl std::error::Error + 'static) -> StorageError {
    StorageIOError::write_logs(openraft::AnyError::new(&error)).into()
}

fn segment_path(dir: &Path, first: u64) -> PathBuf {
    dir.join(format!("{first:020}.seg"))
}

fn encode(entry: &Entry) -> io::Result<Vec<u8>> {
    let payload = serde_json::to_vec(entry)?;
    let mut record = Vec::with_capacity(payload.len() + 8);
    record.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    record.extend_from_slice(&crc32fast::hash(&payload).to_le_bytes());
    record.extend_from_slice(&payload);
    Ok(record)
}

/// Read every intact record of a segment; a torn or corrupt tail ends it.
fn read_segment(path: &Path) -> io::Result<(Vec<Entry>, u64)> {
    let mut bytes = Vec::new();
    File::open(path)?.read_to_end(&mut bytes)?;
    let mut entries = Vec::new();
    let mut at = 0usize;
    while at + 8 <= bytes.len() {
        let (Ok(len), Ok(crc)) = (
            <[u8; 4]>::try_from(&bytes[at..at + 4]),
            <[u8; 4]>::try_from(&bytes[at + 4..at + 8]),
        ) else {
            break;
        };
        let (len, crc) = (u32::from_le_bytes(len) as usize, u32::from_le_bytes(crc));
        let Some(payload) = bytes.get(at + 8..at + 8 + len) else {
            break;
        };
        if crc32fast::hash(payload) != crc {
            break;
        }
        match serde_json::from_slice::<Entry>(payload) {
            Ok(entry) => entries.push(entry),
            Err(_) => break,
        }
        at += 8 + len;
    }
    Ok((entries, at as u64))
}

fn sync_dir(dir: &Path) -> io::Result<()> {
    File::open(dir)?.sync_all()
}

/// Who waits for an append to be durable.
enum Ack {
    Raft(LogFlushed<TypeConfig>),
    #[cfg(test)]
    Test(tokio::sync::oneshot::Sender<io::Result<()>>),
}

impl Ack {
    fn done(self, result: io::Result<()>) {
        match self {
            Self::Raft(callback) => callback.log_io_completed(result),
            #[cfg(test)]
            Self::Test(tx) => {
                let _ = tx.send(result);
            }
        }
    }
}

const REWRITE_TMP: &str = "rewrite.tmp";
const REWRITE_READY: &str = "rewrite.ready";

/// Finish a rewrite a crash interrupted: a ready one replaces every
/// segment, a tmp one never committed and is dropped.
fn recover_rewrite(dir: &Path) -> io::Result<()> {
    let ready = dir.join(REWRITE_READY);
    if ready.exists() {
        let (records, _) = read_segment(&ready)?;
        for entry in fs::read_dir(dir)? {
            let path = entry?.path();
            if path.extension().is_some_and(|ext| ext == "seg") {
                fs::remove_file(&path)?;
            }
        }
        let first = records.first().map(|entry| entry.log_id.index);
        match first {
            Some(first) => fs::rename(&ready, segment_path(dir, first))?,
            None => fs::remove_file(&ready)?,
        }
        sync_dir(dir)?;
    }
    match fs::remove_file(dir.join(REWRITE_TMP)) {
        Ok(()) => sync_dir(dir),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

/// Where a crash test may stop the log, as if the process died there.
#[cfg(test)]
pub(crate) mod crash {
    use std::cell::RefCell;
    use std::io;

    thread_local! {
        static ARMED: RefCell<Option<&'static str>> = const { RefCell::new(None) };
    }

    /// Stop at `point` the next time the log reaches it.
    pub(crate) fn arm(point: &'static str) {
        ARMED.with(|armed| *armed.borrow_mut() = Some(point));
    }

    pub(crate) fn point(name: &'static str) -> io::Result<()> {
        ARMED.with(|armed| {
            let mut armed = armed.borrow_mut();
            if *armed == Some(name) {
                *armed = None;
                return Err(io::Error::other(format!("crash at {name}")));
            }
            Ok(())
        })
    }
}

#[cfg(test)]
use crash::point as crash_point;

#[cfg(not(test))]
#[inline(always)]
fn crash_point(_name: &'static str) -> io::Result<()> {
    Ok(())
}

impl Wal {
    /// Open (or create) the WAL in `dir`, recovering intact records.
    pub fn open(dir: impl AsRef<Path>) -> io::Result<Self> {
        let dir = dir.as_ref().to_path_buf();
        fs::create_dir_all(&dir)?;
        recover_rewrite(&dir)?;
        let meta: Meta = match fs::read(dir.join("meta.json")) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(io::Error::other)?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => Meta::default(),
            Err(error) => return Err(error),
        };
        let mut firsts: Vec<u64> = fs::read_dir(&dir)?
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| {
                let name = entry.file_name().into_string().ok()?;
                name.strip_suffix(".seg")?.parse().ok()
            })
            .collect();
        firsts.sort_unstable();
        let mut entries = BTreeMap::new();
        let mut segments = Vec::new();
        let mut tail_len = 0;
        for (i, first) in firsts.iter().enumerate() {
            let path = segment_path(&dir, *first);
            let (records, intact) = read_segment(&path)?;
            for entry in records {
                entries.insert(entry.log_id.index, entry);
            }
            let is_last = i + 1 == firsts.len();
            if !is_last && intact != fs::metadata(&path)?.len() {
                return Err(io::Error::other(format!(
                    "segment {} is torn before the active one: the log has a hole",
                    path.display()
                )));
            }
            if is_last {
                // Drop a torn tail so new records follow intact ones.
                let file = OpenOptions::new().write(true).open(&path)?;
                file.set_len(intact)?;
                file.sync_all()?;
                tail_len = intact;
            }
            segments.push(Segment {
                first: *first,
                path,
            });
        }
        if let Some(purged) = meta.purged {
            entries.retain(|index, _| *index > purged.index);
        }
        if segments.is_empty() {
            let first = meta.purged.map(|p| p.index + 1).unwrap_or(0);
            segments.push(Segment {
                first,
                path: segment_path(&dir, first),
            });
        }
        let Some(active) = segments.last() else {
            return Err(io::Error::other("the log has no segment"));
        };
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&active.path)?;
        sync_dir(&dir)?;
        let sizes: BTreeMap<u64, u64> = entries
            .iter()
            .map(|(index, entry)| (*index, encode(entry).map(|r| r.len() as u64).unwrap_or(0)))
            .collect();
        let held = sizes.values().sum();
        let wal = Self {
            inner: Arc::new(Mutex::new(Inner {
                dir,
                entries,
                sizes,
                held,
                segments,
                writer: BufWriter::with_capacity(1 << 20, file),
                written: tail_len,
                purged: meta.purged,
                vote: meta.vote,
                committed: meta.committed,
                pending: Vec::new(),
                segment_bytes: SEGMENT_BYTES,
                stats: WalStats::default(),
            })),
            flush: Arc::new(Notify::new()),
        };
        wal.spawn_flusher();
        Ok(wal)
    }

    /// Buffer `entries` and queue `ack` for the next group fsync.
    fn write_entries(&self, entries: impl IntoIterator<Item = Entry>, ack: Ack) -> io::Result<()> {
        {
            let mut inner = self.inner.lock();
            for entry in entries {
                if inner.written >= inner.segment_bytes {
                    inner.roll(entry.log_id.index)?;
                }
                let record = encode(&entry)?;
                inner.writer.write_all(&record)?;
                inner.written += record.len() as u64;
                inner.stats.entries += 1;
                inner.held += record.len() as u64;
                if let Some(old) = inner.sizes.insert(entry.log_id.index, record.len() as u64) {
                    inner.held -= old;
                }
                inner.entries.insert(entry.log_id.index, entry);
            }
            inner.stats.appends += 1;
            inner.pending.push(ack);
        }
        self.flush.notify_one();
        Ok(())
    }

    pub fn stats(&self) -> WalStats {
        self.inner.lock().stats
    }

    /// Bytes of log entries held in memory (everything not yet purged).
    pub fn held_bytes(&self) -> u64 {
        self.inner.lock().held
    }

    /// Bytes of the entries held in memory that come after `index`.
    pub fn held_after(&self, index: u64) -> u64 {
        let inner = self.inner.lock();
        inner
            .sizes
            .range(index.saturating_add(1)..)
            .map(|(_, size)| size)
            .sum()
    }

    /// The group-commit loop: fsync once for everything written so far, then
    /// acknowledge it all.
    fn spawn_flusher(&self) {
        let inner = Arc::downgrade(&self.inner);
        let notify = self.flush.clone();
        tokio::spawn(async move {
            loop {
                notify.notified().await;
                let Some(inner) = inner.upgrade() else { return };
                let (file, callbacks) = {
                    let mut guard = inner.lock();
                    if guard.pending.is_empty() {
                        continue;
                    }
                    let flushed = guard.writer.flush();
                    let file = flushed.and_then(|_| guard.writer.get_ref().try_clone());
                    guard.stats.fsyncs += 1;
                    (file, std::mem::take(&mut guard.pending))
                };
                let t = std::time::Instant::now();
                let n = callbacks.len() as u64;
                let result = match file {
                    Ok(file) => tokio::task::spawn_blocking(move || {
                        let r = file.sync_data();
                        crate::prof::add("fsync", t, 1);
                        r
                    })
                    .await
                    .unwrap_or_else(|e| Err(io::Error::other(e))),
                    Err(error) => Err(error),
                };
                crate::prof::add("fsync_callbacks", std::time::Instant::now(), n);
                for callback in callbacks {
                    let result = match &result {
                        Ok(()) => Ok(()),
                        Err(error) => Err(io::Error::new(error.kind(), error.to_string())),
                    };
                    callback.done(result);
                }
            }
        });
    }
}

impl Inner {
    fn save_meta(&self) -> io::Result<()> {
        let meta = Meta {
            purged: self.purged,
            vote: self.vote,
            committed: self.committed,
        };
        let tmp = self.dir.join("meta.json.tmp");
        let mut file = File::create(&tmp)?;
        file.write_all(&serde_json::to_vec(&meta)?)?;
        file.sync_all()?;
        fs::rename(&tmp, self.dir.join("meta.json"))?;
        sync_dir(&self.dir)
    }

    fn roll(&mut self, first: u64) -> io::Result<()> {
        self.writer.flush()?;
        self.writer.get_ref().sync_data()?;
        let path = segment_path(&self.dir, first);
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        crash_point("roll:created")?;
        sync_dir(&self.dir)?;
        self.writer = BufWriter::with_capacity(1 << 20, file);
        self.written = 0;
        self.segments.push(Segment { first, path });
        Ok(())
    }

    /// Make everything written durable and acknowledge waiting appends.
    fn sync_now(&mut self) -> io::Result<()> {
        self.writer.flush()?;
        self.writer.get_ref().sync_data()?;
        for callback in std::mem::take(&mut self.pending) {
            callback.done(Ok(()));
        }
        Ok(())
    }

    /// Rewrite the whole log as one segment from the entries in memory.
    /// Only truncation (a new leader overwriting uncommitted entries) needs
    /// this, which is rare, so simplicity wins over copying less.
    fn rewrite_all(&mut self) -> io::Result<()> {
        self.sync_now()?;
        let first = self
            .entries
            .keys()
            .next()
            .copied()
            .unwrap_or_else(|| self.purged.map(|p| p.index + 1).unwrap_or(0));
        let tmp = self.dir.join(REWRITE_TMP);
        let ready = self.dir.join(REWRITE_READY);
        let mut file = BufWriter::new(File::create(&tmp)?);
        let mut written = 0;
        for entry in self.entries.values() {
            let record = encode(entry)?;
            written += record.len() as u64;
            file.write_all(&record)?;
        }
        file.flush()?;
        file.get_ref().sync_all()?;
        crash_point("rewrite:tmp-synced")?;
        // The commit point: from here a crash finishes the rewrite on open.
        fs::rename(&tmp, &ready)?;
        sync_dir(&self.dir)?;
        crash_point("rewrite:ready")?;
        for segment in self.segments.drain(..) {
            fs::remove_file(&segment.path)?;
            crash_point("rewrite:segment-removed")?;
        }
        let path = segment_path(&self.dir, first);
        fs::rename(&ready, &path)?;
        sync_dir(&self.dir)?;
        self.writer =
            BufWriter::with_capacity(1 << 20, OpenOptions::new().append(true).open(&path)?);
        self.segments.push(Segment { first, path });
        self.written = written;
        Ok(())
    }
}

impl RaftLogReader<TypeConfig> for Wal {
    async fn try_get_log_entries<RB: RangeBounds<u64> + Clone + Debug + OptionalSend>(
        &mut self,
        range: RB,
    ) -> Result<Vec<Entry>, StorageError> {
        let inner = self.inner.lock();
        Ok(inner
            .entries
            .range(range)
            .map(|(_, entry)| entry.clone())
            .collect())
    }
}

impl RaftLogStorage<TypeConfig> for Wal {
    type LogReader = Self;

    async fn get_log_state(&mut self) -> Result<LogState<TypeConfig>, StorageError> {
        let inner = self.inner.lock();
        let last = inner
            .entries
            .iter()
            .next_back()
            .map(|(_, entry)| entry.log_id)
            .or(inner.purged);
        Ok(LogState {
            last_purged_log_id: inner.purged,
            last_log_id: last,
        })
    }

    async fn get_log_reader(&mut self) -> Self::LogReader {
        self.clone()
    }

    async fn save_vote(&mut self, vote: &Vote) -> Result<(), StorageError> {
        let mut inner = self.inner.lock();
        inner.vote = Some(*vote);
        inner.save_meta().map_err(|e| {
            StorageError::from(StorageIOError::write_vote(openraft::AnyError::new(&e)))
        })
    }

    async fn read_vote(&mut self) -> Result<Option<Vote>, StorageError> {
        Ok(self.inner.lock().vote)
    }

    async fn save_committed(&mut self, committed: Option<LogId>) -> Result<(), StorageError> {
        // Not fsynced on every commit: the state machine rebuilds from the
        // snapshot plus the log, and Raft re-learns the commit index.
        self.inner.lock().committed = committed;
        Ok(())
    }

    async fn read_committed(&mut self) -> Result<Option<LogId>, StorageError> {
        Ok(self.inner.lock().committed)
    }

    async fn append<I>(
        &mut self,
        entries: I,
        callback: LogFlushed<TypeConfig>,
    ) -> Result<(), StorageError>
    where
        I: IntoIterator<Item = Entry> + OptionalSend,
        I::IntoIter: OptionalSend,
    {
        self.write_entries(entries, Ack::Raft(callback))
            .map_err(io_err)
    }

    async fn truncate(&mut self, log_id: LogId) -> Result<(), StorageError> {
        let mut inner = self.inner.lock();
        inner.entries.split_off(&log_id.index);
        let dropped: u64 = inner.sizes.split_off(&log_id.index).values().sum();
        inner.held -= dropped;
        inner.rewrite_all().map_err(io_err)
    }

    async fn purge(&mut self, log_id: LogId) -> Result<(), StorageError> {
        let mut inner = self.inner.lock();
        inner.sync_now().map_err(io_err)?;
        inner.purged = Some(log_id);
        inner.save_meta().map_err(io_err)?;
        crash_point("purge:meta-saved").map_err(io_err)?;
        inner.entries.retain(|index, _| *index > log_id.index);
        let kept = inner.sizes.split_off(&(log_id.index + 1));
        let dropped: u64 = std::mem::replace(&mut inner.sizes, kept).values().sum();
        inner.held -= dropped;
        // A segment is wholly purged when the next one starts at or before
        // the first entry we keep.
        while inner.segments.len() > 1 && inner.segments[1].first <= log_id.index + 1 {
            let segment = inner.segments.remove(0);
            let _ = fs::remove_file(&segment.path);
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "wal_tests.rs"]
pub(crate) mod tests;

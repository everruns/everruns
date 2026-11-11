//! The storage proof for the write-ahead log: what a crash can leave on
//! disk, and what the log must still hold after it.
//!
//! The crash model: the process dies at any point. Bytes not yet fsynced may
//! be lost, and a record being written may be cut anywhere (a torn tail).
//! Named crash points (`crash::arm`) stop the log between the steps of a
//! multi-file change (a rewrite, a roll, a purge). After every crash the log
//! is reopened and checked:
//! - every acknowledged entry is there, unchanged;
//! - the entries form one run with no hole;
//! - nothing appears that was never appended;
//! - new appends after recovery survive the next crash too.
//!
//! Each seeded run is deterministic: a failing seed replays exactly.

use std::collections::BTreeMap;
use std::path::Path;

use openraft::storage::RaftLogStorage;
use openraft::{CommittedLeaderId, RaftLogReader};

use super::*;

/// A small deterministic generator (xorshift64*), so a seed replays exactly
/// without a dependency.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
    fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }
}

fn entry(term: u64, index: u64) -> Entry {
    openraft::testing::blank_ent::<TypeConfig>(term, 1, index)
}

fn open(dir: &Path, segment_bytes: u64) -> Wal {
    let wal = Wal::open(dir).expect("the log opens");
    wal.inner.lock().segment_bytes = segment_bytes;
    wal
}

/// Append and wait for the group fsync, as Raft does before it counts the
/// entries.
async fn append_durable(wal: &Wal, entries: Vec<Entry>) {
    let (tx, rx) = tokio::sync::oneshot::channel();
    wal.write_entries(entries, Ack::Test(tx)).expect("append");
    rx.await.expect("the flusher answers").expect("fsync");
}

/// Append without waiting: the entries may or may not survive a crash.
fn append_unacked(wal: &Wal, entries: Vec<Entry>) {
    let (tx, _rx) = tokio::sync::oneshot::channel();
    wal.write_entries(entries, Ack::Test(tx)).expect("append");
}

/// The log ids the log holds, by index (entries compare by their id).
async fn held(wal: &Wal) -> BTreeMap<u64, LogId> {
    let mut reader = wal.clone();
    reader
        .try_get_log_entries(..)
        .await
        .expect("read")
        .into_iter()
        .map(|entry| (entry.log_id.index, entry.log_id))
        .collect()
}

fn segments(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut paths: Vec<_> = fs::read_dir(dir)
        .expect("dir")
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "seg"))
        .collect();
    paths.sort();
    paths
}

/// Kill the process: drop the log, then cut the active segment anywhere
/// past the last acknowledged byte and maybe leave garbage where the next
/// record was being written.
async fn crash(wal: Wal, dir: &Path, durable_len: u64, rng: &mut Rng) {
    drop(wal);
    // Let the flusher see the log is gone.
    tokio::task::yield_now().await;
    let Some(active) = segments(dir).pop() else {
        return;
    };
    let len = fs::metadata(&active).expect("segment").len();
    let floor = durable_len.min(len);
    let cut = floor + rng.below(len - floor + 1);
    let file = OpenOptions::new()
        .write(true)
        .open(&active)
        .expect("segment");
    file.set_len(cut).expect("cut");
    if rng.chance(30) {
        let garbage: Vec<u8> = (0..rng.below(40) + 1).map(|_| rng.next() as u8).collect();
        let mut file = OpenOptions::new()
            .append(true)
            .open(&active)
            .expect("segment");
        file.write_all(&garbage).expect("garbage");
    }
}

fn active_len(dir: &Path) -> u64 {
    segments(dir)
        .pop()
        .map(|path| fs::metadata(path).expect("segment").len())
        .unwrap_or(0)
}

fn check_run(entries: &BTreeMap<u64, LogId>, seed: u64) {
    let indexes: Vec<u64> = entries.keys().copied().collect();
    for pair in indexes.windows(2) {
        assert_eq!(
            pair[1],
            pair[0] + 1,
            "seed {seed}: a hole after {}",
            pair[0]
        );
    }
}

#[tokio::test]
async fn acknowledged_entries_survive_any_crash_and_torn_tail() {
    for seed in 0..200 {
        let dir = tempfile::tempdir().expect("tempdir");
        let dir = dir.path();
        let mut rng = Rng::new(seed);
        // Small segments, so rolls happen in most runs.
        let segment_bytes = 256 + rng.below(2048);
        let mut acked: BTreeMap<u64, LogId> = BTreeMap::new();
        let mut written: BTreeMap<u64, LogId> = BTreeMap::new();
        let mut term = 1;
        let mut next = 1;
        for _round in 0..6 {
            let wal = open(dir, segment_bytes);
            let after = held(&wal).await;
            check_run(&after, seed);
            for (index, entry) in &acked {
                assert_eq!(
                    after.get(index),
                    Some(entry),
                    "seed {seed}: acked {index} lost"
                );
            }
            for (index, entry) in &after {
                assert_eq!(
                    written.get(index),
                    Some(entry),
                    "seed {seed}: {index} was never appended"
                );
            }
            // What survived unacknowledged is now part of the log.
            acked = after.clone();
            next = after.keys().next_back().map_or(next, |last| last + 1);
            let mut durable_len = active_len(dir);
            for _ in 0..rng.below(6) + 1 {
                // Sometimes a new leader overwrites the unacknowledged end.
                if rng.chance(15) && next > 2 {
                    let from = next - 1 - rng.below(next.min(4) - 1);
                    if acked.keys().next_back().is_none_or(|last| *last < from) {
                        let mut store = wal.clone();
                        store
                            .truncate(openraft::LogId::new(CommittedLeaderId::new(term, 1), from))
                            .await
                            .expect("truncate");
                        written.split_off(&from);
                        next = from;
                        term += 1;
                        durable_len = active_len(dir);
                    }
                }
                let batch: Vec<Entry> = (0..rng.below(8) + 1)
                    .map(|i| entry(term, next + i))
                    .collect();
                next += batch.len() as u64;
                for entry in &batch {
                    written.insert(entry.log_id.index, entry.log_id);
                }
                if rng.chance(70) {
                    for entry in &batch {
                        acked.insert(entry.log_id.index, entry.log_id);
                    }
                    append_durable(&wal, batch).await;
                    durable_len = active_len(dir);
                } else {
                    append_unacked(&wal, batch);
                }
            }
            crash(wal, dir, durable_len, &mut rng).await;
        }
    }
}

/// The bug the proof found: a truncation rewrote the log by deleting the
/// segments, then renaming the rewrite in. A crash between the two left an
/// empty log.
#[tokio::test]
async fn a_crash_anywhere_in_a_rewrite_keeps_every_entry_before_the_truncation() {
    for point in [
        "rewrite:tmp-synced",
        "rewrite:ready",
        "rewrite:segment-removed",
    ] {
        let dir = tempfile::tempdir().expect("tempdir");
        let dir = dir.path();
        let wal = open(dir, 300);
        let entries: Vec<Entry> = (1..=40).map(|index| entry(1, index)).collect();
        append_durable(&wal, entries.clone()).await;
        assert!(segments(dir).len() > 2, "the log spans segments");
        crash::arm(point);
        let mut store = wal.clone();
        let truncated = store
            .truncate(openraft::LogId::new(CommittedLeaderId::new(1, 1), 30))
            .await;
        assert!(truncated.is_err(), "{point}: the crash point fired");
        drop(store);
        drop(wal);
        let after = held(&open(dir, 300)).await;
        check_run(&after, 0);
        for entry in &entries[..29] {
            assert_eq!(
                after.get(&entry.log_id.index),
                Some(&entry.log_id),
                "{point}"
            );
        }
        // Before the commit point the old tail may come back (Raft
        // truncates it again); after it, the rewrite wins.
        let expected_last = if point == "rewrite:tmp-synced" {
            40
        } else {
            29
        };
        assert_eq!(after.keys().next_back(), Some(&expected_last), "{point}");
        assert!(!dir.join(REWRITE_TMP).exists() && !dir.join(REWRITE_READY).exists());
    }
}

#[tokio::test]
async fn a_crash_after_a_purge_is_recorded_keeps_what_follows_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    let dir = dir.path();
    let wal = open(dir, 300);
    let entries: Vec<Entry> = (1..=40).map(|index| entry(1, index)).collect();
    append_durable(&wal, entries.clone()).await;
    crash::arm("purge:meta-saved");
    let mut store = wal.clone();
    let purged = store
        .purge(openraft::LogId::new(CommittedLeaderId::new(1, 1), 20))
        .await;
    assert!(purged.is_err());
    drop(store);
    drop(wal);
    let mut reopened = open(dir, 300);
    let state = reopened.get_log_state().await.expect("state");
    assert_eq!(state.last_purged_log_id.map(|id| id.index), Some(20));
    let after = held(&reopened).await;
    assert_eq!(after.keys().next(), Some(&21));
    assert_eq!(after.keys().next_back(), Some(&40));
}

#[tokio::test]
async fn a_crash_while_rolling_a_segment_keeps_what_was_acknowledged() {
    let dir = tempfile::tempdir().expect("tempdir");
    let dir = dir.path();
    let wal = open(dir, 300);
    let first: Vec<Entry> = (1..=8).map(|index| entry(1, index)).collect();
    append_durable(&wal, first.clone()).await;
    crash::arm("roll:created");
    let more: Vec<Entry> = (9..=40).map(|index| entry(1, index)).collect();
    assert!(
        wal.write_entries(more, Ack::Test(tokio::sync::oneshot::channel().0))
            .is_err()
    );
    drop(wal);
    let wal = open(dir, 300);
    let after = held(&wal).await;
    check_run(&after, 0);
    for entry in &first {
        assert_eq!(after.get(&entry.log_id.index), Some(&entry.log_id));
    }
    let next = after.keys().next_back().map_or(1, |last| last + 1);
    append_durable(&wal, vec![entry(1, next)]).await;
    drop(wal);
    assert_eq!(held(&open(dir, 300)).await.keys().next_back(), Some(&next));
}

#[tokio::test]
async fn a_tear_before_the_active_segment_refuses_to_open() {
    let dir = tempfile::tempdir().expect("tempdir");
    let dir = dir.path();
    let wal = open(dir, 300);
    append_durable(&wal, (1..=40).map(|index| entry(1, index)).collect()).await;
    drop(wal);
    let older = segments(dir).remove(0);
    let len = fs::metadata(&older).expect("segment").len();
    OpenOptions::new()
        .write(true)
        .open(&older)
        .expect("segment")
        .set_len(len - 3)
        .expect("tear");
    let error = Wal::open(dir).err().expect("a hole refuses to open");
    assert!(error.to_string().contains("hole"), "{error}");
}

/// openraft's own storage conformance suite, against the log and the state
/// machine a node runs.
#[test]
fn the_log_and_state_machine_pass_openraft_storage_suite() {
    struct Builder;

    impl
        openraft::testing::StoreBuilder<TypeConfig, Wal, crate::sm::StateMachine, tempfile::TempDir>
        for Builder
    {
        async fn build(
            &self,
        ) -> Result<(tempfile::TempDir, Wal, crate::sm::StateMachine), StorageError> {
            let dir = tempfile::tempdir().expect("tempdir");
            let wal = Wal::open(dir.path().join("wal")).expect("wal");
            let store = crate::store::Store::open("memory:///").expect("store");
            let shared = crate::sm::Shared::new(store, 0);
            let sm = crate::sm::StateMachine::open(dir.path().join("snapshots"), shared)
                .expect("state machine");
            Ok((dir, wal, sm))
        }
    }

    openraft::testing::Suite::test_all(Builder).expect("openraft's storage suite");
}

/// The node snapshots on the log written since the last snapshot, not on
/// the tail kept behind it for lagging followers.
#[tokio::test]
async fn held_after_counts_only_the_entries_past_an_index() {
    let dir = tempfile::tempdir().expect("tempdir");
    let wal = open(dir.path(), 1 << 20);
    append_durable(&wal, (1..=40).map(|index| entry(1, index)).collect()).await;
    let all = wal.held_bytes();
    assert_eq!(wal.held_after(0), all);
    assert_eq!(wal.held_after(40), 0);
    let tail = wal.held_after(30);
    assert!(tail > 0 && tail < wal.held_after(20) && wal.held_after(20) < all);
    let mut store = wal.clone();
    store
        .purge(openraft::LogId::new(CommittedLeaderId::new(1, 1), 20))
        .await
        .expect("purge");
    assert_eq!(wal.held_bytes(), wal.held_after(20));
    assert_eq!(wal.held_after(30), tail);
}

//! Restore points: what disaster recovery rebuilds a shard from (data plane
//! v2 design, "Disaster recovery").
//!
//! A restore point is an archived snapshot plus the log its shard shipped
//! after it, and everything they name: session segments, blobs, workspace
//! objects. In the object store:
//!
//! - `shards/<shard>/snapshots/i<inc>-n<index>-t<clock_ms>-<id>.snap`: the
//!   archived snapshots, named by the incarnation, the last log index and
//!   the shard clock they hold, so a restore picks one without reading it.
//! - `shards/<shard>/log/i<inc>/<first>-<last>.json`: the shipped log of
//!   each incarnation (`ship.rs`). A rebuilt shard reuses log indexes the
//!   lost one had used, so each incarnation ships under its own prefix.
//! - `shards/<shard>/incarnations/<inc>`: one object per incarnation, created
//!   once (`claim_incarnation`), so two restores never share a number.
//! - `retired/<hour>/<session>/<segment>`: legacy segments (objects of
//!   their own) a compaction replaced, by the hour they were replaced in.
//! - `packs/`, `shards/<shard>/packrefs/` and `packs-pinned/`: segment
//!   packs, which snapshot names which, and what a restore brought back
//!   (`packs.rs`, swept with the retired segments).
//!
//! Decisions:
//! - Every point inside the restore window (7 days by default,
//!   `AGENTD_RESTORE_WINDOW_HOURS`) can be restored. Nothing a point in the
//!   window refers to is deleted: a replaced segment waits a full window
//!   after it was replaced; the shipped log stays while a kept snapshot is
//!   older than it.
//! - Snapshot copies are thinned, not all kept: a shard at full load
//!   snapshots every few minutes and a copy is over 100 MB. Kept: the
//!   newest [`SNAPSHOT_COPIES`], the first of each hour inside the window,
//!   and the newest one before the window, the base for a point in its
//!   first hour.
//! - Only expired restore points are ever deleted. Session records stay
//!   (data rule); what goes are superseded copies of them.

use std::time::Duration;

use everruns_contracts::typed_id::SessionId;
use futures::TryStreamExt;
use object_store::path::Path;
use object_store::{ObjectStoreExt, PutMode, PutOptions, PutPayload};

use crate::store::{SegRef, Store, parse_segment, retry};

/// Snapshot copies always kept per shard, whatever their age.
pub const SNAPSHOT_COPIES: usize = 3;

const HOUR_MS: i64 = 3_600_000;

/// How far back a shard can be restored.
pub fn restore_window() -> Duration {
    let hours = std::env::var("AGENTD_RESTORE_WINDOW_HOURS")
        .ok()
        .and_then(|h| h.parse::<u64>().ok())
        .unwrap_or(7 * 24);
    Duration::from_secs(hours * 3600)
}

/// One archived snapshot, as its name describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotCopy {
    /// The shard incarnation that took it.
    pub inc: u64,
    /// Its last log index.
    pub index: u64,
    /// The shard clock it holds (the newest command time it applied).
    pub clock_ms: i64,
    /// When it was stored.
    pub stored_ms: i64,
    pub(crate) key: Path,
}

/// What [`Store::collect`] deleted.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Collected {
    pub snapshots: usize,
    pub logs: usize,
    pub segments: usize,
    pub packs: usize,
}

impl Store {
    /// Keep a copy of a shard snapshot, then thin the shard's copies.
    pub async fn put_snapshot(
        &self,
        shard: u32,
        id: &str,
        inc: u64,
        index: u64,
        clock_ms: i64,
        parts: impl IntoIterator<Item = bytes::Bytes>,
    ) -> anyhow::Result<()> {
        let id: String = id
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
        let key = self.key(&format!(
            "shards/{shard}/snapshots/i{inc}-n{index}-t{clock_ms}-{id}.snap"
        ));
        let parts: Vec<bytes::Bytes> = parts.into_iter().collect();
        // The sidecar first (`packs.rs`): a kept snapshot always has one.
        if let Some(state) = parts.last() {
            self.put_pack_refs(shard, &key, state.clone()).await?;
        }
        let payload: PutPayload = parts.into_iter().collect();
        retry(|| async {
            crate::store::count_put(&key);
            self.inner.put(&key, payload.clone()).await.map(|_| ())
        })
        .await?;
        let now = now_ms();
        let copies = self.snapshot_copies(shard).await?;
        for gone in thin(&copies, restore_window(), now) {
            let _ = self.inner.delete(&gone.key).await;
            let _ = self
                .inner
                .delete(&self.pack_refs_key(shard, &gone.key))
                .await;
        }
        Ok(())
    }

    /// Every archived snapshot of `shard`, oldest first.
    pub async fn snapshot_copies(&self, shard: u32) -> anyhow::Result<Vec<SnapshotCopy>> {
        let dir = self.key(&format!("shards/{shard}/snapshots"));
        let metas: Vec<_> = self.inner.list(Some(&dir)).try_collect().await?;
        let mut copies: Vec<SnapshotCopy> = metas
            .into_iter()
            .filter_map(|meta| {
                let (inc, index, clock_ms) = parse_snapshot(meta.location.filename()?)?;
                Some(SnapshotCopy {
                    inc,
                    index,
                    clock_ms,
                    stored_ms: meta.last_modified.timestamp_millis(),
                    key: meta.location,
                })
            })
            .collect();
        copies.sort_by_key(|c| c.stored_ms);
        Ok(copies)
    }

    /// The archived snapshot `copy`, as stored.
    pub async fn get_snapshot(&self, copy: &SnapshotCopy) -> anyhow::Result<bytes::Bytes> {
        self.get(copy.key.clone()).await
    }

    /// Ship committed log entries `first..=last` of `shard`'s incarnation
    /// `inc` (see `ship`). Idempotent: the same range of one incarnation
    /// always holds the same entries.
    pub async fn put_log(
        &self,
        shard: u32,
        inc: u64,
        first: u64,
        last: u64,
        bytes: Vec<u8>,
    ) -> anyhow::Result<()> {
        self.put(self.log_key(shard, inc, first, last), bytes).await
    }

    /// The shipped ranges of `shard`'s incarnation `inc`, by first index.
    pub async fn shipped(&self, shard: u32, inc: u64) -> anyhow::Result<Vec<(u64, u64)>> {
        Ok(self
            .shipped_objects(shard, inc)
            .await?
            .into_iter()
            .map(|(range, _, _)| range)
            .collect())
    }

    async fn shipped_objects(
        &self,
        shard: u32,
        inc: u64,
    ) -> anyhow::Result<Vec<((u64, u64), i64, Path)>> {
        let dir = self.key(&format!("shards/{shard}/log/i{inc}"));
        let objects: Vec<_> = self.inner.list(Some(&dir)).try_collect().await?;
        let mut ranges: Vec<_> = objects
            .into_iter()
            .filter_map(|meta| {
                let (first, last) = meta
                    .location
                    .filename()?
                    .strip_suffix(".json")?
                    .split_once('-')?;
                let range = (first.parse().ok()?, last.parse().ok()?);
                Some((range, meta.last_modified.timestamp_millis(), meta.location))
            })
            .collect();
        ranges.sort_unstable_by_key(|(range, ..)| *range);
        Ok(ranges)
    }

    pub async fn get_log(
        &self,
        shard: u32,
        inc: u64,
        first: u64,
        last: u64,
    ) -> anyhow::Result<bytes::Bytes> {
        self.get(self.log_key(shard, inc, first, last)).await
    }

    fn log_key(&self, shard: u32, inc: u64, first: u64, last: u64) -> Path {
        self.key(&format!(
            "shards/{shard}/log/i{inc}/{first:020}-{last:020}.json"
        ))
    }

    /// The incarnations of `shard` taken so far, ascending.
    pub async fn incarnations(&self, shard: u32) -> anyhow::Result<Vec<u64>> {
        let dir = self.key(&format!("shards/{shard}/incarnations"));
        let metas: Vec<_> = self.inner.list(Some(&dir)).try_collect().await?;
        let mut taken: Vec<u64> = metas
            .iter()
            .filter_map(|m| m.location.filename()?.parse().ok())
            .collect();
        taken.sort_unstable();
        Ok(taken)
    }

    /// Take the next free incarnation of `shard`, at least `at_least`. The
    /// object is created only if absent, so two restores racing for a
    /// number never both get it.
    pub async fn claim_incarnation(&self, shard: u32, at_least: u64) -> anyhow::Result<u64> {
        let mut next = self
            .incarnations(shard)
            .await?
            .last()
            .map_or(at_least, |n| (n + 1).max(at_least));
        loop {
            let key = self.key(&format!("shards/{shard}/incarnations/{next}"));
            crate::store::count_put(&key);
            let created = self
                .inner
                .put_opts(
                    &key,
                    PutPayload::from(now_ms().to_string().into_bytes()),
                    PutOptions::from(PutMode::Create),
                )
                .await;
            match created {
                Ok(_) => return Ok(next),
                Err(object_store::Error::AlreadyExists { .. }) => next += 1,
                Err(error) => return Err(error.into()),
            }
        }
    }

    /// Mark `segments` of `session` replaced now; [`collect`](Self::collect)
    /// deletes them a restore window later.
    pub async fn retire_segments(
        &self,
        session: SessionId,
        segments: &[SegRef],
    ) -> anyhow::Result<()> {
        let hour = now_ms() / HOUR_MS;
        for seg in segments {
            let name = self
                .segment_key(session, *seg)
                .filename()
                .unwrap_or_default()
                .to_owned();
            self.put(
                self.key(&format!("retired/{hour:010}/{session}/{name}")),
                Vec::new(),
            )
            .await?;
        }
        Ok(())
    }

    /// Take back the retirement of every segment in `referenced`: a restored
    /// state refers to them again. Returns how many marks went.
    ///
    /// Decision: a rebuild or an org restore can bring back a state from
    /// before a compaction, whose pieces were retired since. Their marks
    /// would otherwise expire a window later and the collector would
    /// delete history the restored sessions read. Restores are rare, so
    /// listing every mark is affordable there and costs nothing on the
    /// write path.
    pub async fn unretire(
        &self,
        referenced: &std::collections::HashSet<(SessionId, SegRef)>,
    ) -> anyhow::Result<usize> {
        if referenced.is_empty() {
            return Ok(0);
        }
        let marks: Vec<_> = self
            .inner
            .list(Some(&self.key("retired")))
            .try_collect()
            .await?;
        let mut taken = 0;
        for mark in marks {
            let parts: Vec<_> = mark.location.parts().collect();
            let n = parts.len();
            if n >= 2
                && let (Ok(session), Some(seg)) = (
                    SessionId::parse(parts[n - 2].as_ref()),
                    parse_segment(parts[n - 1].as_ref()),
                )
                && referenced.contains(&(session, seg))
            {
                self.inner.delete(&mark.location).await?;
                taken += 1;
            }
        }
        Ok(taken)
    }

    /// Delete what no restore point inside `window` can refer to: thinned
    /// snapshot copies and shipped log of `shard`, and (with `segments`) the
    /// segments retired longer than `window` ago, fleet-wide.
    pub async fn collect(
        &self,
        shard: u32,
        segments: bool,
        window: Duration,
        now_ms: i64,
    ) -> anyhow::Result<Collected> {
        let mut done = Collected::default();
        let copies = self.snapshot_copies(shard).await?;
        for gone in thin(&copies, window, now_ms) {
            self.inner.delete(&gone.key).await?;
            let _ = self
                .inner
                .delete(&self.pack_refs_key(shard, &gone.key))
                .await;
            done.snapshots += 1;
        }
        let kept: Vec<_> = self.snapshot_copies(shard).await?;
        let window_ms = window.as_millis() as i64;
        let mut incs: Vec<u64> = kept.iter().map(|c| c.inc).collect();
        incs.sort_unstable();
        incs.dedup();
        for inc in incs {
            // The log before an incarnation's oldest kept snapshot is no
            // base for anything once it is older than the window.
            let Some(base) = kept.iter().filter(|c| c.inc == inc).map(|c| c.index).min() else {
                continue;
            };
            for ((_, last), stored_ms, key) in self.shipped_objects(shard, inc).await? {
                if last < base && stored_ms < now_ms - window_ms {
                    self.inner.delete(&key).await?;
                    done.logs += 1;
                }
            }
        }
        if segments {
            done.segments = self.collect_segments(window, now_ms).await?;
            let swept = self.sweep_packs(window, now_ms).await?;
            if let Some(why) = &swept.skipped {
                tracing::warn!(%why, "pack sweep skipped");
            }
            done.packs = swept.deleted;
        }
        Ok(done)
    }

    async fn collect_segments(&self, window: Duration, now_ms: i64) -> anyhow::Result<usize> {
        let before = (now_ms - window.as_millis() as i64) / HOUR_MS;
        let root = self.key("retired");
        let listed = self.inner.list_with_delimiter(Some(&root)).await?;
        let mut deleted = 0;
        for hour in listed.common_prefixes {
            let expired = hour
                .filename()
                .and_then(|h| h.parse::<i64>().ok())
                .is_some_and(|h| h < before);
            if !expired {
                continue;
            }
            let marks: Vec<_> = self.inner.list(Some(&hour)).try_collect().await?;
            for mark in marks {
                let parts: Vec<_> = mark.location.parts().collect();
                let n = parts.len();
                if n >= 2
                    && let (Ok(session), Some(seg)) = (
                        SessionId::parse(parts[n - 2].as_ref()),
                        parse_segment(parts[n - 1].as_ref()),
                    )
                {
                    // Decision: only the copy the mark retired goes. A
                    // segment written again after its mark (the same
                    // range re-flushed by a later leader) is a new object
                    // and stays. The object store has no "delete if
                    // unchanged", so this is a read then a delete; a write
                    // landing between the two is the remaining window,
                    // the width of two requests at most an hour after the
                    // mark expired.
                    let key = self.segment_key(session, seg);
                    let rewritten = match self.inner.head(&key).await {
                        Ok(meta) => meta.last_modified > mark.last_modified,
                        Err(object_store::Error::NotFound { .. }) => false,
                        Err(error) => return Err(error.into()),
                    };
                    if !rewritten {
                        let _ = self.inner.delete(&key).await;
                        let _ = self.inner.delete(&self.messages_key(session, seg)).await;
                        deleted += 1;
                    }
                }
                self.inner.delete(&mark.location).await?;
            }
        }
        Ok(deleted)
    }
}

/// `i2-n4100-t1700000000000-<id>.snap` as (incarnation, index, clock).
fn parse_snapshot(name: &str) -> Option<(u64, u64, i64)> {
    let rest = name.strip_suffix(".snap")?.strip_prefix('i')?;
    let mut parts = rest.splitn(4, '-');
    let inc = parts.next()?.parse().ok()?;
    let index = parts.next()?.strip_prefix('n')?.parse().ok()?;
    let clock = parts.next()?.strip_prefix('t')?.parse().ok()?;
    Some((inc, index, clock))
}

/// The copies to delete from `copies` (oldest first) at `now_ms`.
fn thin(copies: &[SnapshotCopy], window: Duration, now_ms: i64) -> Vec<SnapshotCopy> {
    let start = now_ms - window.as_millis() as i64;
    let newest: Vec<&SnapshotCopy> = copies.iter().rev().take(SNAPSHOT_COPIES).collect();
    let base = copies.iter().rev().find(|c| c.stored_ms < start);
    let mut hours = std::collections::BTreeSet::new();
    let mut gone = Vec::new();
    for copy in copies {
        let keep = newest.contains(&copy)
            || base == Some(copy)
            || (copy.stored_ms >= start && hours.insert(copy.stored_ms / HOUR_MS));
        if !keep {
            gone.push(copy.clone());
        }
    }
    gone
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn copy(stored_ms: i64) -> SnapshotCopy {
        SnapshotCopy {
            inc: 1,
            index: stored_ms as u64,
            clock_ms: stored_ms,
            stored_ms,
            key: Path::from(format!("s/{stored_ms}")),
        }
    }

    #[test]
    fn thinning_keeps_one_copy_an_hour_inside_the_window() {
        let now = 100 * HOUR_MS;
        let window = Duration::from_secs(10 * 3600);
        // Every 20 minutes for the last 30 hours.
        let copies: Vec<_> = (0..90)
            .map(|i| copy(now - 30 * HOUR_MS + i * HOUR_MS / 3))
            .collect();
        let gone = thin(&copies, window, now);
        let kept: Vec<_> = copies.iter().filter(|c| !gone.contains(c)).collect();
        let inside: Vec<_> = kept
            .iter()
            .filter(|c| c.stored_ms >= now - 10 * HOUR_MS)
            .collect();
        // One an hour for 10 hours, plus the newest three (two of them in
        // the newest hour already), plus one base before the window.
        assert_eq!(inside.len(), 12, "{inside:?}");
        assert_eq!(kept.len() - inside.len(), 1, "one base before the window");
        assert!(kept.iter().any(|c| c.stored_ms == copies[89].stored_ms));
    }

    #[test]
    fn snapshot_names_carry_what_a_restore_picks_by() {
        assert_eq!(
            parse_snapshot("i2-n4100-t1700000000000-1_2_4100_0.snap"),
            Some((2, 4100, 1_700_000_000_000))
        );
        assert_eq!(parse_snapshot("1-2-4100-0.snap"), None);
    }

    #[tokio::test]
    async fn an_incarnation_is_claimed_once() {
        let store = Store::open("memory:///").unwrap();
        assert_eq!(store.claim_incarnation(3, 2).await.unwrap(), 2);
        assert_eq!(store.claim_incarnation(3, 2).await.unwrap(), 3);
        assert_eq!(store.claim_incarnation(3, 1).await.unwrap(), 4);
        assert_eq!(store.incarnations(3).await.unwrap(), vec![2, 3, 4]);
    }

    #[tokio::test]
    async fn a_retired_segment_waits_a_window_before_it_goes() {
        let store = Store::open("memory:///").unwrap();
        let session = SessionId::new();
        let seg = SegRef::legacy(1, 5, 1);
        store
            .put(store.segment_key(session, seg), b"[]".to_vec())
            .await
            .unwrap();
        store.retire_segments(session, &[seg]).await.unwrap();
        let window = Duration::from_secs(3600);
        let now = now_ms();
        let early = store.collect(0, true, window, now).await.unwrap();
        assert_eq!(early.segments, 0);
        assert!(store.get(store.segment_key(session, seg)).await.is_ok());
        let late = store
            .collect(0, true, window, now + 3 * HOUR_MS)
            .await
            .unwrap();
        assert_eq!(late.segments, 1);
        assert!(store.get(store.segment_key(session, seg)).await.is_err());
    }

    /// One process, no race: write, retire, write the same segment again.
    /// The retirement was of the first write; the collector must not take
    /// the second with it (the bug class a test double hides when its
    /// "delete if unchanged" is atomic and the real store's is not).
    #[tokio::test]
    async fn a_segment_written_again_after_retiring_outlives_the_mark() {
        let store = Store::open("memory:///").unwrap();
        let session = SessionId::new();
        let seg = SegRef::legacy(1, 5, 1);
        let key = store.segment_key(session, seg);
        store.put(key.clone(), b"[]".to_vec()).await.unwrap();
        store.retire_segments(session, &[seg]).await.unwrap();
        // Object times have millisecond resolution or worse.
        tokio::time::sleep(Duration::from_millis(20)).await;
        store.put(key.clone(), b"[]".to_vec()).await.unwrap();
        let window = Duration::from_secs(3600);
        let late = store
            .collect(0, true, window, now_ms() + 3 * HOUR_MS)
            .await
            .unwrap();
        assert_eq!(late.segments, 0);
        assert!(store.get(key).await.is_ok(), "the second write was deleted");
    }

    /// A restore back to before a compaction refers to its retired pieces
    /// again; they must outlive their marks.
    #[tokio::test]
    async fn a_restored_reference_takes_back_the_retirement() {
        let store = Store::open("memory:///").unwrap();
        let session = SessionId::new();
        let (kept, gone) = (SegRef::legacy(1, 5, 1), SegRef::legacy(6, 9, 1));
        for seg in [kept, gone] {
            store
                .put(store.segment_key(session, seg), b"[]".to_vec())
                .await
                .unwrap();
        }
        store.retire_segments(session, &[kept, gone]).await.unwrap();
        let referenced = std::collections::HashSet::from([(session, kept)]);
        assert_eq!(store.unretire(&referenced).await.unwrap(), 1);
        let late = store
            .collect(0, true, Duration::from_secs(3600), now_ms() + 3 * HOUR_MS)
            .await
            .unwrap();
        assert_eq!(late.segments, 1);
        assert!(store.get(store.segment_key(session, kept)).await.is_ok());
        assert!(store.get(store.segment_key(session, gone)).await.is_err());
    }
}

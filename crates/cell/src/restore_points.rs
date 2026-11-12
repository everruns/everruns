//! Restore points: what disaster recovery rebuilds a cell from (data plane
//! v2 design, "Disaster recovery").
//!
//! A restore point is an archived snapshot plus the log its cell shipped
//! after it, and everything they name: session segments, blobs, workspace
//! objects. In the object store:
//!
//! - `cells/<cell>/snapshots/i<inc>-n<index>-t<clock_ms>-<id>.snap`: the
//!   archived snapshots, named by the incarnation, the last log index and
//!   the cell clock they hold, so a restore picks one without reading it.
//! - `cells/<cell>/log/i<inc>/<first>-<last>.json`: the shipped log of
//!   each incarnation (`ship.rs`). A rebuilt cell reuses log indexes the
//!   lost one had used, so each incarnation ships under its own prefix.
//! - `cells/<cell>/incarnations/<inc>`: one object per incarnation, created
//!   once (`claim_incarnation`), so two restores never share a number.
//! - `retired/<hour>/<session>/<segment>`: segments a compaction replaced,
//!   by the hour they were replaced in.
//!
//! Decisions:
//! - Every point inside the restore window (7 days by default,
//!   `CELL_RESTORE_WINDOW_HOURS`) can be restored. Nothing a point in the
//!   window refers to is deleted: a replaced segment waits a full window
//!   after it was replaced; the shipped log stays while a kept snapshot is
//!   older than it.
//! - Snapshot copies are thinned, not all kept: a cell at full load
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

/// Snapshot copies always kept per cell, whatever their age.
pub const SNAPSHOT_COPIES: usize = 3;

const HOUR_MS: i64 = 3_600_000;

/// How far back a cell can be restored.
pub fn restore_window() -> Duration {
    let hours = std::env::var("CELL_RESTORE_WINDOW_HOURS")
        .ok()
        .and_then(|h| h.parse::<u64>().ok())
        .unwrap_or(7 * 24);
    Duration::from_secs(hours * 3600)
}

/// One archived snapshot, as its name describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotCopy {
    /// The cell incarnation that took it.
    pub inc: u64,
    /// Its last log index.
    pub index: u64,
    /// The cell clock it holds (the newest command time it applied).
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
}

impl Store {
    /// Keep a copy of a cell snapshot, then thin the cell's copies.
    pub async fn put_snapshot(
        &self,
        cell: u32,
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
            "cells/{cell}/snapshots/i{inc}-n{index}-t{clock_ms}-{id}.snap"
        ));
        let payload: PutPayload = parts.into_iter().collect();
        retry(|| async { self.inner.put(&key, payload.clone()).await.map(|_| ()) }).await?;
        let now = now_ms();
        let copies = self.snapshot_copies(cell).await?;
        for gone in thin(&copies, restore_window(), now) {
            let _ = self.inner.delete(&gone.key).await;
        }
        Ok(())
    }

    /// Every archived snapshot of `cell`, oldest first.
    pub async fn snapshot_copies(&self, cell: u32) -> anyhow::Result<Vec<SnapshotCopy>> {
        let dir = self.key(&format!("cells/{cell}/snapshots"));
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

    /// Ship committed log entries `first..=last` of `cell`'s incarnation
    /// `inc` (see `ship`). Idempotent: the same range of one incarnation
    /// always holds the same entries.
    pub async fn put_log(
        &self,
        cell: u32,
        inc: u64,
        first: u64,
        last: u64,
        bytes: Vec<u8>,
    ) -> anyhow::Result<()> {
        self.put(self.log_key(cell, inc, first, last), bytes).await
    }

    /// The shipped ranges of `cell`'s incarnation `inc`, by first index.
    pub async fn shipped(&self, cell: u32, inc: u64) -> anyhow::Result<Vec<(u64, u64)>> {
        Ok(self
            .shipped_objects(cell, inc)
            .await?
            .into_iter()
            .map(|(range, _, _)| range)
            .collect())
    }

    async fn shipped_objects(
        &self,
        cell: u32,
        inc: u64,
    ) -> anyhow::Result<Vec<((u64, u64), i64, Path)>> {
        let dir = self.key(&format!("cells/{cell}/log/i{inc}"));
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
        cell: u32,
        inc: u64,
        first: u64,
        last: u64,
    ) -> anyhow::Result<bytes::Bytes> {
        self.get(self.log_key(cell, inc, first, last)).await
    }

    fn log_key(&self, cell: u32, inc: u64, first: u64, last: u64) -> Path {
        self.key(&format!("cells/{cell}/log/i{inc}/{first:020}-{last:020}.json"))
    }

    /// The incarnations of `cell` taken so far, ascending.
    pub async fn incarnations(&self, cell: u32) -> anyhow::Result<Vec<u64>> {
        let dir = self.key(&format!("cells/{cell}/incarnations"));
        let metas: Vec<_> = self.inner.list(Some(&dir)).try_collect().await?;
        let mut taken: Vec<u64> = metas
            .iter()
            .filter_map(|m| m.location.filename()?.parse().ok())
            .collect();
        taken.sort_unstable();
        Ok(taken)
    }

    /// Take the next free incarnation of `cell`, at least `at_least`. The
    /// object is created only if absent, so two restores racing for a
    /// number never both get it.
    pub async fn claim_incarnation(&self, cell: u32, at_least: u64) -> anyhow::Result<u64> {
        let mut next = self
            .incarnations(cell)
            .await?
            .last()
            .map_or(at_least, |n| (n + 1).max(at_least));
        loop {
            let key = self.key(&format!("cells/{cell}/incarnations/{next}"));
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

    /// Delete what no restore point inside `window` can refer to: thinned
    /// snapshot copies and shipped log of `cell`, and (with `segments`) the
    /// segments retired longer than `window` ago, fleet-wide.
    pub async fn collect(
        &self,
        cell: u32,
        segments: bool,
        window: Duration,
        now_ms: i64,
    ) -> anyhow::Result<Collected> {
        let mut done = Collected::default();
        let copies = self.snapshot_copies(cell).await?;
        for gone in thin(&copies, window, now_ms) {
            self.inner.delete(&gone.key).await?;
            done.snapshots += 1;
        }
        let kept: Vec<_> = self.snapshot_copies(cell).await?;
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
            for ((_, last), stored_ms, key) in self.shipped_objects(cell, inc).await? {
                if last < base && stored_ms < now_ms - window_ms {
                    self.inner.delete(&key).await?;
                    done.logs += 1;
                }
            }
        }
        if segments {
            done.segments = self.collect_segments(window, now_ms).await?;
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
                    let _ = self.inner.delete(&self.segment_key(session, seg)).await;
                    let _ = self.inner.delete(&self.messages_key(session, seg)).await;
                    deleted += 1;
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
        let copies: Vec<_> = (0..90).map(|i| copy(now - 30 * HOUR_MS + i * HOUR_MS / 3)).collect();
        let gone = thin(&copies, window, now);
        let kept: Vec<_> = copies.iter().filter(|c| !gone.contains(c)).collect();
        let inside: Vec<_> = kept.iter().filter(|c| c.stored_ms >= now - 10 * HOUR_MS).collect();
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
        let seg = SegRef {
            from: 1,
            to: 5,
            inc: 1,
        };
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
}

//! Segment packs: every session's new history segments from one store pass
//! of a shard's leader, in one object (knowledge/execution/data-plane-v2/
//! segment-packs.md).
//!
//! `packs/<shard>/i<incarnation>/<writer>-<stamp>.pack` holds, per entry, a
//! segment's events as JSON followed by its context events as JSON. A
//! session's `SegRef` names the pack and both byte ranges, so a read is one
//! ranged GET and the pack needs no header.
//!
//! Decisions:
//! - One PUT per pass, not two per session per flush. Object-store requests
//!   were the data plane's largest cost: a coding turn wrote 3.8 segment
//!   objects and 1.0 retirement marks (75k verifier G1, 2026-11-26).
//! - Packs are immutable and their names unique (writer node, then a stamp
//!   from its clock that only grows), so readers cache by range and a
//!   retried PUT rewrites the same bytes.
//! - Mark and sweep, not retirement marks. A pack holds live and replaced
//!   entries of many sessions, so nothing is deleted per entry. Each archived
//!   snapshot gets a sidecar listing the packs its state names
//!   (`shards/<shard>/packrefs/`), written before the snapshot, and a restore
//!   pins the packs it brought back (`packs-pinned/`). The directory shard's
//!   leader deletes a pack only once it is older than the restore window
//!   plus [`PACK_GRACE`] and no kept snapshot or pin names it. A state names
//!   an old pack only if it did across every kept snapshot since (a session
//!   moving between shards is in the source's earlier copies), so a pack no
//!   restore point inside the window can name is the only kind that goes.
//! - Legacy segments (their own objects, `pack: None`) keep their retirement
//!   marks (`restore_points.rs`) until compaction has rewritten them all.
//! - Not yet: repacking a pack whose live share is small. A quiet session
//!   pins every other entry of its packs; that costs storage (about $0.023
//!   per GB-month), not requests.

use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use everruns_core::events::Event;
use futures::TryStreamExt;
use object_store::ObjectStoreExt;
use object_store::path::Path;
use serde::{Deserialize, Serialize};

use crate::store::{SegRef, Store, is_context};

/// A pass writes another pack once the current one holds this much.
pub const PACK_CAP: usize = 8 << 20;

/// How long past the restore window an unreferenced pack waits before the
/// sweep deletes it: covers the time between a pack's write and the first
/// snapshot that names it, with room to spare.
pub const PACK_GRACE: Duration = Duration::from_secs(24 * 3600);

/// Where one pack is: written by node `writer` of shard `shard`'s
/// incarnation `inc`, at `stamp` (microseconds-ish, unique per writer).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PackId {
    pub shard: u32,
    pub inc: u64,
    pub writer: u64,
    pub stamp: u64,
}

static STAMP: AtomicU64 = AtomicU64::new(0);

impl PackId {
    /// A new pack id for `writer`, unique among its packs.
    pub fn new(shard: u32, inc: u64, writer: u64) -> Self {
        let now = crate::node::now_ms().max(0) as u64 * 1000;
        let prev = STAMP
            .try_update(Ordering::AcqRel, Ordering::Acquire, |prev| {
                Some(now.max(prev + 1))
            })
            .unwrap_or(0);
        Self {
            shard,
            inc,
            writer,
            stamp: now.max(prev + 1),
        }
    }

    /// `<shard>/i<inc>/<writer>-<stamp>.pack` as a pack id.
    fn from_path(path: &Path) -> Option<Self> {
        let parts: Vec<_> = path.parts().collect();
        let n = parts.len();
        if n < 3 {
            return None;
        }
        let (writer, stamp) = parts[n - 1]
            .as_ref()
            .strip_suffix(".pack")?
            .split_once('-')?;
        Some(Self {
            shard: parts[n - 3].as_ref().parse().ok()?,
            inc: parts[n - 2].as_ref().strip_prefix('i')?.parse().ok()?,
            writer: writer.parse().ok()?,
            stamp: stamp.parse().ok()?,
        })
    }
}

impl std::fmt::Display for PackId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}.{}.{}.{}",
            self.shard, self.inc, self.writer, self.stamp
        )
    }
}

impl std::str::FromStr for PackId {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> anyhow::Result<Self> {
        let mut parts = s.split('.').map(str::parse::<u64>);
        let mut next = || -> anyhow::Result<u64> {
            Ok(parts
                .next()
                .ok_or_else(|| anyhow::anyhow!("not a pack id: {s}"))??)
        };
        let id = Self {
            shard: u32::try_from(next()?)?,
            inc: next()?,
            writer: next()?,
            stamp: next()?,
        };
        anyhow::ensure!(parts.next().is_none(), "not a pack id: {s}");
        Ok(id)
    }
}

// A segment list names its packs in every snapshot: a short string keeps
// them small.
impl Serialize for PackId {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for PackId {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = std::borrow::Cow::<'de, str>::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// One pack being built by a store pass.
pub struct PackBuilder {
    id: PackId,
    body: Vec<u8>,
}

impl PackBuilder {
    pub fn new(id: PackId) -> Self {
        Self {
            id,
            body: Vec::new(),
        }
    }

    /// Add `events` (dense, `from..=to`) of history branch `branch` as one
    /// segment, and return where it is.
    pub fn add(&mut self, events: &[Event], branch: u64) -> anyhow::Result<SegRef> {
        let start = self.body.len();
        serde_json::to_writer(&mut self.body, events)?;
        let mid = self.body.len();
        let context: Vec<&Event> = events
            .iter()
            .filter(|e| is_context(&e.event_type))
            .collect();
        serde_json::to_writer(&mut self.body, &context)?;
        let end = self.body.len();
        let at = |n: usize| u32::try_from(n).map_err(|_| anyhow::anyhow!("pack too large"));
        Ok(SegRef {
            from: events.first().and_then(|e| e.sequence).unwrap_or(0),
            to: events.last().and_then(|e| e.sequence).unwrap_or(0),
            branch,
            pack: Some(self.id),
            events: (at(start)?, at(mid - start)?),
            context: (at(mid)?, at(end - mid)?),
        })
    }

    pub fn len(&self) -> usize {
        self.body.len()
    }

    pub fn is_empty(&self) -> bool {
        self.body.is_empty()
    }

    pub fn id(&self) -> PackId {
        self.id
    }
}

/// What [`Store::sweep_packs`] did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Swept {
    pub deleted: usize,
    pub kept: usize,
    /// Why nothing was deleted, when the live set could not be known.
    pub skipped: Option<String>,
}

impl Store {
    pub(crate) fn pack_key(&self, id: PackId) -> Path {
        self.key(&format!(
            "packs/{}/i{}/{}-{}.pack",
            id.shard, id.inc, id.writer, id.stamp
        ))
    }

    /// Write `pack` (nothing when empty).
    pub async fn put_pack(&self, pack: PackBuilder) -> anyhow::Result<()> {
        if pack.is_empty() {
            return Ok(());
        }
        self.put(self.pack_key(pack.id), pack.body).await
    }

    /// The sidecar of the archived snapshot at `snapshot`: the packs its
    /// state names.
    pub(crate) fn pack_refs_key(&self, shard: u32, snapshot: &Path) -> Path {
        let name = snapshot.filename().unwrap_or_default();
        let stem = name.strip_suffix(".snap").unwrap_or(name);
        self.key(&format!("shards/{shard}/packrefs/{stem}.json"))
    }

    /// Write the sidecar of `snapshot` from its state's JSON. Called before
    /// the snapshot itself is written, so a kept snapshot has its sidecar.
    pub(crate) async fn put_pack_refs(
        &self,
        shard: u32,
        snapshot: &Path,
        state: bytes::Bytes,
    ) -> anyhow::Result<()> {
        let packs = tokio::task::spawn_blocking(move || packs_of(&state)).await??;
        self.put(
            self.pack_refs_key(shard, snapshot),
            serde_json::to_vec(&packs)?,
        )
        .await
    }

    /// Keep every pack `segments` name for a restore window: a restore
    /// brought them back into a state no snapshot holds yet.
    pub async fn pin_packs<'a>(
        &self,
        segments: impl IntoIterator<Item = &'a SegRef>,
    ) -> anyhow::Result<()> {
        let packs: HashSet<PackId> = segments.into_iter().filter_map(|s| s.pack).collect();
        if packs.is_empty() {
            return Ok(());
        }
        let stamp = PackId::new(0, 0, 0).stamp;
        self.put(
            self.key(&format!("packs-pinned/{stamp}.json")),
            serde_json::to_vec(&packs)?,
        )
        .await
    }

    /// Delete the packs no restore point inside `window` can name (module
    /// decisions). Fleet-wide; the directory shard's leader runs it.
    pub async fn sweep_packs(&self, window: Duration, now_ms: i64) -> anyhow::Result<Swept> {
        let horizon = now_ms - (window + PACK_GRACE).as_millis() as i64;
        // Packs first: one written after this list is no candidate.
        let packs: Vec<_> = self
            .inner
            .list(Some(&self.key("packs")))
            .try_collect()
            .await?;
        let Some(first_pack) = packs
            .iter()
            .map(|meta| meta.last_modified.timestamp_millis())
            .min()
        else {
            return Ok(Swept::default());
        };
        let mut live: HashSet<PackId> = HashSet::new();
        let shards = self
            .inner
            .list_with_delimiter(Some(&self.key("shards")))
            .await?;
        for dir in shards.common_prefixes {
            let Some(shard) = dir.filename().and_then(|s| s.parse::<u32>().ok()) else {
                continue;
            };
            // Snapshots before their sidecars: a snapshot listed here had
            // its sidecar written first.
            let copies = self.snapshot_copies(shard).await?;
            let sidecars: HashSet<Path> = self
                .inner
                .list(Some(&self.key(&format!("shards/{shard}/packrefs"))))
                .map_ok(|meta| meta.location)
                .try_collect()
                .await?;
            for copy in copies {
                let sidecar = self.pack_refs_key(shard, &copy.key);
                if !sidecars.contains(&sidecar) {
                    // A snapshot stored before the first pack names none.
                    if copy.stored_ms < first_pack {
                        continue;
                    }
                    return Ok(Swept {
                        skipped: Some(format!("{} has no pack refs", copy.key)),
                        ..Default::default()
                    });
                }
                let named: Vec<PackId> = serde_json::from_slice(&self.get(sidecar).await?)?;
                live.extend(named);
            }
        }
        let pins: Vec<_> = self
            .inner
            .list(Some(&self.key("packs-pinned")))
            .try_collect()
            .await?;
        for pin in pins {
            if pin.last_modified.timestamp_millis() < horizon {
                let _ = self.inner.delete(&pin.location).await;
                continue;
            }
            let named: Vec<PackId> = serde_json::from_slice(&self.get(pin.location).await?)?;
            live.extend(named);
        }
        let mut swept = Swept::default();
        for meta in packs {
            let old = meta.last_modified.timestamp_millis() < horizon;
            match PackId::from_path(&meta.location) {
                Some(id) if old && !live.contains(&id) => {
                    self.inner.delete(&meta.location).await?;
                    swept.deleted += 1;
                }
                _ => swept.kept += 1,
            }
        }
        Ok(swept)
    }
}

/// The packs a shard state's JSON names, without building the state.
fn packs_of(state: &[u8]) -> anyhow::Result<Vec<PackId>> {
    #[derive(Deserialize)]
    struct Scan {
        #[serde(default)]
        sessions: std::collections::HashMap<String, ScanSession>,
    }
    #[derive(Deserialize)]
    struct ScanSession {
        #[serde(default)]
        segments: Vec<ScanSeg>,
    }
    #[derive(Deserialize)]
    struct ScanSeg {
        #[serde(rename = "p", default)]
        pack: Option<PackId>,
    }
    let scan: Scan = serde_json::from_slice(state)?;
    let mut packs: Vec<PackId> = scan
        .sessions
        .values()
        .flat_map(|s| s.segments.iter().filter_map(|seg| seg.pack))
        .collect();
    packs.sort_unstable();
    packs.dedup();
    Ok(packs)
}

#[cfg(test)]
#[path = "packs_tests.rs"]
mod tests;

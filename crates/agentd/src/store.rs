//! The object store: session history segments and shard snapshots.
//!
//! Decision: a session's events are immutable once committed, so its history
//! is kept as segments (`sessions/<id>/<from>-<to>.json`, inclusive
//! sequences) in one fleet-wide bucket. The content of a range is the same on
//! every replica, so any replica may write any segment and any replica may
//! read one another wrote. That lets a shard snapshot carry only per-session
//! metadata (segment lists), lets a replica unload an idle session's events
//! and page them back from the bucket, and lets a move hand over a session
//! without copying its history.
//!
//! Next to each segment, `<from>-<to>.messages.json` holds only the events
//! a turn's context is rebuilt from ([`is_context`]): an executor reads
//! those for earlier turns instead of the whole log.
//!
//! Any S3-compatible store works (`s3://bucket/prefix`, configured by the
//! usual `AWS_*` variables); `file:///path` serves local runs and tests.

use std::sync::Arc;
use std::time::Duration;

use everruns_contracts::typed_id::SessionId;
use everruns_core::events::Event;
use futures::TryStreamExt;
use object_store::path::Path;
use object_store::{ObjectStore, ObjectStoreExt, PutPayload};
use serde::{Deserialize, Serialize};

/// Events the engine rebuilds a turn's context from: the messages
/// (Everruns projects model input from exactly these three) and
/// `session.started`, whose presence tells the runtime the session began.
pub fn is_context(event_type: &str) -> bool {
    use everruns_contracts::runtime::events::{
        INPUT_MESSAGE, OUTPUT_MESSAGE_COMPLETED, SESSION_STARTED, TOOL_COMPLETED,
    };
    matches!(
        event_type,
        INPUT_MESSAGE | OUTPUT_MESSAGE_COMPLETED | TOOL_COMPLETED | SESSION_STARTED
    )
}

/// One stored segment: the session's events `from..=to`, of history
/// branch `branch` (0 until the session is first restored).
///
/// Decision: a restored session (a rebuilt shard, or a one-org restore;
/// `restore.rs`) writes events at sequences its earlier history may have
/// used for events the restore left out, so a segment's name carries the
/// session's history branch (`SessionState::branch`). A name then always
/// holds the same events, whoever reads it (caches included), and a
/// restore point of the earlier branch still reads the history it saw.
///
/// A segment written since segment packs (`packs.rs`) sits in a pack, at
/// `events` (every event) and `context` (its context events, [`is_context`])
/// byte ranges; one without a pack is a legacy pair of objects of its own.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
pub struct SegRef {
    pub from: i32,
    pub to: i32,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub branch: u64,
    #[serde(rename = "p", default, skip_serializing_if = "Option::is_none")]
    pub pack: Option<crate::packs::PackId>,
    /// `(offset, length)` of the segment's events in its pack.
    #[serde(rename = "e", default, skip_serializing_if = "is_empty_range")]
    pub events: (u32, u32),
    /// `(offset, length)` of its context events in its pack.
    #[serde(rename = "c", default, skip_serializing_if = "is_empty_range")]
    pub context: (u32, u32),
}

impl SegRef {
    /// A legacy segment (objects of its own) over `from..=to`.
    pub fn legacy(from: i32, to: i32, branch: u64) -> Self {
        Self {
            from,
            to,
            branch,
            ..Default::default()
        }
    }
}

fn is_zero(n: &u64) -> bool {
    *n == 0
}

fn is_empty_range(range: &(u32, u32)) -> bool {
    *range == (0, 0)
}

#[derive(Clone)]
pub struct Store {
    pub(crate) inner: Arc<dyn ObjectStore>,
    prefix: Path,
    /// Immutable objects (segments, blobs) read recently, when enabled.
    cache: Option<Arc<ByteCache>>,
}

/// A bounded cache of immutable objects, least recently used out first.
///
/// Decision (design.md, "Executors"): affinity alone doesn't keep a session
/// warm, because Engine rebuilds its stored view per epoch; an executor
/// keeps the history segments and workspace blobs it read across
/// activations. Every cached key names immutable content (a segment's
/// range, a blob's hash), so nothing is ever invalidated.
pub struct ByteCache {
    capacity: usize,
    inner: parking_lot::Mutex<CacheInner>,
    pub hits: std::sync::atomic::AtomicU64,
    pub misses: std::sync::atomic::AtomicU64,
}

#[derive(Default)]
struct CacheInner {
    map: std::collections::HashMap<String, (bytes::Bytes, u64)>,
    order: std::collections::BTreeMap<u64, String>,
    tick: u64,
    size: usize,
}

impl ByteCache {
    fn new(capacity: usize) -> Self {
        Self {
            capacity,
            inner: Default::default(),
            hits: Default::default(),
            misses: Default::default(),
        }
    }

    fn get(&self, key: &str) -> Option<bytes::Bytes> {
        use std::sync::atomic::Ordering::Relaxed;
        let mut inner = self.inner.lock();
        inner.tick += 1;
        let tick = inner.tick;
        let found = inner.map.get_mut(key).map(|(bytes, at)| {
            let old = std::mem::replace(at, tick);
            (bytes.clone(), old)
        });
        match found {
            Some((bytes, old)) => {
                inner.order.remove(&old);
                inner.order.insert(tick, key.to_string());
                self.hits.fetch_add(1, Relaxed);
                Some(bytes)
            }
            None => {
                self.misses.fetch_add(1, Relaxed);
                None
            }
        }
    }

    fn put(&self, key: &str, bytes: bytes::Bytes) {
        if bytes.len() > self.capacity / 4 {
            return;
        }
        let mut inner = self.inner.lock();
        inner.tick += 1;
        let tick = inner.tick;
        if let Some((old, at)) = inner.map.insert(key.to_string(), (bytes.clone(), tick)) {
            inner.size -= old.len();
            inner.order.remove(&at);
        }
        inner.size += bytes.len();
        inner.order.insert(tick, key.to_string());
        while inner.size > self.capacity {
            let Some((_, oldest)) = inner.order.pop_first() else {
                break;
            };
            if let Some((gone, _)) = inner.map.remove(&oldest) {
                inner.size -= gone.len();
            }
        }
    }
}

impl std::fmt::Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Store({})", self.inner)
    }
}

/// `AGENTD_STORE_THROTTLE_MS` (`store_throttle.rs`); unset or 0: no wait.
fn throttled(inner: Arc<dyn ObjectStore>) -> Arc<dyn ObjectStore> {
    let ms: u64 = std::env::var("AGENTD_STORE_THROTTLE_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    if ms == 0 {
        return inner;
    }
    Arc::new(crate::store_throttle::Slow::new(
        inner,
        Duration::from_millis(ms),
    ))
}

impl Store {
    /// Open `url`: `file:///dir`, `memory://`, or `s3://bucket/prefix`.
    pub fn open(url: &str) -> anyhow::Result<Self> {
        let parsed = url::Url::parse(url)?;
        if parsed.scheme() == "file" {
            std::fs::create_dir_all(parsed.path())?;
        }
        let options = std::env::vars()
            .filter(|(key, _)| key.starts_with("AWS_"))
            .map(|(key, value)| (key.to_ascii_lowercase(), value));
        let (inner, prefix) = object_store::parse_url_opts(&parsed, options)?;
        let inner: Arc<dyn ObjectStore> = Arc::from(inner);
        Ok(Self {
            inner: throttled(inner),
            prefix,
            cache: None,
        })
    }

    /// Keep up to `bytes` of immutable objects read through this store.
    pub fn with_cache(mut self, bytes: usize) -> Self {
        self.cache = Some(Arc::new(ByteCache::new(bytes)));
        self
    }

    /// Cache hits and misses so far (zero without a cache).
    pub fn cache_stats(&self) -> (u64, u64) {
        use std::sync::atomic::Ordering::Relaxed;
        self.cache
            .as_ref()
            .map_or((0, 0), |c| (c.hits.load(Relaxed), c.misses.load(Relaxed)))
    }

    fn blob_key(&self, sha256: &str) -> Path {
        self.key(&format!("blobs/{sha256}"))
    }

    /// Store `bytes` by their SHA-256 (hex), which names them. Idempotent.
    pub async fn put_blob(&self, bytes: Vec<u8>) -> anyhow::Result<String> {
        use sha2::Digest;
        let sha256 = hex::encode(sha2::Sha256::digest(&bytes));
        self.put(self.blob_key(&sha256), bytes).await?;
        Ok(sha256)
    }

    /// Write `bytes` at `name` under the store's prefix (exports).
    pub async fn put_object(&self, name: &str, bytes: Vec<u8>) -> anyhow::Result<()> {
        self.put(self.key(name), bytes).await
    }

    /// The object at `name` (exports).
    pub async fn get_object(&self, name: &str) -> anyhow::Result<bytes::Bytes> {
        self.get(self.key(name)).await
    }

    /// Delivered feed records of `source` (one shard incarnation), kept so a
    /// restored control plane can ask for them again (feed replay).
    pub async fn put_feed(
        &self,
        source: &str,
        incarnation: u64,
        records: &[crate::dp::FeedRecord],
    ) -> anyhow::Result<()> {
        let (Some(first), Some(last)) = (records.first(), records.last()) else {
            return Ok(());
        };
        let name = format!(
            "feed/{source}/{incarnation}/{:020}-{:010}-{:020}-{:010}.json",
            first.index, first.ordinal, last.index, last.ordinal
        );
        self.put(self.key(&name), serde_json::to_vec(records)?)
            .await
    }

    /// The archived feed records of `source` from `from_index` on, in order.
    pub async fn feed_since(
        &self,
        source: &str,
        incarnation: u64,
        from_index: u64,
    ) -> anyhow::Result<Vec<crate::dp::FeedRecord>> {
        let dir = self.key(&format!("feed/{source}/{incarnation}"));
        let mut metas: Vec<_> = self.inner.list(Some(&dir)).try_collect().await?;
        metas.sort_by(|a, b| a.location.as_ref().cmp(b.location.as_ref()));
        let mut out: Vec<crate::dp::FeedRecord> = Vec::new();
        for meta in metas {
            // `<first index>-<ordinal>-<last index>-<ordinal>.json`
            let last = meta
                .location
                .filename()
                .and_then(|f| f.split('-').nth(2))
                .and_then(|n| n.parse::<u64>().ok());
            if last.is_some_and(|last| last < from_index) {
                continue;
            }
            let bytes = self.get(meta.location.clone()).await?;
            let records: Vec<crate::dp::FeedRecord> = serde_json::from_slice(&bytes)?;
            out.extend(records.into_iter().filter(|r| r.index >= from_index));
        }
        // Overlapping archives (a resend after a lost ack) hold the same
        // records under the same ids.
        out.sort_by_key(|r| (r.index, r.ordinal));
        out.dedup_by_key(|r| (r.index, r.ordinal));
        Ok(out)
    }

    /// The blob named `sha256`.
    pub async fn get_blob(&self, sha256: &str) -> anyhow::Result<bytes::Bytes> {
        if sha256.len() != 64 || !sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
            anyhow::bail!("not a blob name: {sha256}");
        }
        self.get_immutable(self.blob_key(sha256)).await
    }

    /// `get` for content that never changes under its key, through the cache.
    async fn get_immutable(&self, key: Path) -> anyhow::Result<bytes::Bytes> {
        let Some(cache) = &self.cache else {
            return self.get(key).await;
        };
        if let Some(bytes) = cache.get(key.as_ref()) {
            return Ok(bytes);
        }
        let bytes = self.get(key.clone()).await?;
        cache.put(key.as_ref(), bytes.clone());
        Ok(bytes)
    }

    /// [`get_immutable`](Self::get_immutable) for the byte range
    /// `(offset, length)` of an immutable object (a pack's entry).
    async fn get_immutable_range(
        &self,
        key: Path,
        (offset, len): (u32, u32),
    ) -> anyhow::Result<bytes::Bytes> {
        let range = u64::from(offset)..u64::from(offset) + u64::from(len);
        let fetch =
            || async { retry(|| async { self.inner.get_range(&key, range.clone()).await }).await };
        let Some(cache) = &self.cache else {
            return fetch().await;
        };
        let name = format!("{key}#{offset}+{len}");
        if let Some(bytes) = cache.get(&name) {
            return Ok(bytes);
        }
        let bytes = fetch().await?;
        cache.put(&name, bytes.clone());
        Ok(bytes)
    }

    pub(crate) fn key(&self, rest: &str) -> Path {
        Path::from(format!("{}/{rest}", self.prefix).trim_start_matches('/'))
    }

    pub(crate) fn segment_key(&self, session: SessionId, seg: SegRef) -> Path {
        self.key(&format!("sessions/{session}/{}.json", seg_name(seg)))
    }

    pub(crate) fn messages_key(&self, session: SessionId, seg: SegRef) -> Path {
        self.key(&format!(
            "sessions/{session}/{}.messages.json",
            seg_name(seg)
        ))
    }

    /// Write `events` (dense, `from..=to`) as one segment of history
    /// branch `branch`, and its context events beside it. Idempotent.
    pub async fn put_segment(
        &self,
        session: SessionId,
        events: &[Event],
        branch: u64,
    ) -> anyhow::Result<SegRef> {
        let seg = SegRef::legacy(
            events.first().and_then(|e| e.sequence).unwrap_or(0),
            events.last().and_then(|e| e.sequence).unwrap_or(0),
            branch,
        );
        // The messages first: a segment that exists has its messages.
        let messages: Vec<&Event> = events
            .iter()
            .filter(|e| is_context(&e.event_type))
            .collect();
        self.put(
            self.messages_key(session, seg),
            serde_json::to_vec(&messages)?,
        )
        .await?;
        self.put(self.segment_key(session, seg), serde_json::to_vec(events)?)
            .await?;
        Ok(seg)
    }

    /// The context events ([`is_context`]) among `from..=to`, read from the
    /// messages beside each covering segment (or from the segment itself,
    /// for one stored before messages were).
    pub async fn load_context(
        &self,
        session: SessionId,
        segments: &[SegRef],
        from: i32,
        to: i32,
    ) -> anyhow::Result<Vec<Event>> {
        if to < from {
            return Ok(Vec::new());
        }
        let mut chain = segments.to_vec();
        for attempt in 0.. {
            match self.read_context(session, &chain, from, to).await {
                Ok(events) => return Ok(events),
                Err(error) if attempt < 3 && is_not_found(&error) => {
                    chain = cover(self.segments_of(session).await?, &chain, from, to);
                }
                Err(error) => return Err(error),
            }
        }
        unreachable!()
    }

    async fn read_context(
        &self,
        session: SessionId,
        segments: &[SegRef],
        from: i32,
        to: i32,
    ) -> anyhow::Result<Vec<Event>> {
        let segments: Vec<_> = segments
            .iter()
            .filter(|s| s.to >= from && s.from <= to)
            .collect();
        let covered = segments.first().is_some_and(|s| s.from <= from)
            && segments.windows(2).all(|w| w[1].from <= w[0].to + 1)
            && segments.last().is_some_and(|s| s.to >= to);
        if !covered {
            anyhow::bail!("segments of {session} do not cover {from}..={to}");
        }
        let mut out = Vec::new();
        for seg in segments {
            let events = self.segment_context(session, *seg).await?;
            let last = out_last(&out);
            out.extend(
                events
                    .into_iter()
                    .filter(|e| e.sequence.is_some_and(|s| s >= from && s <= to && s > last)),
            );
        }
        Ok(out)
    }

    /// The events `from..=to` of `session`, read from the segments that
    /// cover them (`segments` is contiguous from sequence 1). A legacy
    /// segment that is gone is read from whatever stored legacy segments
    /// cover the range now.
    pub async fn load(
        &self,
        session: SessionId,
        segments: &[SegRef],
        from: i32,
        to: i32,
    ) -> anyhow::Result<Vec<Event>> {
        if to < from {
            return Ok(Vec::new());
        }
        let mut chain = segments.to_vec();
        for attempt in 0.. {
            match self.read_range(session, &chain, from, to).await {
                Ok(events) => return Ok(events),
                Err(error) if attempt < 3 && is_not_found(&error) => {
                    chain = cover(self.segments_of(session).await?, &chain, from, to);
                }
                Err(error) => return Err(error),
            }
        }
        unreachable!()
    }

    async fn read_range(
        &self,
        session: SessionId,
        segments: &[SegRef],
        from: i32,
        to: i32,
    ) -> anyhow::Result<Vec<Event>> {
        let mut out = Vec::new();
        for seg in segments.iter().filter(|s| s.to >= from && s.from <= to) {
            let events = self.segment_events(session, *seg).await?;
            let last = out_last(&out);
            out.extend(
                events
                    .into_iter()
                    .filter(|e| e.sequence.is_some_and(|s| s >= from && s <= to && s > last)),
            );
        }
        let expected = (to - from + 1) as usize;
        if out.len() != expected {
            anyhow::bail!(
                "segments of {session} hold {} of events {from}..={to}",
                out.len()
            );
        }
        Ok(out)
    }

    /// Every event of one segment, from its pack or its own object.
    pub(crate) async fn segment_events(
        &self,
        session: SessionId,
        seg: SegRef,
    ) -> anyhow::Result<Vec<Event>> {
        let bytes = match seg.pack {
            Some(pack) => {
                self.get_immutable_range(self.pack_key(pack), seg.events)
                    .await?
            }
            None => self.get_immutable(self.segment_key(session, seg)).await?,
        };
        Ok(serde_json::from_slice(&bytes)?)
    }

    /// One segment's context events: its pack's context range, the
    /// messages object beside a legacy segment, or (for a segment stored
    /// before messages were) the segment itself, filtered.
    pub(crate) async fn segment_context(
        &self,
        session: SessionId,
        seg: SegRef,
    ) -> anyhow::Result<Vec<Event>> {
        if let Some(pack) = seg.pack {
            let bytes = self
                .get_immutable_range(self.pack_key(pack), seg.context)
                .await?;
            return Ok(serde_json::from_slice(&bytes)?);
        }
        match self.get_immutable(self.messages_key(session, seg)).await {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
            Err(error) if is_not_found(&error) => {
                let events = self.segment_events(session, seg).await?;
                Ok(events
                    .into_iter()
                    .filter(|e| is_context(&e.event_type))
                    .collect())
            }
            Err(error) => Err(error),
        }
    }

    /// Every segment of `session` in the store.
    async fn segments_of(&self, session: SessionId) -> anyhow::Result<Vec<SegRef>> {
        let prefix = self.key(&format!("sessions/{session}"));
        let metas: Vec<_> = self.inner.list(Some(&prefix)).try_collect().await?;
        Ok(metas
            .iter()
            .filter_map(|meta| parse_segment(meta.location.filename()?))
            .collect())
    }

    pub(crate) async fn put(&self, key: Path, bytes: Vec<u8>) -> anyhow::Result<()> {
        let payload = PutPayload::from(bytes);
        retry(|| async {
            count_put(&key);
            self.inner.put(&key, payload.clone()).await.map(|_| ())
        })
        .await
    }

    /// Delete every object under `rest` (the whole store with ""). For
    /// scratch prefixes only (`store_check.rs`).
    pub(crate) async fn delete_prefix(&self, rest: &str) -> anyhow::Result<usize> {
        let root = if rest.is_empty() {
            self.prefix.clone()
        } else {
            self.key(rest)
        };
        let listed: Vec<_> = self.inner.list(Some(&root)).try_collect().await?;
        for meta in &listed {
            self.inner.delete(&meta.location).await?;
        }
        Ok(listed.len())
    }

    pub(crate) async fn get(&self, key: Path) -> anyhow::Result<bytes::Bytes> {
        retry(|| async { self.inner.get(&key).await?.bytes().await }).await
    }
}

fn out_last(out: &[Event]) -> i32 {
    out.last().and_then(|e| e.sequence).unwrap_or(i32::MIN)
}

pub(crate) fn is_not_found(error: &anyhow::Error) -> bool {
    matches!(
        error.downcast_ref::<object_store::Error>(),
        Some(object_store::Error::NotFound { .. })
    )
}

/// A segment's file name, without its suffix: `<from>-<to>`, then
/// `@<inc>` for a segment of an incarnation.
fn seg_name(seg: SegRef) -> String {
    if seg.branch == 0 {
        format!("{:010}-{:010}", seg.from, seg.to)
    } else {
        format!("{:010}-{:010}@{}", seg.from, seg.to, seg.branch)
    }
}

/// `0000000001-0000000022.json` (or `...@2.json`) as a segment.
pub(crate) fn parse_segment(name: &str) -> Option<SegRef> {
    let (from, rest) = name.strip_suffix(".json")?.split_once('-')?;
    let (to, branch) = match rest.split_once('@') {
        Some((to, branch)) => (to, branch.parse().ok()?),
        None => (rest, 0),
    };
    Some(SegRef::legacy(from.parse().ok()?, to.parse().ok()?, branch))
}

/// A chain of `stored` segments covering `from..=to`, standing in for
/// `chain` (a session's own list, which named a segment that is gone): at
/// each step the segment that starts at or before the next needed sequence
/// and reaches furthest, among those of the incarnation `chain` reads that
/// sequence from. Another incarnation's segment over the same sequences
/// may hold events a rebuild lost.
fn cover(mut stored: Vec<SegRef>, chain: &[SegRef], from: i32, to: i32) -> Vec<SegRef> {
    stored.sort_by_key(|s| s.from);
    let (mut out, mut next) = (Vec::new(), from);
    while next <= to {
        let branch = chain
            .iter()
            .find(|s| s.from <= next && s.to >= next)
            .map(|s| s.branch);
        let Some(best) = stored
            .iter()
            .filter(|s| {
                s.from <= next && s.to >= next && branch.is_none_or(|branch| s.branch == branch)
            })
            .max_by_key(|s| s.to)
        else {
            break;
        };
        out.push(*best);
        next = best.to + 1;
    }
    out
}

/// What an object write was for, by where it goes: the request counts
/// `agentd_store_puts_total` reports, for knowing what a run costs.
pub const PUT_KINDS: [&str; 8] = [
    "log",
    "snapshot",
    "segment",
    "feed",
    "blob",
    "retired",
    "incarnation",
    "other",
];

/// Object writes this process made, by [`PUT_KINDS`], retries included
/// (each is a billed request).
static PUTS: [std::sync::atomic::AtomicU64; PUT_KINDS.len()] =
    [const { std::sync::atomic::AtomicU64::new(0) }; PUT_KINDS.len()];

/// The kind of an object write to `key`, an index into [`PUT_KINDS`].
fn put_kind(key: &Path) -> usize {
    let parts: Vec<String> = key.parts().map(|p| p.as_ref().to_owned()).collect();
    // The store's prefix comes first; the first known root names the kind.
    for (i, part) in parts.iter().enumerate() {
        let kind = match part.as_str() {
            "shards" => match parts.get(i + 2).map(String::as_str) {
                Some("log") => "log",
                Some("snapshots") | Some("packrefs") => "snapshot",
                Some("incarnations") => "incarnation",
                _ => continue,
            },
            "sessions" | "packs" => "segment",
            "feed" => "feed",
            "blobs" => "blob",
            "retired" => "retired",
            _ => continue,
        };
        return PUT_KINDS.iter().position(|k| *k == kind).unwrap_or(7);
    }
    PUT_KINDS.len() - 1
}

/// Count one object write to `key`.
pub(crate) fn count_put(key: &Path) {
    PUTS[put_kind(key)].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

/// Object writes so far, by kind.
pub fn puts() -> Vec<(&'static str, u64)> {
    PUT_KINDS
        .iter()
        .zip(&PUTS)
        .map(|(kind, n)| (*kind, n.load(std::sync::atomic::Ordering::Relaxed)))
        .collect()
}

/// Health of this process's object store, from its own calls.
///
/// Decision: an outage is store-wide, not one object's: calls keep failing
/// (not with "not found") and none has succeeded since, for a while. A
/// session whose own object is missing or unreadable while other calls
/// succeed is that session's problem and must not pass for an outage, or a
/// turn that can never load would be retried forever instead of sealed
/// (`Engine::activate`, `SessionHosting::event_store_down`).
static STORE_OK_MS: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);
static STORE_ERR_MS: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);
static STORE_ERR_STREAK: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Consecutive failed calls, with none succeeding since, that mark an outage.
const OUTAGE_STREAK: u32 = 8;
/// An outage is over once no call has failed for this long.
const OUTAGE_QUIET_MS: i64 = 30_000;

fn store_now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

fn store_succeeded() {
    use std::sync::atomic::Ordering::Relaxed;
    STORE_OK_MS.store(store_now_ms(), Relaxed);
    STORE_ERR_STREAK.store(0, Relaxed);
}

fn store_failed() {
    use std::sync::atomic::Ordering::Relaxed;
    STORE_ERR_MS.store(store_now_ms(), Relaxed);
    STORE_ERR_STREAK.fetch_add(1, Relaxed);
}

/// Whether this process's object store looks down as a whole (above).
pub fn outage() -> bool {
    use std::sync::atomic::Ordering::Relaxed;
    let (ok, err) = (STORE_OK_MS.load(Relaxed), STORE_ERR_MS.load(Relaxed));
    STORE_ERR_STREAK.load(Relaxed) >= OUTAGE_STREAK
        && err >= ok
        && store_now_ms() - err < OUTAGE_QUIET_MS
}

/// Retry transient store failures a few times; a missing object is final.
pub(crate) async fn retry<T, F, Fut>(op: F) -> anyhow::Result<T>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = object_store::Result<T>>,
{
    let mut delay = Duration::from_millis(50);
    for attempt in 0.. {
        let result = op().await;
        match &result {
            Ok(_) | Err(object_store::Error::NotFound { .. }) => store_succeeded(),
            Err(_) => store_failed(),
        }
        match result {
            Ok(value) => return Ok(value),
            Err(error @ object_store::Error::NotFound { .. }) => return Err(error.into()),
            Err(error) if attempt >= 5 => return Err(error.into()),
            Err(error) => {
                tracing::debug!(%error, "object store retry");
                tokio::time::sleep(delay).await;
                delay *= 2;
            }
        }
    }
    unreachable!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_object_is_not_an_outage_but_calls_failing_together_are() {
        use std::sync::atomic::Ordering::Relaxed;
        STORE_ERR_STREAK.store(0, Relaxed);
        store_succeeded();
        for _ in 0..20 {
            // NotFound counts as an answer: the store is up.
            store_succeeded();
        }
        assert!(!outage());
        for _ in 0..OUTAGE_STREAK - 1 {
            store_failed();
        }
        assert!(!outage(), "a few failures are not an outage yet");
        store_failed();
        assert!(outage());
        store_succeeded();
        assert!(!outage(), "one success ends it");
    }

    #[test]
    fn object_writes_count_by_kind() {
        let kind = |key: &str| PUT_KINDS[put_kind(&Path::from(key))];
        assert_eq!(kind("live/shards/3/log/i2/1-9.json"), "log");
        assert_eq!(kind("shards/3/snapshots/i2-n9-t1-x.snap"), "snapshot");
        assert_eq!(kind("runs/r1/shards/0/incarnations/2"), "incarnation");
        assert_eq!(kind("live/sessions/s1/1-9.messages.json"), "segment");
        assert_eq!(kind("live/packs/3/i0/1-17.pack"), "segment");
        assert_eq!(kind("shards/3/packrefs/i0-n9-t1-x.json"), "snapshot");
        assert_eq!(kind("feed/0/1/a.json"), "feed");
        assert_eq!(kind("blobs/abc"), "blob");
        assert_eq!(kind("retired/0000000001/s1/1-9.json"), "retired");
        assert_eq!(kind("exports/org.json"), "other");
    }

    fn seg(from: i32, to: i32) -> SegRef {
        SegRef::legacy(from, to, 0)
    }

    #[test]
    fn a_pruned_range_is_read_from_the_segments_that_cover_it() {
        assert_eq!(
            parse_segment("0000000001-0000000022.json"),
            Some(seg(1, 22))
        );
        assert_eq!(parse_segment("current.json"), None);
        assert_eq!(parse_segment("0000000001-0000000022.messages.json"), None);
        // A compacted 1..=30 and an overlapping re-flush of 25..=40.
        let stored = vec![seg(25, 40), seg(1, 30), seg(31, 35)];
        assert_eq!(
            cover(stored.clone(), &[], 1, 40),
            vec![seg(1, 30), seg(25, 40)]
        );
        assert_eq!(cover(stored.clone(), &[], 32, 33), vec![seg(25, 40)]);
        // A gap leaves the chain short; the length check reports it.
        assert_eq!(cover(vec![seg(1, 10)], &[], 1, 20), vec![seg(1, 10)]);
    }

    #[test]
    fn a_rebuilt_shards_segments_never_stand_in_for_an_earlier_incarnations() {
        let old = |from, to| SegRef::legacy(from, to, 1);
        let new = |from, to| SegRef::legacy(from, to, 2);
        assert_eq!(
            parse_segment("0000000001-0000000022@2.json"),
            Some(new(1, 22))
        );
        assert_eq!(parse_segment("0000000001-0000000022@2.messages.json"), None);
        // The session reads 1..=80 from incarnation 1 and 81.. from 2; the
        // lost incarnation 1 also wrote 81..=200, which must not be read.
        let chain = vec![old(1, 80), new(81, 120)];
        let stored = vec![old(1, 40), old(41, 80), old(81, 200), new(81, 120)];
        assert_eq!(
            cover(stored, &chain, 1, 120),
            vec![old(1, 40), old(41, 80), new(81, 120)]
        );
    }
}

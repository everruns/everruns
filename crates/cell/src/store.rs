//! The object store: session history segments and cell snapshots.
//!
//! Decision: a session's events are immutable once committed, so its history
//! is kept as segments (`sessions/<id>/<from>-<to>.json`, inclusive
//! sequences) in one fleet-wide bucket. The content of a range is the same on
//! every replica, so any replica may write any segment and any replica may
//! read one another wrote. That lets a cell snapshot carry only per-session
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

/// One stored segment: the session's events `from..=to`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct SegRef {
    pub from: i32,
    pub to: i32,
}

#[derive(Clone)]
pub struct Store {
    inner: Arc<dyn ObjectStore>,
    prefix: Path,
}

impl std::fmt::Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Store({})", self.inner)
    }
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
        Ok(Self {
            inner: Arc::from(inner),
            prefix,
        })
    }

    fn key(&self, rest: &str) -> Path {
        Path::from(format!("{}/{rest}", self.prefix).trim_start_matches('/'))
    }

    fn segment_key(&self, session: SessionId, seg: SegRef) -> Path {
        self.key(&format!(
            "sessions/{session}/{:010}-{:010}.json",
            seg.from, seg.to
        ))
    }

    fn messages_key(&self, session: SessionId, seg: SegRef) -> Path {
        self.key(&format!(
            "sessions/{session}/{:010}-{:010}.messages.json",
            seg.from, seg.to
        ))
    }

    /// Write `events` (dense, `from..=to`) as one segment, and its context
    /// events beside it. Idempotent.
    pub async fn put_segment(
        &self,
        session: SessionId,
        events: &[Event],
    ) -> anyhow::Result<SegRef> {
        let seg = SegRef {
            from: events.first().and_then(|e| e.sequence).unwrap_or(0),
            to: events.last().and_then(|e| e.sequence).unwrap_or(0),
        };
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
                    chain = cover(self.segments_of(session).await?, from, to);
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
            let events: Vec<Event> = match self.get(self.messages_key(session, *seg)).await {
                Ok(bytes) => serde_json::from_slice(&bytes)?,
                Err(error) if is_not_found(&error) => {
                    let bytes = self.get(self.segment_key(session, *seg)).await?;
                    let events: Vec<Event> = serde_json::from_slice(&bytes)?;
                    events
                        .into_iter()
                        .filter(|e| is_context(&e.event_type))
                        .collect()
                }
                Err(error) => return Err(error),
            };
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
    /// cover them (`segments` is contiguous from sequence 1). A segment that
    /// was pruned (see [`prune_segments`](Self::prune_segments)) is read
    /// from whatever stored segments cover the range now.
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
                    chain = cover(self.segments_of(session).await?, from, to);
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
            let bytes = self.get(self.segment_key(session, *seg)).await?;
            let events: Vec<Event> = serde_json::from_slice(&bytes)?;
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

    /// Every segment of `session` in the store.
    async fn segments_of(&self, session: SessionId) -> anyhow::Result<Vec<SegRef>> {
        let prefix = self.key(&format!("sessions/{session}"));
        let metas: Vec<_> = self.inner.list(Some(&prefix)).try_collect().await?;
        Ok(metas
            .iter()
            .filter_map(|meta| parse_segment(meta.location.filename()?))
            .collect())
    }

    /// Delete `session`'s segments that `newer` (a compaction) covers.
    /// Best effort.
    ///
    /// Decision: compaction and re-flushes leave older segments behind,
    /// and replicas (and a moved session's new cell) may still list them.
    /// Deleting is safe because a read that misses a segment re-reads the
    /// range from the segments that cover it, and only segments that `newer`
    /// fully covers go.
    pub async fn prune_segments(&self, session: SessionId, newer: SegRef) {
        let Ok(stored) = self.segments_of(session).await else {
            return;
        };
        for seg in stored {
            if seg != newer && seg.from >= newer.from && seg.to <= newer.to {
                let _ = self.inner.delete(&self.segment_key(session, seg)).await;
                let _ = self.inner.delete(&self.messages_key(session, seg)).await;
            }
        }
    }

    /// Keep a copy of a cell snapshot (for disaster recovery and audits),
    /// and the newest [`SNAPSHOT_COPIES`] of that cell only.
    pub async fn put_snapshot(&self, cell: u32, id: &str, bytes: Vec<u8>) -> anyhow::Result<()> {
        let dir = self.key(&format!("cells/{cell}/snapshots"));
        self.put(
            self.key(&format!("cells/{cell}/snapshots/{id}.json")),
            bytes,
        )
        .await?;
        let mut copies: Vec<_> = self.inner.list(Some(&dir)).try_collect().await?;
        copies.sort_by_key(|meta| std::cmp::Reverse(meta.last_modified));
        for old in copies.iter().skip(SNAPSHOT_COPIES) {
            let _ = self.inner.delete(&old.location).await;
        }
        Ok(())
    }

    async fn put(&self, key: Path, bytes: Vec<u8>) -> anyhow::Result<()> {
        let payload = PutPayload::from(bytes);
        retry(|| async { self.inner.put(&key, payload.clone()).await.map(|_| ()) }).await
    }

    async fn get(&self, key: Path) -> anyhow::Result<bytes::Bytes> {
        retry(|| async { self.inner.get(&key).await?.bytes().await }).await
    }
}

/// Snapshot copies kept per cell.
pub const SNAPSHOT_COPIES: usize = 3;

fn out_last(out: &[Event]) -> i32 {
    out.last().and_then(|e| e.sequence).unwrap_or(i32::MIN)
}

fn is_not_found(error: &anyhow::Error) -> bool {
    matches!(
        error.downcast_ref::<object_store::Error>(),
        Some(object_store::Error::NotFound { .. })
    )
}

/// `0000000001-0000000022.json` as a segment.
fn parse_segment(name: &str) -> Option<SegRef> {
    let (from, to) = name.strip_suffix(".json")?.split_once('-')?;
    Some(SegRef {
        from: from.parse().ok()?,
        to: to.parse().ok()?,
    })
}

/// A chain of `stored` segments covering `from..=to`: at each step the
/// segment that starts at or before the next needed sequence and reaches
/// furthest.
fn cover(mut stored: Vec<SegRef>, from: i32, to: i32) -> Vec<SegRef> {
    stored.sort_by_key(|s| s.from);
    let (mut chain, mut next) = (Vec::new(), from);
    while next <= to {
        let Some(best) = stored
            .iter()
            .filter(|s| s.from <= next && s.to >= next)
            .max_by_key(|s| s.to)
        else {
            break;
        };
        chain.push(*best);
        next = best.to + 1;
    }
    chain
}

/// Retry transient store failures a few times; a missing object is final.
async fn retry<T, F, Fut>(op: F) -> anyhow::Result<T>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = object_store::Result<T>>,
{
    let mut delay = Duration::from_millis(50);
    for attempt in 0.. {
        match op().await {
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

    fn seg(from: i32, to: i32) -> SegRef {
        SegRef { from, to }
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
        assert_eq!(cover(stored.clone(), 1, 40), vec![seg(1, 30), seg(25, 40)]);
        assert_eq!(cover(stored.clone(), 32, 33), vec![seg(25, 40)]);
        // A gap leaves the chain short; the length check reports it.
        assert_eq!(cover(vec![seg(1, 10)], 1, 20), vec![seg(1, 10)]);
    }
}

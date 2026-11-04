//! Cache immutable decoded indexes, never manifests, authorization or payloads.
//! Readers must select and lock live manifests before touching this cache.
use crate::{
    CacheStats, Limits, ReadProfile, Scope, query,
    segment::{self, Segment},
};
use anyhow::{Context, Result, ensure};
use arrow::{
    array::{Array, Int64Array, StringArray, UInt64Array},
    record_batch::RecordBatch,
};
use object_store::{ObjectStore, ObjectStoreExt, path::Path};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Instant,
};

pub(crate) struct Index {
    pub batches: Vec<RecordBatch>,
    pub bytes: usize,
}
struct Entry {
    index: Arc<Index>,
    used: u64,
    charge: usize,
    org_id: i64,
    session_id: uuid::Uuid,
}
#[derive(Default)]
struct State {
    entries: HashMap<String, Entry>,
    clock: u64,
    bytes: usize,
    evictions: u64,
}
pub(crate) struct IndexCache {
    inner: Mutex<State>,
    decoders: Arc<tokio::sync::Semaphore>,
}
impl Default for IndexCache {
    fn default() -> Self {
        Self {
            inner: Mutex::default(),
            decoders: Arc::new(tokio::sync::Semaphore::new(1)),
        }
    }
}
impl IndexCache {
    pub fn evict_session(&self, scope: Scope, session: uuid::Uuid) {
        if let Ok(mut state) = self.inner.lock() {
            let keys = state
                .entries
                .iter()
                .filter(|(_, entry)| entry.org_id == scope.org_id() && entry.session_id == session)
                .map(|(key, _)| key.clone())
                .collect::<Vec<_>>();
            for key in keys {
                if let Some(entry) = state.entries.remove(&key) {
                    state.bytes -= entry.charge;
                    state.evictions += 1;
                }
            }
        }
    }
    pub fn stats(&self) -> CacheStats {
        self.inner.lock().map_or_else(
            |_| CacheStats::default(),
            |s| CacheStats {
                bytes: s.bytes,
                entries: s.entries.len(),
                evictions: s.evictions,
            },
        )
    }
    pub async fn load(
        &self,
        store: &Arc<dyn ObjectStore>,
        scope: Scope,
        segment: &Segment,
        limits: &Limits,
        profile: &mut ReadProfile,
    ) -> Result<Arc<Index>> {
        // Include the immutable manifest identity, sizes and checksums. Never
        // key by session alone: compaction and export publish new segments.
        let key = format!(
            "{}:{}:{}:{}:{}:{}:{}:{}:{}",
            scope.org_id(),
            segment.index_key,
            segment.index_sha256,
            segment.payload_key,
            segment.index_bytes,
            segment.payload_bytes,
            segment.row_count,
            segment.first_sequence,
            segment.last_sequence
        );
        {
            let mut state = self
                .inner
                .lock()
                .map_err(|_| anyhow::anyhow!("index cache lock poisoned"))?;
            state.clock = state.clock.saturating_add(1);
            let used = state.clock;
            if let Some(entry) = state.entries.get_mut(&key) {
                entry.used = used;
                profile.cache_hits += 1;
                return Ok(entry.index.clone());
            }
        }
        profile.cache_misses += 1;
        let size = u64::try_from(segment.index_bytes)?;
        ensure!(
            size > 0 && size <= limits.max_query_index_bytes,
            "invalid index byte budget"
        );
        let start = Instant::now();
        // Exactly one bounded range GET; no HEAD, footer requests or listing.
        let bytes = store
            .get_range(&Path::from(segment.index_key.as_str()), 0..size)
            .await?;
        profile.index_io_seconds += start.elapsed().as_secs_f64();
        profile.index_bytes_read += bytes.len() as u64;
        ensure!(bytes.len() as u64 == size, "index object length mismatch");
        let start = Instant::now();
        ensure!(
            segment::sha256(&bytes) == segment.index_sha256,
            "index checksum mismatch"
        );
        ensure!(segment.payload_bytes > 0, "invalid payload object size");
        // Keep the decode permit inside the blocking task: cancellation of a
        // reader must not admit an unbounded number of still-running decoders.
        let decoder = self.decoders.clone().acquire_owned().await?;
        let manifest = segment.clone();
        let budget = limits.query_memory_bytes;
        let event_budget = limits.max_event_bytes;
        let index = tokio::task::spawn_blocking(move || -> Result<Index> {
            let _decoder = decoder;
            let builder = ParquetRecordBatchReaderBuilder::try_new(bytes)?;
            ensure!(
                builder.schema().fields() == segment::schema().fields(),
                "invalid index schema"
            );
            ensure!(
                manifest.row_count > 0
                    && manifest.row_count <= 100_000
                    && builder.metadata().file_metadata().num_rows() == manifest.row_count,
                "index row count mismatch"
            );
            let reader = builder.with_batch_size(512).build()?;
            let mut batches = Vec::new();
            let mut size = 4096_usize;
            let mut rows = 0_i64;
            let mut previous = 0_i64;
            for batch in reader {
                let batch = batch?;
                size = size
                    .checked_add(batch.get_array_memory_size())
                    .context("decoded index size overflow")?;
                ensure!(size <= budget, "decoded index byte budget exceeded");
                for (field, column) in batch.schema().fields().iter().zip(batch.columns()) {
                    ensure!(
                        field.is_nullable() || column.null_count() == 0,
                        "null required index field"
                    );
                }
                let orgs = batch
                    .column_by_name("org_id")
                    .and_then(|a| a.as_any().downcast_ref::<Int64Array>())
                    .context("index org type")?;
                let sessions = batch
                    .column_by_name("session_id")
                    .and_then(|a| a.as_any().downcast_ref::<StringArray>())
                    .context("index session type")?;
                let sequences = batch
                    .column_by_name("sequence")
                    .and_then(|a| a.as_any().downcast_ref::<Int64Array>())
                    .context("index sequence type")?;
                let keys = batch
                    .column_by_name("payload_key")
                    .and_then(|a| a.as_any().downcast_ref::<StringArray>())
                    .context("index key type")?;
                let offsets = batch
                    .column_by_name("payload_offset")
                    .and_then(|a| a.as_any().downcast_ref::<UInt64Array>())
                    .context("index offset type")?;
                let lengths = batch
                    .column_by_name("payload_length")
                    .and_then(|a| a.as_any().downcast_ref::<UInt64Array>())
                    .context("index length type")?;
                let session = manifest.session_id.to_string();
                for i in 0..batch.num_rows() {
                    let seq = sequences.value(i);
                    ensure!(
                        orgs.value(i) == scope.org_id() && sessions.value(i) == session,
                        "index row scope mismatch"
                    );
                    ensure!(
                        seq > previous
                            && seq >= i64::from(manifest.first_sequence)
                            && seq <= i64::from(manifest.last_sequence),
                        "invalid index ordering"
                    );
                    previous = seq;
                    ensure!(
                        keys.value(i) == manifest.payload_key
                            && lengths.value(i) <= event_budget as u64
                            && offsets
                                .value(i)
                                .checked_add(lengths.value(i))
                                .is_some_and(|end| end <= manifest.payload_bytes as u64),
                        "index payload locator outside manifest"
                    );
                    query::decode_row(&batch, i)?;
                }
                rows += i64::try_from(batch.num_rows())?;
                batches.push(batch);
            }
            ensure!(
                rows == manifest.row_count,
                "decoded index row count mismatch"
            );
            Ok(Index {
                batches,
                bytes: size,
            })
        })
        .await
        .context("index decoder task failed")??;
        profile.index_decode_seconds += start.elapsed().as_secs_f64();
        let index = Arc::new(index);
        let charge = index
            .bytes
            .checked_add(key.len() + 256)
            .context("cache size overflow")?;
        if charge <= limits.index_cache_bytes {
            let mut state = self
                .inner
                .lock()
                .map_err(|_| anyhow::anyhow!("index cache lock poisoned"))?;
            while state.bytes.saturating_add(charge) > limits.index_cache_bytes
                || state.entries.len() >= 4096
            {
                let oldest = state
                    .entries
                    .iter()
                    .min_by_key(|(_, e)| e.used)
                    .map(|(k, _)| k.clone());
                if let Some(oldest) = oldest {
                    if let Some(entry) = state.entries.remove(&oldest) {
                        state.bytes -= entry.charge;
                        state.evictions += 1;
                    }
                } else {
                    break;
                }
            }
            state.clock = state.clock.saturating_add(1);
            let used = state.clock;
            if let Some(old) = state.entries.insert(
                key,
                Entry {
                    index: index.clone(),
                    used,
                    charge,
                    org_id: scope.org_id(),
                    session_id: segment.session_id,
                },
            ) {
                state.bytes -= old.charge;
            }
            state.bytes += charge;
        }
        Ok(index)
    }
}

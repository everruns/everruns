use crate::{
    CacheStats, EventQuery, EventSummary, Limits, ReadEngine, ReadProfile, Scope,
    index_cache::IndexCache,
    segment::{self, Segment},
};
use anyhow::{Context, Result, ensure};
use arrow::{
    array::{Array, Int64Array, StringArray, UInt64Array},
    record_batch::RecordBatch,
};
use chrono::DateTime;
use datafusion::{
    datasource::MemTable,
    execution::{
        disk_manager::{DiskManagerBuilder, DiskManagerMode},
        runtime_env::RuntimeEnvBuilder,
    },
    prelude::*,
};
use object_store::{ObjectStore, path::Path};
use std::sync::Arc;
use uuid::Uuid;

pub(crate) struct Hit {
    pub summary: EventSummary,
    pub payload_key: String,
    pub offset: u64,
    pub length: u64,
    pub sha256: String,
}

pub(crate) struct QueryEngine {
    context: SessionContext,
    cache: IndexCache,
    // Bound live decoded batches outside the DataFusion operator memory pool.
    reads: tokio::sync::Semaphore,
}

impl QueryEngine {
    pub fn new(limits: &Limits) -> Result<Self> {
        let runtime = RuntimeEnvBuilder::new()
            .with_memory_limit(limits.query_memory_bytes, 1.0)
            .with_disk_manager_builder(
                DiskManagerBuilder::default().with_mode(DiskManagerMode::Disabled),
            )
            .build_arc()?;
        Ok(Self {
            context: SessionContext::new_with_config_rt(
                SessionConfig::new()
                    .with_target_partitions(2)
                    .with_batch_size(512),
                runtime,
            ),
            cache: IndexCache::default(),
            reads: tokio::sync::Semaphore::new(1),
        })
    }
    pub fn cache_stats(&self) -> CacheStats {
        self.cache.stats()
    }
    pub fn evict_session(&self, scope: Scope, session: Uuid) {
        self.cache.evict_session(scope, session);
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn execute(
        &self,
        store: Arc<dyn ObjectStore>,
        prefix: &Path,
        scope: Scope,
        segments: &[Segment],
        query: &EventQuery,
        limits: &Limits,
        event_id: Option<Uuid>,
        force_datafusion: bool,
        profile: &mut ReadProfile,
    ) -> Result<Vec<Hit>> {
        if segments.is_empty() {
            return Ok(Vec::new());
        }
        validate(prefix, scope, segments, limits)?;
        let _permit = self.reads.acquire().await?;
        if !force_datafusion
            && query.session_id.is_some()
            && query.event_types.is_empty()
            && query.turn_id.is_none()
            && query.from.is_none()
            && query.until.is_none()
            && event_id.is_none()
        {
            profile.engine = ReadEngine::SessionIndex;
            let mut previous = 0;
            // Catalog ordering plus disjoint manifest bounds gives the same
            // session/sequence ordering as the DataFusion sort, without a plan.
            for segment in segments {
                ensure!(
                    Some(segment.session_id) == query.session_id
                        && segment.first_sequence > previous,
                    "overlapping or unordered session manifests"
                );
                previous = segment.last_sequence;
            }
            let mut hits = Vec::new();
            let mut result_bytes = 0_usize;
            for segment in segments {
                let index = self
                    .cache
                    .load(&store, scope, segment, limits, profile)
                    .await?;
                profile.segments_scanned += 1;
                let start = std::time::Instant::now();
                for batch in &index.batches {
                    let sequences = batch
                        .column_by_name("sequence")
                        .and_then(|a| a.as_any().downcast_ref::<Int64Array>())
                        .context("invalid sequence column")?;
                    for i in 0..batch.num_rows() {
                        let seq = sequences.value(i);
                        if query
                            .after_sequence
                            .is_some_and(|after| seq <= i64::from(after))
                        {
                            continue;
                        }
                        if query
                            .through_sequence
                            .is_some_and(|through| seq > i64::from(through))
                        {
                            break;
                        }
                        let hit = decode_row(batch, i)?;
                        result_bytes = result_bytes.saturating_add(
                            hit.summary.event_type.len()
                                + hit.summary.turn_id.as_ref().map_or(0, String::len)
                                + hit.summary.exec_id.as_ref().map_or(0, String::len)
                                + hit.summary.parent_exec_id.as_ref().map_or(0, String::len)
                                + hit.payload_key.len()
                                + hit.sha256.len()
                                + std::mem::size_of::<Hit>(),
                        );
                        ensure!(
                            result_bytes <= limits.query_memory_bytes,
                            "query result byte budget exceeded"
                        );
                        hits.push(hit);
                        if hits.len() > query.limit {
                            profile.execution_seconds += start.elapsed().as_secs_f64();
                            return Ok(hits);
                        }
                    }
                }
                profile.execution_seconds += start.elapsed().as_secs_f64();
            }
            return Ok(hits);
        }
        profile.engine = ReadEngine::Datafusion;
        let mut batches = Vec::new();
        let mut decoded_bytes = 0_usize;
        for segment in segments {
            let index = self
                .cache
                .load(&store, scope, segment, limits, profile)
                .await?;
            profile.segments_scanned += 1;
            decoded_bytes = decoded_bytes
                .checked_add(index.bytes)
                .context("decoded query size overflow")?;
            ensure!(
                decoded_bytes <= limits.query_memory_bytes,
                "decoded query index budget exceeded"
            );
            batches.extend(index.batches.iter().cloned());
        }
        let start = std::time::Instant::now();
        // A query-local provider keeps tenant data out of the shared table registry.
        // The shared context has no per-query object-store registrations to race.
        let table = MemTable::try_new(Arc::new(segment::schema()), vec![batches])?;
        let mut df = self
            .context
            .read_table(Arc::new(table))?
            .filter(col("org_id").eq(lit(scope.org_id())))?;
        if let Some(id) = query.session_id {
            df = df.filter(col("session_id").eq(lit(id.to_string())))?;
        }
        if let Some(id) = event_id {
            df = df.filter(col("event_id").eq(lit(id.to_string())))?;
        }
        if let Some(seq) = query.after_sequence {
            df = df.filter(col("sequence").gt(lit(i64::from(seq))))?;
        }
        if let Some(seq) = query.through_sequence {
            df = df.filter(col("sequence").lt_eq(lit(i64::from(seq))))?;
        }
        if let Some(from) = query.from {
            df = df.filter(col("occurred_at").gt_eq(lit(from.timestamp_micros())))?;
        }
        if let Some(until) = query.until {
            df = df.filter(col("occurred_at").lt(lit(until.timestamp_micros())))?;
        }
        if let Some(id) = &query.turn_id {
            df = df.filter(col("turn_id").eq(lit(id.clone())))?;
        }
        if !query.event_types.is_empty() {
            df = df.filter(
                col("event_type")
                    .in_list(query.event_types.iter().cloned().map(lit).collect(), false),
            )?;
        }
        let plan = df
            .sort(vec![
                col("session_id").sort(true, true),
                col("sequence").sort(true, true),
            ])?
            .limit(0, Some(query.limit + 1))?
            .create_physical_plan()
            .await?;
        profile.planning_seconds += start.elapsed().as_secs_f64();
        let start = std::time::Instant::now();
        let batches = datafusion::physical_plan::collect(plan, self.context.task_ctx()).await?;
        let result = batches.iter().try_fold(Vec::new(), |mut out, batch| {
            out.extend(decode(batch)?);
            Ok(out)
        });
        profile.execution_seconds += start.elapsed().as_secs_f64();
        result
    }

    /// Point lookup over a validated immutable index; no SQL planner or rescan.
    #[allow(clippy::too_many_arguments)]
    pub async fn lookup(
        &self,
        store: Arc<dyn ObjectStore>,
        prefix: &Path,
        scope: Scope,
        segments: &[Segment],
        id: Uuid,
        sequence: Option<i32>,
        limits: &Limits,
        profile: &mut ReadProfile,
    ) -> Result<Option<Hit>> {
        validate(prefix, scope, segments, limits)?;
        let _permit = self.reads.acquire().await?;
        profile.engine = ReadEngine::Locator;
        let id = id.to_string();
        let mut result = None;
        let mut decoded_bytes = 0_usize;
        for segment in segments {
            let index = self
                .cache
                .load(&store, scope, segment, limits, profile)
                .await?;
            profile.segments_scanned += 1;
            decoded_bytes = decoded_bytes
                .checked_add(index.bytes)
                .context("decoded query size overflow")?;
            ensure!(
                decoded_bytes <= limits.query_memory_bytes,
                "decoded query index budget exceeded"
            );
            let start = std::time::Instant::now();
            for batch in &index.batches {
                let ids = batch
                    .column_by_name("event_id")
                    .and_then(|a| a.as_any().downcast_ref::<StringArray>())
                    .context("invalid event id column")?;
                let sequences = batch
                    .column_by_name("sequence")
                    .and_then(|a| a.as_any().downcast_ref::<Int64Array>())
                    .context("invalid sequence column")?;
                for i in 0..batch.num_rows() {
                    if ids.value(i) == id
                        && sequence.is_none_or(|s| sequences.value(i) == i64::from(s))
                    {
                        ensure!(
                            result.is_none(),
                            "duplicate canonical event in trace catalog"
                        );
                        result = Some(decode_row(batch, i)?);
                    }
                }
            }
            profile.execution_seconds += start.elapsed().as_secs_f64();
        }
        Ok(result)
    }
}

fn validate(prefix: &Path, scope: Scope, segments: &[Segment], limits: &Limits) -> Result<()> {
    ensure!(
        segments.len() <= limits.max_query_segments,
        "query segment budget exceeded; narrow the session or time range"
    );
    let mut bytes = 0_u64;
    for s in segments {
        ensure!(s.format_version == 1, "unsupported segment format");
        let root = prefix
            .clone()
            .join(format!("org-{}", scope.org_id()))
            .join(s.session_id.to_string())
            .join(s.id.to_string());
        ensure!(
            s.index_key == root.clone().join("index.parquet").as_ref()
                && s.payload_key == root.clone().join("payloads.jsonl").as_ref(),
            "manifest object scope mismatch"
        );
        bytes = bytes
            .checked_add(u64::try_from(s.index_bytes)?)
            .context("index size overflow")?;
    }
    ensure!(
        bytes <= limits.max_query_index_bytes,
        "query index byte budget exceeded"
    );
    Ok(())
}

#[cfg(test)]
pub(crate) async fn execute(
    store: Arc<dyn ObjectStore>,
    prefix: &Path,
    scope: Scope,
    segments: &[Segment],
    query: &EventQuery,
    limits: &Limits,
    event_id: Option<Uuid>,
) -> Result<Vec<Hit>> {
    QueryEngine::new(limits)?
        .execute(
            store,
            prefix,
            scope,
            segments,
            query,
            limits,
            event_id,
            false,
            &mut ReadProfile::default(),
        )
        .await
}

fn decode(batch: &RecordBatch) -> Result<Vec<Hit>> {
    (0..batch.num_rows())
        .map(|i| decode_row(batch, i))
        .collect()
}

pub(crate) fn decode_row(batch: &RecordBatch, i: usize) -> Result<Hit> {
    fn strings<'a>(batch: &'a RecordBatch, name: &str) -> Result<&'a StringArray> {
        batch
            .column_by_name(name)
            .context("missing index column")?
            .as_any()
            .downcast_ref()
            .context("invalid index column type")
    }
    let sessions = strings(batch, "session_id")?;
    let ids = strings(batch, "event_id")?;
    let types = strings(batch, "event_type")?;
    let turns = strings(batch, "turn_id")?;
    let execs = strings(batch, "exec_id")?;
    let parents = strings(batch, "parent_exec_id")?;
    let keys = strings(batch, "payload_key")?;
    let hashes = strings(batch, "payload_sha256")?;
    let ints = |name: &str| -> Result<&Int64Array> {
        batch
            .column_by_name(name)
            .context("missing integer column")?
            .as_any()
            .downcast_ref()
            .context("invalid integer column")
    };
    let uints = |name: &str| -> Result<&UInt64Array> {
        batch
            .column_by_name(name)
            .context("missing offset column")?
            .as_any()
            .downcast_ref()
            .context("invalid offset column")
    };
    let sequences = ints("sequence")?;
    let times = ints("occurred_at")?;
    let offsets = uints("payload_offset")?;
    let lengths = uints("payload_length")?;
    let optional = |a: &StringArray, i| {
        if a.is_null(i) {
            None
        } else {
            Some(a.value(i).to_owned())
        }
    };
    Ok(Hit {
        summary: EventSummary {
            event_id: ids.value(i).parse()?,
            session_id: sessions.value(i).parse()?,
            sequence: sequences.value(i).try_into()?,
            event_type: types.value(i).into(),
            occurred_at: DateTime::from_timestamp_micros(times.value(i))
                .context("invalid event timestamp")?,
            turn_id: optional(turns, i),
            exec_id: optional(execs, i),
            parent_exec_id: optional(parents, i),
        },
        payload_key: keys.value(i).into(),
        offset: offsets.value(i),
        length: lengths.value(i),
        sha256: hashes.value(i).into(),
    })
}

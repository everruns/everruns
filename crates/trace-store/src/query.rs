use crate::{
    EventQuery, EventSummary, Limits, Scope,
    segment::{self, Segment},
};
use anyhow::{Context, Result, ensure};
use arrow::{
    array::{Array, Int64Array, StringArray, UInt64Array},
    record_batch::RecordBatch,
};
use chrono::DateTime;
use datafusion::{
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

pub(crate) async fn execute(
    store: Arc<dyn ObjectStore>,
    prefix: &Path,
    scope: Scope,
    segments: &[Segment],
    query: &EventQuery,
    limits: &Limits,
    event_id: Option<Uuid>,
) -> Result<Vec<Hit>> {
    if segments.is_empty() {
        return Ok(Vec::new());
    }
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
    let runtime = RuntimeEnvBuilder::new()
        .with_memory_limit(limits.query_memory_bytes, 1.0)
        .with_disk_manager_builder(
            DiskManagerBuilder::default().with_mode(DiskManagerMode::Disabled),
        )
        .build_arc()?;
    let ctx = SessionContext::new_with_config_rt(
        SessionConfig::new()
            .with_target_partitions(2)
            .with_batch_size(512),
        runtime,
    );
    ctx.register_object_store(
        &url::Url::parse("s3://trace-store")?,
        Arc::new(crate::datafusion_store::ReadStore::new(
            store,
            segments.iter().map(|s| s.index_key.clone()).collect(),
        )),
    );
    let paths = segments
        .iter()
        .map(|s| format!("s3://trace-store/{}", s.index_key))
        .collect::<Vec<_>>();
    let schema = segment::schema();
    let mut df = ctx
        .read_parquet(paths, ParquetReadOptions::default().schema(&schema))
        .await?
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
            col("event_type").in_list(query.event_types.iter().cloned().map(lit).collect(), false),
        )?;
    }
    let batches = df
        .sort(vec![
            col("session_id").sort(true, true),
            col("sequence").sort(true, true),
        ])?
        .limit(0, Some(query.limit + 1))?
        .collect()
        .await?;
    batches.iter().try_fold(Vec::new(), |mut out, batch| {
        out.extend(decode(batch)?);
        Ok(out)
    })
}

fn decode(batch: &RecordBatch) -> Result<Vec<Hit>> {
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
    (0..batch.num_rows())
        .map(|i| {
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
        })
        .collect()
}

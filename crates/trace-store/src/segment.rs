//! Immutable, narrow Parquet indexes plus a separately fetched JSONL payload file.
//! Keeping payloads out of Parquet lets list queries avoid all prompt/tool bytes.
use std::sync::Arc;

use anyhow::{Context, Result, ensure};
use arrow::{
    array::{ArrayRef, Int64Array, StringArray, UInt64Array},
    datatypes::{DataType, Field, Schema},
    record_batch::RecordBatch,
};
use bytes::Bytes;
use chrono::{DateTime, Utc};
use object_store::{ObjectStore, PutMode, PutOptions, path::Path};
use parquet::{arrow::ArrowWriter, basic::Compression, file::properties::WriterProperties};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{EventRecord, Limits, Scope};

#[derive(Clone, Debug, sqlx::FromRow)]
pub(crate) struct Segment {
    pub id: Uuid,
    pub session_id: Uuid,
    pub first_sequence: i32,
    pub last_sequence: i32,
    pub min_ts: DateTime<Utc>,
    pub max_ts: DateTime<Utc>,
    pub row_count: i64,
    pub index_key: String,
    pub payload_key: String,
    pub index_bytes: i64,
    pub payload_bytes: i64,
    pub index_sha256: String,
    pub payload_sha256: String,
    pub format_version: i32,
}

pub(crate) const SEGMENT_COLUMNS: &str = "m.id, m.session_id, m.first_sequence, m.last_sequence, m.min_ts, m.max_ts, m.row_count, m.index_key, m.payload_key, m.index_bytes, m.payload_bytes, m.index_sha256, m.payload_sha256, m.format_version";

pub(crate) fn schema() -> Schema {
    Schema::new(vec![
        Field::new("org_id", DataType::Int64, false),
        Field::new("session_id", DataType::Utf8, false),
        Field::new("event_id", DataType::Utf8, false),
        Field::new("sequence", DataType::Int64, false),
        Field::new("event_type", DataType::Utf8, false),
        Field::new("occurred_at", DataType::Int64, false),
        Field::new("turn_id", DataType::Utf8, true),
        Field::new("exec_id", DataType::Utf8, true),
        Field::new("parent_exec_id", DataType::Utf8, true),
        Field::new("payload_key", DataType::Utf8, false),
        Field::new("payload_offset", DataType::UInt64, false),
        Field::new("payload_length", DataType::UInt64, false),
        Field::new("payload_sha256", DataType::Utf8, false),
    ])
}

pub(crate) fn sha256(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// Returns a complete unpublished segment. Nothing becomes queryable until the
/// caller commits its manifest. A failed transaction may leave harmless orphans.
pub(crate) async fn write(
    store: &Arc<dyn ObjectStore>,
    prefix: &Path,
    scope: Scope,
    events: &[EventRecord],
    limits: &Limits,
) -> Result<Segment> {
    let first = events.first().context("cannot write an empty segment")?;
    ensure!(
        events.len() <= limits.max_batch_rows,
        "segment row budget exceeded"
    );
    let id = Uuid::now_v7();
    let root = prefix
        .clone()
        .join(format!("org-{}", scope.org_id()))
        .join(first.session_id.to_string())
        .join(id.to_string());
    let index_key = root.clone().join("index.parquet").to_string();
    let payload_key = root.clone().join("payloads.jsonl").to_string();
    let mut payload = Vec::new();
    let mut offsets = Vec::with_capacity(events.len());
    let mut lengths = Vec::with_capacity(events.len());
    let mut hashes = Vec::with_capacity(events.len());
    let mut previous = 0;
    for event in events {
        ensure!(
            event.session_id == first.session_id && event.sequence > previous,
            "segment requires one session in strictly increasing sequence order"
        );
        previous = event.sequence;
        let encoded = serde_json::to_vec(event)?;
        ensure!(
            encoded.len() <= limits.max_event_bytes,
            "event payload budget exceeded"
        );
        ensure!(
            payload
                .len()
                .checked_add(encoded.len() + 1)
                .is_some_and(|n| n <= limits.max_batch_bytes),
            "segment byte budget exceeded"
        );
        offsets.push(payload.len() as u64);
        lengths.push(encoded.len() as u64);
        hashes.push(sha256(&encoded));
        payload.extend(encoded);
        payload.push(b'\n');
    }
    let strings = |f: fn(&EventRecord) -> String| -> ArrayRef {
        Arc::new(StringArray::from(events.iter().map(f).collect::<Vec<_>>()))
    };
    let context = |key: &str| -> ArrayRef {
        Arc::new(StringArray::from(
            events
                .iter()
                .map(|e| {
                    e.context
                        .get(key)
                        .and_then(|v| v.as_str())
                        .filter(|value| value.len() <= 128)
                })
                .collect::<Vec<_>>(),
        ))
    };
    let arrays: Vec<ArrayRef> = vec![
        Arc::new(Int64Array::from(vec![scope.org_id(); events.len()])),
        strings(|e| e.session_id.to_string()),
        strings(|e| e.id.to_string()),
        Arc::new(Int64Array::from_iter_values(
            events.iter().map(|e| i64::from(e.sequence)),
        )),
        strings(|e| e.event_type.clone()),
        Arc::new(Int64Array::from_iter_values(
            events.iter().map(|e| e.ts.timestamp_micros()),
        )),
        context("turn_id"),
        context("exec_id"),
        context("parent_exec_id"),
        Arc::new(StringArray::from(vec![payload_key.as_str(); events.len()])),
        Arc::new(UInt64Array::from(offsets)),
        Arc::new(UInt64Array::from(lengths)),
        Arc::new(StringArray::from(hashes)),
    ];
    let batch = RecordBatch::try_new(Arc::new(schema()), arrays)?;
    let properties = WriterProperties::builder()
        .set_compression(Compression::ZSTD(Default::default()))
        .set_max_row_group_row_count(Some(512))
        .build();
    let mut index = Vec::new();
    {
        let mut writer = ArrowWriter::try_new(&mut index, batch.schema(), Some(properties))?;
        writer.write(&batch)?;
        writer.close()?;
    }
    let manifest = Segment {
        id,
        session_id: first.session_id,
        first_sequence: first.sequence,
        last_sequence: previous,
        min_ts: events.iter().map(|e| e.ts).min().context("empty segment")?,
        max_ts: events.iter().map(|e| e.ts).max().context("empty segment")?,
        row_count: events.len().try_into()?,
        index_bytes: index.len().try_into()?,
        payload_bytes: payload.len().try_into()?,
        index_sha256: sha256(&index),
        payload_sha256: sha256(&payload),
        index_key,
        payload_key,
        format_version: 1,
    };
    // Unique keys and Create mode make retries unable to mutate visible files.
    store
        .put_opts(
            &Path::from(manifest.payload_key.as_str()),
            Bytes::from(payload).into(),
            PutOptions {
                mode: PutMode::Create,
                ..Default::default()
            },
        )
        .await?;
    store
        .put_opts(
            &Path::from(manifest.index_key.as_str()),
            Bytes::from(index).into(),
            PutOptions {
                mode: PutMode::Create,
                ..Default::default()
            },
        )
        .await?;
    Ok(manifest)
}

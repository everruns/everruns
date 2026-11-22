//! The warehouse export (data plane v2, step 11.5): a v2 org's session
//! events as files in object storage, for analytics in the org's own
//! warehouse. There is no SQL across a v2 org's sessions, so this is the
//! path to it (design, "Limits and risks").
//!
//! Decisions:
//! - Opt-in per org (`everruns-dataplane warehouse <org> on`), kept as a
//!   feature of the org's policy. While it is on, the org's data plane
//!   copies every event onto the feed (`event_tap::wanted`).
//! - The control plane writes the files as feed batches arrive, before it
//!   stores the batch: a file that cannot be written fails the batch, and
//!   the data plane sends it again. So an event lands at least once; a
//!   retry that starts later in the batch writes a second file, and readers
//!   dedupe on the event's `id`.
//! - Files are gzipped NDJSON, one per org, hour and batch, under
//!   `warehouse/{org}/date=YYYY-MM-DD/hour=HH/`, the layout warehouses read
//!   as partitions. The name is the batch's feed position, so a retried
//!   batch overwrites its own file.
//! - Lines are the events as the session stream serves them (their public
//!   projection, as tapped).
//! - THREAT[TM-TENANT]: an event goes into its org's prefix only when its
//!   session is that org's.

use std::collections::{BTreeMap, HashMap};
use std::io::Write as _;

use everruns_contracts::dataplane::{FeedBatch, Record, TenantPolicy};
use everruns_contracts::typed_id::SessionId;

use super::{ControlPlane, org_internal, org_public};
use crate::storage::StorageBackend;
use crate::storage::blob_store::BlobMetadata;

/// The policy feature that turns the export on.
pub const FEATURE: &str = "warehouse_export";

pub fn enabled(policy: &TenantPolicy) -> bool {
    policy.features.iter().any(|f| f == FEATURE)
}

/// Turn the export on or off for `org_id`; refused on when this control
/// plane has no object storage to write to.
pub async fn set(db: &StorageBackend, org_id: i64, on: bool) -> anyhow::Result<()> {
    anyhow::ensure!(
        !on || db.blob_store().is_some(),
        "the warehouse export needs object storage (STORAGE_BLOB_BACKEND=s3)"
    );
    anyhow::ensure!(
        db.dp_org(org_id).await?.is_some(),
        "{} is not on a data plane",
        org_public(org_id)
    );
    let mut policy = super::event_tap::policy(db, org_id).await?;
    policy.features.retain(|f| f != FEATURE);
    if on {
        policy.features.push(FEATURE.into());
    }
    super::put_policy(db, org_id, &policy).await?;
    super::event_tap::sync(db, org_id).await
}

/// The object key for `org`'s events of one hour from a batch.
pub fn key(org: &str, hour: &str, source: &str, incarnation: u64, first: (u64, u32)) -> String {
    format!(
        "warehouse/{org}/{hour}/{source}-{incarnation}-{}-{}.ndjson.gz",
        first.0, first.1
    )
}

/// Write the files for `batch`'s events after `stored` (the feed position
/// already stored) of every org that has the export on.
pub async fn export(
    cp: &ControlPlane,
    batch: &FeedBatch,
    stored: (u64, u32),
) -> anyhow::Result<()> {
    let records: Vec<_> = batch
        .records
        .iter()
        .filter(|r| (r.index, r.ordinal) > stored && matches!(r.record, Record::Event { .. }))
        .collect();
    let Some(first) = records.first().map(|r| (r.index, r.ordinal)) else {
        return Ok(());
    };
    let mut wanted: HashMap<i64, bool> = HashMap::new();
    let mut files: BTreeMap<(String, String), Vec<u8>> = BTreeMap::new();
    for record in records {
        let Record::Event { session_id, event } = &record.record else {
            continue;
        };
        let Some(org_id) = org_internal(&record.org_id) else {
            continue;
        };
        let on = match wanted.get(&org_id) {
            Some(on) => *on,
            None => {
                let on = cp.db.dp_org(org_id).await?.is_some()
                    && enabled(&super::event_tap::policy(&cp.db, org_id).await?);
                wanted.insert(org_id, on);
                on
            }
        };
        let Ok(session) = session_id.parse::<SessionId>() else {
            continue;
        };
        if !on || cp.db.session_org_id(session).await? != Some(org_id) {
            continue;
        }
        let hour = event["ts"]
            .as_str()
            .and_then(|ts| chrono::DateTime::parse_from_rfc3339(ts).ok())
            .map_or(record.at_unix_ms, |ts| ts.timestamp_millis());
        let hour = chrono::DateTime::from_timestamp_millis(hour)
            .unwrap_or_default()
            .format("date=%Y-%m-%d/hour=%H")
            .to_string();
        let lines = files.entry((record.org_id.clone(), hour)).or_default();
        serde_json::to_writer(&mut *lines, event)?;
        lines.push(b'\n');
    }
    if files.is_empty() {
        return Ok(());
    }
    let store = cp
        .db
        .blob_store()
        .ok_or_else(|| anyhow::anyhow!("the warehouse export has no object storage"))?;
    for ((org, hour), lines) in files {
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        gz.write_all(&lines)?;
        let key = key(&org, &hour, &batch.source, batch.incarnation, first);
        let metadata = BlobMetadata::new("warehouse_export", serde_json::json!({ "org": org }))
            .with_content_type(Some("application/gzip".into()));
        store.put(&key, gz.finish()?, &metadata).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_are_partitioned_by_org_and_hour_and_named_by_feed_position() {
        assert_eq!(
            key("org_1", "date=2026-11-17/hour=14", "shard-1", 2, (40, 3)),
            "warehouse/org_1/date=2026-11-17/hour=14/shard-1-2-40-3.ndjson.gz"
        );
        let mut policy = TenantPolicy::default();
        assert!(!enabled(&policy));
        policy.features.push(FEATURE.into());
        assert!(enabled(&policy));
    }
}

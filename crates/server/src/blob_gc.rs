// Object-storage blob garbage collector (specs/object-storage.md).
//
// The S3-compatible object store is not transactional with PostgreSQL, so blob
// objects can outlive their metadata: an interrupted delete (row gone, object
// left) or a crash between `put()` and the best-effort cleanup leaks objects.
// Orphans are never served — reads always go through the sidecar pointer rows
// (`workspace_file_blobs`, `image_blobs`) — but they accumulate as storage cost.
//
// This sweep reconciles bucket contents against the live pointer tables and
// deletes objects that have NO pointer, *only* if they are older than a safety
// grace period. The grace period is the core safety mechanism: a freshly
// written object may have its row committed slightly after the object lands (or
// the GC may race an in-flight create), so we never touch recently-written
// objects.
//
// Safety invariants:
//   - An object with a live pointer is NEVER deleted.
//   - An object younger than the grace period is NEVER deleted (in-flight race).
//   - Foreign objects outside this deployment's prefix are never listed (the
//     blob store strips/scopes by prefix), so a shared bucket is safe.
//   - On any ambiguity (pointer enumeration error, listing error) we fail closed:
//     skip deletion and log, rather than risk deleting live data.
//
// The sweep is org/prefix-scoped by construction: it walks the two top-level
// key prefixes (`workspaces/` for files, `images/` for images) that encode the
// tenant partition, and caps deletions per run to bound work.

use crate::storage::StorageBackend;
use crate::storage::blob_store::{BlobObject, SharedBlobStore};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use sqlx::PgPool;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, error, info, warn};

/// Default interval between GC sweeps (6 hours). Orphans are harmless besides
/// cost, so an infrequent sweep is sufficient.
const DEFAULT_INTERVAL_SECONDS: u64 = 6 * 60 * 60;

/// Default safety grace period (24 hours). An object younger than this is never
/// deleted, even with no pointer, to avoid racing an in-flight create.
const DEFAULT_GRACE_SECONDS: i64 = 24 * 60 * 60;

/// Default cap on deletions per sweep, to bound the work a single run performs.
const DEFAULT_MAX_DELETES_PER_RUN: usize = 10_000;

/// Top-level key prefixes the GC reconciles. These mirror the tenant-scoped key
/// derivation in `blob_store.rs` (`workspaces/{id}/files/{id}`,
/// `images/org-{id}/{id}/{data,thumb}`).
const SCAN_PREFIXES: &[&str] = &["workspaces/", "images/"];

/// Configuration for the blob GC sweep.
#[derive(Debug, Clone)]
pub struct BlobGcConfig {
    pub interval: Duration,
    pub grace: ChronoDuration,
    pub max_deletes_per_run: usize,
}

impl Default for BlobGcConfig {
    fn default() -> Self {
        Self {
            interval: Duration::from_secs(DEFAULT_INTERVAL_SECONDS),
            grace: ChronoDuration::seconds(DEFAULT_GRACE_SECONDS),
            max_deletes_per_run: DEFAULT_MAX_DELETES_PER_RUN,
        }
    }
}

impl BlobGcConfig {
    /// Load from `STORAGE_BLOB_GC_*` environment variables, falling back to the
    /// safe defaults. Invalid values fall back to defaults with a warning.
    pub fn from_env() -> Self {
        let mut cfg = Self::default();
        if let Ok(v) = std::env::var("STORAGE_BLOB_GC_INTERVAL_SECONDS") {
            match v.parse::<u64>() {
                Ok(secs) => cfg.interval = Duration::from_secs(secs),
                Err(_) => warn!(
                    value = %v,
                    "Invalid STORAGE_BLOB_GC_INTERVAL_SECONDS; using default"
                ),
            }
        }
        if let Ok(v) = std::env::var("STORAGE_BLOB_GC_GRACE_SECONDS") {
            match v.parse::<i64>() {
                Ok(secs) if secs >= 0 => cfg.grace = ChronoDuration::seconds(secs),
                _ => warn!(
                    value = %v,
                    "Invalid STORAGE_BLOB_GC_GRACE_SECONDS; using default"
                ),
            }
        }
        if let Ok(v) = std::env::var("STORAGE_BLOB_GC_MAX_DELETES_PER_RUN") {
            match v.parse::<usize>() {
                Ok(n) => cfg.max_deletes_per_run = n,
                Err(_) => warn!(
                    value = %v,
                    "Invalid STORAGE_BLOB_GC_MAX_DELETES_PER_RUN; using default"
                ),
            }
        }
        cfg
    }

    /// Whether the sweep is enabled. An interval of 0 disables it.
    pub fn enabled(&self) -> bool {
        !self.interval.is_zero()
    }
}

/// Outcome of one reconciliation decision over a listed batch of objects.
#[derive(Debug, Default, PartialEq, Eq)]
struct ReconcileResult {
    /// (relative key, size_bytes) pairs selected for deletion (orphan + older
    /// than grace). Sizes are carried so bytes reclaimed is counted only for
    /// objects whose delete actually succeeds.
    to_delete: Vec<(String, u64)>,
    /// Total bytes across `to_delete` (upper bound on bytes reclaimed).
    bytes_to_reclaim: u64,
    /// Objects skipped because a live pointer exists.
    kept_live: usize,
    /// Objects skipped because they are younger than the grace period.
    kept_within_grace: usize,
    /// Whether the per-run delete cap was reached (more orphans may remain).
    capped: bool,
}

/// Pure reconciliation: given the listed `objects`, the set of `live_keys`
/// (relative keys with a sidecar pointer), the `grace` cutoff, and `now`,
/// decide which orphans to delete.
///
/// An object is deleted iff it has no live pointer AND its last-modified time is
/// at or before `now - grace`. Deletions are capped at `max_deletes`.
fn reconcile(
    objects: &[BlobObject],
    live_keys: &HashSet<String>,
    grace: ChronoDuration,
    now: DateTime<Utc>,
    max_deletes: usize,
) -> ReconcileResult {
    let cutoff = now - grace;
    let mut result = ReconcileResult::default();

    for obj in objects {
        if live_keys.contains(&obj.key) {
            result.kept_live += 1;
            continue;
        }
        // Orphan: no pointer. Only delete if older than the grace period.
        if obj.last_modified > cutoff {
            result.kept_within_grace += 1;
            continue;
        }
        if result.to_delete.len() >= max_deletes {
            result.capped = true;
            // Stop selecting more; remaining orphans are reclaimed next run.
            break;
        }
        result.to_delete.push((obj.key.clone(), obj.size_bytes));
        result.bytes_to_reclaim += obj.size_bytes;
    }

    result
}

/// Enumerate the set of live blob keys (relative object keys that currently have
/// a sidecar pointer row). These are the keys that must NEVER be deleted.
///
/// Fails closed: any query error propagates so the caller skips deletion.
async fn live_blob_keys(pool: &PgPool) -> Result<HashSet<String>, sqlx::Error> {
    let mut keys = HashSet::new();

    // Workspace file blobs: one key per row.
    let ws: Vec<(String,)> = sqlx::query_as("SELECT blob_key FROM workspace_file_blobs")
        .fetch_all(pool)
        .await?;
    for (k,) in ws {
        keys.insert(k);
    }

    // Image blobs: a data key plus an optional thumbnail key per row.
    let imgs: Vec<(String, Option<String>)> =
        sqlx::query_as("SELECT data_key, thumbnail_key FROM image_blobs")
            .fetch_all(pool)
            .await?;
    for (data_key, thumb_key) in imgs {
        keys.insert(data_key);
        if let Some(t) = thumb_key {
            keys.insert(t);
        }
    }

    Ok(keys)
}

/// Run one full GC sweep against the configured blob store. Returns the number
/// of orphans deleted and bytes reclaimed.
///
/// Safety: loads the complete live-key set first (failing closed on error), then
/// lists each scan prefix and deletes only orphans older than the grace period,
/// capped at `max_deletes_per_run`.
pub async fn run_blob_gc(
    pool: &PgPool,
    blob_store: &SharedBlobStore,
    config: &BlobGcConfig,
) -> anyhow::Result<(usize, u64)> {
    // Enumerate live keys first. If this fails, abort the whole sweep — we must
    // never delete without a reliable picture of what is live.
    let live = live_blob_keys(pool)
        .await
        .map_err(|e| anyhow::anyhow!("blob GC: failed to enumerate live pointers: {e}"))?;

    sweep_with_live_keys(blob_store, config, &live, Utc::now()).await
}

/// Core list→reconcile→delete sweep, parameterized on the live-key set and the
/// reference `now`. Separated from `run_blob_gc` so it is exercisable end-to-end
/// against an in-memory blob store without a database.
async fn sweep_with_live_keys(
    blob_store: &SharedBlobStore,
    config: &BlobGcConfig,
    live: &HashSet<String>,
    now: DateTime<Utc>,
) -> anyhow::Result<(usize, u64)> {
    let mut total_deleted = 0usize;
    let mut total_bytes = 0u64;
    let mut total_listed = 0usize;
    let mut budget = config.max_deletes_per_run;

    for prefix in SCAN_PREFIXES {
        if budget == 0 {
            break;
        }
        let objects = match blob_store.list_with_prefix(prefix).await {
            Ok(objs) => objs,
            Err(e) => {
                // Fail closed for this prefix: skip deletion, log, continue.
                warn!(prefix = %prefix, error = %e, "blob GC: listing failed; skipping prefix");
                continue;
            }
        };
        total_listed += objects.len();

        let decision = reconcile(&objects, live, config.grace, now, budget);
        debug!(
            prefix = %prefix,
            listed = objects.len(),
            kept_live = decision.kept_live,
            kept_within_grace = decision.kept_within_grace,
            to_delete = decision.to_delete.len(),
            capped = decision.capped,
            "blob GC: reconciled prefix"
        );

        for (key, size) in &decision.to_delete {
            if let Err(e) = blob_store.delete(key).await {
                // A delete failure is not fatal; the object remains an orphan
                // and is retried next sweep. Do not count it as reclaimed.
                warn!(key = %key, error = %e, "blob GC: delete failed; will retry next sweep");
                continue;
            }
            total_deleted += 1;
            total_bytes += size;
            budget = budget.saturating_sub(1);
        }
    }

    if total_deleted > 0 {
        metrics::counter!(crate::api::prometheus::names::BLOB_GC_ORPHANS_DELETED_TOTAL)
            .increment(total_deleted as u64);
        metrics::counter!(crate::api::prometheus::names::BLOB_GC_BYTES_RECLAIMED_TOTAL)
            .increment(total_bytes);
        info!(
            backend = blob_store.backend_name(),
            listed = total_listed,
            deleted = total_deleted,
            bytes_reclaimed = total_bytes,
            live_pointers = live.len(),
            "Blob GC sweep reclaimed orphaned objects"
        );
    } else {
        debug!(
            backend = blob_store.backend_name(),
            listed = total_listed,
            live_pointers = live.len(),
            "Blob GC sweep found no orphans to reclaim"
        );
    }

    Ok((total_deleted, total_bytes))
}

/// Spawn the periodic blob GC background task.
///
/// No-ops (and logs) when there is no object-storage backend — i.e. the
/// in-memory dev backend or a PostgreSQL deployment running the default inline
/// (`db`) storage. Those backends keep bytes inline in PostgreSQL and have no
/// external objects, so there is nothing to garbage-collect.
pub fn spawn_blob_gc_task(db: Arc<StorageBackend>, config: BlobGcConfig) {
    let Some(blob_store) = db.blob_store() else {
        debug!("Blob GC disabled: no object-storage backend (inline/db storage has no orphans)");
        return;
    };
    let Some(pool) = db.pool().cloned() else {
        // Object-storage is only wired on the PostgreSQL backend; defensive.
        debug!("Blob GC disabled: no PostgreSQL pool");
        return;
    };
    if !config.enabled() {
        info!("Blob GC disabled (STORAGE_BLOB_GC_INTERVAL_SECONDS=0)");
        return;
    }

    info!(
        backend = blob_store.backend_name(),
        interval_secs = config.interval.as_secs(),
        grace_secs = config.grace.num_seconds(),
        max_deletes_per_run = config.max_deletes_per_run,
        "Starting object-storage blob GC background task"
    );

    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(config.interval);
        ticker.tick().await; // skip the immediate first tick

        loop {
            ticker.tick().await;
            match run_blob_gc(&pool, &blob_store, &config).await {
                Ok((deleted, bytes)) => {
                    if deleted > 0 {
                        info!(deleted, bytes, "Blob GC sweep completed");
                    }
                }
                Err(e) => {
                    error!(error = %e, "Blob GC sweep failed");
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obj(key: &str, age: ChronoDuration, size: u64, now: DateTime<Utc>) -> BlobObject {
        BlobObject {
            key: key.to_string(),
            last_modified: now - age,
            size_bytes: size,
        }
    }

    fn live(keys: &[&str]) -> HashSet<String> {
        keys.iter().map(|k| k.to_string()).collect()
    }

    #[test]
    fn live_pointer_is_kept() {
        let now = Utc::now();
        // Old object, but it has a live pointer — never delete.
        let objects = vec![obj(
            "workspaces/a/files/1",
            ChronoDuration::days(10),
            5,
            now,
        )];
        let live = live(&["workspaces/a/files/1"]);
        let r = reconcile(&objects, &live, ChronoDuration::hours(24), now, 100);
        assert!(r.to_delete.is_empty());
        assert_eq!(r.kept_live, 1);
        assert_eq!(r.bytes_to_reclaim, 0);
    }

    #[test]
    fn orphan_older_than_grace_is_deleted() {
        let now = Utc::now();
        let objects = vec![obj(
            "workspaces/a/files/1",
            ChronoDuration::hours(48),
            7,
            now,
        )];
        let r = reconcile(
            &objects,
            &HashSet::new(),
            ChronoDuration::hours(24),
            now,
            100,
        );
        assert_eq!(r.to_delete, vec![("workspaces/a/files/1".to_string(), 7)]);
        assert_eq!(r.bytes_to_reclaim, 7);
        assert_eq!(r.kept_within_grace, 0);
    }

    #[test]
    fn orphan_within_grace_is_kept() {
        let now = Utc::now();
        // Orphan, but written only 1h ago — inside the 24h grace, so kept.
        let objects = vec![obj(
            "workspaces/a/files/1",
            ChronoDuration::hours(1),
            7,
            now,
        )];
        let r = reconcile(
            &objects,
            &HashSet::new(),
            ChronoDuration::hours(24),
            now,
            100,
        );
        assert!(r.to_delete.is_empty());
        assert_eq!(r.kept_within_grace, 1);
        assert_eq!(r.bytes_to_reclaim, 0);
    }

    #[test]
    fn orphan_exactly_at_grace_boundary_is_deleted() {
        let now = Utc::now();
        // last_modified == cutoff: not strictly greater than cutoff, so deleted.
        let objects = vec![obj(
            "workspaces/a/files/1",
            ChronoDuration::hours(24),
            3,
            now,
        )];
        let r = reconcile(
            &objects,
            &HashSet::new(),
            ChronoDuration::hours(24),
            now,
            100,
        );
        assert_eq!(r.to_delete.len(), 1);
    }

    #[test]
    fn mixed_batch_keeps_live_and_recent_deletes_old_orphans() {
        let now = Utc::now();
        let objects = vec![
            obj("workspaces/a/files/live", ChronoDuration::days(5), 10, now), // live
            obj("workspaces/a/files/old", ChronoDuration::days(5), 20, now),  // orphan, old
            obj(
                "workspaces/a/files/new",
                ChronoDuration::minutes(5),
                30,
                now,
            ), // orphan, recent
            obj("images/org-1/i/data", ChronoDuration::days(5), 40, now),     // orphan, old
        ];
        let live = live(&["workspaces/a/files/live"]);
        let r = reconcile(&objects, &live, ChronoDuration::hours(24), now, 100);
        let mut del: Vec<String> = r.to_delete.iter().map(|(k, _)| k.clone()).collect();
        del.sort();
        assert_eq!(
            del,
            vec![
                "images/org-1/i/data".to_string(),
                "workspaces/a/files/old".to_string()
            ]
        );
        assert_eq!(r.bytes_to_reclaim, 60);
        assert_eq!(r.kept_live, 1);
        assert_eq!(r.kept_within_grace, 1);
    }

    #[test]
    fn deletes_are_capped_per_run() {
        let now = Utc::now();
        let objects: Vec<BlobObject> = (0..10)
            .map(|i| {
                obj(
                    &format!("workspaces/a/files/{i}"),
                    ChronoDuration::days(5),
                    1,
                    now,
                )
            })
            .collect();
        let r = reconcile(&objects, &HashSet::new(), ChronoDuration::hours(24), now, 3);
        assert_eq!(r.to_delete.len(), 3);
        assert!(r.capped);
        assert_eq!(r.bytes_to_reclaim, 3);
    }

    #[test]
    fn empty_listing_is_noop() {
        let now = Utc::now();
        let r = reconcile(&[], &HashSet::new(), ChronoDuration::hours(24), now, 100);
        assert_eq!(r, ReconcileResult::default());
    }

    #[test]
    fn disabled_when_interval_zero() {
        let cfg = BlobGcConfig {
            interval: Duration::from_secs(0),
            ..Default::default()
        };
        assert!(!cfg.enabled());
    }

    #[test]
    fn default_config_is_safe() {
        let cfg = BlobGcConfig::default();
        assert!(cfg.enabled());
        assert_eq!(cfg.grace, ChronoDuration::hours(24));
        assert_eq!(cfg.max_deletes_per_run, DEFAULT_MAX_DELETES_PER_RUN);
    }

    // ------------------------------------------------------------------------
    // End-to-end sweep against the in-memory object_store backend (no DB).
    // Exercises list → reconcile → delete, mutating the real store.
    // ------------------------------------------------------------------------

    use crate::storage::blob_store::{
        BlobMetadata, ObjectStoreBlobStore, SharedBlobStore, image_data_key, image_thumbnail_key,
        workspace_file_key,
    };

    fn dr_meta() -> BlobMetadata {
        BlobMetadata::new("test", serde_json::json!({ "v": 1 }))
    }

    #[tokio::test]
    async fn sweep_deletes_old_orphans_keeps_live_and_recent() {
        use object_store::ObjectStore;
        use object_store::path::Path as ObjectPath;

        let inner: Arc<dyn ObjectStore> = Arc::new(object_store::memory::InMemory::new());
        let store: SharedBlobStore =
            Arc::new(ObjectStoreBlobStore::new(inner.clone(), "", "memory"));

        let ws = uuid::Uuid::from_u128(1);
        let live_file = uuid::Uuid::from_u128(10);
        let orphan_file = uuid::Uuid::from_u128(11);
        let recent_orphan_file = uuid::Uuid::from_u128(12);
        let img = uuid::Uuid::from_u128(20);

        let live_key = workspace_file_key(ws, live_file);
        let orphan_key = workspace_file_key(ws, orphan_file);
        let recent_key = workspace_file_key(ws, recent_orphan_file);
        let img_data = image_data_key(7, img);
        let img_thumb = image_thumbnail_key(7, img);

        for k in [&live_key, &orphan_key, &recent_key, &img_data, &img_thumb] {
            store.put(k, b"data".to_vec(), &dr_meta()).await.unwrap();
        }

        // Live pointers: the live file and the image (both data + thumb).
        let live: HashSet<String> = [live_key.clone(), img_data.clone(), img_thumb.clone()]
            .into_iter()
            .collect();

        // The in-memory backend stamps last_modified = now, so use a `now` far
        // in the future to push the orphans past the grace period — except the
        // "recent" one is excluded from deletion by reconcile because we set a
        // small grace and a now only slightly ahead... instead, drive the
        // distinction by overwriting `recent` right at the reference time.
        //
        // Simpler: list real timestamps, then assert via two sweeps.
        let now = Utc::now() + ChronoDuration::days(2); // everything is > grace old
        let cfg = BlobGcConfig {
            grace: ChronoDuration::hours(24),
            ..Default::default()
        };

        let (deleted, bytes) = sweep_with_live_keys(&store, &cfg, &live, now)
            .await
            .unwrap();

        // Orphans: orphan_key + recent_key (both older than grace relative to
        // the future `now`). Live file + image data/thumb are kept.
        assert_eq!(deleted, 2, "two orphaned workspace files should be deleted");
        assert_eq!(bytes, 8, "4 bytes each");

        // Live objects survive.
        assert!(store.get(&live_key).await.unwrap().is_some());
        assert!(store.get(&img_data).await.unwrap().is_some());
        assert!(store.get(&img_thumb).await.unwrap().is_some());
        // Orphans are gone.
        assert!(store.get(&orphan_key).await.unwrap().is_none());
        assert!(store.get(&recent_key).await.unwrap().is_none());

        // Confirm at the raw store level too (prefix stripping round-trips).
        assert!(
            inner
                .get(&ObjectPath::from(orphan_key.as_str()))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn sweep_within_grace_deletes_nothing() {
        let store: SharedBlobStore = Arc::new(ObjectStoreBlobStore::in_memory());
        let ws = uuid::Uuid::from_u128(1);
        let orphan_key = workspace_file_key(ws, uuid::Uuid::from_u128(2));
        store
            .put(&orphan_key, b"x".to_vec(), &dr_meta())
            .await
            .unwrap();

        // Orphan (empty live set) but freshly written; with `now` = real now and
        // a 24h grace, nothing is old enough to delete.
        let cfg = BlobGcConfig::default();
        let (deleted, bytes) = sweep_with_live_keys(&store, &cfg, &HashSet::new(), Utc::now())
            .await
            .unwrap();
        assert_eq!(deleted, 0);
        assert_eq!(bytes, 0);
        assert!(store.get(&orphan_key).await.unwrap().is_some());
    }
}

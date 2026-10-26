// Organisation feature flag overrides, cached in memory while they can be
// kept exact.
//
// Why: flags are resolved on every enforcement path (each worker command,
// each turn context, the auth middleware), about nine reads of the same
// org's rows per chat turn. Overrides change a few times a month.
//
// Decision: overrides are authorization inputs, so a cached value must never
// outlive a committed change (knowledge/security/feature-flags.md). A trigger
// on `org_feature_flags` notifies `org_feature_flags_changed` with the org id
// on every committed insert, update or delete, whoever wrote it, and every
// server listening clears that org. The cache serves reads only while its
// listener is connected: it starts off, and a dropped connection clears it and
// turns it off until LISTEN succeeds again, so a missed notification can never
// leave a stale entry. Without a listener (DEV_MODE, tests, embedded backends)
// every read goes to the table, as before.
//
// Decision: a read that started before an invalidation does not store what it
// read (the generation check), and reads inside a command transaction neither
// use nor fill the cache, since they may see uncommitted rows.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use anyhow::Result;
use moka::sync::Cache;

use super::StorageBackend;

const CAPACITY: u64 = 10_000;
/// Bounds memory and a row written with notifications somehow off; exactness
/// comes from the notifications, not from this.
const TTL: Duration = Duration::from_secs(300);

/// The PostgreSQL channel the `org_feature_flags` trigger notifies.
pub const ORG_FEATURE_FLAGS_CHANNEL: &str = "org_feature_flags_changed";

/// Shared by a backend and the backends derived from it (`for_background`).
#[derive(Clone)]
pub struct OrgFeatureFlagCache(Arc<Inner>);

struct Inner {
    entries: Cache<i64, HashMap<String, bool>>,
    /// The invalidation listener is connected and listening.
    live: AtomicBool,
    /// Bumped by every invalidation.
    generation: AtomicU64,
}

impl Default for OrgFeatureFlagCache {
    fn default() -> Self {
        Self(Arc::new(Inner {
            entries: Cache::builder()
                .max_capacity(CAPACITY)
                .time_to_live(TTL)
                .build(),
            live: AtomicBool::new(false),
            generation: AtomicU64::new(0),
        }))
    }
}

impl OrgFeatureFlagCache {
    /// The listener started (or stopped) receiving notifications. Either way
    /// nothing cached before can be trusted.
    pub fn set_live(&self, live: bool) {
        self.0.live.store(false, Ordering::SeqCst);
        self.clear();
        self.0.live.store(live, Ordering::SeqCst);
    }

    /// Whether reads are being served from memory.
    pub fn is_live(&self) -> bool {
        self.0.live.load(Ordering::SeqCst)
    }

    /// An org's overrides changed.
    pub fn invalidate(&self, org_id: i64) {
        self.0.generation.fetch_add(1, Ordering::SeqCst);
        self.0.entries.invalidate(&org_id);
    }

    fn clear(&self) {
        self.0.generation.fetch_add(1, Ordering::SeqCst);
        self.0.entries.invalidate_all();
    }

    fn generation(&self) -> u64 {
        self.0.generation.load(Ordering::SeqCst)
    }

    /// Keep what a read returned, unless something was invalidated since the
    /// read began: its rows may predate that change.
    fn store_unless_invalidated(
        &self,
        org_id: i64,
        generation: u64,
        flags: &HashMap<String, bool>,
    ) {
        if self.usable() && self.generation() == generation {
            self.0.entries.insert(org_id, flags.clone());
        }
    }

    fn usable(&self) -> bool {
        self.0.live.load(Ordering::SeqCst) && !crate::storage::transaction::in_transaction()
    }
}

impl StorageBackend {
    /// Derived backends share the cache, so one listener keeps them all exact.
    pub(super) fn with_org_feature_flag_cache(mut self, cache: OrgFeatureFlagCache) -> Self {
        self.org_feature_flags = cache;
        self
    }

    pub(super) fn sharing_org_feature_flags(self, from: &StorageBackend) -> Self {
        self.with_org_feature_flag_cache(from.org_feature_flags.clone())
    }

    /// The cache the invalidation listener keeps exact.
    pub fn org_feature_flag_cache(&self) -> &OrgFeatureFlagCache {
        &self.org_feature_flags
    }

    /// An org's stored overrides, from memory while the cache is exact.
    pub async fn list_org_feature_flags(&self, org_id: i64) -> Result<HashMap<String, bool>> {
        let cache = &self.org_feature_flags;
        if !cache.usable() {
            return self.db.list_org_feature_flags(org_id).await;
        }
        if let Some(flags) = cache.0.entries.get(&org_id) {
            return Ok(flags);
        }
        let generation = cache.generation();
        let flags = self.db.list_org_feature_flags(org_id).await?;
        cache.store_unless_invalidated(org_id, generation, &flags);
        Ok(flags)
    }

    /// Store overrides. This process stops serving the old value at once;
    /// other servers (and this one again, after commit) hear the trigger.
    pub async fn replace_org_feature_flags(
        &self,
        org_id: i64,
        flags: &HashMap<String, bool>,
    ) -> Result<()> {
        let result = self.db.replace_org_feature_flags(org_id, flags).await;
        self.org_feature_flags.invalidate(org_id);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn without_a_live_listener_every_read_reaches_the_table() {
        let db = StorageBackend::test_database();
        db.replace_org_feature_flags(1, &HashMap::from([("skills".into(), true)]))
            .await
            .unwrap();
        assert_eq!(db.list_org_feature_flags(1).await.unwrap()["skills"], true);
        // A write that bypasses the backend is still seen: nothing was cached.
        db.database()
            .replace_org_feature_flags(1, &HashMap::from([("skills".into(), false)]))
            .await
            .unwrap();
        assert_eq!(db.list_org_feature_flags(1).await.unwrap()["skills"], false);
        assert_eq!(db.org_feature_flag_cache().0.entries.entry_count(), 0);
    }

    #[tokio::test]
    async fn a_live_cache_serves_reads_and_a_write_clears_it() {
        let db = StorageBackend::test_database();
        db.org_feature_flag_cache().set_live(true);
        db.replace_org_feature_flags(1, &HashMap::from([("skills".into(), true)]))
            .await
            .unwrap();
        assert_eq!(db.list_org_feature_flags(1).await.unwrap()["skills"], true);
        db.org_feature_flag_cache().0.entries.run_pending_tasks();
        assert_eq!(db.org_feature_flag_cache().0.entries.entry_count(), 1);

        // The revocation is read back immediately.
        db.replace_org_feature_flags(1, &HashMap::from([("skills".into(), false)]))
            .await
            .unwrap();
        assert_eq!(db.list_org_feature_flags(1).await.unwrap()["skills"], false);
    }

    #[tokio::test]
    async fn a_read_that_overlaps_an_invalidation_is_not_kept() {
        let cache = OrgFeatureFlagCache::default();
        cache.set_live(true);
        let generation = cache.generation();
        // The flags change while the read is in flight.
        cache.invalidate(1);
        cache.store_unless_invalidated(1, generation, &HashMap::from([("skills".into(), true)]));
        assert!(cache.0.entries.get(&1).is_none());

        let generation = cache.generation();
        cache.store_unless_invalidated(1, generation, &HashMap::new());
        assert!(cache.0.entries.get(&1).is_some());
    }

    #[tokio::test]
    async fn losing_the_listener_drops_everything_and_stops_caching() {
        let db = StorageBackend::test_database();
        let cache = db.org_feature_flag_cache().clone();
        cache.set_live(true);
        db.list_org_feature_flags(1).await.unwrap();
        cache.set_live(false);
        cache.0.entries.run_pending_tasks();
        assert_eq!(cache.0.entries.entry_count(), 0);
        db.list_org_feature_flags(1).await.unwrap();
        cache.0.entries.run_pending_tasks();
        assert_eq!(cache.0.entries.entry_count(), 0);
    }
}

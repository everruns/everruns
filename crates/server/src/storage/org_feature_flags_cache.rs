//! In-process read-through cache for per-organization feature-flag opt-ins (EVE-637).
//!
//! Org feature flags are read on essentially every org-scoped request (via
//! `ResolvedOrg::with_effective_feature_flags`) but change rarely. Before this
//! cache the Postgres backend ran a `SELECT ... FROM org_feature_flags` on every
//! such request. This wraps that read in a bounded, short-TTL moka cache so a hit
//! performs zero database round-trips.
//!
//! Freshness: the cache lives on the Postgres `Database`, so the read path and the
//! `replace_org_feature_flags` write path share one instance — a flag update on the
//! same instance invalidates the entry immediately. The short TTL is the backstop
//! for the multi-instance case: instance A's PATCH does not reach instance B's
//! in-process cache, so B can serve a stale value for at most `TTL`. This bounded
//! staleness is acceptable for feature flags (they gate features, not access) and
//! matches the EVE-637 design intent.

use std::collections::HashMap;
use std::future::Future;
use std::time::Duration;

use moka::future::Cache;

/// TTL for cached org feature-flag entries. Bounds cross-instance staleness; the
/// owning instance invalidates immediately on update.
const ORG_FEATURE_FLAGS_CACHE_TTL: Duration = Duration::from_secs(30);

/// Max distinct orgs held in the cache. Bounds memory across many orgs.
const ORG_FEATURE_FLAGS_CACHE_MAX_CAPACITY: u64 = 10_000;

/// Read-through cache of `org_id -> {flag_name -> enabled}`.
///
/// Cloning shares the underlying store (moka is `Arc`-backed), so all clones of a
/// `Database` observe the same cache and invalidations.
#[derive(Clone)]
pub struct OrgFeatureFlagsCache {
    cache: Cache<i64, HashMap<String, bool>>,
}

impl OrgFeatureFlagsCache {
    pub fn new() -> Self {
        Self::with_ttl(ORG_FEATURE_FLAGS_CACHE_TTL)
    }

    fn with_ttl(ttl: Duration) -> Self {
        Self {
            cache: Cache::builder()
                .max_capacity(ORG_FEATURE_FLAGS_CACHE_MAX_CAPACITY)
                .time_to_live(ttl)
                .build(),
        }
    }

    /// Return the cached flags for `org_id`, or invoke `loader` to fetch them and
    /// populate the cache. On a hit, `loader` is not called (zero DB round-trips).
    pub async fn get_or_load<F, Fut>(
        &self,
        org_id: i64,
        loader: F,
    ) -> anyhow::Result<HashMap<String, bool>>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = anyhow::Result<HashMap<String, bool>>>,
    {
        if let Some(flags) = self.cache.get(&org_id).await {
            return Ok(flags);
        }
        let flags = loader().await?;
        self.cache.insert(org_id, flags.clone()).await;
        Ok(flags)
    }

    /// Drop the cached entry for `org_id`. Called after a write so the next read
    /// reflects the new state immediately on this instance.
    pub async fn invalidate(&self, org_id: i64) {
        self.cache.invalidate(&org_id).await;
    }

    #[cfg(test)]
    async fn entry_count(&self) -> u64 {
        self.cache.run_pending_tasks().await;
        self.cache.entry_count()
    }
}

impl Default for OrgFeatureFlagsCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn flags(pairs: &[(&str, bool)]) -> HashMap<String, bool> {
        pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }

    #[tokio::test]
    async fn hit_does_not_invoke_loader() {
        let cache = OrgFeatureFlagsCache::new();
        let calls = Arc::new(AtomicUsize::new(0));

        let load = |calls: Arc<AtomicUsize>, value: HashMap<String, bool>| {
            move || {
                let calls = calls.clone();
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Ok(value)
                }
            }
        };

        let first = cache
            .get_or_load(1, load(calls.clone(), flags(&[("public_chat", true)])))
            .await
            .unwrap();
        assert_eq!(first, flags(&[("public_chat", true)]));
        assert_eq!(calls.load(Ordering::SeqCst), 1, "miss loads once");

        // Second read with a loader that would return a *different* value: a hit
        // must serve the cached value and must not invoke the loader.
        let second = cache
            .get_or_load(1, load(calls.clone(), flags(&[("public_chat", false)])))
            .await
            .unwrap();
        assert_eq!(second, flags(&[("public_chat", true)]), "served from cache");
        assert_eq!(calls.load(Ordering::SeqCst), 1, "hit performs zero loads");
    }

    #[tokio::test]
    async fn invalidate_forces_reload() {
        let cache = OrgFeatureFlagsCache::new();
        let calls = Arc::new(AtomicUsize::new(0));

        let load = |calls: Arc<AtomicUsize>, value: HashMap<String, bool>| {
            move || {
                let calls = calls.clone();
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Ok(value)
                }
            }
        };

        cache
            .get_or_load(7, load(calls.clone(), flags(&[("a", true)])))
            .await
            .unwrap();
        cache.invalidate(7).await;

        let after = cache
            .get_or_load(7, load(calls.clone(), flags(&[("a", false)])))
            .await
            .unwrap();
        assert_eq!(after, flags(&[("a", false)]), "reload after invalidation");
        assert_eq!(calls.load(Ordering::SeqCst), 2, "invalidation re-loads");
    }

    #[tokio::test]
    async fn keys_are_independent() {
        let cache = OrgFeatureFlagsCache::new();
        let load = |value: HashMap<String, bool>| move || async move { Ok(value) };

        cache
            .get_or_load(1, load(flags(&[("a", true)])))
            .await
            .unwrap();
        cache
            .get_or_load(2, load(flags(&[("b", true)])))
            .await
            .unwrap();
        cache.invalidate(1).await;

        // org 2 still cached: loader returning a different value is ignored.
        let org2 = cache
            .get_or_load(2, load(flags(&[("b", false)])))
            .await
            .unwrap();
        assert_eq!(org2, flags(&[("b", true)]), "unrelated key untouched");
    }

    #[tokio::test]
    async fn ttl_expiry_reloads() {
        let cache = OrgFeatureFlagsCache::with_ttl(Duration::from_millis(50));
        let calls = Arc::new(AtomicUsize::new(0));
        let load = |calls: Arc<AtomicUsize>| {
            move || {
                let calls = calls.clone();
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Ok(HashMap::new())
                }
            }
        };

        cache.get_or_load(1, load(calls.clone())).await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        tokio::time::sleep(Duration::from_millis(120)).await;
        cache.get_or_load(1, load(calls.clone())).await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 2, "expired entry re-loads");
    }

    #[tokio::test]
    async fn entry_count_tracks_inserts() {
        let cache = OrgFeatureFlagsCache::new();
        let load = |value: HashMap<String, bool>| move || async move { Ok(value) };
        cache
            .get_or_load(1, load(flags(&[("a", true)])))
            .await
            .unwrap();
        cache
            .get_or_load(2, load(flags(&[("a", true)])))
            .await
            .unwrap();
        assert_eq!(cache.entry_count().await, 2);
        cache.invalidate(1).await;
        assert_eq!(cache.entry_count().await, 1);
    }
}

//! Per-session MCP tool-discovery cache.
//!
//! Mirrors the hosted server's stale-while-revalidate + single-flight tool
//! cache (`crates/server`, #2131), adapted to the in-process runtime's
//! in-memory, per-session lifetime. The in-process runtime rediscovers a
//! session's scoped MCP servers on every turn (`tools/list` per server). That
//! is correct but costly: with many configured servers each turn pays repeated
//! round-trips for a tool set that rarely changes. This cache makes discovery
//! cheap on the hot path:
//!
//!   - **Fresh** (within TTL): served from memory, no upstream call.
//!   - **Stale** (past TTL, previously fetched): the cached tools are returned
//!     immediately and a background refresh is kicked off
//!     (stale-while-revalidate), so a turn never blocks on `tools/list`.
//!   - **Cold** (never fetched): the caller blocks on a single-flight refresh
//!     shared across concurrent callers, so a burst of turns triggers one fetch.
//!
//! Unlike the server domain, every scoped server here is background-refreshable:
//! the runtime's `McpAuthProvider` is re-invoked on each `discover`, so there is
//! no OAuth "cannot mint a token in the background" carve-out. Background
//! refreshes reuse the same egress-bound `McpClient`, so the runtime's
//! DNS-pinned SSRF protection still applies.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use everruns_core::ToolDefinition;
use tokio::sync::Mutex as AsyncMutex;
use uuid::Uuid;

/// How long cached tools are considered fresh. Matches the hosted server's
/// `TOOL_CACHE_TTL` (#2131) so both paths revalidate on the same cadence.
pub(crate) const TOOL_CACHE_TTL: Duration = Duration::from_secs(3600);

/// Identifies one server's cache entry: `(session, sanitized server name)`.
type CacheKey = (Uuid, String);

struct CacheEntry {
    tools: Vec<ToolDefinition>,
    cached_at: Instant,
}

/// Freshness of a cache lookup at a given instant.
enum Freshness {
    Fresh(Vec<ToolDefinition>),
    Stale(Vec<ToolDefinition>),
    Cold,
}

/// Per-key async locks for single-flight refresh coordination.
#[derive(Default)]
struct KeyedLocks {
    map: Mutex<HashMap<CacheKey, Arc<AsyncMutex<()>>>>,
}

impl KeyedLocks {
    /// Return the shared async lock for `key`, creating it on first use. Idle
    /// locks (held only by this map) are pruned so the map stays bounded by the
    /// number of keys currently refreshing rather than ever seen.
    fn lock_for(&self, key: &CacheKey) -> Arc<AsyncMutex<()>> {
        // Best-effort coordinator: recover from a poisoned mutex rather than
        // propagating a panic into every later tool resolution. The guarded map
        // is plain data, so continuing with the recovered value is safe.
        let mut map = self.map.lock().unwrap_or_else(|e| e.into_inner());
        map.retain(|k, lock| k == key || Arc::strong_count(lock) > 1);
        map.entry(key.clone()).or_default().clone()
    }
}

/// In-memory, per-session MCP tool-discovery cache.
pub(crate) struct McpDiscoveryCache {
    entries: Mutex<HashMap<CacheKey, CacheEntry>>,
    locks: KeyedLocks,
    ttl: Duration,
}

impl McpDiscoveryCache {
    pub(crate) fn new() -> Self {
        Self::with_ttl(TOOL_CACHE_TTL)
    }

    fn with_ttl(ttl: Duration) -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            locks: KeyedLocks::default(),
            ttl,
        }
    }

    fn entries(&self) -> std::sync::MutexGuard<'_, HashMap<CacheKey, CacheEntry>> {
        self.entries.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Classify the entry for `key` at `now`.
    fn classify(&self, key: &CacheKey, now: Instant) -> Freshness {
        match self.entries().get(key) {
            None => Freshness::Cold,
            Some(entry) if now.duration_since(entry.cached_at) < self.ttl => {
                Freshness::Fresh(entry.tools.clone())
            }
            Some(entry) => Freshness::Stale(entry.tools.clone()),
        }
    }

    fn store(&self, key: CacheKey, tools: Vec<ToolDefinition>, now: Instant) {
        self.entries().insert(
            key,
            CacheEntry {
                tools,
                cached_at: now,
            },
        );
    }

    /// Resolve tools for `key`, applying stale-while-revalidate + single-flight.
    ///
    /// `refresh` performs the upstream fetch, returning `Some(tools)` on success
    /// or `None` on failure (the failure is the closure's to log). It may be
    /// called from a background task, so it must be `Send + Sync + 'static`.
    pub(crate) async fn resolve<F, Fut>(
        self: &Arc<Self>,
        key: CacheKey,
        refresh: F,
    ) -> Vec<ToolDefinition>
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Option<Vec<ToolDefinition>>> + Send + 'static,
    {
        match self.classify(&key, Instant::now()) {
            Freshness::Fresh(tools) => tools,
            Freshness::Stale(tools) => {
                self.spawn_background_refresh(key, refresh);
                tools
            }
            Freshness::Cold => self.refresh_coalesced(key, refresh).await,
        }
    }

    /// Cold path: serialize concurrent callers on the per-key lock. A caller
    /// that finds the cache made fresh while it waited returns that instead of
    /// re-fetching (single-flight), so a burst of cold turns triggers one fetch.
    /// A failed fetch degrades to whatever is cached — empty for a truly cold key.
    async fn refresh_coalesced<F, Fut>(
        self: &Arc<Self>,
        key: CacheKey,
        refresh: F,
    ) -> Vec<ToolDefinition>
    where
        F: Fn() -> Fut,
        Fut: Future<Output = Option<Vec<ToolDefinition>>>,
    {
        let lock = self.locks.lock_for(&key);
        let _guard = lock.lock_owned().await;

        if let Freshness::Fresh(tools) = self.classify(&key, Instant::now()) {
            return tools;
        }

        match refresh().await {
            Some(tools) => {
                self.store(key, tools.clone(), Instant::now());
                tools
            }
            None => match self.classify(&key, Instant::now()) {
                Freshness::Fresh(tools) | Freshness::Stale(tools) => tools,
                Freshness::Cold => Vec::new(),
            },
        }
    }

    /// Background revalidation: deduplicated via `try_lock`, so at most one
    /// refresh runs per key at a time and extra triggers are dropped. The caller
    /// has already been served the stale cache, so a failure is only logged
    /// (inside `refresh`) and leaves the cache stale for the next turn.
    fn spawn_background_refresh<F, Fut>(self: &Arc<Self>, key: CacheKey, refresh: F)
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Option<Vec<ToolDefinition>>> + Send + 'static,
    {
        let lock = self.locks.lock_for(&key);
        let Ok(guard) = lock.try_lock_owned() else {
            return; // a refresh is already in flight for this key
        };
        let cache = self.clone();
        tokio::spawn(async move {
            let _guard = guard; // held for the refresh duration -> single-flight
            if let Some(tools) = refresh().await {
                cache.store(key, tools, Instant::now());
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_core::tool_types::{BuiltinTool, DeferrablePolicy, ToolHints, ToolPolicy};
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn def(name: &str) -> ToolDefinition {
        ToolDefinition::Builtin(BuiltinTool {
            name: name.to_string(),
            display_name: None,
            description: String::new(),
            parameters: serde_json::json!({}),
            policy: ToolPolicy::default(),
            category: None,
            deferrable: DeferrablePolicy::default(),
            hints: ToolHints::default(),
            full_parameters: None,
        })
    }

    fn names(tools: &[ToolDefinition]) -> Vec<String> {
        tools.iter().map(|t| t.name().to_string()).collect()
    }

    fn key() -> CacheKey {
        (Uuid::nil(), "docs".to_string())
    }

    /// A cold key fetches once and is served from cache on the next call.
    #[tokio::test]
    async fn cold_fetches_then_serves_from_cache() {
        let cache = Arc::new(McpDiscoveryCache::new());
        let calls = Arc::new(AtomicUsize::new(0));

        let refresh = {
            let calls = calls.clone();
            move || {
                let calls = calls.clone();
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Some(vec![def("docs__search")])
                }
            }
        };

        let first = cache.resolve(key(), refresh.clone()).await;
        assert_eq!(names(&first), ["docs__search"]);
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        // Within TTL the second call is a pure cache hit — no refresh.
        let second = cache.resolve(key(), refresh).await;
        assert_eq!(names(&second), ["docs__search"]);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    /// A cold fetch failure degrades to an empty list without caching.
    #[tokio::test]
    async fn cold_failure_returns_empty_and_does_not_cache() {
        let cache = Arc::new(McpDiscoveryCache::new());
        let out = cache
            .resolve(key(), || async { None::<Vec<ToolDefinition>> })
            .await;
        assert!(out.is_empty());
        assert!(matches!(
            cache.classify(&key(), Instant::now()),
            Freshness::Cold
        ));
    }

    /// Many concurrent cold callers collapse into a single upstream fetch.
    #[tokio::test]
    async fn concurrent_cold_callers_are_single_flight() {
        let cache = Arc::new(McpDiscoveryCache::new());
        let calls = Arc::new(AtomicUsize::new(0));

        let mut handles = Vec::new();
        for _ in 0..16 {
            let cache = cache.clone();
            let calls = calls.clone();
            handles.push(tokio::spawn(async move {
                cache
                    .resolve(key(), move || {
                        let calls = calls.clone();
                        async move {
                            // Yield so siblings queue on the single-flight lock
                            // before the first refresh stores a fresh entry.
                            tokio::task::yield_now().await;
                            calls.fetch_add(1, Ordering::SeqCst);
                            Some(vec![def("docs__search")])
                        }
                    })
                    .await
            }));
        }
        for h in handles {
            assert_eq!(names(&h.await.unwrap()), ["docs__search"]);
        }
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "single-flight must fetch once"
        );
    }

    /// A stale entry is served immediately and revalidated in the background.
    #[tokio::test]
    async fn stale_serves_cached_then_revalidates_in_background() {
        // TTL of zero makes any stored entry immediately stale.
        let cache = Arc::new(McpDiscoveryCache::with_ttl(Duration::ZERO));
        cache.store(key(), vec![def("v1")], Instant::now());

        let calls = Arc::new(AtomicUsize::new(0));
        let stale = cache
            .resolve(key(), {
                let calls = calls.clone();
                move || {
                    let calls = calls.clone();
                    async move {
                        calls.fetch_add(1, Ordering::SeqCst);
                        Some(vec![def("v2")])
                    }
                }
            })
            .await;

        // The caller gets the stale value without blocking on the refresh.
        assert_eq!(names(&stale), ["v1"]);

        // The background refresh runs and replaces the cached value with v2.
        let mut updated = false;
        for _ in 0..200 {
            tokio::task::yield_now().await;
            if let Freshness::Stale(tools) | Freshness::Fresh(tools) =
                cache.classify(&key(), Instant::now())
                && names(&tools) == ["v2"]
            {
                updated = true;
                break;
            }
        }
        assert!(updated, "background refresh must replace the stale entry");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}

//! Per-session host MCP tool-discovery cache.
//!
//! Mirrors the hosted server's stale-while-revalidate + single-flight tool
//! cache, adapted to the in-process runtime's
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

use everruns_contracts::tool_types::ToolDefinition;
use tokio::sync::Mutex as AsyncMutex;
use uuid::Uuid;

/// How long cached tools are considered fresh. Matches the hosted server's
/// `TOOL_CACHE_TTL` (#2131) so both paths revalidate on the same cadence.
pub(crate) const TOOL_CACHE_TTL: Duration = Duration::from_secs(3600);

// THREAT[TM-DOS-031]: Bound retained discovery catalogs and failed attachment entries.
/// Maximum cached scoped MCP server discoveries retained by one runtime.
const MAX_CACHE_ENTRIES: usize = 1024;

/// Maximum number of tools retained for one scoped MCP server.
const MAX_TOOLS_PER_ENTRY: usize = crate::mcp::MCP_DISCOVERY_MAX_TOOLS;

/// Maximum serialized size retained for one scoped MCP server discovery.
const MAX_ENTRY_BYTES: usize = crate::mcp::MCP_DISCOVERY_MAX_BYTES;

/// Maximum serialized tool data retained across one runtime's discovery cache.
const MAX_CACHE_BYTES: usize = 128 * 1024 * 1024;

/// Identifies one attachment: session, sanitized name, and configuration digest.
pub(crate) type CacheKey = (Uuid, String, [u8; 32]);

#[derive(Clone, Copy, Debug)]
pub(crate) enum DiscoveryFailure {
    Authentication,
    Transient,
}

struct CacheEntry {
    tools: Vec<ToolDefinition>,
    serialized_bytes: usize,
    cached_at: Instant,
    last_used: Instant,
    failure: Option<DiscoveryFailure>,
}

fn cached_bytes(entries: &HashMap<CacheKey, CacheEntry>) -> usize {
    entries.values().fold(0usize, |total, entry| {
        total.saturating_add(entry.serialized_bytes)
    })
}

/// Counts serialized bytes without retaining them, erroring once the running
/// total exceeds `limit`. Used to size-check cache admission without allocating
/// the full serialized form of attacker-controlled tool definitions.
struct LimitCountingWriter {
    written: usize,
    limit: usize,
}

impl std::io::Write for LimitCountingWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.written = self.written.saturating_add(buf.len());
        if self.written > self.limit {
            return Err(std::io::Error::other(
                "mcp discovery entry exceeds cache byte limit",
            ));
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
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
    byte_budget: usize,
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
            byte_budget: MAX_CACHE_BYTES,
        }
    }

    #[cfg(test)]
    fn with_test_byte_budget(ttl: Duration, byte_budget: usize) -> Self {
        Self {
            byte_budget,
            ..Self::with_ttl(ttl)
        }
    }

    fn entries(&self) -> std::sync::MutexGuard<'_, HashMap<CacheKey, CacheEntry>> {
        self.entries.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Drop every cached discovery for a session after its live capability set
    /// changes. Capability-contributed MCP servers may have been added,
    /// removed, or replaced under an existing logical server name.
    pub(crate) fn invalidate_session(&self, session_id: Uuid) {
        // Retire in-flight refresh identities before clearing entries. Their
        // completion must not repopulate the cache after a reload.
        let mut locks = self.locks.map.lock().unwrap_or_else(|e| e.into_inner());
        locks.retain(|(id, _, _), _| *id != session_id);
        self.entries()
            .retain(|(cached_session_id, _, _), _| *cached_session_id != session_id);
    }

    /// Classify the entry for `key` at `now`.
    fn classify(&self, key: &CacheKey, now: Instant) -> Freshness {
        match self.entries().get_mut(key) {
            None => Freshness::Cold,
            Some(entry) if matches!(entry.failure, Some(DiscoveryFailure::Authentication)) => {
                entry.last_used = now;
                Freshness::Fresh(Vec::new())
            }
            Some(entry) if matches!(entry.failure, Some(DiscoveryFailure::Transient)) => {
                entry.last_used = now;
                if now.duration_since(entry.cached_at) < Duration::from_secs(5) {
                    Freshness::Fresh(entry.tools.clone())
                } else if entry.tools.is_empty() {
                    Freshness::Cold
                } else {
                    Freshness::Stale(entry.tools.clone())
                }
            }
            Some(entry) if now.duration_since(entry.cached_at) < self.ttl => {
                entry.last_used = now;
                Freshness::Fresh(entry.tools.clone())
            }
            Some(entry) => {
                entry.last_used = now;
                Freshness::Stale(entry.tools.clone())
            }
        }
    }

    fn store(&self, key: CacheKey, tools: Vec<ToolDefinition>, now: Instant) {
        if !Self::cacheable(&tools) {
            tracing::warn!(
                server = %key.1,
                tool_count = tools.len(),
                max_tools = MAX_TOOLS_PER_ENTRY,
                max_bytes = MAX_ENTRY_BYTES,
                "scoped MCP tool discovery result exceeds cache limits; skipping cache store"
            );
            return;
        }

        self.store_entry(key, tools, now, None);
    }

    fn store_entry(
        &self,
        key: CacheKey,
        tools: Vec<ToolDefinition>,
        now: Instant,
        failure: Option<DiscoveryFailure>,
    ) {
        let serialized_bytes = Self::serialized_size(&tools).unwrap_or(0);
        let mut entries = self.entries();
        entries.insert(
            key.clone(),
            CacheEntry {
                tools,
                serialized_bytes,
                cached_at: now,
                last_used: now,
                failure,
            },
        );
        while entries.len() > MAX_CACHE_ENTRIES || cached_bytes(&entries) > self.byte_budget {
            let Some(evict_key) = entries
                .iter()
                .filter(|(candidate, _)| *candidate != &key)
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(candidate, _)| candidate.clone())
            else {
                break;
            };
            entries.remove(&evict_key);
        }
    }

    fn store_refresh(
        &self,
        key: CacheKey,
        lock: &Arc<AsyncMutex<()>>,
        result: Result<Vec<ToolDefinition>, DiscoveryFailure>,
    ) {
        let locks = self.locks.map.lock().unwrap_or_else(|e| e.into_inner());
        if !locks
            .get(&key)
            .is_some_and(|current| Arc::ptr_eq(current, lock))
        {
            return;
        }
        match result {
            Ok(tools) => self.store(key, tools, Instant::now()),
            Err(failure) => {
                let tools = match failure {
                    DiscoveryFailure::Authentication => Vec::new(),
                    DiscoveryFailure::Transient => self
                        .entries()
                        .get(&key)
                        .map(|entry| entry.tools.clone())
                        .unwrap_or_default(),
                };
                self.store_entry(key, tools, Instant::now(), Some(failure));
            }
        }
    }

    fn cacheable(tools: &[ToolDefinition]) -> bool {
        if tools.len() > MAX_TOOLS_PER_ENTRY {
            return false;
        }
        Self::serialized_size(tools).is_some()
    }

    fn serialized_size(tools: &[ToolDefinition]) -> Option<usize> {
        // Measure the serialized size by streaming into a counting writer that
        // short-circuits once the running total crosses MAX_ENTRY_BYTES. This
        // avoids materializing a full serialized buffer per tool just to size
        // it, so a huge attacker-controlled tool definition can't force a large
        // transient allocation before the admission check rejects it.
        let mut writer = LimitCountingWriter {
            written: 0,
            limit: MAX_ENTRY_BYTES,
        };
        for tool in tools {
            if serde_json::to_writer(&mut writer, tool).is_err() {
                return None;
            }
        }
        Some(writer.written)
    }

    /// Resolve tools for `key`, applying stale-while-revalidate + single-flight.
    ///
    /// `refresh` performs the upstream fetch, returning `Ok(tools)` on success
    /// or a classified failure (the failure is the closure's to log). It may be
    /// called from a background task, so it must be `Send + Sync + 'static`.
    pub(crate) async fn resolve<F, Fut>(
        self: &Arc<Self>,
        key: CacheKey,
        refresh: F,
    ) -> Vec<ToolDefinition>
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Vec<ToolDefinition>, DiscoveryFailure>> + Send + 'static,
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
        Fut: Future<Output = Result<Vec<ToolDefinition>, DiscoveryFailure>>,
    {
        let lock = self.locks.lock_for(&key);
        let _guard = lock.clone().lock_owned().await;

        if let Freshness::Fresh(tools) = self.classify(&key, Instant::now()) {
            return tools;
        }

        let result = refresh().await;
        let tools = match &result {
            Ok(tools) => tools.clone(),
            Err(DiscoveryFailure::Authentication) => Vec::new(),
            Err(DiscoveryFailure::Transient) => match self.classify(&key, Instant::now()) {
                Freshness::Fresh(tools) | Freshness::Stale(tools) => tools,
                Freshness::Cold => Vec::new(),
            },
        };
        self.store_refresh(key, &lock, result);
        tools
    }

    /// Background revalidation: deduplicated via `try_lock`, so at most one
    /// refresh runs per key at a time and extra triggers are dropped. The caller
    /// has already been served the stale cache, so a failure is only logged
    /// (inside `refresh`) and enters a bounded cooldown or authentication hold.
    fn spawn_background_refresh<F, Fut>(self: &Arc<Self>, key: CacheKey, refresh: F)
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Vec<ToolDefinition>, DiscoveryFailure>> + Send + 'static,
    {
        let lock = self.locks.lock_for(&key);
        let Ok(guard) = lock.clone().try_lock_owned() else {
            return; // a refresh is already in flight for this key
        };
        let cache = self.clone();
        tokio::spawn(async move {
            let _guard = guard; // held for the refresh duration -> single-flight
            let result = refresh().await;
            cache.store_refresh(key, &lock, result);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_contracts::tool_types::{BuiltinTool, DeferrablePolicy, ToolHints, ToolPolicy};
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

    fn described_def(name: &str, size: usize) -> ToolDefinition {
        ToolDefinition::Builtin(BuiltinTool {
            name: name.to_string(),
            display_name: None,
            description: "x".repeat(size),
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
        (Uuid::nil(), "docs".to_string(), [0; 32])
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
                    Ok(vec![def("docs__search")])
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

    /// Transient failures have a brief cooldown, then can be fetched again.
    #[tokio::test]
    async fn transient_failure_returns_empty_with_a_short_cooldown() {
        let cache = Arc::new(McpDiscoveryCache::new());
        let out = cache
            .resolve(key(), || async { Err(DiscoveryFailure::Transient) })
            .await;
        assert!(out.is_empty());
        assert!(matches!(
            cache.classify(&key(), Instant::now()),
            Freshness::Fresh(_)
        ));
        assert!(matches!(
            cache.classify(&key(), Instant::now() + Duration::from_secs(6)),
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
                            Ok(vec![def("docs__search")])
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

    #[test]
    fn oversized_results_are_not_cached() {
        let cache = McpDiscoveryCache::with_ttl(TOOL_CACHE_TTL);
        let tools = (0..=MAX_TOOLS_PER_ENTRY)
            .map(|i| def(&format!("tool_{i}")))
            .collect::<Vec<_>>();

        cache.store(key(), tools, Instant::now());

        assert!(matches!(
            cache.classify(&key(), Instant::now()),
            Freshness::Cold
        ));
    }

    #[test]
    fn oversized_serialized_catalog_is_not_cached() {
        let cache = McpDiscoveryCache::new();
        cache.store(
            key(),
            vec![described_def("too_large", MAX_ENTRY_BYTES)],
            Instant::now(),
        );
        assert!(matches!(
            cache.classify(&key(), Instant::now()),
            Freshness::Cold
        ));
    }

    #[test]
    fn two_thousand_tool_discovery_is_cached_and_served_as_a_hit() {
        let cache = McpDiscoveryCache::new();
        let tools = (0..2_000)
            .map(|i| def(&format!("tool_{i}")))
            .collect::<Vec<_>>();
        cache.store(key(), tools, Instant::now());
        assert!(matches!(
            cache.classify(&key(), Instant::now()),
            Freshness::Fresh(tools) if tools.len() == 2_000
        ));
    }

    #[test]
    fn ten_thousand_tool_catalog_fits_the_per_entry_cache_bound() {
        let cache = McpDiscoveryCache::new();
        let tools = (0..MAX_TOOLS_PER_ENTRY)
            .map(|i| def(&format!("tool_{i}")))
            .collect::<Vec<_>>();
        cache.store(key(), tools, Instant::now());
        assert!(matches!(
            cache.classify(&key(), Instant::now()),
            Freshness::Fresh(tools) if tools.len() == MAX_TOOLS_PER_ENTRY
        ));
    }

    #[test]
    fn weighted_global_budget_evicts_least_recently_used_catalog() {
        let tools = vec![described_def("large", 1_000)];
        let bytes = McpDiscoveryCache::serialized_size(&tools).unwrap();
        let cache = McpDiscoveryCache::with_test_byte_budget(TOOL_CACHE_TTL, bytes + 1);
        let first = (Uuid::from_u128(1), "first".into(), [0; 32]);
        let second = (Uuid::from_u128(2), "second".into(), [0; 32]);
        let base = Instant::now();
        cache.store(first.clone(), tools.clone(), base);
        cache.store(second.clone(), tools, base + Duration::from_millis(1));
        assert!(matches!(
            cache.classify(&first, Instant::now()),
            Freshness::Cold
        ));
        assert!(matches!(
            cache.classify(&second, Instant::now()),
            Freshness::Fresh(_)
        ));
    }

    #[test]
    fn cache_evicts_least_recently_used_entry_at_capacity() {
        let cache = McpDiscoveryCache::with_ttl(TOOL_CACHE_TTL);
        let base = Instant::now();
        let first = (Uuid::from_u128(1), "first".to_string(), [0; 32]);
        let recent = (Uuid::from_u128(2), "recent".to_string(), [0; 32]);

        cache.store(first.clone(), vec![def("first")], base);
        for i in 2..=MAX_CACHE_ENTRIES {
            cache.store(
                (Uuid::from_u128(i as u128), format!("server_{i}"), [0; 32]),
                vec![def("filler")],
                base + Duration::from_millis(i as u64),
            );
        }
        assert!(matches!(
            cache.classify(&first, base + Duration::from_millis(1_500)),
            Freshness::Fresh(_)
        ));
        cache.store(
            recent.clone(),
            vec![def("recent")],
            base + Duration::from_millis(2_000),
        );

        assert!(matches!(
            cache.classify(&first, base + Duration::from_millis(2_001)),
            Freshness::Fresh(_)
        ));
        assert!(matches!(
            cache.classify(
                &(Uuid::from_u128(2), "server_2".to_string(), [0; 32]),
                base + Duration::from_millis(2_001)
            ),
            Freshness::Cold
        ));
        assert!(matches!(
            cache.classify(&recent, base + Duration::from_millis(2_001)),
            Freshness::Fresh(_)
        ));
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
                        Ok(vec![def("v2")])
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

    #[tokio::test]
    async fn failed_catalog_refresh_keeps_only_the_previous_complete_catalog() {
        let cache = Arc::new(McpDiscoveryCache::with_ttl(Duration::ZERO));
        cache.store(key(), vec![def("complete_v1")], Instant::now());
        let served = cache
            .resolve(key(), || async { Err(DiscoveryFailure::Transient) })
            .await;
        assert_eq!(names(&served), ["complete_v1"]);
        assert!(matches!(
            cache.classify(&key(), Instant::now()),
            Freshness::Stale(tools) if names(&tools) == ["complete_v1"]
        ));
    }
}

#[cfg(test)]
mod failure_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn auth_failure_is_single_flight_until_invalidated_or_configuration_changes() {
        let cache = Arc::new(McpDiscoveryCache::new());
        let key = (Uuid::nil(), "private".into(), [0; 32]);
        let calls = Arc::new(AtomicUsize::new(0));
        let mut workers = Vec::new();
        for _ in 0..20 {
            let cache = cache.clone();
            let key = key.clone();
            let calls = calls.clone();
            workers.push(tokio::spawn(async move {
                cache
                    .resolve(key, move || {
                        let calls = calls.clone();
                        async move {
                            calls.fetch_add(1, Ordering::SeqCst);
                            tokio::task::yield_now().await;
                            Err(DiscoveryFailure::Authentication)
                        }
                    })
                    .await
            }));
        }
        for worker in workers {
            assert!(worker.await.unwrap().is_empty());
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(matches!(
            cache.classify(&key, Instant::now() + TOOL_CACHE_TTL * 2),
            Freshness::Fresh(_)
        ));
        let mut changed = key.clone();
        changed.2 = [1; 32];
        assert!(matches!(
            cache.classify(&changed, Instant::now()),
            Freshness::Cold
        ));
        cache.invalidate_session(key.0);
        assert!(matches!(
            cache.classify(&key, Instant::now()),
            Freshness::Cold
        ));
    }
}

#[cfg(test)]
mod reload_tests {
    use super::*;
    #[tokio::test]
    async fn invalidation_during_refresh_does_not_restore_old_auth_failure() {
        let cache = Arc::new(McpDiscoveryCache::new());
        let key = (Uuid::nil(), "private".into(), [0; 32]);
        let started = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let pending = {
            let cache = cache.clone();
            let key = key.clone();
            let started = started.clone();
            let release = release.clone();
            tokio::spawn(async move {
                cache
                    .resolve(key, move || {
                        let started = started.clone();
                        let release = release.clone();
                        async move {
                            started.notify_one();
                            release.notified().await;
                            Err(DiscoveryFailure::Authentication)
                        }
                    })
                    .await
            })
        };
        started.notified().await;
        cache.invalidate_session(key.0);
        release.notify_one();
        pending.await.unwrap();
        assert!(matches!(
            cache.classify(&key, Instant::now()),
            Freshness::Cold
        ));
    }
}

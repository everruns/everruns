// Agent, harness and provider configuration rows, cached in memory while they
// can be kept exact.
//
// Why: every RPC and listener of a turn resolves the same agent and harness
// chain again (turn context, capability checks, session titles, notifications,
// observers): about 35 reads of rows that do not change during the turn.
//
// Decision: the same contract as the feature flag cache
// (`org_feature_flag_cache.rs`). Triggers on `agents`, `agent_capabilities`,
// `harnesses`, `harness_capabilities` and `providers` notify
// `config_rows_changed` with `agent:<id>`, `harness:<id>` or `provider:<id>` on
// every committed change, whoever wrote it,
// and every listening server drops that entry. Reads are served from memory
// only while the listener is connected; a lost connection clears everything.
// A read that overlapped an invalidation is not stored, and reads inside a
// command transaction bypass the cache. A change is seen once its notification
// arrives, normally within milliseconds of the commit; until then a read on
// another request can still get the previous row.
//
// Decision: the agent row's usage counters (`total_*`) change on every model
// call, so the trigger ignores updates that touch only them (and the
// `updated_at` they bump), and a cached row carries both as of its load. Only
// `get_agent_config` serves the cached row; `get_agent` and the public-id
// lookups stay uncached so anything that shows usage reads it fresh.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use anyhow::Result;
use everruns_contracts::typed_id::{AgentId, HarnessId};
use moka::sync::Cache;
use uuid::Uuid;

use super::StorageBackend;
use crate::storage::{AgentCapabilityRow, AgentRow, HarnessCapabilityRow, HarnessRow, ProviderRow};

const CAPACITY: u64 = 20_000;
/// Bounds memory and a row written with notifications somehow off; exactness
/// comes from the notifications, not from this.
const TTL: Duration = Duration::from_secs(300);

/// The PostgreSQL channel the configuration triggers notify.
pub const CONFIG_ROWS_CHANNEL: &str = "config_rows_changed";

/// Shared by a backend and the backends derived from it (`for_background`).
#[derive(Clone)]
pub struct ConfigRowCache(Arc<Inner>);

struct Inner {
    agents: Cache<Uuid, AgentRow>,
    agent_capabilities: Cache<Uuid, Vec<AgentCapabilityRow>>,
    harnesses: Cache<Uuid, HarnessRow>,
    harness_capabilities: Cache<Uuid, Vec<HarnessCapabilityRow>>,
    providers: Cache<Uuid, ProviderRow>,
    /// The invalidation listener is connected and listening.
    live: AtomicBool,
    /// Bumped by every invalidation.
    generation: AtomicU64,
}

fn cache<V: Clone + Send + Sync + 'static>() -> Cache<Uuid, V> {
    Cache::builder()
        .max_capacity(CAPACITY)
        .time_to_live(TTL)
        .build()
}

impl Default for ConfigRowCache {
    fn default() -> Self {
        Self(Arc::new(Inner {
            agents: cache(),
            agent_capabilities: cache(),
            harnesses: cache(),
            harness_capabilities: cache(),
            providers: cache(),
            live: AtomicBool::new(false),
            generation: AtomicU64::new(0),
        }))
    }
}

/// What a `config_rows_changed` notification names.
#[derive(Debug, PartialEq, Eq)]
pub enum ConfigRowChange {
    Agent(Uuid),
    Harness(Uuid),
    Provider(Uuid),
}

impl ConfigRowChange {
    /// `agent:<uuid>`, `harness:<uuid>` or `provider:<uuid>`; anything else is
    /// `None`.
    pub fn parse(payload: &str) -> Option<Self> {
        let (kind, id) = payload.split_once(':')?;
        let id = id.parse().ok()?;
        match kind {
            "agent" => Some(Self::Agent(id)),
            "harness" => Some(Self::Harness(id)),
            "provider" => Some(Self::Provider(id)),
            _ => None,
        }
    }
}

impl ConfigRowCache {
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

    /// A row changed.
    pub fn invalidate(&self, change: ConfigRowChange) {
        self.0.generation.fetch_add(1, Ordering::SeqCst);
        match change {
            ConfigRowChange::Agent(id) => {
                self.0.agents.invalidate(&id);
                self.0.agent_capabilities.invalidate(&id);
            }
            ConfigRowChange::Harness(id) => {
                self.0.harnesses.invalidate(&id);
                self.0.harness_capabilities.invalidate(&id);
            }
            ConfigRowChange::Provider(id) => self.0.providers.invalidate(&id),
        }
    }

    fn clear(&self) {
        self.0.generation.fetch_add(1, Ordering::SeqCst);
        self.0.agents.invalidate_all();
        self.0.agent_capabilities.invalidate_all();
        self.0.harnesses.invalidate_all();
        self.0.harness_capabilities.invalidate_all();
        self.0.providers.invalidate_all();
    }

    fn generation(&self) -> u64 {
        self.0.generation.load(Ordering::SeqCst)
    }

    fn usable(&self) -> bool {
        self.0.live.load(Ordering::SeqCst) && !crate::storage::transaction::in_transaction()
    }

    /// Keep what a read returned, unless something was invalidated since the
    /// read began: it may predate that change.
    fn store<V: Clone + Send + Sync + 'static>(
        &self,
        entries: &Cache<Uuid, V>,
        id: Uuid,
        generation: u64,
        value: &V,
    ) {
        if self.usable() && self.generation() == generation {
            entries.insert(id, value.clone());
        }
    }

    /// Serve `id` from `entries`, or load and keep it.
    async fn read_through<V, F>(
        &self,
        entries: &Cache<Uuid, V>,
        id: Uuid,
        load: F,
    ) -> Result<Option<V>>
    where
        V: Clone + Send + Sync + 'static,
        F: std::future::Future<Output = Result<Option<V>>>,
    {
        if !self.usable() {
            return load.await;
        }
        if let Some(value) = entries.get(&id) {
            return Ok(Some(value));
        }
        let generation = self.generation();
        let value = load.await?;
        if let Some(value) = &value {
            self.store(entries, id, generation, value);
        }
        Ok(value)
    }
}

impl StorageBackend {
    /// Derived backends share the cache, so one listener keeps them all exact.
    pub(super) fn with_config_row_cache(mut self, cache: ConfigRowCache) -> Self {
        self.config_rows = cache;
        self
    }

    /// The cache the invalidation listener keeps exact.
    pub fn config_row_cache(&self) -> &ConfigRowCache {
        &self.config_rows
    }

    /// An agent's configuration row, from memory while the cache is exact.
    ///
    /// The usage counters (`total_*`) and `updated_at` are as of when the row
    /// was loaded; read
    /// them through [`StorageBackend::get_agent`].
    pub async fn get_agent_config(&self, org_id: i64, id: AgentId) -> Result<Option<AgentRow>> {
        #[cfg(test)]
        self.fail_if_forced("get_agent")?;
        #[cfg(test)]
        self.record_session_list_lookup().await;
        let cache = &self.config_rows;
        let row = cache
            .read_through(&cache.0.agents, id.uuid(), async {
                self.db.get_agent(org_id, id).await
            })
            .await?;
        // Keyed by id alone: another org's row is not this org's agent.
        Ok(row.filter(|row| row.org_id == org_id))
    }

    pub(super) async fn cached_agent_capabilities(
        &self,
        agent_id: Uuid,
    ) -> Result<Vec<AgentCapabilityRow>> {
        let cache = &self.config_rows;
        Ok(cache
            .read_through(&cache.0.agent_capabilities, agent_id, async {
                self.db.get_agent_capabilities(agent_id).await.map(Some)
            })
            .await?
            .unwrap_or_default())
    }

    pub(super) async fn cached_harness(
        &self,
        org_id: i64,
        id: HarnessId,
    ) -> Result<Option<HarnessRow>> {
        let cache = &self.config_rows;
        let row = cache
            .read_through(&cache.0.harnesses, id.uuid(), async {
                self.db.get_harness(org_id, id).await
            })
            .await?;
        Ok(row.filter(|row| row.org_id == org_id))
    }

    /// A provider row, from memory while the cache is exact. Callers that
    /// show or edit a provider read it fresh through `get_provider`.
    pub async fn get_provider_config(&self, org_id: i64, id: Uuid) -> Result<Option<ProviderRow>> {
        let cache = &self.config_rows;
        let row = cache
            .read_through(&cache.0.providers, id, async {
                self.db.get_provider(org_id, id).await
            })
            .await?;
        Ok(row.filter(|row| row.org_id == org_id))
    }

    pub(super) async fn cached_harness_capabilities(
        &self,
        harness_id: Uuid,
    ) -> Result<Vec<HarnessCapabilityRow>> {
        let cache = &self.config_rows;
        Ok(cache
            .read_through(&cache.0.harness_capabilities, harness_id, async {
                self.db.get_harness_capabilities(harness_id).await.map(Some)
            })
            .await?
            .unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notifications_name_an_agent_or_a_harness() {
        let id = Uuid::now_v7();
        assert_eq!(
            ConfigRowChange::parse(&format!("agent:{id}")),
            Some(ConfigRowChange::Agent(id))
        );
        assert_eq!(
            ConfigRowChange::parse(&format!("harness:{id}")),
            Some(ConfigRowChange::Harness(id))
        );
        assert_eq!(
            ConfigRowChange::parse(&format!("provider:{id}")),
            Some(ConfigRowChange::Provider(id))
        );
        assert_eq!(ConfigRowChange::parse("agent:nope"), None);
        assert_eq!(ConfigRowChange::parse(&format!("model:{id}")), None);
    }

    #[tokio::test]
    async fn a_read_that_overlaps_an_invalidation_is_not_kept() {
        let cache = ConfigRowCache::default();
        cache.set_live(true);
        let id = Uuid::now_v7();
        let generation = cache.generation();
        cache.invalidate(ConfigRowChange::Agent(id));
        cache.store(&cache.0.agent_capabilities, id, generation, &Vec::new());
        assert!(cache.0.agent_capabilities.get(&id).is_none());

        let generation = cache.generation();
        cache.store(&cache.0.agent_capabilities, id, generation, &Vec::new());
        assert!(cache.0.agent_capabilities.get(&id).is_some());
        cache.invalidate(ConfigRowChange::Agent(id));
        assert!(cache.0.agent_capabilities.get(&id).is_none());
    }

    #[tokio::test]
    async fn losing_the_listener_drops_everything_and_stops_caching() {
        let cache = ConfigRowCache::default();
        cache.set_live(true);
        let id = Uuid::now_v7();
        let generation = cache.generation();
        cache.store(&cache.0.harness_capabilities, id, generation, &Vec::new());
        cache.set_live(false);
        cache.0.harness_capabilities.run_pending_tasks();
        assert_eq!(cache.0.harness_capabilities.entry_count(), 0);
        let generation = cache.generation();
        cache.store(&cache.0.harness_capabilities, id, generation, &Vec::new());
        assert!(cache.0.harness_capabilities.get(&id).is_none());
    }
}

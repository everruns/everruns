//! An executor outside the shards (data plane v2 step 6, design.md
//! "Executors"): a stateless process that pulls activations from every shard
//! with credits and runs them with the Everruns Engine.
//!
//! Decisions:
//! - The executor runs Engine's activation against [`RaftHosting`] over a
//!   [`Remote`] link, the same authority code a shard node runs; everything
//!   it knows of a session is its mirror, refreshed by its own writes and by
//!   one long poll per running session.
//! - Free slots are split among the shards as credits, one pull in flight
//!   per shard; a shard returns credits it can't use, and they go back to the
//!   pool for any shard.
//! - Recently run sessions stay mirrored (an LRU, `mirror_sessions` deep),
//!   so a session that comes back here, as soft affinity prefers, starts
//!   warm; segment and blob reads go through the store's cache.
//! - Draining (a rolling deploy): stop pulling, ask every running epoch to
//!   hand off at its next step boundary, and exit once none runs. Another
//!   executor resumes each turn from the journal.
//! - Memory (memory design, "Executor"): an executor offers a shard the
//!   smaller of its free slots and the turns its memory has room for
//!   (`memwatch::turn_credits`). At 95% of its limit it starts nothing and
//!   hands its newest turns back, which resume on another executor from the
//!   session's log; at 98% it drains and exits for a clean restart, before
//!   the kernel would kill it. Past half its limit it drops its coldest idle
//!   mirrors first (`memwatch::mirror_trim`): they are a cache.
//! - Capability memory classes (memory design, "Capabilities"): only
//!   capabilities with a hard memory cap run inside the executor process
//!   (the virtual shell's `max_memory`, 10 MB, in `integrations::bashkit`;
//!   Lua's `set_memory_limit`). Browsers, desktops and user code with real
//!   processes run in sandboxes outside it (`sandbox_*` capabilities), so no
//!   one turn's tool can take the executor's memory. A large payload a turn
//!   reads comes back from the blob store (`payload.rs`) and is counted in
//!   the executor's own reading like any other.
//! - Freshness (`tenants.rs`): an executor that hasn't heard from the
//!   directory shard within the bound pulls nothing, and one offered a
//!   session of a suspended or deleted tenant gives it back unrun.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use everruns_contracts::typed_id::SessionId;
use parking_lot::Mutex;

use crate::cluster::TopologySource;
use crate::host::Host;
use crate::hosting::RaftHosting;
use crate::node::now_ms;
use crate::remote::{ExecutorId, Remote};
use crate::state::{Placement, ShardId};
use crate::store::Store;

#[derive(Debug, Clone)]
pub struct ExecutorConfig {
    pub id: ExecutorId,
    pub topology: TopologySource,
    pub store: Store,
    /// Turns this executor runs at once.
    pub slots: usize,
    /// Sessions kept mirrored after their activation ended.
    pub mirror_sessions: usize,
    /// How long a shard may hold this executor's credits.
    pub pull_wait: Duration,
    /// How often due dreams are looked for (`Executor::dreams_loop`).
    pub dream_sweep: Duration,
    /// Memory limit in bytes; 0: the container's, if any.
    pub memory_limit: u64,
}

pub struct Executor {
    pub cfg: ExecutorConfig,
    pub remote: Arc<Remote>,
    pub hosting: RaftHosting,
    pub engine: everruns::Engine,
    free: AtomicUsize,
    freed: tokio::sync::Notify,
    /// Sessions running here, and since when.
    running: Mutex<HashMap<SessionId, std::time::Instant>>,
    /// Running sessions handed back for memory: their activation loop
    /// stops at its next end instead of starting the session again.
    shed: Mutex<HashSet<SessionId>>,
    /// Set when memory reached the restart level (`serve_executor` exits).
    restart: AtomicBool,
    pub restart_wanted: tokio::sync::Notify,
    /// Sessions mirrored but not running, least recently run first.
    recent: Mutex<VecDeque<SessionId>>,
    draining: AtomicBool,
    pub tenants: crate::tenants::TenantView,
    pub stats: crate::prom::Counters,
    /// The data plane's key (`AGENTD_DP_KEY`), which opens org data keys.
    dp_key: Mutex<Option<everruns_contracts::dataplane_seal::Key>>,
    /// MCP credentials of the orgs whose turns run here.
    pub mcp: Arc<crate::mcp::OrgMcp>,
    /// Managed sandboxes of the sessions whose turns run here.
    pub sandboxes: Arc<crate::sandbox::OrgSandboxes>,
}

impl Executor {
    pub fn start(cfg: ExecutorConfig, host: Arc<dyn Host>) -> anyhow::Result<Arc<Self>> {
        anyhow::ensure!(
            cfg.id >= crate::remote::EXECUTOR_IDS,
            "executor ids start at {}",
            crate::remote::EXECUTOR_IDS
        );
        let remote = Remote::new(cfg.id, cfg.store.clone())?;
        remote.set_shards(cfg.topology.read()?.shards);
        let hosting = RaftHosting::remote(remote.clone());
        let sandboxes = Arc::new(crate::sandbox::OrgSandboxes::default());
        hosting.set_sandboxes(sandboxes.clone());
        hosting.set_gateway(crate::gateway::Gateway::new(cfg.topology.clone()));
        let orgs = Arc::new(crate::providers::OrgProviders::default());
        let mcp = Arc::new(crate::mcp::OrgMcp::from_env());
        let services = crate::host::OrgServices {
            providers: orgs.host_providers(),
            mcp: mcp.clone(),
        };
        let engine = crate::node::engine(hosting.clone(), host, Some(services));
        let executor = Arc::new(Self {
            free: AtomicUsize::new(cfg.slots),
            cfg,
            remote,
            hosting,
            engine,
            freed: tokio::sync::Notify::new(),
            running: Mutex::default(),
            shed: Mutex::default(),
            restart: AtomicBool::new(false),
            restart_wanted: tokio::sync::Notify::new(),
            recent: Mutex::default(),
            draining: AtomicBool::new(false),
            tenants: Default::default(),
            stats: Default::default(),
            dp_key: Mutex::new(data_plane_key()?),
            mcp,
            sandboxes,
        });
        let material: Arc<dyn crate::providers::OrgMaterial> = executor.clone();
        orgs.bind(Arc::downgrade(&material));
        executor.mcp.bind(Arc::downgrade(&material));
        executor.sandboxes.bind(Arc::downgrade(&material));
        executor.hosting.set_material(Arc::downgrade(&material));
        crate::lookups::serve_org_egress(Arc::downgrade(&material));
        // Whatever refreshes the mirror wakes Engine's waiters, as a node's
        // applied changes do (`Remote::absorb`); a session that left the
        // mirror or froze hands off.
        let weak = Arc::downgrade(&executor);
        let _ = executor
            .remote
            .shared
            .on_change
            .set(Box::new(move |change| {
                let (crate::state::Change::Session(session)
                | crate::state::Change::Event(session, _)) = change;
                let Some(executor) = weak.upgrade() else {
                    return;
                };
                let frozen = executor
                    .remote
                    .shared
                    .state
                    .read()
                    .sessions
                    .get(session)
                    .is_none_or(|s| s.placement == Placement::Frozen);
                executor.hosting.changed(*session, frozen);
            }));
        Ok(executor)
    }

    /// Start pulling from every shard, and keep up with the topology and
    /// the tenants.
    pub fn spawn(self: &Arc<Self>) {
        tokio::spawn(self.clone().tenants_loop());
        tokio::spawn(self.clone().dreams_loop());
        tokio::spawn(self.clone().leases_loop());
        tokio::spawn(self.clone().memory_loop());
        let executor = self.clone();
        tokio::spawn(async move {
            let mut pulling: HashSet<ShardId> = HashSet::new();
            loop {
                if let Ok(topology) = executor.cfg.topology.read() {
                    executor.remote.set_shards(topology.shards);
                }
                for shard in executor.remote.shards() {
                    if pulling.insert(shard) {
                        tokio::spawn(executor.clone().puller(shard));
                    }
                }
                if executor.draining.load(Ordering::SeqCst) {
                    return;
                }
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        });
    }

    /// Use `key` as the data plane's key (instead of `AGENTD_DP_KEY`).
    pub fn set_dp_key(&self, key: everruns_contracts::dataplane_seal::Key) {
        *self.dp_key.lock() = Some(key);
    }

    /// `org`'s secret `secret_id`, opened in memory. Never stored or logged.
    pub async fn secret(&self, org: &str, secret_id: &str) -> anyhow::Result<Vec<u8>> {
        self.open_secret(org, secret_id)
            .await?
            .map(|s| s.plain)
            .ok_or_else(|| anyhow::anyhow!("org {org} has no secret {secret_id}"))
    }

    /// [`secret`](Self::secret) with its revision and the org's data key,
    /// to write its next revision (`mcp.rs`).
    pub async fn open_secret(
        &self,
        org: &str,
        secret_id: &str,
    ) -> anyhow::Result<Option<crate::mcp::OpenedSecret>> {
        let dp_key = self
            .dp_key
            .lock()
            .ok_or_else(|| anyhow::anyhow!("no data plane key (AGENTD_DP_KEY)"))?;
        let directory = self.cfg.topology.read()?.directory_shard;
        let found: Option<crate::gateway_dp::SecretForExecutor> = self
            .remote
            .post(
                directory,
                &format!("/shard/orgs/{org}/secrets/{secret_id}"),
                &serde_json::json!({}),
                Duration::from_secs(5),
            )
            .await?;
        let Some(found) = found else {
            return Ok(None);
        };
        let data_key = everruns_contracts::dataplane_seal::unwrap_data_key(
            &dp_key,
            org,
            &found.data_key.data_key_id,
            &found.data_key.wrapped,
        )?;
        let plain = found.open(org, &dp_key)?;
        // A pushed secret may name where its value comes from instead.
        let plain = crate::secret_sources::open(plain, org, secret_id).await?;
        Ok(Some(crate::mcp::OpenedSecret {
            revision: found.secret.revision,
            plain,
            data_key,
            data_key_id: found.data_key.data_key_id,
        }))
    }

    /// `org`'s data key from the directory shard, unwrapped in memory;
    /// `None` on a standalone shard, which has no data plane key
    /// (`AGENTD_DP_KEY`) because no control plane wraps keys for it.
    pub async fn data_key(
        &self,
        org: &str,
    ) -> anyhow::Result<Option<(everruns_contracts::dataplane_seal::Key, String)>> {
        let Some(dp_key) = *self.dp_key.lock() else {
            return Ok(None);
        };
        let directory = self.cfg.topology.read()?.directory_shard;
        let wrapped: Option<crate::control::WrappedKey> = self
            .remote
            .post(
                directory,
                &format!("/shard/orgs/{org}/data-key"),
                &serde_json::json!({}),
                Duration::from_secs(5),
            )
            .await?;
        let wrapped = wrapped.ok_or_else(|| anyhow::anyhow!("org {org} has no data key"))?;
        let key = everruns_contracts::dataplane_seal::unwrap_data_key(
            &dp_key,
            org,
            &wrapped.data_key_id,
            &wrapped.wrapped,
        )?;
        Ok(Some((key, wrapped.data_key_id)))
    }

    /// Lease or write `org`'s secret in the directory shard.
    pub async fn secret_op(
        &self,
        org: &str,
        op: crate::secrets::SecretOp,
    ) -> anyhow::Result<crate::secrets::SecretReply> {
        let directory = self.cfg.topology.read()?.directory_shard;
        let command = crate::state::Command::Secret {
            org: org.to_owned(),
            request: op,
            now_ms: crate::node::now_ms(),
        };
        let proposed: crate::node::Proposed = self
            .remote
            .post(
                directory,
                "/shard/propose",
                &command,
                Duration::from_secs(5),
            )
            .await?;
        match proposed.response {
            crate::state::Response::Value { value } => Ok(serde_json::from_value(value)?),
            other => anyhow::bail!("the directory shard answered {other:?}"),
        }
    }

    /// `org`'s connection `id`, from the directory shard.
    pub async fn connection(
        &self,
        org: &str,
        id: &str,
    ) -> anyhow::Result<Option<everruns_contracts::dataplane::Connection>> {
        let directory = self.cfg.topology.read()?.directory_shard;
        self.remote
            .post(
                directory,
                &format!("/shard/orgs/{org}/connections/{id}"),
                &serde_json::json!({}),
                Duration::from_secs(5),
            )
            .await
    }

    pub fn running(&self) -> usize {
        self.running.lock().len()
    }

    /// Stop taking work and hand running epochs back at their next step.
    pub fn drain(&self) {
        self.draining.store(true, Ordering::SeqCst);
        let running: Vec<SessionId> = self.running.lock().keys().copied().collect();
        for session in running {
            self.hosting.changed(session, true);
        }
    }

    /// Credits to offer one shard: an even share of the free slots, no
    /// more than the turns memory has room for.
    fn take_credits(&self) -> usize {
        let shards = self.remote.shards().len().max(1);
        let share = self.cfg.slots.div_ceil(shards).max(1);
        let (used, limit) = crate::memwatch::reading(self.cfg.memory_limit);
        let room = crate::memwatch::turn_credits(used, limit);
        let mut free = self.free.load(Ordering::SeqCst);
        loop {
            let take = free.min(share).min(room);
            if take == 0 {
                return 0;
            }
            match self
                .free
                .compare_exchange(free, free - take, Ordering::SeqCst, Ordering::SeqCst)
            {
                Ok(_) => return take,
                Err(now) => free = now,
            }
        }
    }

    /// Whether memory reached the restart level.
    pub fn wants_restart(&self) -> bool {
        self.restart.load(Ordering::SeqCst)
    }

    /// Every second: act on this executor's memory level.
    async fn memory_loop(self: Arc<Self>) {
        use crate::memwatch::ExecLevel;
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        loop {
            tick.tick().await;
            let (used, limit) = crate::memwatch::reading(self.cfg.memory_limit);
            let idle = self.recent.lock().len();
            let trim = crate::memwatch::mirror_trim(used, limit, idle);
            if trim > 0 {
                self.forget_coldest(trim);
                self.stats.add(&[("mirror", "trimmed")], trim as u64);
            }
            match crate::memwatch::exec_level(used, limit) {
                ExecLevel::Ok => {}
                ExecLevel::Stop => {
                    let handed = self.hand_back_newest();
                    if handed > 0 {
                        tracing::warn!(
                            used_mb = used >> 20,
                            limit_mb = limit >> 20,
                            handed,
                            "executor memory at 95%: handing its newest turns back"
                        );
                    }
                }
                ExecLevel::Restart => {
                    if !self.restart.swap(true, Ordering::SeqCst) {
                        tracing::error!(
                            used_mb = used >> 20,
                            limit_mb = limit >> 20,
                            "executor memory at 98%: draining to restart"
                        );
                        self.stats.inc(&[("memory", "restart")]);
                        self.drain();
                        self.restart_wanted.notify_one();
                    }
                    return;
                }
            }
        }
    }

    /// Hand back the newest quarter of the running turns (at least one);
    /// returns how many.
    fn hand_back_newest(&self) -> usize {
        let newest: Vec<SessionId> = {
            let running = self.running.lock();
            let shed = self.shed.lock();
            let mut by_age: Vec<_> = running
                .iter()
                .filter(|(s, _)| !shed.contains(*s))
                .map(|(s, at)| (*at, *s))
                .collect();
            by_age.sort_by_key(|(at, _)| std::cmp::Reverse(*at));
            let n = by_age.len().div_ceil(4);
            by_age.into_iter().take(n).map(|(_, s)| s).collect()
        };
        for session in &newest {
            self.shed.lock().insert(*session);
            self.hosting.changed(*session, true);
            self.stats.inc(&[("memory", "handed_back")]);
        }
        newest.len()
    }

    fn give_back(&self, credits: usize) {
        if credits > 0 {
            self.free.fetch_add(credits, Ordering::SeqCst);
            self.freed.notify_waiters();
        }
    }

    async fn tenants_loop(self: Arc<Self>) {
        let mut tick = tokio::time::interval(crate::tenants::POLL);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            if self.draining.load(Ordering::SeqCst) {
                return;
            }
            let Ok(topology) = self.cfg.topology.read() else {
                continue;
            };
            match self
                .remote
                .post::<_, crate::tenants::Tenants>(
                    topology.directory_shard,
                    "/shard/tenants",
                    &serde_json::json!({}),
                    Duration::from_secs(5),
                )
                .await
            {
                Ok(tenants) => {
                    self.tenants.heard(tenants.stopped);
                }
                Err(error) => tracing::debug!(%error, "tenants not read"),
            }
        }
    }

    /// Run the due dreams of the sessions this executor mirrors.
    ///
    /// Decision: a turn that leaves a dream owed runs it from its own timer
    /// (`after_turn` in core's dreaming host); this sweep is what a v1
    /// worker's `run_due` is, for a dream that timer did not finish (a
    /// retry, a restart, a budget that reopened). A session's dreaming
    /// state is its own (`session_records.rs`), so the sweep reads the due
    /// time from the sessions it mirrors. Every executor may sweep the same
    /// session: the dream's claim is a compare-exchange, so one runs it.
    async fn dreams_loop(self: Arc<Self>) {
        let mut tick = tokio::time::interval(self.cfg.dream_sweep);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            if self.draining.load(Ordering::SeqCst) {
                return;
            }
            let now = everruns_contracts::rt::utc_now().timestamp();
            let due: Vec<SessionId> = self
                .remote
                .shared
                .state
                .read()
                .sessions
                .iter()
                .filter(|(id, s)| s.platform.dream_due(**id, now))
                .map(|(id, _)| *id)
                .take(32)
                .collect();
            for session in due {
                match self.engine.maintain_dreaming(session).await {
                    Ok(ran) => self
                        .stats
                        .inc(&[("event", if ran { "dreamed" } else { "dream_waited" })]),
                    Err(error) => {
                        tracing::warn!(%session, %error, "a due dream did not run");
                    }
                }
            }
        }
    }

    /// Release the leased provider resources of the sessions this executor
    /// mirrors once their leases are due (`lease_sweep.rs`).
    async fn leases_loop(self: Arc<Self>) {
        let mut tick = tokio::time::interval(crate::lease_sweep::EVERY);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            if self.draining.load(Ordering::SeqCst) {
                return;
            }
            let now = everruns_contracts::rt::utc_now();
            let due: Vec<_> = self
                .remote
                .shared
                .state
                .read()
                .sessions
                .iter()
                .flat_map(|(id, s)| {
                    s.platform
                        .records
                        .due_leases(now, crate::lease_sweep::STALE_AFTER)
                        .map(|lease| (*id, lease.clone()))
                        .collect::<Vec<_>>()
                })
                .take(crate::lease_sweep::BATCH)
                .collect();
            if due.is_empty() {
                continue;
            }
            let pass = crate::lease_sweep::sweep(&self.hosting, due).await;
            for (event, n) in [
                ("lease_released", pass.released),
                ("lease_failed", pass.failed),
            ] {
                for _ in 0..n {
                    self.stats.inc(&[("event", event)]);
                }
            }
        }
    }

    async fn puller(self: Arc<Self>, shard: ShardId) {
        loop {
            if self.draining.load(Ordering::SeqCst) {
                return;
            }
            if !self.tenants.is_fresh() {
                // No word from the directory: start nothing new.
                tokio::time::sleep(Duration::from_millis(500)).await;
                continue;
            }
            let credits = self.take_credits();
            if credits == 0 {
                let _ =
                    tokio::time::timeout(Duration::from_millis(500), self.freed.notified()).await;
                continue;
            }
            match self.remote.pull(shard, credits, self.cfg.pull_wait).await {
                Ok(sessions) => {
                    let mut used = 0;
                    for (session, tenant) in sessions.into_iter().take(credits) {
                        if self.tenants.admit(tenant.as_deref()) != crate::tenants::Admit::Yes {
                            self.stats.inc(&[("event", "refused")]);
                            continue;
                        }
                        used += 1;
                        self.stats.inc(&[("event", "pulled")]);
                        self.clone().run(shard, session);
                    }
                    self.give_back(credits - used);
                }
                Err(error) => {
                    self.give_back(credits);
                    tracing::debug!(shard, %error, "pull failed");
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
            }
        }
    }

    /// Run `session`'s activations here until it owes no more work. Holds
    /// one slot.
    fn run(self: Arc<Self>, shard: ShardId, session: SessionId) {
        {
            let mut running = self.running.lock();
            if running.contains_key(&session) {
                drop(running);
                self.give_back(1);
                return;
            }
            running.insert(session, std::time::Instant::now());
        }
        self.recent.lock().retain(|s| *s != session);
        if self.remote.bind(session, shard) {
            // Moved since it last ran here: the writes this hosting waits
            // to see were at positions in the old shard's log.
            self.hosting.forget(session);
        }
        let watching = Arc::new(AtomicBool::new(true));
        tokio::spawn(self.clone().watch(session, watching.clone()));
        tokio::spawn(async move {
            for _ in 0..64 {
                let started = std::time::Instant::now();
                // Its own task: an activation that panics ends as an error
                // here, and its slot and claim go back, instead of the
                // slot staying held for good.
                let engine = self.clone();
                let activation = tokio::spawn(async move { engine.engine.activate(session).await });
                let outcome = match activation.await {
                    Ok(Ok(())) => "completed",
                    Ok(Err(error)) => {
                        tracing::warn!(%session, %error, "activation stopped");
                        "error"
                    }
                    Err(error) => {
                        tracing::error!(%session, %error, "activation panicked");
                        "panicked"
                    }
                };
                self.stats.inc(&[("activation", outcome)]);
                self.stats.add(
                    &[("activation_ms", "sum")],
                    started.elapsed().as_millis() as u64,
                );
                if outcome != "completed"
                    || self.draining.load(Ordering::SeqCst)
                    || self.shed.lock().contains(&session)
                {
                    break;
                }
                if self.remote.refresh(session, 0).await.is_err() {
                    break;
                }
                let again = self
                    .remote
                    .shared
                    .state
                    .read()
                    .sessions
                    .get(&session)
                    .is_some_and(|s| {
                        s.placement == Placement::Live
                            && s.authority.holder.is_none()
                            && s.authority.owed(now_ms())
                    });
                if !again {
                    break;
                }
            }
            watching.store(false, Ordering::SeqCst);
            self.running.lock().remove(&session);
            self.shed.lock().remove(&session);
            self.retire(session);
            self.give_back(1);
        });
    }

    /// Keep `session`'s mirror current while it runs: input, cancels and a
    /// freeze for a move reach Engine's waiters and hand-off flag.
    async fn watch(self: Arc<Self>, session: SessionId, watching: Arc<AtomicBool>) {
        // A change the watch folds in announces itself (`Remote::absorb`).
        while watching.load(Ordering::SeqCst) {
            if self
                .remote
                .watch(session, Duration::from_secs(10))
                .await
                .is_err()
            {
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        }
    }

    /// Keep `session` mirrored for a while; evict the least recently run
    /// beyond the bound.
    fn retire(&self, session: SessionId) {
        let evict: Vec<SessionId> = {
            let mut recent = self.recent.lock();
            recent.push_back(session);
            let running = self.running.lock();
            let mut out = Vec::new();
            while recent.len() > self.cfg.mirror_sessions {
                if let Some(oldest) = recent.pop_front()
                    && !running.contains_key(&oldest)
                {
                    out.push(oldest);
                }
            }
            out
        };
        self.forget_all(evict);
    }

    /// Drop the `n` least recently run idle mirrors (memory, see
    /// `memwatch::mirror_trim`).
    fn forget_coldest(&self, n: usize) {
        let evict: Vec<SessionId> = {
            let mut recent = self.recent.lock();
            let running = self.running.lock();
            let mut out = Vec::new();
            while out.len() < n {
                let Some(oldest) = recent.pop_front() else {
                    break;
                };
                if !running.contains_key(&oldest) {
                    out.push(oldest);
                }
            }
            out
        };
        self.forget_all(evict);
    }

    fn forget_all(&self, sessions: Vec<SessionId>) {
        for session in sessions {
            self.remote.evict(session);
            self.hosting.forget(session);
        }
    }
}

/// The data plane's key from `AGENTD_DP_KEY` (base64, 32 bytes); none when
/// unset. A key that is set but malformed stops the executor at start.
pub fn data_plane_key() -> anyhow::Result<Option<everruns_contracts::dataplane_seal::Key>> {
    match std::env::var("AGENTD_DP_KEY") {
        Ok(key) if !key.is_empty() => Ok(Some(
            everruns_contracts::dataplane_seal::key_from_base64(&key)
                .map_err(|e| anyhow::anyhow!("AGENTD_DP_KEY: {e}"))?,
        )),
        _ => Ok(None),
    }
}

/// `/health` and `/metrics` of an executor.
pub fn router(executor: Arc<Executor>) -> axum::Router {
    use axum::routing::get;
    axum::Router::new()
        .route("/health", get(|| async { "ok" }))
        .route(
            "/metrics",
            get(
                |axum::extract::State(executor): axum::extract::State<Arc<Executor>>| async move {
                    let id = executor.cfg.id.to_string();
                    let mut w = crate::prom::Writer::new(&[("executor", &id)]);
                    w.gauge(
                        "executor_running_turns",
                        "Turns running here.",
                        executor.running() as f64,
                    );
                    w.gauge(
                        "executor_free_slots",
                        "Slots not running or offered.",
                        executor.free.load(Ordering::SeqCst) as f64,
                    );
                    let (used, limit) = crate::memwatch::reading(executor.cfg.memory_limit);
                    w.gauge(
                        "agentd_memory_container_bytes",
                        "Memory in use as the kernel counts it (the resident set outside a container).",
                        used as f64,
                    );
                    w.gauge("agentd_memory_limit_bytes", "Memory limit turns are admitted against (0: none).", limit as f64);
                    w.gauge(
                        "agentd_memory_level",
                        "0: normal, 3: starting nothing and handing the newest turns back, 4: draining to restart.",
                        match crate::memwatch::exec_level(used, limit) {
                            crate::memwatch::ExecLevel::Ok => 0.0,
                            crate::memwatch::ExecLevel::Stop => 3.0,
                            crate::memwatch::ExecLevel::Restart => 4.0,
                        },
                    );
                    w.gauge(
                        "executor_mirrored_sessions",
                        "Sessions in the mirror.",
                        executor.remote.mirrored() as f64,
                    );
                    let (hits, misses) = executor.cfg.store.cache_stats();
                    w.counter(
                        "executor_cache_hits_total",
                        "Segment and blob cache hits.",
                        hits as f64,
                    );
                    w.counter(
                        "executor_cache_misses_total",
                        "Segment and blob cache misses (object store reads).",
                        misses as f64,
                    );
                    w.counters(
                        "executor_shard_calls_total",
                        "Calls to shards, by kind.",
                        &executor.remote.calls,
                    );
                    w.counters(
                        "executor_events_total",
                        "Pulls and activations.",
                        &executor.stats,
                    );
                    w.histograms(
                        "executor_mcp_credential_seconds",
                        "Time to find an MCP call's credentials, by kind.",
                        &executor.mcp.resolve,
                    );
                    w.counters(
                        "executor_mcp_refreshes_total",
                        "MCP OAuth token refreshes, by outcome.",
                        &executor.mcp.refreshes,
                    );
                    w.histograms(
                        "executor_sandbox_sync_seconds",
                        "Time to sync a session's workspace with its sandbox, by direction and kind.",
                        &executor.sandboxes.sync,
                    );
                    w.counters(
                        "executor_sandbox_sync_files_total",
                        "Files a workspace sync copied, by direction and op.",
                        &executor.sandboxes.files,
                    );
                    w.counters(
                        "executor_sandbox_orphans_total",
                        "Orphaned sandboxes removed before a create, by provider.",
                        &executor.sandboxes.orphans,
                    );
                    w.out
                },
            ),
        )
        .with_state(executor)
}

#[async_trait::async_trait]
impl crate::providers::OrgMaterial for Executor {
    fn tenant(&self, session: SessionId) -> Option<String> {
        self.remote
            .shared
            .state
            .read()
            .sessions
            .get(&session)
            .and_then(|s| s.tenant.clone())
    }

    fn scopes(&self, session: SessionId) -> Vec<String> {
        let mut scopes = vec![crate::install::session_scope(&session.to_string())];
        if let Some(agent) = self
            .remote
            .shared
            .state
            .read()
            .sessions
            .get(&session)
            .and_then(|s| s.labels.get(crate::install::AGENT_LABEL).cloned())
        {
            scopes.push(crate::install::agent_scope(&agent));
        }
        scopes
    }

    async fn connection(
        &self,
        org: &str,
        id: &str,
    ) -> anyhow::Result<Option<everruns_contracts::dataplane::Connection>> {
        // A group's own connection first, then its install's (`install.rs`).
        for org in crate::install::lookup_orgs(org) {
            if let Some(found) = Executor::connection(self, &org, id).await? {
                return Ok(Some(found));
            }
        }
        Ok(None)
    }

    async fn secret(&self, org: &str, secret_id: &str) -> anyhow::Result<Vec<u8>> {
        let orgs = crate::install::lookup_orgs(org);
        for org in &orgs {
            if let Some(opened) = Executor::open_secret(self, org, secret_id).await? {
                return Ok(opened.plain);
            }
        }
        anyhow::bail!("org {org} has no secret {secret_id}")
    }

    async fn open_secret(
        &self,
        org: &str,
        secret_id: &str,
    ) -> anyhow::Result<Option<crate::mcp::OpenedSecret>> {
        Executor::open_secret(self, org, secret_id).await
    }

    async fn secret_op(
        &self,
        org: &str,
        op: crate::secrets::SecretOp,
    ) -> anyhow::Result<crate::secrets::SecretReply> {
        Executor::secret_op(self, org, op).await
    }

    fn seals(&self) -> bool {
        self.dp_key.lock().is_some()
    }

    fn data_plane_key(&self) -> Option<everruns_contracts::dataplane_seal::Key> {
        *self.dp_key.lock()
    }

    fn mcp(&self) -> Option<Arc<crate::mcp::OrgMcp>> {
        Some(self.mcp.clone())
    }

    fn turn_actor(
        &self,
        session: SessionId,
    ) -> Option<everruns_contracts::runtime::capabilities::injected::SessionActor> {
        let state = self.remote.shared.state.read();
        let s = state.sessions.get(&session)?;
        let message = s.authority.open.and(s.authority.open_input)?;
        s.platform.senders.of(message).map(|sub| {
            everruns_contracts::runtime::capabilities::injected::SessionActor {
                sub: sub.to_owned(),
                turn: Some(message.uuid().to_string()),
            }
        })
    }

    fn message_actor(
        &self,
        session: SessionId,
        message: everruns_contracts::typed_id::MessageId,
    ) -> Option<everruns_contracts::runtime::capabilities::injected::SessionActor> {
        let state = self.remote.shared.state.read();
        let s = state.sessions.get(&session)?;
        s.platform.senders.of(message).map(|sub| {
            everruns_contracts::runtime::capabilities::injected::SessionActor {
                sub: sub.to_owned(),
                turn: Some(message.uuid().to_string()),
            }
        })
    }

    fn owner(&self, session: SessionId) -> Option<String> {
        let state = self.remote.shared.state.read();
        state.sessions.get(&session)?.platform.owner.clone()
    }

    fn controller_url(&self, session: SessionId, name: &str) -> Option<String> {
        let state = self.remote.shared.state.read();
        state
            .sessions
            .get(&session)?
            .platform
            .servers
            .get(name)
            .cloned()
    }

    async fn data_key(
        &self,
        org: &str,
    ) -> anyhow::Result<Option<(everruns_contracts::dataplane_seal::Key, String)>> {
        Executor::data_key(self, org).await
    }
}

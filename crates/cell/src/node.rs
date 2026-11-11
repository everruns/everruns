//! One node of a cell: a Raft replica, an Everruns Engine executor, and
//! (while it leads) the cell's dispatcher.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::PathBuf;

use futures::StreamExt;
use std::sync::Arc;
use std::time::{Duration, Instant};

use everruns_contracts::typed_id::SessionId;
use everruns_core::events::Event;
use openraft::error::RaftError;
use openraft::{BasicNode, Config, SnapshotPolicy};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use crate::host::Host;
use crate::metrics::Metrics;
use crate::network::Network;
use crate::sm::{Shared, StateMachine};
use crate::state::{CellId, Change, Command, Placed, Response};
use crate::types::{NodeId, Raft};
use crate::wal::Wal;

/// How long a prepared copy waits for its source's move before asking the
/// directory itself.
const PREPARED_GRACE: Duration = Duration::from_secs(2);

/// How a node is started.
#[derive(Debug, Clone)]
pub struct NodeConfig {
    pub cell: CellId,
    pub node_id: NodeId,
    /// Every member of the cell, this node included: id -> `host:port`.
    pub members: BTreeMap<NodeId, String>,
    pub data_dir: PathBuf,
    /// Session history segments and snapshot copies.
    pub store: crate::store::Store,
    /// Unload idle sessions when resident events exceed this many bytes.
    pub memory_budget: u64,
    /// Also unload sessions idle this long (`None`: only under pressure).
    pub unload_idle: Option<Duration>,
    /// How long a claim lives without renewal.
    pub claim_ttl: Duration,
    /// Turns one node runs at once. A session whose turn would exceed it
    /// waits, queued, until a slot frees.
    pub max_turns_per_node: usize,
    /// Entries between snapshots.
    pub snapshot_every: u64,
    /// Entries kept behind each snapshot. A follower further behind than
    /// this catches up by snapshot (a copy of the whole cell).
    pub keep_log: u64,
}

/// What a proposal returned, as sent between nodes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Proposed {
    pub response: Response,
    pub index: u64,
}

/// The runtime turns execute on.
///
/// Decision: turns (model calls, tools, JSON over whole sessions) run on
/// their own threads, apart from Raft and the API. On one shared runtime a
/// burst of turns delayed append handlers past openraft's per-append
/// deadline, so a follower under load stopped catching up.
#[expect(
    clippy::expect_used,
    reason = "a runtime fails to build only when the process cannot start threads, and no turn could run then"
)]
fn turn_runtime() -> &'static tokio::runtime::Handle {
    static RUNTIME: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();
    RUNTIME
        .get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .thread_name("cell-turn")
                .enable_all()
                .build()
                .expect("turn runtime")
        })
        .handle()
}

pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

pub struct Node {
    pub cfg: NodeConfig,
    pub raft: Raft,
    pub shared: Arc<Shared>,
    pub wal: Wal,
    pub http: reqwest::Client,
    /// Ephemeral events from turns this node executes.
    pub live: broadcast::Sender<(SessionId, Arc<Event>)>,
    pub host: Arc<dyn Host>,
    pub metrics: Metrics,
    /// Counters and histograms for `/metrics`.
    pub prom: crate::prom::Prom,
    /// Sessions this node runs an activation of now.
    running: Mutex<HashSet<SessionId>>,
    /// Peers the leader recently reached, for executor placement.
    alive: Mutex<HashMap<NodeId, Instant>>,
    /// Sessions the dispatcher asked an executor to activate, and when;
    /// cleared once the session's claim shows.
    dispatched: Mutex<HashMap<SessionId, Instant>>,
    /// Engine's session authority, backed by this cell.
    pub hosting: crate::hosting::RaftHosting,
    /// The Everruns Engine this node runs sessions with.
    pub engine: everruns::Engine,
    /// Running turns per executor, recounted at most every 100 ms.
    load: Mutex<Option<(Instant, BTreeMap<NodeId, usize>)>>,
    batcher: crate::batch::Batcher,
    /// Reaches other cells, for finishing moves.
    pub gateway: std::sync::OnceLock<Arc<crate::gateway::Gateway>>,
    /// Sessions whose move this leader is finishing.
    moving: Mutex<HashSet<SessionId>>,
    /// Ships the committed log to the object store while this node leads.
    shipper: crate::ship::Shipper,
    /// The cell's members (voters and learners) as Raft last committed
    /// them, refreshed by the liveness loop.
    members: parking_lot::RwLock<BTreeMap<NodeId, String>>,
}

impl Node {
    /// Open storage, start Raft and the background loops.
    pub async fn start(cfg: NodeConfig, host: Arc<dyn Host>) -> anyhow::Result<Arc<Self>> {
        let shared = Shared::new(cfg.store.clone(), cfg.cell);
        let wal = Wal::open(cfg.data_dir.join("wal"))?;
        let (shared_store, ship_wal) = (cfg.store.clone(), wal.clone());
        let sm = StateMachine::open(cfg.data_dir.join("snapshots"), shared.clone())?;
        let config = Config {
            cluster_name: format!("cell-{}", cfg.cell),
            // Decision: elections wait 1 to 2 s. At 0.4 to 0.8 s a follower
            // on a loaded host, catching up by snapshot, timed out between
            // chunks, called an election and cancelled its own catch-up,
            // over and over (75k-session run). Failover takes about a
            // second longer; a cell that can't heal costs far more.
            heartbeat_interval: 150,
            election_timeout_min: 1_000,
            election_timeout_max: 2_000,
            // Raw bytes now (see `network::SnapshotChunk`): fewer, larger
            // chunks.
            snapshot_max_chunk_size: 16 << 20,
            max_payload_entries: 256,
            snapshot_policy: SnapshotPolicy::LogsSinceLast(cfg.snapshot_every),
            // A follower further behind than this gets a snapshot. That
            // costs a copy of the whole cell, so keep enough log for a
            // follower that fell a few seconds behind under load.
            max_in_snapshot_log_to_keep: cfg.keep_log,
            // Shipping a snapshot of tens of thousands of sessions takes
            // longer than openraft's default.
            install_snapshot_timeout: 30_000,
            ..Default::default()
        }
        .validate()?;
        let raft = Raft::new(
            cfg.node_id,
            Arc::new(config),
            Network::new(),
            wal.clone(),
            sm,
        )
        .await?;
        let batcher = crate::batch::Batcher::spawn(raft.clone());
        let http = reqwest::Client::builder()
            .no_proxy() // fleet traffic is direct, whatever the system proxy says
            .pool_max_idle_per_host(256)
            .tcp_nodelay(true)
            .build()?;
        let node = Arc::new_cyclic(|weak: &std::sync::Weak<Node>| {
            let hosting = crate::hosting::RaftHosting::new(weak.clone());
            let hydrator_host = host.clone();
            // Decision: the Engine builds agents from definitions the cell
            // keeps, not a fixed registry: a build is (definition key,
            // definition format), and a definition is whatever a client
            // sent. The cell replicates the definition with the session, so
            // any node, of any deploy, can rebuild it (see
            // `hosting::BUILD_ID`).
            let engine = everruns::Engine::builder()
                .hosting(hosting.clone())
                .hydrator(move |definition: &everruns::hosting::AgentDefinition| {
                    let agent = hydrator_host
                        .agent(&definition.runtime_agent.agent)
                        .map_err(everruns_contracts::error::AgentLoopError::config)?;
                    Ok(everruns::hosting::Hydrated {
                        agent,
                        harness: None,
                    })
                })
                .build();
            Self {
                batcher,
                raft,
                shared,
                wal,
                http,
                live: broadcast::channel(65_536).0,
                host,
                metrics: Metrics::default(),
                prom: Default::default(),
                running: Mutex::default(),
                alive: Mutex::default(),
                dispatched: Mutex::default(),
                hosting,
                engine,
                load: Mutex::default(),
                gateway: std::sync::OnceLock::new(),
                moving: Mutex::default(),
                shipper: crate::ship::Shipper::new(shared_store, cfg.cell, ship_wal),
                members: parking_lot::RwLock::new(cfg.members.clone()),
                cfg,
            }
        });
        let weak = Arc::downgrade(&node);
        let _ = node.shared.on_change.set(Box::new(move |change| {
            let (Change::Session(session) | Change::Event(session, _)) = change;
            let Some(node) = weak.upgrade() else { return };
            let frozen = matches!(change, Change::Session(_))
                && node
                    .shared
                    .state
                    .read()
                    .sessions
                    .get(session)
                    .is_some_and(|s| s.placement == crate::state::Placement::Frozen);
            node.hosting.changed(*session, frozen);
        }));
        Ok(node)
    }

    /// Form the cell if it has never been initialized. Every member calls
    /// this with the same membership, which Raft allows.
    pub async fn bootstrap(&self) -> anyhow::Result<()> {
        if self.raft.is_initialized().await? {
            return Ok(());
        }
        let members: BTreeMap<NodeId, BasicNode> = self
            .cfg
            .members
            .iter()
            .map(|(id, addr)| (*id, BasicNode { addr: addr.clone() }))
            .collect();
        match self.raft.initialize(members).await {
            Ok(()) => Ok(()),
            Err(RaftError::APIError(_)) => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

    pub fn spawn_loops(self: &Arc<Self>) {
        tokio::spawn(self.clone().ship_loop());
        tokio::spawn(self.clone().scheduler_loop());
        tokio::spawn(self.clone().liveness_loop());
        tokio::spawn(self.clone().unload_loop());
        tokio::spawn(self.clone().register_loop());
        tokio::spawn(self.clone().rebalance_loop());
    }

    /// While this node leads: ship the committed log to the object store
    /// (see `ship`).
    async fn ship_loop(self: Arc<Self>) {
        let mut tick = tokio::time::interval(crate::ship::SHIP_EVERY);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            if !self.is_leader() {
                self.shipper.reset();
                continue;
            }
            let applied = *self.shared.applied.borrow();
            self.shipper.observe(applied);
            if let Err(error) = self.shipper.ship(applied).await {
                tracing::warn!(cell = self.cfg.cell, %error, "log shipping failed; retrying");
            }
        }
    }

    /// On the directory cell's leader: move work from the busiest cell to
    /// the least busy one, a few sessions at a time.
    ///
    /// Decision: placement alone cannot balance. A session placed today may
    /// run a turn for days, and a cell added later starts empty while the
    /// old ones keep every long turn they hold. So running turns move too:
    /// the source cell parks a turn at its next step boundary and the
    /// target resumes it from the log (see `maybe_move`). Moves are bounded
    /// per round and by the source's moves in flight, and need a clear gap
    /// (4 turns, and a fifth of the busiest cell's work),
    /// so the fleet does not churn sessions back and forth.
    async fn rebalance_loop(self: Arc<Self>) {
        let mut tick = tokio::time::interval(Duration::from_secs(5));
        loop {
            tick.tick().await;
            let Some(gateway) = self.gateway.get() else {
                continue;
            };
            if !self.is_leader() || gateway.directory_cell().ok() != Some(self.cfg.cell) {
                continue;
            }
            let all = gateway.loads();
            let draining = gateway.draining();
            // Turn slots the cell's work costs, rounded up (see `CellLoad::work`).
            let work = |l: &crate::api::CellLoad| l.work().ceil() as u64;
            // Cells that take sessions; draining ones only give.
            let loads: BTreeMap<CellId, crate::api::CellLoad> = all
                .iter()
                .filter(|(cell, _)| !draining.contains(cell))
                .map(|(cell, load)| (*cell, *load))
                .collect();
            let mut plan: Vec<(&str, CellId, CellId, u64)> = Vec::new();
            // Draining cells empty out first, busy and idle sessions alike,
            // spread over the cells that take sessions, least loaded first.
            let mut targets: Vec<(CellId, f64)> =
                loads.iter().map(|(c, l)| (*c, l.score())).collect();
            targets.sort_by(|a, b| a.1.total_cmp(&b.1));
            if !targets.is_empty() {
                for (&cell, load) in all
                    .iter()
                    .filter(|(c, l)| draining.contains(c) && l.sessions > 0)
                {
                    // Every running turn may go at once: each parks at its
                    // own next step, so the moves spread out by themselves.
                    // A drain moves every turn, waiting ones too.
                    let busy = load.active + load.queued;
                    let idle = 128u64.saturating_sub(load.moving);
                    let k = targets.len() as u64;
                    for (i, (target, _)) in targets.iter().enumerate() {
                        let share = |n: u64| n / k + u64::from((i as u64) < n % k);
                        plan.push(("drain", cell, *target, share(busy)));
                        plan.push(("drain-idle", cell, *target, share(idle)));
                    }
                }
            }
            let hottest = loads.iter().max_by_key(|(_, l)| work(l));
            let coldest = loads.iter().min_by_key(|(_, l)| work(l));
            if let (true, Some((&hot, hot_load)), Some((&cold, cold_load))) =
                (loads.len() >= 2 && plan.is_empty(), hottest, coldest)
            {
                // A session still moving out counts as the cold cell's
                // already, or every round would move the same gap again.
                let gap = work(hot_load)
                    .saturating_sub(work(cold_load))
                    .saturating_sub(2 * hot_load.moving);
                if hot != cold && gap >= 4.max(work(hot_load) / 5) {
                    // Moves in flight out of one cell: 8, or a quarter of
                    // its work when that is more (each waits up to a step
                    // to park).
                    let cap = 8.max(work(hot_load) / 4);
                    let n = (gap / 2).min(cap.saturating_sub(hot_load.moving));
                    plan.push(("busy", hot, cold, n));
                } else {
                    // Idle sessions are future work: even out their count.
                    let biggest = loads.iter().max_by_key(|(_, l)| l.sessions);
                    let smallest = loads.iter().min_by_key(|(_, l)| l.sessions);
                    if let (Some((&big, big_load)), Some((&small, small_load))) =
                        (biggest, smallest)
                    {
                        let gap = big_load
                            .sessions
                            .saturating_sub(small_load.sessions)
                            .saturating_sub(2 * big_load.moving);
                        if big != small && gap > 50 && gap > big_load.sessions / 5 {
                            let n = (gap / 2).min(32u64.saturating_sub(big_load.moving));
                            plan.push(("idle", big, small, n));
                        }
                    }
                }
            }
            for (kind, from, to, n) in plan {
                if n > 0 {
                    self.start_moves(gateway, kind, from, to, n).await;
                }
            }
            let takers: Vec<CellId> = loads.keys().copied().collect();
            self.rebalance_shards(gateway, &takers, &draining).await;
        }
    }

    /// Directory leader: spread the directory's shards evenly over the
    /// cells that take sessions, and empty draining cells of theirs. A few
    /// shards per round; each hand-over seals the shard in its old cell,
    /// copies its entries to the new one, then reassigns it here.
    async fn rebalance_shards(
        &self,
        gateway: &crate::gateway::Gateway,
        takers: &[CellId],
        draining: &std::collections::BTreeSet<CellId>,
    ) {
        if takers.is_empty() {
            return;
        }
        let directory = self.cfg.cell;
        let assigned = self.shared.state.read().shards.clone();
        let owner = |shard: u16| assigned.get(&shard).copied().unwrap_or(directory);
        let mut held: BTreeMap<CellId, Vec<u16>> = takers.iter().map(|c| (*c, vec![])).collect();
        let mut leaving = Vec::new();
        for shard in 0..crate::state::SHARDS {
            let cell = owner(shard);
            match held.get_mut(&cell) {
                Some(shards) if !draining.contains(&cell) => shards.push(shard),
                _ => leaving.push(shard),
            }
        }
        let mut moves = Vec::new();
        for _ in 0..32 {
            // `takers` is not empty, so `held` never is.
            let Some((&small, small_shards)) = held.iter().min_by_key(|(_, s)| s.len()) else {
                break;
            };
            let small_len = small_shards.len();
            let shard = match leaving.pop() {
                Some(shard) => shard,
                None => {
                    let Some((&big, shards)) = held.iter().max_by_key(|(_, s)| s.len()) else {
                        break;
                    };
                    if shards.len() <= small_len + 1 {
                        break;
                    }
                    let Some(shard) = held.get_mut(&big).and_then(Vec::pop) else {
                        break;
                    };
                    shard
                }
            };
            if let Some(shards) = held.get_mut(&small) {
                shards.push(shard);
            }
            moves.push((shard, owner(shard), small));
        }
        for (shard, from, to) in moves {
            let path = format!("/cell/shards/{shard}/handoff?to={to}");
            let copied: Result<usize, _> = gateway
                .call(from, &path, Some(&serde_json::json!({})))
                .await;
            if let Err((_, error)) = copied {
                tracing::debug!(shard, from, to, %error, "shard hand-over failed");
                return;
            }
            if let Err(error) = self.propose(Command::AssignShard { shard, to }).await {
                tracing::debug!(shard, %error, "shard assignment failed");
                return;
            }
            // Gateways learn the new owner within a registry refresh; until
            // then the old one still answers lookups (a stale answer leads
            // to a forward). Then it forgets the entries.
            if let Some(gw) = self.gateway.get().cloned() {
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_secs(10)).await;
                    let _ = gw.propose(from, &Command::DropShard { shard }).await;
                });
            }
            tracing::info!(shard, from, to, "directory shard moved");
        }
        gateway.refresh_registry().await;
    }

    /// Freeze up to `n` of `from`'s movable sessions for a move to `to`.
    /// Fire and forget: the source's leader finishes each move.
    async fn start_moves(
        &self,
        gateway: &crate::gateway::Gateway,
        kind: &str,
        from: CellId,
        to: CellId,
        n: u64,
    ) {
        let which = if kind.ends_with("idle") {
            "idle"
        } else {
            "busy"
        };
        let sessions = match gateway.movable(from, n as usize, which).await {
            Ok(sessions) => sessions,
            Err((_, error)) => {
                tracing::debug!(%error, "rebalance: no movable sessions");
                return;
            }
        };
        if sessions.is_empty() {
            return;
        }
        tracing::info!(from, to, kind, count = sessions.len(), "rebalancing");
        for session in sessions {
            let command = Command::Freeze {
                session,
                to: Some(to),
            };
            match gateway.propose(from, &command).await {
                Ok(Response::Ok) => gateway.rebalanced.inc(&[("kind", kind)]),
                Ok(other) => tracing::debug!(%session, ?other, "rebalance freeze refused"),
                Err((_, error)) => tracing::debug!(%session, %error, "rebalance freeze failed"),
            }
        }
    }

    /// While this node leads, keep the cell's entry in the directory's cell
    /// registry equal to its Raft membership: that is how a new cell joins
    /// the fleet and how gateways learn a cell's nodes changed.
    async fn register_loop(self: Arc<Self>) {
        let mut registered: Option<BTreeMap<NodeId, String>> = None;
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        loop {
            tick.tick().await;
            if !self.is_leader() {
                registered = None;
                continue;
            }
            let Some(gateway) = self.gateway.get() else {
                continue;
            };
            let members = self.members();
            if registered.as_ref() == Some(&members) {
                continue;
            }
            let Ok(directory) = gateway.directory_cell() else {
                continue;
            };
            let command = Command::RegisterCell {
                cell: self.cfg.cell,
                members: members.clone().into_iter().collect(),
            };
            match gateway.propose(directory, &command).await {
                Ok(_) => {
                    tracing::info!(cell = self.cfg.cell, ?members, "cell registered");
                    registered = Some(members);
                }
                Err((_, error)) => tracing::debug!(%error, "cell registration failed"),
            }
        }
    }

    /// Unload idle sessions, coldest first, while resident events exceed the
    /// memory budget (down to 80% of it), and any idle past `unload_idle`.
    ///
    /// Decision: unloading is a replica-local choice, not a Raft command. It
    /// changes where events live (memory or the object store), never what
    /// they are, so replicas may differ in what they hold resident and still
    /// apply every command identically. Writing segments is not: the leader
    /// writes them and records each with `Stored`, so a replica drops only
    /// events the cell's one copy in the store already holds.
    async fn unload_loop(self: Arc<Self>) {
        let mut tick = tokio::time::interval(Duration::from_millis(500));
        let mut last_snapshot = Instant::now();
        // Return freed pages to the OS after unloading or purging.
        let mut trim = false;
        loop {
            tick.tick().await;
            if std::mem::take(&mut trim) {
                release_free_pages();
            }
            // Decision: the Raft log keeps every entry in memory until a
            // snapshot lets it purge them, and entries carry the events. So
            // the log counts against the memory budget too: past a quarter
            // of it, take a snapshot and purge.
            //
            // A snapshot copies the whole cell state, so with many sessions
            // it is not cheap: wait at least ten times as long as the last
            // one took, or snapshots alone would eat the node.
            let cost = Duration::from_millis(
                self.shared
                    .snapshot_ms
                    .load(std::sync::atomic::Ordering::Relaxed)
                    * 10,
            );
            if self.wal.held_bytes() > self.cfg.memory_budget / 4
                && last_snapshot.elapsed() > cost.max(Duration::from_secs(5))
            {
                last_snapshot = Instant::now();
                let _ = self.raft.trigger().snapshot().await;
                trim = true;
            }
            self.shared.sweep_watchers();
            if self.is_leader() {
                self.store_events().await;
            }
            let mut sessions = Vec::new();
            let (bytes, events) = self.shared.resident();
            let budget = self.cfg.memory_budget;
            if bytes > budget {
                let per_event = (bytes / events.max(1) as u64).max(1);
                let mut to_free = bytes.saturating_sub(budget * 8 / 10) / per_event;
                let resident: HashMap<SessionId, usize> = {
                    let state = self.shared.state.read();
                    state
                        .sessions
                        .iter()
                        .map(|(id, s)| (*id, s.events.len()))
                        .collect()
                };
                for id in self.shared.unload_candidates() {
                    if to_free == 0 {
                        break;
                    }
                    to_free =
                        to_free.saturating_sub(resident.get(&id).copied().unwrap_or(0) as u64);
                    sessions.push(id);
                }
            }
            if let Some(age) = self.cfg.unload_idle {
                sessions.extend(self.shared.idle_longer_than(age));
            }
            let mut freed: usize = sessions.into_iter().map(|id| self.shared.unload(id)).sum();
            // Still over: trim the stored history of busy sessions too.
            if self.shared.resident().0 > budget {
                for (id, _) in self.shared.largest_resident() {
                    freed += self.shared.trim(id);
                    if self.shared.resident().0 <= budget * 8 / 10 {
                        break;
                    }
                }
            }
            trim |= freed > 0;
            self.metrics
                .unloaded_events
                .fetch_add(freed as u64, std::sync::atomic::Ordering::Relaxed);
            tracing::debug!(freed, "unloaded idle sessions");
        }
    }

    /// Leader only: store sessions' unstored events in the object store and
    /// record the segments with `Stored`, so every replica can unload them.
    /// A compaction (one segment for a trailing run) then deletes the pieces
    /// it replaces.
    async fn store_events(&self) {
        let stored = self.shared.flush(512).await;
        if stored.is_empty() {
            return;
        }
        let compacted: Vec<_> = stored
            .iter()
            .filter(|(_, _, compact)| *compact)
            .map(|(session, seg, _)| (*session, *seg))
            .collect();
        let commands = stored
            .into_iter()
            .map(|(session, seg, _)| Command::Stored { session, seg })
            .collect();
        if let Err(error) = self.propose(Command::Batch { commands }).await {
            tracing::debug!(%error, "recording stored segments failed");
            return;
        }
        let store = self.shared.store.clone();
        tokio::spawn(async move {
            futures::stream::iter(compacted)
                .for_each_concurrent(16, |(session, seg)| {
                    let store = store.clone();
                    async move { store.prune_segments(session, seg).await }
                })
                .await;
        });
    }

    /// Decision: the cell's membership lives in Raft, not in the topology
    /// file. The file seeds the first membership; nodes join and leave with
    /// membership changes (`add_member`, `remove_member`), and every replica
    /// learns the result from the log.
    pub fn members(&self) -> BTreeMap<NodeId, String> {
        self.members.read().clone()
    }

    fn member_addr(&self, id: NodeId) -> Option<String> {
        self.members.read().get(&id).cloned()
    }

    fn refresh_members(&self) {
        let metrics = self.raft.metrics().borrow().clone();
        let nodes: BTreeMap<NodeId, String> = metrics
            .membership_config
            .nodes()
            .map(|(id, node)| (*id, node.addr.clone()))
            .collect();
        if !nodes.is_empty() {
            *self.members.write() = nodes;
        }
    }

    /// Add `id` at `addr` to this cell: catch it up as a learner, then make
    /// it a voter. Run on the leader.
    pub async fn add_member(&self, id: NodeId, addr: String) -> anyhow::Result<()> {
        self.raft
            .add_learner(id, BasicNode { addr }, true)
            .await
            .map_err(|e| anyhow::anyhow!("add learner: {e}"))?;
        let mut voters: BTreeSet<NodeId> = self
            .raft
            .metrics()
            .borrow()
            .membership_config
            .membership()
            .voter_ids()
            .collect();
        voters.insert(id);
        self.raft
            .change_membership(voters, false)
            .await
            .map_err(|e| anyhow::anyhow!("change membership: {e}"))?;
        self.refresh_members();
        Ok(())
    }

    /// Remove `id` from this cell. Its turns move once it stops answering.
    /// Run on the leader.
    pub async fn remove_member(&self, id: NodeId) -> anyhow::Result<()> {
        let voters: BTreeSet<NodeId> = self
            .raft
            .metrics()
            .borrow()
            .membership_config
            .membership()
            .voter_ids()
            .filter(|v| *v != id)
            .collect();
        if voters.is_empty() {
            anyhow::bail!("a cell keeps at least one voter");
        }
        self.raft
            .change_membership(voters, false)
            .await
            .map_err(|e| anyhow::anyhow!("change membership: {e}"))?;
        self.refresh_members();
        Ok(())
    }

    pub fn leader(&self) -> Option<NodeId> {
        self.raft.metrics().borrow().current_leader
    }

    pub fn is_leader(&self) -> bool {
        self.leader() == Some(self.cfg.node_id)
    }

    /// Commit `command` through the leader, wherever it is, and return the
    /// state machine's answer with the entry's index.
    ///
    /// Retries across leader changes. A retried command may already have
    /// committed (the reply was lost); every command is idempotent for that
    /// reason (message ids, event ids, claims).
    pub async fn propose(&self, command: Command) -> anyhow::Result<(Response, u64)> {
        let started = Instant::now();
        if std::env::var_os("CELL_PROF").is_some() {
            static KINDS: std::sync::LazyLock<parking_lot::Mutex<(Instant, HashMap<String, u64>)>> =
                std::sync::LazyLock::new(|| {
                    parking_lot::Mutex::new((Instant::now(), HashMap::new()))
                });
            let kind = serde_json::to_value(&command)
                .ok()
                .and_then(|v| {
                    let op = v.get("op")?.as_str()?.to_string();
                    let sub = v
                        .get("command")
                        .and_then(|c| c.get("type").or(c.get("kind")))
                        .and_then(|t| t.as_str())
                        .map(|t| format!("/{t}"))
                        .unwrap_or_default();
                    let sub2 = v
                        .get("change")
                        .map(|c| match c {
                            serde_json::Value::Object(m) => format!(
                                "/{}",
                                m.iter()
                                    .next()
                                    .map(|(k, v)| if k == "op" || k == "type" {
                                        v.as_str().unwrap_or("").to_string()
                                    } else {
                                        k.clone()
                                    })
                                    .unwrap_or_default()
                            ),
                            serde_json::Value::String(t) => format!("/{t}"),
                            _ => String::new(),
                        })
                        .unwrap_or_default();
                    Some(format!("{op}{sub}{sub2}"))
                })
                .unwrap_or_default();
            let mut k = KINDS.lock();
            *k.1.entry(kind).or_default() += 1;
            if k.0.elapsed() > Duration::from_secs(20) {
                k.0 = Instant::now();
                let mut v: Vec<_> = k.1.drain().collect();
                v.sort_by_key(|v| std::cmp::Reverse(v.1));
                tracing::info!(target: "everruns_cell::prof", ?v, "proposals");
            }
        }
        let deadline = started + Duration::from_secs(15);
        let mut backoff = Duration::from_millis(10);
        loop {
            let attempt = if self.is_leader() {
                if self.shipper.behind() && admits(&command) {
                    // Shipping is behind its bound: new work waits (callers
                    // retry a frozen session) so the loss window holds.
                    return Ok((Response::Frozen, 0));
                }
                self.batcher.submit(command.clone()).await
            } else {
                match self.leader().and_then(|id| self.member_addr(id)) {
                    Some(addr) => {
                        let t = Instant::now();
                        let r = self.forward(&addr, &command).await;
                        crate::prof::add("forward", t, 1);
                        r
                    }
                    None => Err(anyhow::anyhow!("no leader for cell {}", self.cfg.cell)),
                }
            };
            match attempt {
                Ok(done) => {
                    self.metrics.propose.record(started.elapsed());
                    self.prom.propose_seconds.observe(&[], started.elapsed());
                    return Ok(done);
                }
                Err(error) if Instant::now() >= deadline => return Err(error),
                Err(error) => tracing::debug!(%error, "proposal retry"),
            }
            tokio::time::sleep(backoff).await;
            backoff = (backoff * 2).min(Duration::from_millis(200));
        }
    }

    async fn forward(&self, addr: &str, command: &Command) -> anyhow::Result<(Response, u64)> {
        let reply: Proposed = self
            .http
            .post(format!("http://{addr}/cell/propose"))
            .json(command)
            .timeout(Duration::from_secs(10))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok((reply.response, reply.index))
    }

    // ---------------------------------------------------------------- executor

    fn all_sessions(&self) -> Vec<SessionId> {
        self.shared.state.read().sessions.keys().copied().collect()
    }

    /// Run Engine's activation of `session` here, unless one already runs.
    ///
    /// Engine claims an epoch, drives the session (start, steer or resume a
    /// turn, drain queued input) and releases. Input admitted while the
    /// epoch is being released is picked up here at once, without waiting
    /// for the dispatcher.
    pub fn activate(self: &Arc<Self>, session: SessionId) {
        if !self.running.lock().insert(session) {
            return;
        }
        let node = self.clone();
        turn_runtime().spawn(async move {
            let agent = node
                .shared
                .state
                .read()
                .sessions
                .get(&session)
                .map(|s| s.agent.clone())
                .unwrap_or_default();
            for _ in 0..64 {
                let running = Running::new(&node, &agent);
                let started = Instant::now();
                let t = Instant::now();
                let activated = node.engine.activate(session).await;
                crate::prof::add("activation", t, 1);
                match activated {
                    Ok(()) => {
                        // Handed back for a move (the session is frozen or
                        // already gone): the turn continues in its new cell.
                        let _ = node.hosting.caught_up(session).await;
                        let moved =
                            node.shared
                                .state
                                .read()
                                .sessions
                                .get(&session)
                                .is_none_or(|s| {
                                    s.placement == crate::state::Placement::Frozen
                                        && s.authority.runnable
                                });
                        let outcome = if moved { "parked" } else { "completed" };
                        running.done(outcome, started.elapsed());
                    }
                    Err(error) => {
                        let lost = error.to_string().contains("lease");
                        running.done(if lost { "lost" } else { "error" }, started.elapsed());
                        tracing::warn!(%session, %error, "activation stopped");
                        break;
                    }
                }
                if node.hosting.caught_up(session).await.is_err() {
                    break;
                }
                let again = node
                    .shared
                    .state
                    .read()
                    .sessions
                    .get(&session)
                    .is_some_and(|s| {
                        s.placement == crate::state::Placement::Live
                            && s.authority.holder.is_none()
                            && s.authority.owed(now_ms())
                    });
                if !again {
                    break;
                }
            }
            node.running.lock().remove(&session);
        });
    }

    pub fn running_count(&self) -> usize {
        self.running.lock().len()
    }

    // --------------------------------------------------------------- scheduler

    /// While this node leads: send sessions that owe work to an executor.
    async fn scheduler_loop(self: Arc<Self>) {
        let mut changes = self.shared.changes.subscribe();
        let mut tick = tokio::time::interval(Duration::from_millis(250));
        loop {
            let candidates: Vec<SessionId> = tokio::select! {
                change = changes.recv() => match change {
                    Ok(Change::Session(session)) => vec![session],
                    Ok(Change::Event(..)) => continue,
                    Err(broadcast::error::RecvError::Lagged(_)) => self.all_sessions(),
                    Err(broadcast::error::RecvError::Closed) => return,
                },
                _ = tick.tick() => {
                    self.hosting.sweep();
                    self.all_sessions()
                }
            };
            if !self.is_leader() {
                continue;
            }
            for session in candidates {
                self.schedule(session);
            }
        }
    }

    /// Finish the move of a frozen session whose epoch is gone: install it
    /// in the target cell, repoint the directory, forward from here. Every
    /// step is idempotent, so a new leader simply runs the move again.
    ///
    /// Decision: a running turn does not hold a move up until it ends (a
    /// turn may run for days). Freezing raises the epoch's hand-off flag;
    /// Engine's driver hands the epoch back at its next step boundary
    /// (`SessionActorStore::hand_off_actor`) and the target cell resumes the
    /// open turn from the journal. An epoch whose holder stopped answering
    /// expires, and the move goes ahead without it.
    fn maybe_move(self: &Arc<Self>, session: SessionId) {
        let to = {
            let state = self.shared.state.read();
            let Some(s) = state.sessions.get(&session) else {
                return;
            };
            if s.placement == crate::state::Placement::Prepared {
                drop(state);
                self.settle_prepared(session);
                return;
            }
            if s.placement != crate::state::Placement::Frozen {
                return;
            }
            let Some(to) = s.moving_to else {
                return;
            };
            // An expired epoch can no longer write: the move goes ahead
            // and the target's next activation supersedes it.
            if s.authority.executing(now_ms()) {
                return; // hands off at its next step
            }
            to
        };
        if !self.moving.lock().insert(session) {
            return;
        }
        let node = self.clone();
        tokio::spawn(async move {
            if let Err(error) = node.finish_move(session, to).await {
                tracing::warn!(%session, to, %error, "move step failed; retrying");
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            node.moving.lock().remove(&session);
        });
    }

    /// The move protocol, after the `Moves` model: the source (leaving at
    /// generation `g`) installs a prepared copy at `g + 1` in the target,
    /// the directory commits `g + 1` over `g` (the ownership transfer), the
    /// target activates its copy, and the source forwards. A target that
    /// refuses the copy abandons the move: the directory takes the session
    /// back here at `g + 2`, above anything the move may have installed.
    async fn finish_move(&self, session: SessionId, to: CellId) -> anyhow::Result<()> {
        if to == self.cfg.cell {
            self.propose(crate::moves::unfreeze(self.cfg.cell, session))
                .await?;
            return Ok(());
        }
        let gateway = self
            .gateway
            .get()
            .ok_or_else(|| anyhow::anyhow!("no gateway to reach cell {to}"))?;
        // End the epoch through the log first, so nothing writes to the
        // leaving copy after it is taken (see `Command::Revoke`).
        match self
            .propose(Command::Revoke {
                session,
                now_ms: now_ms(),
            })
            .await?
            .0
        {
            Response::Ok => {}
            Response::Moved { .. } | Response::NotFound => return Ok(()), // already forwarded
            other => anyhow::bail!("revoke: {other:?}"),
        }
        // History stays in the object store; only metadata and the
        // unflushed tail travel.
        let Some(image) = self.shared.image(session, 0, true).await else {
            return Ok(()); // already forwarded
        };
        let mut image = image?;
        let leaving = image.generation;
        image.generation = leaving + 1;
        let imported = gateway
            .propose(
                to,
                &Command::Import {
                    session,
                    image: Box::new(image),
                },
            )
            .await
            .map_err(|(status, error)| anyhow::anyhow!("import: {status} {error}"))?;
        if let Response::Rejected { reason } = &imported {
            tracing::warn!(%session, to, %reason, "target refused the session; abandoning the move");
        }
        let proposed = crate::moves::after_import(self.cfg.cell, to, leaving, &imported)
            .map_err(anyhow::Error::msg)?;
        let placed = gateway
            .place_entry(session, proposed.cell, proposed.generation, leaving)
            .await
            .map_err(|(status, error)| anyhow::anyhow!("place: {status} {error}"))?;
        let current =
            crate::moves::after_place(leaving, proposed, &placed).map_err(anyhow::Error::msg)?;
        self.settle_move(session, leaving, current).await
    }

    /// Finish the move the directory committed: `current` is the entry
    /// above `leaving`, this cell's departing copy.
    async fn settle_move(
        &self,
        session: SessionId,
        leaving: u64,
        current: Placed,
    ) -> anyhow::Result<()> {
        for (cell, command) in crate::moves::settle(self.cfg.cell, session, leaving, current) {
            let response = if cell == self.cfg.cell {
                self.propose(command).await?.0
            } else {
                let gateway = self
                    .gateway
                    .get()
                    .ok_or_else(|| anyhow::anyhow!("no gateway to reach cell {cell}"))?;
                gateway
                    .propose(cell, &command)
                    .await
                    .map_err(|(status, error)| anyhow::anyhow!("{status} {error}"))?
            };
            if !crate::moves::step_done(&response) {
                anyhow::bail!("move step refused by cell {cell}: {response:?}");
            }
        }
        if current.cell == self.cfg.cell {
            tracing::info!(%session, "move abandoned; session stays");
            return Ok(());
        }
        tracing::info!(%session, to = current.cell, "session moved");
        Ok(())
    }

    /// A prepared copy whose move stalled (its source died after the
    /// directory committed, or the move was abandoned): ask the directory.
    /// Committed here: activate it. Placed elsewhere at its generation or
    /// later: drop it. Neither yet: the move is still running.
    fn settle_prepared(self: &Arc<Self>, session: SessionId) {
        if !self.moving.lock().insert(session) {
            return;
        }
        let node = self.clone();
        tokio::spawn(async move {
            // Give the source's own move the first chance.
            tokio::time::sleep(PREPARED_GRACE).await;
            if let Err(error) = node.settle_prepared_now(session).await {
                tracing::warn!(%session, %error, "prepared copy not settled; retrying");
            }
            node.moving.lock().remove(&session);
        });
    }

    async fn settle_prepared_now(&self, session: SessionId) -> anyhow::Result<()> {
        let generation = {
            let state = self.shared.state.read();
            match state.sessions.get(&session) {
                Some(s) if s.placement == crate::state::Placement::Prepared => s.generation,
                _ => return Ok(()),
            }
        };
        let gateway = self
            .gateway
            .get()
            .ok_or_else(|| anyhow::anyhow!("no gateway to reach the directory"))?;
        let current = gateway
            .placement(session)
            .await
            .map_err(|(status, error)| anyhow::anyhow!("locate: {status} {error}"))?;
        let Some(command) =
            crate::moves::settle_prepared(self.cfg.cell, session, generation, current)
        else {
            return Ok(());
        };
        self.propose(command).await?;
        Ok(())
    }

    fn schedule(self: &Arc<Self>, session: SessionId) {
        self.maybe_move(session);
        let now = now_ms();
        let executor = {
            let state = self.shared.state.read();
            let Some(s) = state.sessions.get(&session) else {
                self.dispatched.lock().remove(&session);
                return;
            };
            if s.placement != crate::state::Placement::Live {
                return;
            }
            let a = &s.authority;
            // Decision: an expired epoch counts as gone. Its writes are
            // refused from expiry on (Everruns' lease contract), so the
            // session is claimable at once; a node that restarted, or lost
            // the session, holds nothing up.
            if a.executing(now) {
                self.dispatched.lock().remove(&session);
                return;
            }
            if !a.owed(now) || a.binding.is_none() {
                return;
            }
            if self
                .dispatched
                .lock()
                .get(&session)
                .is_some_and(|at| at.elapsed() < Duration::from_secs(1))
            {
                return;
            }
            // Decision: a cell starts no more turns than its nodes have
            // slots for. Past that, CPU contention delays Raft heartbeats
            // and lease renewals, leaders change and turns rerun: the cell
            // slows down more the more it is asked. Owed work waits instead
            // (and counts toward the cell's load, so placement and the
            // rebalancer steer work elsewhere).
            let Some(executor) =
                self.pick_executor(&state, None, Some(self.cfg.max_turns_per_node))
            else {
                return;
            };
            executor
        };
        self.dispatched.lock().insert(session, Instant::now());
        if executor == self.cfg.node_id {
            self.activate(session);
            return;
        }
        let Some(addr) = self.member_addr(executor) else {
            return;
        };
        let node = self.clone();
        tokio::spawn(async move {
            let sent = node
                .http
                .post(format!("http://{addr}/cell/activate/{session}"))
                .timeout(Duration::from_secs(2))
                .send()
                .await;
            if sent.is_err() {
                node.dispatched.lock().remove(&session);
            }
        });
    }

    /// The live member with the fewest active turns. The leader counts as
    /// busier so turns prefer followers and a leader change leaves most
    /// in-flight steps where they are.
    /// With `limit`, `None` when every live member already runs that many.
    fn pick_executor(
        &self,
        state: &crate::state::CellState,
        avoid: Option<NodeId>,
        limit: Option<usize>,
    ) -> Option<NodeId> {
        let alive = self.alive_members();
        // Decision: counting running turns scans every session, so the count
        // is cached briefly and bumped on each pick, instead of rescanned per
        // turn start (which made starts O(sessions) under the state lock).
        let mut cache = self.load.lock();
        if cache
            .as_ref()
            .is_none_or(|(at, _)| at.elapsed() > Duration::from_millis(100))
        {
            let mut counts = BTreeMap::new();
            let now = now_ms();
            for s in state.sessions.values() {
                if let Some(holder) = &s.authority.holder
                    && holder.expires_ms > now
                {
                    *counts.entry(holder.node).or_insert(0) += 1;
                }
            }
            *cache = Some((Instant::now(), counts));
        }
        let (_, counts) = cache.as_mut()?;
        let me = self.cfg.node_id;
        let pick = alive
            .iter()
            .filter(|id| Some(**id) != avoid || alive.len() == 1)
            .min_by_key(|id| {
                let count = counts.get(id).copied().unwrap_or(0);
                (count * 2 + usize::from(**id == me) * 3, **id)
            })
            .copied()
            .unwrap_or(me);
        if limit.is_some_and(|limit| counts.get(&pick).copied().unwrap_or(0) >= limit) {
            return None;
        }
        *counts.entry(pick).or_insert(0) += 1;
        Some(pick)
    }

    fn alive_members(&self) -> BTreeSet<NodeId> {
        let fresh = Duration::from_millis(1500);
        let alive = self.alive.lock();
        let mut set: BTreeSet<NodeId> = alive
            .iter()
            .filter(|(_, seen)| seen.elapsed() < fresh)
            .map(|(id, _)| *id)
            .collect();
        set.insert(self.cfg.node_id);
        set
    }

    /// Track which members answer, for executor placement.
    async fn liveness_loop(self: Arc<Self>) {
        let mut tick = tokio::time::interval(Duration::from_millis(300));
        loop {
            tick.tick().await;
            self.refresh_members();
            for (id, addr) in self.members() {
                if id == self.cfg.node_id {
                    continue;
                }
                let node = self.clone();
                tokio::spawn(async move {
                    let ok = node
                        .http
                        .get(format!("http://{addr}/health"))
                        .timeout(Duration::from_millis(250))
                        .send()
                        .await
                        .is_ok_and(|r| r.status().is_success());
                    if ok {
                        node.alive.lock().insert(id, Instant::now());
                    }
                });
            }
        }
    }
}

/// Counts an activation in `/metrics`: running while alive, then one
/// outcome (`completed`, `lost`, `error`).
struct Running {
    node: Arc<Node>,
    agent: String,
    done: bool,
}

impl Running {
    fn new(node: &Arc<Node>, agent: &str) -> Self {
        node.prom.turns_running.add(&[("agent", agent)], 1);
        Self {
            node: node.clone(),
            agent: agent.to_string(),
            done: false,
        }
    }

    fn done(mut self, outcome: &str, elapsed: Duration) {
        let labels = [("agent", self.agent.as_str()), ("outcome", outcome)];
        self.node.prom.turns_executed.inc(&labels);
        self.node
            .prom
            .turn_seconds
            .observe(&[("agent", self.agent.as_str())], elapsed);
        self.done = true;
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        self.node
            .prom
            .turns_running
            .add(&[("agent", self.agent.as_str())], -1);
        if !self.done {
            self.node
                .prom
                .turns_executed
                .inc(&[("agent", self.agent.as_str()), ("outcome", "error")]);
        }
    }
}

/// Hand freed heap pages (unloaded events, purged log) back to the OS, or
/// unloading would only show inside the allocator. glibc keeps them
/// otherwise; elsewhere (macOS, musl) the allocator returns them itself.
fn release_free_pages() {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    // SAFETY: malloc_trim only releases free heap pages.
    unsafe {
        libc::malloc_trim(0);
    }
}

/// Whether `command` admits new work (a new session or new input), which
/// waits while log shipping is behind. Work already admitted keeps going.
fn admits(command: &Command) -> bool {
    use everruns::hosting::spi::session_commands::SessionCommand;
    match command {
        Command::Host { command, .. } => matches!(
            command,
            SessionCommand::Create { .. } | SessionCommand::Admit { .. }
        ),
        _ => false,
    }
}

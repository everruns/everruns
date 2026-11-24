//! Memory pressure: how a shard takes on less work before a node runs out of
//! memory.
//!
//! Decision: a node past its container's memory limit is killed, its shard
//! loses a replica, and the node's rejoin (snapshot, log replay, catch-up)
//! needs more memory still: in EKS run 43747035567 one node went round that
//! loop five times. Unloading events (`Node::unload_loop`) frees what can
//! be freed; past that, the shard stops taking on work, in two steps:
//!
//! - **Full** (a node at 85% of its limit): the shard is closed to new
//!   sessions. Gateways place them in other shards, and the rebalancer moves
//!   idle sessions out to shards that are not full.
//! - **Shedding** (92%): the shard refuses new sessions and new input
//!   outright. Callers get 429 with a retry hint; turns already admitted
//!   keep running and finish, which is what frees memory again.
//!
//! At either level the shard also starts no more turns than its
//! `pressure_turns` floor (`Node::schedule`): input it already queued
//! waits. A slow shard otherwise draws the shared executor pool into itself,
//! since its turns hold executor credits longer (EKS run 44355318472: 17,000
//! running turns in a shard that lost a node, three nodes out of memory).
//!
//! Decision: Full only steers, it does not refuse. A gateway has already
//! written a session's directory entry when it creates the session on its
//! shard, so a refused create comes back to the same shard on retry; steering
//! by the load view (a second old at most) sends the next ones elsewhere
//! instead.
//!
//! - **Emergency** (95%): the shard refuses new work as when shedding and
//!   starts no turns at all. Turns already running finish, which frees
//!   memory; a turn started now would only add to it.
//!
//! Past Emergency a node restarts itself rather than wait for the kernel's
//! kill (level 4): at 98% of its limit, or at Emergency for
//! [`EMERGENCY_FOR`]. It marks the restart in its memory heartbeat (so it is
//! not counted as a kill), stops Raft (its followers elect at once instead
//! of after a lost heartbeat) and exits with `memwatch::RESTART_EXIT`;
//! Kubernetes starts it again from its disk. Decision: only while every
//! peer answers, so a restart never takes the shard's second node down
//! with the first; a node whose peer is already gone waits for it instead.
//!
//! Decision: the restart is exclusive across the shard's replicas, not only
//! within a process. All of a shard's nodes hold the same sessions, so they
//! tend to run hot together, and each answering "every peer is up" in the
//! same second would restart them all (codex review, 2026-11-24). A node
//! whose restart is due first announces it in its reading (`pending`) and
//! waits [`ANNOUNCE`], longer than two rounds of asking plus a read's
//! timeout, so every peer has seen it. It restarts only if every peer still
//! answers and none is restarting, catching up after a restart, or pending
//! with a lower node id. Of nodes due together the lowest id goes; the
//! others wait until it is back and caught up. Not a Raft-committed grant:
//! that would add a command older replicas cannot apply mid-upgrade, and a
//! restart already needs every peer answering, which is stricter than a
//! quorum. A peer that predates these fields never blocks.
//! A restart frees what a kill frees, without a kill's torn writes, and
//! shows as its own count (`agentd_memory_actions_total{action="restart"}`)
//! rather than as an out-of-memory kill. Only the `node` binary exits
//! ([`enable_restart`]); a test fleet shares one process and only counts.
//!
//! Decision: memory is read as the kernel kills on it, the container's
//! working bytes (`memwatch::reading`), not the resident set; outside a
//! container (local fleets) the resident set stands in.
//!
//! The levels have hysteresis (clear at 80%, 88% and 92%), so a shard does not
//! flap at a boundary. The level is the shard's, not the node's: every node
//! holds every session of its shard, so the worst node decides. Each node
//! asks its peers every second; a peer that does not answer for 5 s (down,
//! or not started) does not count.
//!
//! A shard whose leader stops committing sheds the same way. Decision: in
//! EKS run 43774871232 three shards stopped committing for about 20 minutes
//! each (replication kept missing its deadline, see `network::Isolation`).
//! Every request waited out its 15 s proposal deadline, clients retried, and
//! the waiting requests filled the nodes' memory until they were killed. A
//! leader whose log has entries it has not applied, and whose applied index
//! has not moved for [`STALL_AFTER`], refuses new sessions and input with
//! the same retry hint until it commits again.
//!
//! Work already running is never refused: claims, appends, outcomes and
//! moves all go through. Shedding new input bounds how far the log grows
//! while a node is short of memory, which also bounds what a rejoining
//! node must catch up on.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use axum::Json;
use axum::extract::State;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::node::Node;
use crate::state::{Command, Response};
use crate::types::NodeId;

/// Permille of the limit at which a level starts, and at which it clears.
const FULL_AT: u64 = 850;
const FULL_CLEAR: u64 = 800;
const SHED_AT: u64 = 920;
const SHED_CLEAR: u64 = 880;
const EMERG_AT: u64 = 950;
const EMERG_CLEAR: u64 = 920;
/// Permille of its own limit at which a node restarts itself.
const RESTART_AT: u64 = 980;
/// How long a node's own reading may stay at Emergency before it restarts.
pub const EMERGENCY_FOR: Duration = Duration::from_secs(30);
/// How long a due restart is announced before it happens: two rounds of
/// asking peers (1 s each) plus a read's timeout, so every peer has read it.
pub const ANNOUNCE: Duration = Duration::from_secs(3);

/// Set by the `node` binary: a node that wants a restart may exit.
static RESTART_ENABLED: AtomicBool = AtomicBool::new(false);
static RESTART: tokio::sync::Notify = tokio::sync::Notify::const_new();

/// Let this process restart itself out of memory (the `node` binary).
pub fn enable_restart() {
    RESTART_ENABLED.store(true, Ordering::Relaxed);
}

/// Resolves once a node of this process asks to restart.
pub async fn restart_requested() {
    RESTART.notified().await;
}

/// The shard's leader as `raft` knows it; none once it stopped on a fatal
/// error. Its metrics then freeze at the last leader it knew, often itself,
/// and executors and gateways asking it would send work to a dead core
/// (`Node::fence`).
pub fn live_leader(raft: &crate::types::Raft) -> Option<NodeId> {
    let m = raft.metrics();
    let m = m.borrow();
    m.running_state
        .is_ok()
        .then_some(m.current_leader)
        .flatten()
}

static FENCE: tokio::sync::Notify = tokio::sync::Notify::const_new();

/// Exit status of a node whose Raft stopped on a fatal storage error.
pub const FENCE_EXIT: i32 = 74;

/// Resolves once a node of this process fenced itself (`Node::fence`).
pub async fn fence_requested() {
    FENCE.notified().await;
}

/// A peer reading older than this does not count.
const PEER_STALE: Duration = Duration::from_secs(5);
/// A leader whose applied index has not moved this long while it has
/// entries to apply has stalled.
pub const STALL_AFTER: Duration = Duration::from_secs(5);

/// How long a refused caller should wait before trying again. Long enough
/// for a few unload rounds and for running turns to finish; short enough
/// that a caller does not sit idle long once memory frees.
pub const RETRY_AFTER_MS: u64 = 2_000;

/// How close a shard's nodes are to their memory limits.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    #[default]
    Ok,
    /// No new sessions.
    Full,
    /// No new sessions and no new input.
    Shedding,
    /// No new sessions, no new input, no new turns.
    Emergency,
}

impl Level {
    fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Full,
            2 => Self::Shedding,
            3 => Self::Emergency,
            _ => Self::Ok,
        }
    }

    /// The level after a reading of `permille`, from `self`.
    pub fn next(self, permille: u64) -> Self {
        if permille >= EMERG_AT || (self == Self::Emergency && permille >= EMERG_CLEAR) {
            Self::Emergency
        } else if permille >= SHED_AT || (self >= Self::Shedding && permille >= SHED_CLEAR) {
            Self::Shedding
        } else if permille >= FULL_AT || (self >= Self::Full && permille >= FULL_CLEAR) {
            Self::Full
        } else {
            Self::Ok
        }
    }
}

/// One node's reading, as `/shard/pressure` answers it.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct Reading {
    pub rss: u64,
    pub limit: u64,
    /// A restart is due here and announced (module notes).
    #[serde(default)]
    pub pending: bool,
    /// This node is restarting itself.
    #[serde(default)]
    pub restarting: bool,
    /// This node is catching up (after a restart, say).
    #[serde(default)]
    pub behind: bool,
}

impl Reading {
    pub fn permille(&self) -> u64 {
        (self.rss.saturating_mul(1000)) / self.limit.max(1)
    }
}

/// New work a shedding shard refuses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Work {
    Session,
    Input,
}

pub struct Pressure {
    /// This node's memory limit in bytes.
    pub limit: u64,
    own: AtomicU64,
    peers: Mutex<BTreeMap<NodeId, (u64, Instant)>>,
    level: AtomicU8,
    /// The leader's applied index when it last moved, and when.
    progress: Mutex<(u64, Instant)>,
    stalled: AtomicBool,
    /// Refusals since start, by what was refused.
    pub shed_sessions: AtomicU64,
    pub shed_inputs: AtomicU64,
    /// Since when this node's own reading has been at Emergency.
    hot_since: Mutex<Option<Instant>>,
    /// This node asked to restart itself.
    restarting: AtomicBool,
    /// This node fenced itself after Raft stopped on a fatal error.
    fenced: AtomicBool,
    /// Since when this node's restart has been due and announced.
    pending_since: Mutex<Option<Instant>>,
    /// This node is catching up (`catchup`), as its reading tells peers.
    behind: AtomicBool,
    /// Each peer's restart flags from its last reading, and when.
    peer_flags: Mutex<BTreeMap<NodeId, (Reading, Instant)>>,
    /// What memory made the node do, by `action`.
    pub actions: crate::prom::Counters,
}

impl Pressure {
    pub fn new(limit: u64) -> Self {
        Self {
            limit,
            own: AtomicU64::new(0),
            peers: Mutex::default(),
            level: AtomicU8::new(0),
            progress: Mutex::new((0, Instant::now())),
            stalled: AtomicBool::new(false),
            shed_sessions: AtomicU64::new(0),
            shed_inputs: AtomicU64::new(0),
            hot_since: Mutex::new(None),
            restarting: AtomicBool::new(false),
            fenced: AtomicBool::new(false),
            pending_since: Mutex::new(None),
            behind: AtomicBool::new(false),
            peer_flags: Mutex::default(),
            actions: crate::prom::Counters::default(),
        }
    }

    pub fn own(&self) -> Reading {
        Reading {
            rss: self.own.load(Ordering::Relaxed),
            limit: self.limit,
            pending: self.pending_since.lock().is_some(),
            restarting: self.restarting.load(Ordering::Relaxed),
            behind: self.behind.load(Ordering::Relaxed),
        }
    }

    /// Whether this node is catching up, for its reading.
    pub fn set_behind(&self, behind: bool) {
        self.behind.store(behind, Ordering::Relaxed);
    }

    /// Record this node's memory in use and recompute the shard's level.
    pub fn record_own(&self, rss: u64) -> Level {
        self.own.store(rss, Ordering::Relaxed);
        self.update()
    }

    pub fn record_peer(&self, node: NodeId, reading: Reading) -> Level {
        self.peer_flags
            .lock()
            .insert(node, (reading, Instant::now()));
        self.peers
            .lock()
            .insert(node, (reading.permille(), Instant::now()));
        self.update()
    }

    /// The worst fresh reading of the shard's nodes, in permille.
    pub fn shard_permille(&self) -> u64 {
        let own = self.own().permille();
        let peers = self.peers.lock();
        peers
            .values()
            .filter(|(_, at)| at.elapsed() < PEER_STALE)
            .map(|(p, _)| *p)
            .fold(own, u64::max)
    }

    fn update(&self) -> Level {
        let was = self.level();
        let level = was.next(self.shard_permille());
        if level == Level::Emergency && was != Level::Emergency {
            self.actions.inc(&[("action", "emergency")]);
        }
        self.level.store(level as u8, Ordering::Relaxed);
        level
    }

    /// Whether this node (`me`) should restart itself now (level 4), from
    /// its own reading: at [`RESTART_AT`], or at Emergency for
    /// [`EMERGENCY_FOR`]; announced for [`ANNOUNCE`] first, and only while
    /// every peer answers (`peers_answer`) and none holds the restart
    /// (module notes). Once.
    pub fn restart_due(&self, peers_answer: bool, me: NodeId) -> bool {
        self.restart_due_at(peers_answer, me, Instant::now())
    }

    fn restart_due_at(&self, peers_answer: bool, me: NodeId, now: Instant) -> bool {
        let own = self.own().permille();
        let hot = {
            let mut since = self.hot_since.lock();
            if own >= EMERG_AT {
                since.get_or_insert(now);
            } else if own < EMERG_CLEAR {
                *since = None;
            }
            *since
        };
        let due = own >= RESTART_AT || hot.is_some_and(|t| now.duration_since(t) >= EMERGENCY_FOR);
        let since = {
            let mut pending = self.pending_since.lock();
            if !due {
                *pending = None;
                return false;
            }
            *pending.get_or_insert(now)
        };
        if !peers_answer || now.duration_since(since) < ANNOUNCE || self.peer_holds_restart(me, now)
        {
            return false;
        }
        !self.restarting.swap(true, Ordering::Relaxed)
    }

    /// A fresh peer reading that rules this node's restart out: the peer is
    /// restarting, catching up, or announced a restart with a lower id.
    fn peer_holds_restart(&self, me: NodeId, now: Instant) -> bool {
        self.peer_flags.lock().iter().any(|(id, (r, at))| {
            now.saturating_duration_since(*at) < PEER_STALE
                && (r.restarting || r.behind || (r.pending && *id < me))
        })
    }

    pub fn restarting(&self) -> bool {
        self.restarting.load(Ordering::Relaxed)
    }

    pub fn level(&self) -> Level {
        Level::from_u8(self.level.load(Ordering::Relaxed))
    }

    /// Record Raft's progress: whether this node leads, its last log index
    /// and its applied index. Returns whether the shard has stalled.
    pub fn record_progress(&self, leader: bool, last_log: u64, applied: u64) -> bool {
        self.record_progress_at(leader, last_log, applied, Instant::now())
    }

    fn record_progress_at(&self, leader: bool, last_log: u64, applied: u64, now: Instant) -> bool {
        let mut progress = self.progress.lock();
        if !leader || applied >= last_log || applied != progress.0 {
            *progress = (applied, now);
        }
        let stalled = now.duration_since(progress.1) >= STALL_AFTER;
        self.stalled.store(stalled, Ordering::Relaxed);
        stalled
    }

    pub fn stalled(&self) -> bool {
        self.stalled.load(Ordering::Relaxed)
    }

    /// The level the shard acts on: shedding while stalled.
    pub fn effective(&self) -> Level {
        if self.stalled() {
            Level::Shedding
        } else {
            self.level()
        }
    }

    /// Whether the shard refuses `work` now; counts the refusal.
    pub fn refuses(&self, work: Work) -> bool {
        let refused = self.effective() >= Level::Shedding;
        if refused {
            match work {
                Work::Session => &self.shed_sessions,
                Work::Input => &self.shed_inputs,
            }
            .fetch_add(1, Ordering::Relaxed);
        }
        refused
    }
}

/// The new work `command` brings, if any.
pub(crate) fn work_of(command: &Command) -> Option<Work> {
    use everruns::hosting::spi::session_commands::SessionCommand;
    match command {
        Command::Host { command, .. } => match command {
            SessionCommand::Create { .. } => Some(Work::Session),
            SessionCommand::Admit { .. } => Some(Work::Input),
            _ => None,
        },
        _ => None,
    }
}

impl Node {
    /// Leader: refuse new work while shipping is behind its bound (callers
    /// retry a frozen session, so the loss window holds) or while the shard
    /// sheds load. Work already admitted keeps going.
    pub(crate) fn refusal(&self, command: &Command) -> Option<Response> {
        let work = work_of(command)?;
        if self.shipper.behind() {
            return Some(Response::Frozen);
        }
        self.pressure.refuses(work).then_some(Response::Busy {
            retry_after_ms: RETRY_AFTER_MS,
        })
    }

    /// Whether the shard, short of memory, starts no more turns now.
    ///
    /// Decision: at Full or worse a shard starts no turns past
    /// `pressure_turns`. In EKS run 44355318472 a shard that lost a node to
    /// chaos committed slowly, so its turns held executor credits longer and
    /// the shared pool piled into it (2,600 to 17,000 running turns in four
    /// minutes, the other shards steady at 2,700); its other two nodes ran
    /// out of memory three times. Refusing new input (Shedding) did not stop
    /// it: the input was already queued. Below the floor the shard still
    /// makes progress, so memory that only finished turns free does get
    /// freed.
    ///
    /// At Emergency it starts none.
    pub(crate) fn holds_turns(&self, state: &crate::state::ShardState) -> bool {
        match self.pressure.effective() {
            Level::Emergency => true,
            Level::Ok => false,
            _ => self.running_turns(state) >= self.cfg.pressure_turns,
        }
    }

    /// Fence this node after its Raft stopped on a fatal error.
    ///
    /// Decision: a storage error Raft cannot get past (a full disk, a failed
    /// write) stops the node's Raft core but not its process: its metrics
    /// freeze at the last leader it knew, often itself, so it kept claiming
    /// leadership while the other replicas elected a new leader, and turns
    /// queued behind it stalled (disk full on the Mac mini, 2026-11-24: 1,315
    /// inputs waiting 19 minutes). `leader()` reports none once Raft has
    /// stopped, and the `node` binary exits with [`FENCE_EXIT`] so the
    /// orchestrator restarts it; a test fleet, sharing one process, counts.
    fn fence(&self, fatal: &openraft::error::Fatal<NodeId>) {
        if self.pressure.fenced.swap(true, Ordering::Relaxed) {
            return;
        }
        self.pressure.actions.inc(&[("action", "fence")]);
        tracing::error!(shard = self.cfg.shard, node = self.cfg.node_id, %fatal,
            "raft stopped on a fatal error: fencing this node");
        if RESTART_ENABLED.load(Ordering::Relaxed) {
            FENCE.notify_one();
        }
    }

    /// Level 4: restart this node before the kernel kills it (module notes).
    async fn restart_for_memory(&self) {
        let own = self.pressure.own();
        self.pressure.actions.inc(&[("action", "restart")]);
        if !RESTART_ENABLED.load(Ordering::Relaxed) {
            tracing::error!(
                shard = self.cfg.shard,
                permille = own.permille(),
                "out of memory: a restart is due, but this process may not exit"
            );
            return;
        }
        tracing::error!(
            shard = self.cfg.shard,
            node = self.cfg.node_id,
            permille = own.permille(),
            "out of memory: restarting before the kernel kills this node"
        );
        crate::memwatch::note_restart(&self.cfg.data_dir);
        if let Err(error) = self.raft.shutdown().await {
            tracing::warn!(%error, "raft did not stop cleanly");
        }
        RESTART.notify_one();
    }

    /// Every second: read this node's resident set and ask the shard's other
    /// members for theirs.
    pub(crate) async fn pressure_loop(self: Arc<Self>) {
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        let mut was = Level::Ok;
        let mut was_stalled = false;
        loop {
            tick.tick().await;
            self.pressure.set_behind(self.catchup.behind());
            self.pressure
                .record_own(crate::memwatch::reading(self.cfg.memory_limit).0);
            let stalled = {
                let metrics = self.raft.metrics().borrow().clone();
                if let Err(fatal) = &metrics.running_state {
                    self.fence(fatal);
                }
                self.pressure.record_progress(
                    metrics.state == openraft::ServerState::Leader,
                    metrics.last_log_index.unwrap_or(0),
                    metrics.last_applied.map_or(0, |id| id.index),
                )
            };
            if stalled != was_stalled {
                tracing::warn!(
                    shard = self.cfg.shard,
                    stalled,
                    "shard commit progress changed"
                );
                was_stalled = stalled;
            }
            let peers: Vec<(NodeId, String)> = self
                .members()
                .into_iter()
                .filter(|(id, _)| *id != self.cfg.node_id)
                .collect();
            let asked = peers.len();
            let readings = futures::future::join_all(peers.into_iter().map(|(id, addr)| {
                let http = self.http.clone();
                async move {
                    let reading = http
                        .get(format!("http://{addr}/shard/pressure"))
                        .timeout(Duration::from_millis(800))
                        .send()
                        .await
                        .ok()?
                        .json::<Reading>()
                        .await
                        .ok()?;
                    Some((id, reading))
                }
            }))
            .await;
            let mut level = self.pressure.level();
            let mut answered = 0;
            for (id, reading) in readings.into_iter().flatten() {
                answered += 1;
                level = self.pressure.record_peer(id, reading);
            }
            if self
                .pressure
                .restart_due(answered == asked, self.cfg.node_id)
            {
                self.restart_for_memory().await;
            }
            if level != was {
                tracing::warn!(
                    shard = self.cfg.shard,
                    ?was,
                    ?level,
                    permille = self.pressure.shard_permille(),
                    "shard memory pressure changed"
                );
                was = level;
            }
        }
    }
}

/// `GET /shard/pressure`: this node's own reading.
pub async fn reading(State(node): State<Arc<Node>>) -> Json<Reading> {
    Json(node.pressure.own())
}

pub fn write_metrics(p: &Pressure, w: &mut crate::prom::Writer) {
    w.gauge(
        "agentd_shard_memory_limit_bytes",
        "Memory limit memory pressure is measured against.",
        p.limit as f64,
    );
    w.gauge(
        "agentd_shard_memory_pressure",
        "The worst of the shard's nodes' resident set, as a share of its limit.",
        p.shard_permille() as f64 / 1000.0,
    );
    w.gauge(
        "agentd_shard_overload_level",
        "0: taking work, 1: full (no new sessions placed here), 2: shedding (new sessions and input refused), 3: emergency (no new turns either).",
        p.effective() as u8 as f64,
    );
    w.gauge(
        "agentd_shard_commit_stalled",
        "1 while this node leads and has not applied an entry for 5 s with entries to apply.",
        f64::from(u8::from(p.stalled())),
    );
    w.counter(
        "agentd_shard_shed_sessions_total",
        "New sessions refused while shedding load.",
        p.shed_sessions.load(Ordering::Relaxed) as f64,
    );
    w.counter(
        "agentd_shard_shed_inputs_total",
        "New input refused while shedding load.",
        p.shed_inputs.load(Ordering::Relaxed) as f64,
    );
    w.gauge(
        "agentd_memory_level",
        "What memory makes this node do: 0 ok, 1 full, 2 shedding, 3 emergency, 4 restarting.",
        if p.restarting() {
            4.0
        } else {
            f64::from(p.effective() as u8)
        },
    );
    w.counters(
        "agentd_memory_actions_total",
        "What memory made this node do: emergency (entered level 3), restart (level 4).",
        &p.actions,
    );
    w.counter(
        "agentd_memory_restarts_total",
        "Restarts this node did before its memory limit, across restarts.",
        crate::memwatch::restarts() as f64,
    );
}

/// The node's memory by source, and each capped source's cap: the memory
/// design's ledger. `unaccounted` is live heap beyond the counted sources
/// (the remainder the design keeps under 15% of the limit); `fragmentation`
/// is what the allocator holds beyond live bytes.
pub(crate) fn write_ledger(node: &Node, w: &mut crate::prom::Writer) {
    let events = node.shared.resident().0;
    let log = node.wal.held_bytes();
    let heap = crate::heap::heap();
    let allocated = heap.allocated_mb << 20;
    let unaccounted = allocated.saturating_sub(events + log);
    w.gauge(
        "agentd_memory_unaccounted_bytes",
        "Live heap bytes beyond resident events and the Raft log.",
        unaccounted as f64,
    );
    let base = crate::prom::render(&[
        ("shard", &node.cfg.shard.to_string()),
        ("node", &node.cfg.node_id.to_string()),
    ]);
    let row = |source: &str, bytes: u64| (crate::prom::render(&[("source", source)]), bytes as f64);
    w.family(
        "agentd_memory_bytes",
        "gauge",
        "The node's memory by source (events and raft_log counted, unaccounted live heap, allocator fragmentation).",
        &base,
        [
            row("events", events),
            row("raft_log", log),
            row("unaccounted", unaccounted),
            row(
                "fragmentation",
                (heap.resident_mb << 20).saturating_sub(allocated),
            ),
        ],
    );
    w.family(
        "agentd_memory_cap_bytes",
        "gauge",
        "Where each capped source is held down: events unload past the budget, the log snapshots past a sixteenth of it.",
        &base,
        [
            row("events", node.cfg.memory_budget),
            row("raft_log", node.cfg.memory_budget / 16),
        ],
    );
}

/// The memory limit of the container this process runs in, if it has one
/// (cgroup v2, then v1).
pub fn container_limit() -> Option<u64> {
    [
        "/sys/fs/cgroup/memory.max",
        "/sys/fs/cgroup/memory/memory.limit_in_bytes",
    ]
    .iter()
    .filter_map(|path| std::fs::read_to_string(path).ok())
    .find_map(|text| parse_limit(&text))
}

fn parse_limit(text: &str) -> Option<u64> {
    // "max" (v2) or a page-rounded i64::MAX (v1) mean no limit.
    text.trim().parse::<u64>().ok().filter(|v| *v < 1 << 50)
}

/// The limit pressure is measured against: `configured` MiB if set, else
/// the container's, else a third over the memory budget (the headroom the
/// deployments give, 12 GiB for a 9 GiB budget).
pub fn limit_bytes(configured_mb: u64, budget: u64) -> u64 {
    if configured_mb > 0 {
        return configured_mb << 20;
    }
    container_limit().unwrap_or(budget / 3 * 4)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_leader_that_stops_applying_sheds_until_it_applies_again() {
        let p = Pressure::new(1 << 30);
        let t0 = Instant::now();
        let at = |secs| t0 + Duration::from_secs(secs);
        assert!(!p.record_progress_at(true, 110, 100, at(0)));
        assert!(!p.record_progress_at(true, 120, 100, at(4)), "not yet");
        assert!(p.record_progress_at(true, 130, 100, at(5)));
        assert_eq!(p.effective(), Level::Shedding);
        assert!(p.refuses(Work::Input));
        assert_eq!(p.shed_inputs.load(Ordering::Relaxed), 1);
        assert!(
            !p.record_progress_at(true, 130, 101, at(6)),
            "applied moved"
        );
        assert_eq!(p.effective(), Level::Ok);
        assert!(!p.refuses(Work::Session));
        // Nothing to apply, or not the leader: never stalled.
        assert!(!p.record_progress_at(true, 101, 101, at(20)));
        assert!(!p.record_progress_at(false, 200, 101, at(40)));
    }

    #[test]
    fn levels_have_hysteresis() {
        use Level::*;
        assert_eq!(Ok.next(849), Ok);
        assert_eq!(Ok.next(850), Full);
        assert_eq!(Full.next(810), Full, "stays full until 80%");
        assert_eq!(Full.next(799), Ok);
        assert_eq!(Full.next(920), Shedding);
        assert_eq!(Ok.next(940), Shedding, "a jump skips Full");
        assert_eq!(Shedding.next(890), Shedding, "stays shedding until 88%");
        assert_eq!(Shedding.next(870), Full, "then still full");
        assert_eq!(Shedding.next(500), Ok);
        assert_eq!(Shedding.next(950), Emergency);
        assert_eq!(Ok.next(990), Emergency, "a jump skips the steps");
        assert_eq!(Emergency.next(930), Emergency, "stays until 92%");
        assert_eq!(Emergency.next(919), Shedding, "then still shedding");
        assert_eq!(Emergency.next(100), Ok);
    }

    #[test]
    fn a_node_restarts_at_98_percent_or_after_30_s_of_emergency() {
        let p = Pressure::new(1000);
        let t0 = Instant::now();
        let at = |secs| t0 + Duration::from_secs(secs);
        p.record_own(960);
        assert!(!p.restart_due_at(true, 1, at(0)));
        assert!(!p.restart_due_at(true, 1, at(29)));
        // A dip below Emergency's clear starts the clock over.
        p.record_own(900);
        assert!(!p.restart_due_at(true, 1, at(30)));
        p.record_own(960);
        assert!(!p.restart_due_at(true, 1, at(31)));
        assert!(
            !p.restart_due_at(false, 1, at(61)),
            "due and announced, but not while a peer is gone"
        );
        assert!(p.own().pending, "the due restart is announced");
        assert!(p.restart_due_at(true, 1, at(64)));
        assert!(!p.restart_due_at(true, 1, at(65)), "once");
        let p = Pressure::new(1000);
        p.record_own(985);
        assert!(!p.restart_due_at(true, 1, t0), "at 98%, announced first");
        assert!(p.restart_due_at(true, 1, t0 + ANNOUNCE), "then restarts");
        assert_eq!(p.actions.get(&[("action", "emergency")]), 1);
    }

    #[test]
    fn replicas_hot_together_restart_one_at_a_time() {
        // Three replicas of one shard at 98% in the same second, each asking
        // the others every second: the review's race. Only the lowest id
        // restarts; the others wait while it is down and catching up.
        let nodes: Vec<(NodeId, Pressure)> = (1..=3).map(|id| (id, Pressure::new(1000))).collect();
        let t0 = Instant::now();
        let mut restarted = vec![];
        for tick in 0..20u64 {
            let now = t0 + Duration::from_secs(tick);
            for (_, p) in &nodes {
                p.record_own(990);
            }
            let readings: Vec<(NodeId, Reading)> =
                nodes.iter().map(|(id, p)| (*id, p.own())).collect();
            for (id, p) in &nodes {
                for (peer, r) in &readings {
                    if peer != id {
                        p.peer_flags.lock().insert(*peer, (*r, now));
                    }
                }
            }
            for (id, p) in &nodes {
                if p.restart_due_at(true, *id, now) {
                    restarted.push((*id, tick));
                }
            }
        }
        assert_eq!(
            restarted,
            vec![(1, 3)],
            "one replica, the lowest id, after the announcement"
        );
    }

    #[test]
    fn a_peer_catching_up_holds_the_restart() {
        let p = Pressure::new(1000);
        let t0 = Instant::now();
        p.record_own(990);
        let behind = Reading {
            rss: 100,
            limit: 1000,
            behind: true,
            ..Default::default()
        };
        p.peer_flags.lock().insert(2, (behind, t0));
        assert!(!p.restart_due_at(true, 1, t0));
        assert!(
            !p.restart_due_at(true, 1, t0 + ANNOUNCE),
            "a peer is catching up"
        );
        let caught_up = Reading {
            rss: 100,
            limit: 1000,
            ..Default::default()
        };
        p.peer_flags.lock().insert(2, (caught_up, t0 + ANNOUNCE));
        assert!(p.restart_due_at(true, 1, t0 + ANNOUNCE + Duration::from_secs(1)));
    }

    #[test]
    fn the_worst_fresh_node_decides() {
        let p = Pressure::new(1000);
        assert_eq!(p.record_own(500), Level::Ok);
        assert_eq!(
            p.record_peer(
                2,
                Reading {
                    rss: 900,
                    limit: 1000,
                    ..Default::default()
                }
            ),
            Level::Full
        );
        assert!(!p.refuses(Work::Session), "full only steers");
        assert!(!p.refuses(Work::Input));
        assert_eq!(
            p.record_peer(
                3,
                Reading {
                    rss: 930,
                    limit: 1000,
                    ..Default::default()
                }
            ),
            Level::Shedding
        );
        assert!(p.refuses(Work::Session));
        assert!(p.refuses(Work::Input));
        assert_eq!(p.shed_sessions.load(Ordering::Relaxed), 1);
        assert_eq!(p.shed_inputs.load(Ordering::Relaxed), 1);
        // A peer that stops answering stops counting.
        p.peers.lock().insert(3, (930, Instant::now() - PEER_STALE));
        p.peers.lock().insert(2, (500, Instant::now()));
        assert_eq!(p.record_own(500), Level::Ok);
    }

    #[test]
    fn container_limits_parse() {
        assert_eq!(parse_limit("12884901888\n"), Some(12 << 30));
        assert_eq!(parse_limit("max\n"), None);
        assert_eq!(parse_limit("9223372036854771712\n"), None);
        assert_eq!(limit_bytes(100, 0), 100 << 20);
    }
}

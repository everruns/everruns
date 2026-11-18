//! Memory pressure: how a cell takes on less work before a node runs out of
//! memory.
//!
//! Decision: a node past its container's memory limit is killed, its cell
//! loses a replica, and the node's rejoin (snapshot, log replay, catch-up)
//! needs more memory still: in EKS run 43747035567 one node went round that
//! loop five times. Unloading events (`Node::unload_loop`) frees what can
//! be freed; past that, the cell stops taking on work, in two steps:
//!
//! - **Full** (a node at 85% of its limit): the cell is closed to new
//!   sessions. Gateways place them in other cells, and the rebalancer moves
//!   idle sessions out to cells that are not full.
//! - **Shedding** (92%): the cell refuses new sessions and new input
//!   outright. Callers get 429 with a retry hint; turns already admitted
//!   keep running and finish, which is what frees memory again.
//!
//! At either level the cell also starts no more turns than its
//! `pressure_turns` floor (`Node::schedule`): input it already queued
//! waits. A slow cell otherwise draws the shared executor pool into itself,
//! since its turns hold executor credits longer (EKS run 44355318472: 17,000
//! running turns in a cell that lost a node, three nodes out of memory).
//!
//! Decision: Full only steers, it does not refuse. A gateway has already
//! written a session's directory entry when it creates the session on its
//! cell, so a refused create comes back to the same cell on retry; steering
//! by the load view (a second old at most) sends the next ones elsewhere
//! instead.
//!
//! The levels have hysteresis (clear at 80% and 88%), so a cell does not
//! flap at a boundary. The level is the cell's, not the node's: every node
//! holds every session of its cell, so the worst node decides. Each node
//! asks its peers every second; a peer that does not answer for 5 s (down,
//! or not started) does not count.
//!
//! A cell whose leader stops committing sheds the same way. Decision: in
//! EKS run 43774871232 three cells stopped committing for about 20 minutes
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

/// A peer reading older than this does not count.
const PEER_STALE: Duration = Duration::from_secs(5);
/// A leader whose applied index has not moved this long while it has
/// entries to apply has stalled.
pub const STALL_AFTER: Duration = Duration::from_secs(5);

/// How long a refused caller should wait before trying again. Long enough
/// for a few unload rounds and for running turns to finish; short enough
/// that a caller does not sit idle long once memory frees.
pub const RETRY_AFTER_MS: u64 = 2_000;

/// How close a cell's nodes are to their memory limits.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    #[default]
    Ok,
    /// No new sessions.
    Full,
    /// No new sessions and no new input.
    Shedding,
}

impl Level {
    fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Full,
            2 => Self::Shedding,
            _ => Self::Ok,
        }
    }

    /// The level after a reading of `permille`, from `self`.
    pub fn next(self, permille: u64) -> Self {
        if permille >= SHED_AT || (self == Self::Shedding && permille >= SHED_CLEAR) {
            Self::Shedding
        } else if permille >= FULL_AT || (self >= Self::Full && permille >= FULL_CLEAR) {
            Self::Full
        } else {
            Self::Ok
        }
    }
}

/// One node's reading, as `/cell/pressure` answers it.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Reading {
    pub rss: u64,
    pub limit: u64,
}

impl Reading {
    pub fn permille(&self) -> u64 {
        (self.rss.saturating_mul(1000)) / self.limit.max(1)
    }
}

/// New work a shedding cell refuses.
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
        }
    }

    pub fn own(&self) -> Reading {
        Reading {
            rss: self.own.load(Ordering::Relaxed),
            limit: self.limit,
        }
    }

    /// Record this node's resident set and recompute the cell's level.
    pub fn record_own(&self, rss: u64) -> Level {
        self.own.store(rss, Ordering::Relaxed);
        self.update()
    }

    pub fn record_peer(&self, node: NodeId, reading: Reading) -> Level {
        self.peers
            .lock()
            .insert(node, (reading.permille(), Instant::now()));
        self.update()
    }

    /// The worst fresh reading of the cell's nodes, in permille.
    pub fn cell_permille(&self) -> u64 {
        let own = self.own().permille();
        let peers = self.peers.lock();
        peers
            .values()
            .filter(|(_, at)| at.elapsed() < PEER_STALE)
            .map(|(p, _)| *p)
            .fold(own, u64::max)
    }

    fn update(&self) -> Level {
        let level = self.level().next(self.cell_permille());
        self.level.store(level as u8, Ordering::Relaxed);
        level
    }

    pub fn level(&self) -> Level {
        Level::from_u8(self.level.load(Ordering::Relaxed))
    }

    /// Record Raft's progress: whether this node leads, its last log index
    /// and its applied index. Returns whether the cell has stalled.
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

    /// The level the cell acts on: shedding while stalled.
    pub fn effective(&self) -> Level {
        if self.stalled() {
            Level::Shedding
        } else {
            self.level()
        }
    }

    /// Whether the cell refuses `work` now; counts the refusal.
    pub fn refuses(&self, work: Work) -> bool {
        let refused = self.effective() == Level::Shedding;
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
    /// retry a frozen session, so the loss window holds) or while the cell
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

    /// Whether the cell, short of memory, starts no more turns now.
    ///
    /// Decision: at Full or worse a cell starts no turns past
    /// `pressure_turns`. In EKS run 44355318472 a cell that lost a node to
    /// chaos committed slowly, so its turns held executor credits longer and
    /// the shared pool piled into it (2,600 to 17,000 running turns in four
    /// minutes, the other cells steady at 2,700); its other two nodes ran
    /// out of memory three times. Refusing new input (Shedding) did not stop
    /// it: the input was already queued. Below the floor the cell still
    /// makes progress, so memory that only finished turns free does get
    /// freed.
    pub(crate) fn holds_turns(&self, state: &crate::state::CellState) -> bool {
        self.pressure.effective() >= Level::Full
            && self.running_turns(state) >= self.cfg.pressure_turns
    }

    /// Every second: read this node's resident set and ask the cell's other
    /// members for theirs.
    pub(crate) async fn pressure_loop(self: Arc<Self>) {
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        let mut was = Level::Ok;
        let mut was_stalled = false;
        loop {
            tick.tick().await;
            self.pressure.record_own(crate::api::rss_mb() << 20);
            let stalled = {
                let metrics = self.raft.metrics().borrow().clone();
                self.pressure.record_progress(
                    metrics.state == openraft::ServerState::Leader,
                    metrics.last_log_index.unwrap_or(0),
                    metrics.last_applied.map_or(0, |id| id.index),
                )
            };
            if stalled != was_stalled {
                tracing::warn!(
                    cell = self.cfg.cell,
                    stalled,
                    "cell commit progress changed"
                );
                was_stalled = stalled;
            }
            let peers: Vec<(NodeId, String)> = self
                .members()
                .into_iter()
                .filter(|(id, _)| *id != self.cfg.node_id)
                .collect();
            let readings = futures::future::join_all(peers.into_iter().map(|(id, addr)| {
                let http = self.http.clone();
                async move {
                    let reading = http
                        .get(format!("http://{addr}/cell/pressure"))
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
            for (id, reading) in readings.into_iter().flatten() {
                level = self.pressure.record_peer(id, reading);
            }
            if level != was {
                tracing::warn!(
                    cell = self.cfg.cell,
                    ?was,
                    ?level,
                    permille = self.pressure.cell_permille(),
                    "cell memory pressure changed"
                );
                was = level;
            }
        }
    }
}

/// `GET /cell/pressure`: this node's own reading.
pub async fn reading(State(node): State<Arc<Node>>) -> Json<Reading> {
    Json(node.pressure.own())
}

pub fn write_metrics(p: &Pressure, w: &mut crate::prom::Writer) {
    w.gauge(
        "cell_memory_limit_bytes",
        "Memory limit memory pressure is measured against.",
        p.limit as f64,
    );
    w.gauge(
        "cell_memory_pressure",
        "The worst of the cell's nodes' resident set, as a share of its limit.",
        p.cell_permille() as f64 / 1000.0,
    );
    w.gauge(
        "cell_overload_level",
        "0: taking work, 1: full (no new sessions placed here), 2: shedding (new sessions and input refused).",
        p.effective() as u8 as f64,
    );
    w.gauge(
        "cell_commit_stalled",
        "1 while this node leads and has not applied an entry for 5 s with entries to apply.",
        f64::from(u8::from(p.stalled())),
    );
    w.counter(
        "cell_shed_sessions_total",
        "New sessions refused while shedding load.",
        p.shed_sessions.load(Ordering::Relaxed) as f64,
    );
    w.counter(
        "cell_shed_inputs_total",
        "New input refused while shedding load.",
        p.shed_inputs.load(Ordering::Relaxed) as f64,
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
        assert_eq!(Ok.next(950), Shedding, "a jump skips Full");
        assert_eq!(Shedding.next(890), Shedding, "stays shedding until 88%");
        assert_eq!(Shedding.next(870), Full, "then still full");
        assert_eq!(Shedding.next(500), Ok);
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
                    limit: 1000
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
                    limit: 1000
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

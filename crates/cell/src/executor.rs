//! An executor outside the cells (data plane v2 step 6, design.md
//! "Executors"): a stateless process that pulls activations from every cell
//! with credits and runs them with the Everruns Engine.
//!
//! Decisions:
//! - The executor runs Engine's activation against [`RaftHosting`] over a
//!   [`Remote`] link, the same authority code a cell node runs; everything
//!   it knows of a session is its mirror, refreshed by its own writes and by
//!   one long poll per running session.
//! - Free slots are split among the cells as credits, one pull in flight
//!   per cell; a cell returns credits it can't use, and they go back to the
//!   pool for any cell.
//! - Recently run sessions stay mirrored (an LRU, `mirror_sessions` deep),
//!   so a session that comes back here, as soft affinity prefers, starts
//!   warm; segment and blob reads go through the store's cache.
//! - Draining (a rolling deploy): stop pulling, ask every running epoch to
//!   hand off at its next step boundary, and exit once none runs. Another
//!   executor resumes each turn from the journal.

use std::collections::{HashSet, VecDeque};
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
use crate::state::{CellId, Placement};
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
    /// How long a cell may hold this executor's credits.
    pub pull_wait: Duration,
}

pub struct Executor {
    pub cfg: ExecutorConfig,
    pub remote: Arc<Remote>,
    pub hosting: RaftHosting,
    pub engine: everruns::Engine,
    free: AtomicUsize,
    freed: tokio::sync::Notify,
    running: Mutex<HashSet<SessionId>>,
    /// Sessions mirrored but not running, least recently run first.
    recent: Mutex<VecDeque<SessionId>>,
    draining: AtomicBool,
    pub stats: crate::prom::Counters,
}

impl Executor {
    pub fn start(cfg: ExecutorConfig, host: Arc<dyn Host>) -> anyhow::Result<Arc<Self>> {
        anyhow::ensure!(
            cfg.id >= crate::remote::EXECUTOR_IDS,
            "executor ids start at {}",
            crate::remote::EXECUTOR_IDS
        );
        let remote = Remote::new(cfg.id, cfg.store.clone())?;
        remote.set_cells(cfg.topology.read()?.cells);
        let hosting = RaftHosting::remote(remote.clone());
        let engine = crate::node::engine(hosting.clone(), host);
        Ok(Arc::new(Self {
            free: AtomicUsize::new(cfg.slots),
            cfg,
            remote,
            hosting,
            engine,
            freed: tokio::sync::Notify::new(),
            running: Mutex::default(),
            recent: Mutex::default(),
            draining: AtomicBool::new(false),
            stats: Default::default(),
        }))
    }

    /// Start pulling from every cell, and keep up with the topology.
    pub fn spawn(self: &Arc<Self>) {
        let executor = self.clone();
        tokio::spawn(async move {
            let mut pulling: HashSet<CellId> = HashSet::new();
            loop {
                if let Ok(topology) = executor.cfg.topology.read() {
                    executor.remote.set_cells(topology.cells);
                }
                for cell in executor.remote.cells() {
                    if pulling.insert(cell) {
                        tokio::spawn(executor.clone().puller(cell));
                    }
                }
                if executor.draining.load(Ordering::SeqCst) {
                    return;
                }
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        });
    }

    pub fn running(&self) -> usize {
        self.running.lock().len()
    }

    /// Stop taking work and hand running epochs back at their next step.
    pub fn drain(&self) {
        self.draining.store(true, Ordering::SeqCst);
        let running: Vec<SessionId> = self.running.lock().iter().copied().collect();
        for session in running {
            self.hosting.changed(session, true);
        }
    }

    /// Credits to offer one cell: an even share of the free slots.
    fn take_credits(&self) -> usize {
        let cells = self.remote.cells().len().max(1);
        let share = self.cfg.slots.div_ceil(cells).max(1);
        let mut free = self.free.load(Ordering::SeqCst);
        loop {
            let take = free.min(share);
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

    fn give_back(&self, credits: usize) {
        if credits > 0 {
            self.free.fetch_add(credits, Ordering::SeqCst);
            self.freed.notify_waiters();
        }
    }

    async fn puller(self: Arc<Self>, cell: CellId) {
        loop {
            if self.draining.load(Ordering::SeqCst) {
                return;
            }
            let credits = self.take_credits();
            if credits == 0 {
                let _ = tokio::time::timeout(Duration::from_millis(500), self.freed.notified()).await;
                continue;
            }
            match self.remote.pull(cell, credits, self.cfg.pull_wait).await {
                Ok(sessions) => {
                    let used = sessions.len().min(credits);
                    for session in sessions.into_iter().take(credits) {
                        self.stats.inc(&[("event", "pulled")]);
                        self.clone().run(cell, session);
                    }
                    self.give_back(credits - used);
                }
                Err(error) => {
                    self.give_back(credits);
                    tracing::debug!(cell, %error, "pull failed");
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
            }
        }
    }

    /// Run `session`'s activations here until it owes no more work. Holds
    /// one slot.
    fn run(self: Arc<Self>, cell: CellId, session: SessionId) {
        if !self.running.lock().insert(session) {
            self.give_back(1);
            return;
        }
        self.recent.lock().retain(|s| *s != session);
        self.remote.bind(session, cell);
        let watching = Arc::new(AtomicBool::new(true));
        tokio::spawn(self.clone().watch(session, watching.clone()));
        tokio::spawn(async move {
            for _ in 0..64 {
                let started = std::time::Instant::now();
                let outcome = match self.engine.activate(session).await {
                    Ok(()) => "completed",
                    Err(error) => {
                        tracing::warn!(%session, %error, "activation stopped");
                        "error"
                    }
                };
                self.stats.inc(&[("activation", outcome)]);
                self.stats
                    .add(&[("activation_ms", "sum")], started.elapsed().as_millis() as u64);
                if outcome != "completed" || self.draining.load(Ordering::SeqCst) {
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
            self.retire(session);
            self.give_back(1);
        });
    }

    /// Keep `session`'s mirror current while it runs: input, cancels and a
    /// freeze for a move reach Engine's waiters and hand-off flag.
    async fn watch(self: Arc<Self>, session: SessionId, watching: Arc<AtomicBool>) {
        while watching.load(Ordering::SeqCst) {
            match self.remote.watch(session, Duration::from_secs(10)).await {
                Ok(true) => {
                    let frozen = self
                        .remote
                        .shared
                        .state
                        .read()
                        .sessions
                        .get(&session)
                        .is_none_or(|s| s.placement == Placement::Frozen);
                    self.hosting.changed(session, frozen);
                }
                Ok(false) => {}
                Err(_) => tokio::time::sleep(Duration::from_millis(200)).await,
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
                    && !running.contains(&oldest)
                {
                    out.push(oldest);
                }
            }
            out
        };
        for session in evict {
            self.remote.evict(session);
            self.hosting.forget(session);
        }
    }
}

/// `/health` and `/metrics` of an executor.
pub fn router(executor: Arc<Executor>) -> axum::Router {
    use axum::routing::get;
    axum::Router::new()
        .route("/health", get(|| async { "ok" }))
        .route(
            "/metrics",
            get(|axum::extract::State(executor): axum::extract::State<Arc<Executor>>| async move {
                let id = executor.cfg.id.to_string();
                let mut w = crate::prom::Writer::new(&[("executor", &id)]);
                w.gauge("executor_running_turns", "Turns running here.", executor.running() as f64);
                w.gauge(
                    "executor_free_slots",
                    "Slots not running or offered.",
                    executor.free.load(Ordering::SeqCst) as f64,
                );
                w.gauge(
                    "executor_mirrored_sessions",
                    "Sessions in the mirror.",
                    executor.remote.mirrored() as f64,
                );
                let (hits, misses) = executor.cfg.store.cache_stats();
                w.counter("executor_cache_hits_total", "Segment and blob cache hits.", hits as f64);
                w.counter(
                    "executor_cache_misses_total",
                    "Segment and blob cache misses (object store reads).",
                    misses as f64,
                );
                w.counters("executor_cell_calls_total", "Calls to cells, by kind.", &executor.remote.calls);
                w.counters("executor_events_total", "Pulls and activations.", &executor.stats);
                w.out
            }),
        )
        .with_state(executor)
}

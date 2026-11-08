//! Turn admission: how many more session turns a worker leases now.
//!
//! Decisions:
//! - Turns do not take execution slots. A turn spends almost all of its time
//!   waiting: on the model's stream, on a sandbox or the network, on the
//!   control plane. It costs the worker memory, not a thread, so a count cap
//!   sized for CPU work (20 per worker on the hosted deployment) left workers
//!   idle while turns queued for pickup (2026-11-05 load test: pickup p99 28 s
//!   at 250 agents, worker CPU low). `MAX_CONCURRENT_TASKS` still caps the
//!   queued background activities, which do run as work.
//! - The ceiling is memory: [`TurnAdmission::from_env`] divides 85% of the
//!   worker's memory limit (cgroup, or `WORKER_MEMORY_LIMIT_MIB`) by an
//!   assumed [`ASSUMED_TURN_MEMORY`] per running turn, within
//!   [`MIN_AUTO_CEILING`]..[`MAX_AUTO_CEILING`]; `WORKER_MAX_RUNNING_TURNS`
//!   sets it outright. The per-turn figure is an estimate the engine
//!   simulator bench measures (`crates/server/benches/engine_sim.rs`).
//! - Two brakes stop leasing before the ceiling, once at least
//!   [`BRAKE_FLOOR`] turns run (below it a worker always makes progress):
//!   - the process's resident memory is over 85% of the limit, for turns far
//!     bigger than assumed (long contexts, large tool output);
//!   - the async runtime lags: a 50 ms timer fires more than
//!     `WORKER_MAX_RUNTIME_LAG_MS` (25 ms) late, on a moving average. CPU work
//!     inside turns (Bashkit, in-process interpreters, big prompt assembly)
//!     shows up here before it shows anywhere else, so it holds back new turns
//!     without a count cap that would also hold back the waiting ones.
//! - Neither brake stops a running turn; they only stop new leases. Running
//!   turns finish, which frees memory and runtime, and the lease loop polls
//!   again within a second.
//! - The control plane has its own brake: a lease call takes nothing while
//!   its database pool has no free connection (`request_pool_exhausted` in
//!   the server's session actor repository).

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tracing::{debug, info};

/// Memory one running turn is assumed to hold, for the automatic ceiling.
pub const ASSUMED_TURN_MEMORY: u64 = 2 * 1024 * 1024;
/// Share of the memory limit turns may fill, for the ceiling and the brake.
pub const MEMORY_HEADROOM: f64 = 0.85;
/// Bounds of the automatic ceiling.
pub const MIN_AUTO_CEILING: usize = 100;
pub const MAX_AUTO_CEILING: usize = 10_000;
/// Ceiling when the memory limit is unknown (no cgroup, not Linux).
pub const UNKNOWN_LIMIT_CEILING: usize = 2_000;
/// Running turns below which the brakes never hold back a lease.
pub const BRAKE_FLOOR: usize = 32;
/// Default for `WORKER_MAX_RUNTIME_LAG_MS`.
pub const DEFAULT_MAX_RUNTIME_LAG: Duration = Duration::from_millis(25);
/// The lag probe's timer.
const LAG_PROBE_INTERVAL: Duration = Duration::from_millis(50);
/// How long a resident-memory reading is reused.
const RSS_SAMPLE_TTL: Duration = Duration::from_millis(250);

/// Why the worker leases fewer turns than its ceiling allows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Brake {
    Memory,
    RuntimeLag,
}

/// Settings of [`TurnAdmission`].
#[derive(Debug, Clone, Copy)]
pub struct AdmissionConfig {
    /// Most turns running at once.
    pub ceiling: usize,
    /// The worker's memory limit in bytes, when known.
    pub memory_limit: Option<u64>,
    /// Lag past which no new turn is leased; zero turns the brake off.
    pub max_runtime_lag: Duration,
}

impl AdmissionConfig {
    /// From `WORKER_MAX_RUNNING_TURNS`, `WORKER_MEMORY_LIMIT_MIB` (else the
    /// cgroup limit) and `WORKER_MAX_RUNTIME_LAG_MS`.
    pub fn from_env() -> Self {
        use crate::core::config::{env_duration_ms, env_or};
        let memory_limit = match env_or("WORKER_MEMORY_LIMIT_MIB", 0u64) {
            0 => cgroup_memory_limit(),
            mib => Some(mib * 1024 * 1024),
        };
        let ceiling = match env_or("WORKER_MAX_RUNNING_TURNS", 0usize) {
            0 => auto_ceiling(memory_limit),
            ceiling => ceiling,
        };
        Self {
            ceiling,
            memory_limit,
            max_runtime_lag: env_duration_ms("WORKER_MAX_RUNTIME_LAG_MS", DEFAULT_MAX_RUNTIME_LAG),
        }
    }

    /// A fixed ceiling with both brakes off, for tests and embedded workers.
    pub fn fixed(ceiling: usize) -> Self {
        Self {
            ceiling,
            memory_limit: None,
            max_runtime_lag: Duration::ZERO,
        }
    }
}

/// The ceiling for a worker with `memory_limit` bytes. See the module notes.
pub fn auto_ceiling(memory_limit: Option<u64>) -> usize {
    match memory_limit {
        Some(limit) => {
            let turns = (limit as f64 * MEMORY_HEADROOM / ASSUMED_TURN_MEMORY as f64) as usize;
            turns.clamp(MIN_AUTO_CEILING, MAX_AUTO_CEILING)
        }
        None => UNKNOWN_LIMIT_CEILING,
    }
}

/// Counts a worker's running turns and decides how many more it leases.
#[derive(Clone)]
pub struct TurnAdmission {
    running: Arc<AtomicUsize>,
    config: AdmissionConfig,
    memory: Arc<dyn MemoryGauge>,
    lag: Arc<RuntimeLag>,
}

impl TurnAdmission {
    pub fn new(config: AdmissionConfig) -> Self {
        Self::with_gauge(config, Arc::new(ProcessMemory::default()))
    }

    pub fn with_gauge(config: AdmissionConfig, memory: Arc<dyn MemoryGauge>) -> Self {
        info!(
            ceiling = config.ceiling,
            memory_limit_mib = config.memory_limit.map(|limit| limit / (1024 * 1024)),
            max_runtime_lag_ms = config.max_runtime_lag.as_millis() as u64,
            "Turn admission"
        );
        Self {
            running: Arc::new(AtomicUsize::new(0)),
            config,
            memory,
            lag: Arc::new(RuntimeLag::default()),
        }
    }

    pub fn config(&self) -> AdmissionConfig {
        self.config
    }

    /// Turns running now.
    pub fn running(&self) -> usize {
        self.running.load(Ordering::SeqCst)
    }

    /// The runtime lag the brake reads (a moving average).
    pub fn runtime_lag(&self) -> Duration {
        self.lag.current()
    }

    pub fn lag_probe(&self) -> Arc<RuntimeLag> {
        self.lag.clone()
    }

    /// How many more turns to lease now.
    pub fn room(&self) -> usize {
        let running = self.running();
        let room = self.config.ceiling.saturating_sub(running);
        if room == 0 {
            return 0;
        }
        if let Some(brake) = self.brake(running) {
            debug!(running, ?brake, "Turn admission holds back new turns");
            return 0;
        }
        room
    }

    /// The brake that holds back new turns at `running`, if any.
    pub fn brake(&self, running: usize) -> Option<Brake> {
        if running < BRAKE_FLOOR {
            return None;
        }
        if let (Some(limit), Some(resident)) = (self.config.memory_limit, self.memory.resident())
            && resident as f64 > limit as f64 * MEMORY_HEADROOM
        {
            return Some(Brake::Memory);
        }
        if !self.config.max_runtime_lag.is_zero()
            && self.lag.current() > self.config.max_runtime_lag
        {
            return Some(Brake::RuntimeLag);
        }
        None
    }

    /// Count a turn as running until the guard drops.
    pub fn enter(&self) -> RunningTurn {
        self.running.fetch_add(1, Ordering::SeqCst);
        RunningTurn(self.running.clone())
    }
}

/// One running turn, counted until dropped.
pub struct RunningTurn(Arc<AtomicUsize>);

impl Drop for RunningTurn {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// How late the runtime runs a timer, as a moving average.
#[derive(Default)]
pub struct RuntimeLag {
    micros: AtomicU64,
}

impl RuntimeLag {
    pub fn current(&self) -> Duration {
        Duration::from_micros(self.micros.load(Ordering::Relaxed))
    }

    /// Fold in one observed lateness: a fifth of the new reading.
    pub fn observe(&self, late: Duration) {
        let late = late.as_micros().min(u128::from(u64::MAX)) as u64;
        let before = self.micros.load(Ordering::Relaxed);
        self.micros
            .store(before - before / 5 + late / 5, Ordering::Relaxed);
    }

    /// Measure the runtime until the task is aborted.
    pub async fn probe(self: Arc<Self>) {
        loop {
            let start = tokio::time::Instant::now();
            tokio::time::sleep(LAG_PROBE_INTERVAL).await;
            self.observe(start.elapsed().saturating_sub(LAG_PROBE_INTERVAL));
        }
    }
}

/// The worker process's resident memory.
pub trait MemoryGauge: Send + Sync {
    /// Resident bytes now, when the platform reports them.
    fn resident(&self) -> Option<u64>;
}

/// [`MemoryGauge`] over `/proc/self/status`, sampled at most every
/// [`RSS_SAMPLE_TTL`].
#[derive(Default)]
pub struct ProcessMemory {
    sample: Mutex<Option<(Instant, Option<u64>)>>,
}

impl MemoryGauge for ProcessMemory {
    fn resident(&self) -> Option<u64> {
        let mut sample = self.sample.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((at, value)) = *sample
            && at.elapsed() < RSS_SAMPLE_TTL
        {
            return value;
        }
        let value = process_resident_memory();
        *sample = Some((Instant::now(), value));
        value
    }
}

/// `VmRSS` of `/proc/self/status`, in bytes.
pub fn process_resident_memory() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    parse_vm_rss(&status)
}

fn parse_vm_rss(status: &str) -> Option<u64> {
    let line = status.lines().find(|line| line.starts_with("VmRSS:"))?;
    let kib: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kib * 1024)
}

/// The memory limit of the worker's cgroup (v2, then v1), when it has one.
pub fn cgroup_memory_limit() -> Option<u64> {
    let v2 = std::fs::read_to_string("/sys/fs/cgroup/memory.max").ok();
    let v1 = || std::fs::read_to_string("/sys/fs/cgroup/memory/memory.limit_in_bytes").ok();
    v2.or_else(v1).as_deref().and_then(parse_memory_limit)
}

/// A cgroup memory limit file's value: `max` (v2) or a page-rounded huge
/// number (v1) mean no limit.
fn parse_memory_limit(raw: &str) -> Option<u64> {
    let raw = raw.trim();
    if raw == "max" {
        return None;
    }
    raw.parse::<u64>().ok().filter(|limit| *limit < (1 << 60))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixed(Option<u64>);

    impl MemoryGauge for Fixed {
        fn resident(&self) -> Option<u64> {
            self.0
        }
    }

    const GIB: u64 = 1024 * 1024 * 1024;

    fn admission(resident: Option<u64>) -> TurnAdmission {
        TurnAdmission::with_gauge(
            AdmissionConfig {
                ceiling: 1000,
                memory_limit: Some(GIB),
                max_runtime_lag: Duration::from_millis(25),
            },
            Arc::new(Fixed(resident)),
        )
    }

    fn run(admission: &TurnAdmission, turns: usize) -> Vec<RunningTurn> {
        (0..turns).map(|_| admission.enter()).collect()
    }

    #[test]
    fn the_ceiling_follows_the_memory_limit() {
        assert_eq!(auto_ceiling(Some(6 * GIB)), 2611, "a 6 GiB hosted pod");
        assert_eq!(auto_ceiling(Some(GIB / 8)), MIN_AUTO_CEILING);
        assert_eq!(auto_ceiling(Some(1024 * GIB)), MAX_AUTO_CEILING);
        assert_eq!(auto_ceiling(None), UNKNOWN_LIMIT_CEILING);
    }

    #[test]
    fn room_is_what_the_ceiling_leaves() {
        let admission = admission(Some(GIB / 10));
        assert_eq!(admission.room(), 1000);
        let turns = run(&admission, 990);
        assert_eq!(admission.room(), 10);
        let more = run(&admission, 10);
        assert_eq!(admission.room(), 0, "a full worker leases nothing");
        drop(more);
        drop(turns);
        assert_eq!(admission.running(), 0, "a finished turn frees its place");
    }

    #[test]
    fn high_memory_holds_back_new_turns_past_the_floor() {
        let admission = admission(Some(GIB - GIB / 10));
        let few = run(&admission, BRAKE_FLOOR - 1);
        assert!(
            admission.room() > 0,
            "below the floor a worker always leases"
        );
        let _more = run(&admission, 1);
        assert_eq!(admission.brake(admission.running()), Some(Brake::Memory));
        assert_eq!(admission.room(), 0);
        drop(few);
        assert!(admission.room() > 0);
    }

    #[test]
    fn runtime_lag_holds_back_new_turns() {
        let admission = admission(Some(GIB / 10));
        let _turns = run(&admission, BRAKE_FLOOR);
        for _ in 0..20 {
            admission.lag.observe(Duration::from_millis(100));
        }
        assert_eq!(admission.brake(BRAKE_FLOOR), Some(Brake::RuntimeLag));
        assert_eq!(admission.room(), 0);
        for _ in 0..40 {
            admission.lag.observe(Duration::ZERO);
        }
        assert_eq!(admission.brake(BRAKE_FLOOR), None, "the average decays");
    }

    #[test]
    fn a_fixed_ceiling_has_no_brakes() {
        let admission =
            TurnAdmission::with_gauge(AdmissionConfig::fixed(64), Arc::new(Fixed(Some(u64::MAX))));
        admission.lag.observe(Duration::from_secs(10));
        let _turns = run(&admission, 40);
        assert_eq!(admission.room(), 24);
    }

    #[test]
    fn memory_files_parse() {
        assert_eq!(parse_memory_limit("max\n"), None);
        assert_eq!(parse_memory_limit("6442450944\n"), Some(6 * GIB));
        assert_eq!(
            parse_memory_limit("9223372036854771712"),
            None,
            "v1 unlimited"
        );
        let status = "Name:\tworker\nVmPeak:\t  100 kB\nVmRSS:\t  2048 kB\n";
        assert_eq!(parse_vm_rss(status), Some(2 * 1024 * 1024));
        assert_eq!(parse_vm_rss("Name:\tworker\n"), None);
    }

    #[tokio::test(start_paused = true)]
    async fn an_idle_runtime_reads_no_lag() {
        let lag = Arc::new(RuntimeLag::default());
        let probe = tokio::spawn(lag.clone().probe());
        tokio::time::sleep(Duration::from_secs(2)).await;
        probe.abort();
        assert!(lag.current() < Duration::from_millis(5));
    }
}

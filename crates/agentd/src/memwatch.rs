//! The container's memory as the kernel counts it, and the out-of-memory
//! kills it already did to this node.
//!
//! Decisions:
//! - The kernel kills a container on its cgroup's usage, page cache
//!   included, and reclaims inactive file pages before it does. So the
//!   figure that predicts a kill is `memory.current` less `inactive_file`
//!   (`working_bytes`), not the process's resident set.
//! - A killed node cannot count its own kill, and its cgroup's
//!   `memory.events` starts over with the restarted container. So every
//!   second the node writes its working bytes and limit to `memory.json` in
//!   its data directory; a node that starts and finds the last reading at
//!   95% of the limit or more counts the restart as an out-of-memory kill.
//!   The count lives in the same file, so it survives the restarts it
//!   counts. A clean stop writes nothing special: one at 95% would be
//!   counted too, which errs toward reporting.
//! - A node that restarts itself before the kernel would kill it
//!   (`pressure`, level 4) marks its last heartbeat `restarting`, so the
//!   next start counts a controlled restart, not a kill.
//! - One reading per process: the container's memory is the process's. In
//!   a fleet test, several nodes share a process and the last one started
//!   sets the count, which no test reads.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Out-of-memory kills of this node's data directory, across restarts.
static OOM_KILLS: AtomicU64 = AtomicU64::new(0);
/// Controlled restarts of this node's data directory, across restarts.
static RESTARTS: AtomicU64 = AtomicU64::new(0);
/// Set once this process is restarting itself: heartbeats stop, so the
/// marked last one stays.
static RESTARTING: AtomicBool = AtomicBool::new(false);
/// Serializes heartbeat writes with [`note_restart`].
static WRITING: parking_lot::Mutex<()> = parking_lot::Mutex::new(());

/// A last reading at or above this share of the limit means the exit that
/// followed was the kernel's kill.
const KILLED_AT: f64 = 0.95;

/// What `memory.json` holds.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, Default, PartialEq)]
pub struct Heartbeat {
    /// Unix seconds of the reading.
    pub at: u64,
    pub working_bytes: u64,
    pub limit_bytes: u64,
    /// Out-of-memory kills counted so far.
    pub oom_kills: u64,
    /// Controlled restarts counted so far.
    #[serde(default)]
    pub restarts: u64,
    /// The run that wrote this restarted itself.
    #[serde(default)]
    pub restarting: bool,
}

/// The container's usage less what the kernel reclaims before killing,
/// and its limit (0 when it has none or this is not cgroup v2).
pub fn working() -> (u64, u64) {
    let file = |p: &str| std::fs::read_to_string(p).unwrap_or_default();
    (
        working_bytes(
            &file("/sys/fs/cgroup/memory.current"),
            &file("/sys/fs/cgroup/memory.stat"),
        ),
        file("/sys/fs/cgroup/memory.max")
            .trim()
            .parse()
            .unwrap_or(0),
    )
}

/// This process's resident set size, from `/proc` (0 elsewhere).
pub(crate) fn rss_mb() -> u64 {
    std::fs::read_to_string("/proc/self/statm")
        .ok()
        .and_then(|s| s.split_whitespace().nth(1)?.parse::<u64>().ok())
        .map(|pages| pages * 4096 / (1024 * 1024))
        .unwrap_or(0)
}

fn working_bytes(current: &str, stat: &str) -> u64 {
    let current: u64 = current.trim().parse().unwrap_or(0);
    let inactive = stat
        .lines()
        .find_map(|l| l.strip_prefix("inactive_file "))
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(0);
    current.saturating_sub(inactive)
}

/// A process's memory against its limit, read the way the kernel counts
/// it: the container's working bytes against the container's limit. A
/// limit of 0 means none.
///
/// Decision: a `configured` limit (bytes) is the process's own, so it is
/// compared with the process's resident set. Several processes can share
/// one container (a local fleet, the soak's nodes and executors), and the
/// container's working bytes would then count them all against each one's
/// limit. Without one, the container's figures decide, the resident set
/// standing in outside a container.
pub fn reading(configured: u64) -> (u64, u64) {
    if configured > 0 {
        return (rss_mb() << 20, configured);
    }
    let (working, container) = working();
    if container > 0 {
        (working, container)
    } else {
        (rss_mb() << 20, 0)
    }
}

/// What a turn reserves of its executor's memory before it starts.
pub const TURN_BASE: u64 = 8 << 20;
/// Permille of the limit turns may hold together; the rest is headroom
/// and the shard mirrors.
const TURNS_SHARE: u64 = 750;
/// Permille of the limit at which an executor hands its newest turns
/// back and starts none, and at which it drains and restarts.
const STOP_AT: u64 = 950;
const RESTART_AT: u64 = 980;

/// What an executor does about its memory now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ExecLevel {
    Ok,
    /// Start nothing; hand the newest running turns back (they resume on
    /// another executor from the session's log).
    Stop,
    /// Drain and exit with [`RESTART_EXIT`]; Kubernetes restarts it.
    Restart,
}

/// Exit code of an executor that restarted itself before the kernel would
/// have killed it.
pub const RESTART_EXIT: i32 = 75;

pub fn exec_level(used: u64, limit: u64) -> ExecLevel {
    if limit == 0 {
        return ExecLevel::Ok;
    }
    let permille = used.saturating_mul(1000) / limit;
    if permille >= RESTART_AT {
        ExecLevel::Restart
    } else if permille >= STOP_AT {
        ExecLevel::Stop
    } else {
        ExecLevel::Ok
    }
}

/// Turns an executor may start with what its memory has left: the turns'
/// share of the limit less what is in use, in [`TURN_BASE`] units. No
/// limit: no bound.
///
/// Decision: an executor takes turns by memory, not by slot count. In EKS
/// run 44355318472 a slow shard pulled every free slot into itself after a
/// node kill; with memory credits a turn that holds more leaves fewer
/// slots to fill.
pub fn turn_credits(used: u64, limit: u64) -> usize {
    if limit == 0 {
        return usize::MAX;
    }
    let turns = (u128::from(limit) * u128::from(TURNS_SHARE) / 1000) as u64;
    (turns.saturating_sub(used) / TURN_BASE) as usize
}

/// Permille of the limit above which an executor drops its coldest idle
/// mirrors, an eighth of them each second.
const MIRROR_TRIM_AT: u64 = 500;

/// How many idle mirrors to drop now, of `idle` kept.
///
/// Decision (G1, 2026-11-26): the mirror of finished sessions is a warm
/// cache, capped by a count (`--mirror-sessions`), not by bytes. Coding
/// sessions keep about 3.4 MB each, so 1,500 finished sessions held 5 GB of
/// an 8 GiB executor, past the turns' share: it offered no credits and the
/// run stalled with nothing running. Past half the limit the coldest go
/// first; a session that comes back pulls its state again.
pub fn mirror_trim(used: u64, limit: u64, idle: usize) -> usize {
    if limit == 0 || used.saturating_mul(1000) / limit < MIRROR_TRIM_AT {
        return 0;
    }
    idle.div_ceil(8)
}

/// Out-of-memory kills counted for this node, across its restarts.
pub fn oom_kills() -> u64 {
    OOM_KILLS.load(Ordering::Relaxed)
}

/// Count the previous exit from the last heartbeat: a kill when it was at
/// [`KILLED_AT`] of the limit or more. Returns the updated heartbeat.
pub fn previous_exit(last: Option<Heartbeat>) -> Heartbeat {
    let Some(last) = last else {
        return Heartbeat::default();
    };
    let killed = !last.restarting
        && last.limit_bytes > 0
        && last.working_bytes as f64 >= last.limit_bytes as f64 * KILLED_AT;
    if killed {
        tracing::warn!(
            working_mb = last.working_bytes >> 20,
            limit_mb = last.limit_bytes >> 20,
            "the previous run stopped at its memory limit: counted as an out-of-memory kill"
        );
    }
    Heartbeat {
        oom_kills: last.oom_kills + u64::from(killed),
        restarting: false,
        ..last
    }
}

/// Controlled restarts counted for this node, across its restarts.
pub fn restarts() -> u64 {
    RESTARTS.load(Ordering::Relaxed)
}

/// Mark the last heartbeat in `data_dir` as a controlled restart and stop
/// writing heartbeats; the process exits next.
pub fn note_restart(data_dir: &Path) {
    let _one = WRITING.lock();
    RESTARTING.store(true, Ordering::Relaxed);
    let mut beat = read(data_dir).unwrap_or_default();
    beat.restarts += 1;
    beat.restarting = true;
    RESTARTS.store(beat.restarts, Ordering::Relaxed);
    write(data_dir, &beat);
}

fn path(data_dir: &Path) -> PathBuf {
    data_dir.join("memory.json")
}

fn read(data_dir: &Path) -> Option<Heartbeat> {
    serde_json::from_slice(&std::fs::read(path(data_dir)).ok()?).ok()
}

fn write(data_dir: &Path, beat: &Heartbeat) {
    let tmp = data_dir.join("memory.json.tmp");
    if let Ok(bytes) = serde_json::to_vec(beat)
        && std::fs::write(&tmp, bytes).is_ok()
    {
        let _ = std::fs::rename(tmp, path(data_dir));
    }
}

/// Count the previous run's exit, then write a heartbeat every second for
/// the next start to read.
pub fn spawn(data_dir: PathBuf) {
    let mut beat = previous_exit(read(&data_dir));
    OOM_KILLS.store(beat.oom_kills, Ordering::Relaxed);
    RESTARTS.store(beat.restarts, Ordering::Relaxed);
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        loop {
            tick.tick().await;
            let (working_bytes, limit_bytes) = working();
            beat = Heartbeat {
                at: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_secs()),
                working_bytes,
                limit_bytes,
                ..beat
            };
            let _one = WRITING.lock();
            if RESTARTING.load(Ordering::Relaxed) {
                return;
            }
            write(&data_dir, &beat);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn working_bytes_leave_out_inactive_file_pages() {
        let stat = "anon 700\nfile 400\nactive_file 100\ninactive_file 300\n";
        assert_eq!(working_bytes("1100\n", stat), 800);
        assert_eq!(working_bytes("", ""), 0);
        // A stat that reports more inactive pages than usage (a race
        // between the two reads) does not wrap.
        assert_eq!(working_bytes("100", "inactive_file 300"), 0);
    }

    #[test]
    fn an_executor_takes_turns_by_memory() {
        let gib = 1u64 << 30;
        assert_eq!(turn_credits(0, 0), usize::MAX, "no limit, no bound");
        // 4 GiB: turns may hold 3 GiB, 1 GiB in use leaves 2 GiB.
        assert_eq!(turn_credits(gib, 4 * gib), (2 * gib / TURN_BASE) as usize);
        assert_eq!(turn_credits(3 * gib, 4 * gib), 0);
        assert_eq!(exec_level(900, 1000), ExecLevel::Ok);
        assert_eq!(exec_level(950, 1000), ExecLevel::Stop);
        assert_eq!(exec_level(980, 1000), ExecLevel::Restart);
        assert_eq!(exec_level(5000, 0), ExecLevel::Ok);
    }

    #[test]
    fn an_executor_drops_cold_mirrors_past_half_its_limit() {
        assert_eq!(mirror_trim(900, 0, 100), 0, "no limit, keep them");
        assert_eq!(mirror_trim(499, 1000, 100), 0);
        assert_eq!(mirror_trim(500, 1000, 100), 13);
        assert_eq!(mirror_trim(700, 1000, 1), 1);
        assert_eq!(mirror_trim(700, 1000, 0), 0);
    }

    #[test]
    fn a_run_that_stopped_at_its_limit_counts_as_a_kill() {
        let at = |working, kills| Heartbeat {
            at: 1,
            working_bytes: working,
            limit_bytes: 1000,
            oom_kills: kills,
            ..Heartbeat::default()
        };
        assert_eq!(previous_exit(Some(at(960, 2))).oom_kills, 3);
        assert_eq!(previous_exit(Some(at(940, 2))).oom_kills, 2);
        // A run that restarted itself at its limit was not killed.
        let restarted = previous_exit(Some(Heartbeat {
            restarting: true,
            restarts: 1,
            ..at(990, 2)
        }));
        assert_eq!((restarted.oom_kills, restarted.restarts), (2, 1));
        assert!(!restarted.restarting, "the next run starts unmarked");
        // No limit: nothing to compare with.
        let unlimited = Heartbeat {
            limit_bytes: 0,
            ..at(960, 0)
        };
        assert_eq!(previous_exit(Some(unlimited)).oom_kills, 0);
        // First start.
        assert_eq!(previous_exit(None), Heartbeat::default());
    }

    #[test]
    fn the_count_survives_restarts() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            &Heartbeat {
                at: 1,
                working_bytes: 990,
                limit_bytes: 1000,
                oom_kills: 4,
                ..Heartbeat::default()
            },
        );
        let beat = previous_exit(read(dir.path()));
        assert_eq!(beat.oom_kills, 5);
        write(dir.path(), &beat);
        // A restart after a calm run keeps the count.
        write(
            dir.path(),
            &Heartbeat {
                working_bytes: 100,
                ..beat
            },
        );
        assert_eq!(previous_exit(read(dir.path())).oom_kills, 5);
    }
}

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
//! - One reading per process: the container's memory is the process's. In
//!   a fleet test, several nodes share a process and the last one started
//!   sets the count, which no test reads.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Out-of-memory kills of this node's data directory, across restarts.
static OOM_KILLS: AtomicU64 = AtomicU64::new(0);

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

fn working_bytes(current: &str, stat: &str) -> u64 {
    let current: u64 = current.trim().parse().unwrap_or(0);
    let inactive = stat
        .lines()
        .find_map(|l| l.strip_prefix("inactive_file "))
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(0);
    current.saturating_sub(inactive)
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
    let killed =
        last.limit_bytes > 0 && last.working_bytes as f64 >= last.limit_bytes as f64 * KILLED_AT;
    if killed {
        tracing::warn!(
            working_mb = last.working_bytes >> 20,
            limit_mb = last.limit_bytes >> 20,
            "the previous run stopped at its memory limit: counted as an out-of-memory kill"
        );
    }
    Heartbeat {
        oom_kills: last.oom_kills + u64::from(killed),
        ..last
    }
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
                oom_kills: beat.oom_kills,
            };
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
    fn a_run_that_stopped_at_its_limit_counts_as_a_kill() {
        let at = |working, kills| Heartbeat {
            at: 1,
            working_bytes: working,
            limit_bytes: 1000,
            oom_kills: kills,
        };
        assert_eq!(previous_exit(Some(at(960, 2))).oom_kills, 3);
        assert_eq!(previous_exit(Some(at(940, 2))).oom_kills, 2);
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

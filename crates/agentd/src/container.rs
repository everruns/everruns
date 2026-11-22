//! This container's CPU and memory, and its machine's, read from cgroup v2
//! and `/proc` (`cell-testnode usage`).
//!
//! Decisions:
//! - Load runs need CPU and memory for every pod (cells, executors,
//!   drivers) and the machine, and the cluster runs no metrics-server. The
//!   image has no shell, so the binary reads the files itself and a sampler
//!   runs it with `kubectl exec` in each pod.
//! - Counters, not rates: the reading is a snapshot of cumulative CPU time,
//!   and the sampler divides the difference of two readings by the time
//!   between them. One reading can then never misreport a rate.
//! - `/proc/stat` and `/proc/meminfo` are not namespaced, so any container
//!   on a machine reads that machine's totals.

use serde::{Deserialize, Serialize};

/// One reading. Fields a host does not have (no cgroup v2, not Linux) are 0.
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq)]
pub struct Usage {
    /// CPU time this container used since it started, in microseconds.
    pub cpu_usec: u64,
    /// The container's CPU limit in cores; 0 for none.
    pub cpu_limit_cores: f64,
    pub memory_bytes: u64,
    /// The most the container has held since it started; 0 when the kernel
    /// does not report it.
    pub memory_peak_bytes: u64,
    /// 0 for no limit.
    pub memory_limit_bytes: u64,
    /// The machine's busy and total CPU time since boot, in clock ticks.
    pub machine_cpu_busy_ticks: u64,
    pub machine_cpu_total_ticks: u64,
    pub machine_cpus: u64,
    pub machine_memory_bytes: u64,
    pub machine_memory_available_bytes: u64,
}

/// Read the current usage. Missing files leave their fields at 0.
pub fn read() -> Usage {
    let file = |path: &str| std::fs::read_to_string(path).unwrap_or_default();
    let mut usage = Usage {
        cpu_usec: cpu_usec(&file("/sys/fs/cgroup/cpu.stat")),
        cpu_limit_cores: cpu_limit(&file("/sys/fs/cgroup/cpu.max")),
        memory_bytes: number(&file("/sys/fs/cgroup/memory.current")),
        memory_peak_bytes: number(&file("/sys/fs/cgroup/memory.peak")),
        memory_limit_bytes: number(&file("/sys/fs/cgroup/memory.max")),
        ..Usage::default()
    };
    machine_cpu(&file("/proc/stat"), &mut usage);
    machine_memory(&file("/proc/meminfo"), &mut usage);
    usage
}

fn number(text: &str) -> u64 {
    text.trim().parse().unwrap_or(0)
}

fn cpu_usec(stat: &str) -> u64 {
    stat.lines()
        .find_map(|line| line.strip_prefix("usage_usec "))
        .map_or(0, number)
}

/// `cpu.max` is `<quota> <period>`, or `max <period>` for no limit.
fn cpu_limit(max: &str) -> f64 {
    let mut parts = max.split_whitespace();
    let (Some(quota), Some(period)) = (parts.next(), parts.next()) else {
        return 0.0;
    };
    match (quota.parse::<f64>(), period.parse::<f64>()) {
        (Ok(quota), Ok(period)) if period > 0.0 => quota / period,
        _ => 0.0,
    }
}

/// The aggregate `cpu` line: user nice system idle iowait irq softirq
/// steal ...; idle and iowait are the idle time.
fn machine_cpu(stat: &str, usage: &mut Usage) {
    for line in stat.lines() {
        let mut fields = line.split_whitespace();
        match fields.next() {
            Some("cpu") => {
                let ticks: Vec<u64> = fields.take(8).map(number).collect();
                let total: u64 = ticks.iter().sum();
                let idle = ticks.get(3).copied().unwrap_or(0) + ticks.get(4).copied().unwrap_or(0);
                usage.machine_cpu_total_ticks = total;
                usage.machine_cpu_busy_ticks = total.saturating_sub(idle);
            }
            Some(name) if name.starts_with("cpu") => usage.machine_cpus += 1,
            _ => {}
        }
    }
}

fn machine_memory(meminfo: &str, usage: &mut Usage) {
    let kib = |key: &str| {
        meminfo
            .lines()
            .find_map(|line| line.strip_prefix(key))
            .and_then(|rest| rest.split_whitespace().next())
            .map_or(0, number)
            * 1024
    };
    usage.machine_memory_bytes = kib("MemTotal:");
    usage.machine_memory_available_bytes = kib("MemAvailable:");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cgroup_files_parse() {
        assert_eq!(cpu_usec("usage_usec 1234\nuser_usec 1000\n"), 1234);
        assert_eq!(cpu_usec(""), 0);
        assert_eq!(cpu_limit("150000 100000\n"), 1.5);
        assert_eq!(cpu_limit("max 100000\n"), 0.0);
        assert_eq!(cpu_limit(""), 0.0);
        assert_eq!(number("12884901888\n"), 12_884_901_888);
        assert_eq!(number("max\n"), 0);
    }

    #[test]
    fn machine_files_parse() {
        let mut usage = Usage::default();
        machine_cpu(
            "cpu  100 5 50 800 20 1 2 3 0 0\ncpu0 50 2 25 400 10 0 1 1 0 0\n\
             cpu1 50 3 25 400 10 1 1 2 0 0\nintr 1 2 3\n",
            &mut usage,
        );
        assert_eq!(usage.machine_cpu_total_ticks, 981);
        assert_eq!(usage.machine_cpu_busy_ticks, 161);
        assert_eq!(usage.machine_cpus, 2);
        machine_memory(
            "MemTotal:       196608000 kB\nMemFree: 1000 kB\nMemAvailable:   98304000 kB\n",
            &mut usage,
        );
        assert_eq!(usage.machine_memory_bytes, 196_608_000 * 1024);
        assert_eq!(usage.machine_memory_available_bytes, 98_304_000 * 1024);
    }

    #[test]
    fn reading_this_host_does_not_fail() {
        let usage = read();
        // Every Linux machine has CPUs and memory; elsewhere all is zero.
        if cfg!(target_os = "linux") {
            assert!(usage.machine_cpus > 0);
            assert!(usage.machine_memory_bytes > 0);
        }
    }
}

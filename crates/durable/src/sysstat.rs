//! System and process statistics.
//!
//! Decision: replaces the `sysinfo` crate. `sysinfo` supports every platform,
//! which cost this workspace the `windows`, `objc2`, and `ntapi` subtrees for
//! readings the server and worker only ever take inside Linux containers.
//! Everything here is a `/proc` read.
//!
//! Every function returns `None` off Linux rather than reporting a fabricated
//! zero. Resource-threshold backpressure is therefore inactive on other
//! platforms — [`crate::worker::BackpressureState`] simply never receives a
//! sample, and task-count watermarks continue to apply.

#[cfg(target_os = "linux")]
use std::fs;
use std::time::Instant;

/// System-wide memory, in bytes.
#[derive(Debug, Clone, Copy)]
pub struct MemoryStats {
    /// Memory in use, defined as `MemTotal - MemAvailable` to match what
    /// `sysinfo::System::used_memory` reported: page cache that the kernel can
    /// reclaim on demand does not count as pressure.
    pub used_bytes: u64,
    pub total_bytes: u64,
}

/// This process's resource usage.
#[derive(Debug, Clone, Copy)]
pub struct ProcessStats {
    /// Resident set size in bytes.
    pub rss_bytes: u64,
    /// CPU usage since the previous sample, as a percentage of one core. Can
    /// exceed 100 for a multi-threaded process, matching `sysinfo`.
    pub cpu_percent: f32,
}

/// The kernel reports `/proc` CPU times in USER_HZ, which is fixed at 100 on
/// Linux regardless of the kernel's internal tick rate.
const USER_HZ: f64 = 100.0;

/// Reads system memory from `/proc/meminfo`.
pub fn memory() -> Option<MemoryStats> {
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
    #[cfg(target_os = "linux")]
    {
        let meminfo = fs::read_to_string("/proc/meminfo").ok()?;
        let mut total_kb = None;
        let mut available_kb = None;
        for line in meminfo.lines() {
            let (key, rest) = line.split_once(':')?;
            match key {
                "MemTotal" => total_kb = parse_leading_u64(rest),
                "MemAvailable" => available_kb = parse_leading_u64(rest),
                _ => continue,
            }
            if total_kb.is_some() && available_kb.is_some() {
                break;
            }
        }
        let total_kb = total_kb?;
        let available_kb = available_kb?;
        Some(MemoryStats {
            used_bytes: total_kb.saturating_sub(available_kb) * 1024,
            total_bytes: total_kb * 1024,
        })
    }
}

/// Number of logical CPUs.
pub fn cpu_cores() -> usize {
    std::thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(1)
}

/// CPU model name from `/proc/cpuinfo`, or `None` when it is not reported.
///
/// `model name` is the x86 key; ARM kernels omit it, so `Model` and `Hardware`
/// are tried before giving up.
pub fn cpu_brand() -> Option<String> {
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
    #[cfg(target_os = "linux")]
    {
        let cpuinfo = fs::read_to_string("/proc/cpuinfo").ok()?;
        for key in ["model name", "Model", "Hardware", "cpu model"] {
            for line in cpuinfo.lines() {
                let Some((found, value)) = line.split_once(':') else {
                    continue;
                };
                if found.trim() == key && !value.trim().is_empty() {
                    return Some(value.trim().to_string());
                }
            }
        }
        None
    }
}

/// Distribution name from `/etc/os-release`, e.g. `Ubuntu`.
pub fn os_name() -> Option<String> {
    os_release_field("NAME")
}

/// Distribution version from `/etc/os-release`, e.g. `22.04`.
pub fn os_version() -> Option<String> {
    os_release_field("VERSION_ID")
}

/// The host name, from `/proc/sys/kernel/hostname`.
pub fn host_name() -> Option<String> {
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
    #[cfg(target_os = "linux")]
    {
        let name = fs::read_to_string("/proc/sys/kernel/hostname").ok()?;
        let name = name.trim();
        (!name.is_empty()).then(|| name.to_string())
    }
}

#[cfg(target_os = "linux")]
fn os_release_field(key: &str) -> Option<String> {
    let content = fs::read_to_string("/etc/os-release").ok()?;
    for line in content.lines() {
        let Some((found, value)) = line.split_once('=') else {
            continue;
        };
        if found == key {
            let value = value.trim().trim_matches('"');
            return (!value.is_empty()).then(|| value.to_string());
        }
    }
    None
}

#[cfg(not(target_os = "linux"))]
fn os_release_field(_key: &str) -> Option<String> {
    None
}

/// Parses the first integer in a string, ignoring a trailing unit such as `kB`.
#[cfg(target_os = "linux")]
fn parse_leading_u64(text: &str) -> Option<u64> {
    text.split_whitespace().next()?.parse().ok()
}

/// Samples system-wide CPU usage from `/proc/stat`.
///
/// Usage is a delta between consecutive samples, so the first call has no
/// baseline and reports 0.0 — the same behavior `sysinfo` had on its first
/// refresh. Sample at intervals of at least a few hundred milliseconds.
#[derive(Debug, Default)]
pub struct CpuSampler {
    /// (busy ticks, total ticks) from the previous sample.
    previous: Option<(u64, u64)>,
}

impl CpuSampler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns CPU usage as a 0.0–1.0 ratio across all cores.
    pub fn sample(&mut self) -> Option<f64> {
        let (busy, total) = read_cpu_ticks()?;
        let usage = match self.previous {
            Some((prev_busy, prev_total)) => {
                let total_delta = total.saturating_sub(prev_total);
                if total_delta == 0 {
                    // Sampled faster than the clock advances; report no change
                    // rather than dividing by zero.
                    0.0
                } else {
                    busy.saturating_sub(prev_busy) as f64 / total_delta as f64
                }
            }
            None => 0.0,
        };
        self.previous = Some((busy, total));
        Some(usage.clamp(0.0, 1.0))
    }
}

/// Returns (busy, total) CPU ticks from the aggregate `cpu` line of `/proc/stat`.
#[cfg(target_os = "linux")]
fn read_cpu_ticks() -> Option<(u64, u64)> {
    let stat = fs::read_to_string("/proc/stat").ok()?;
    let line = stat.lines().next()?;
    let mut fields = line.split_whitespace();
    if fields.next()? != "cpu" {
        return None;
    }
    let values: Vec<u64> = fields.filter_map(|field| field.parse().ok()).collect();
    if values.len() < 4 {
        return None;
    }
    let total: u64 = values.iter().sum();
    // Fields are user, nice, system, idle, iowait, ... — idle and iowait are
    // both time the CPU was not doing work.
    let idle = values[3] + values.get(4).copied().unwrap_or(0);
    Some((total.saturating_sub(idle), total))
}

#[cfg(not(target_os = "linux"))]
fn read_cpu_ticks() -> Option<(u64, u64)> {
    None
}

/// Samples this process's memory and CPU usage.
#[derive(Debug)]
pub struct ProcessSampler {
    /// (CPU ticks, instant) from the previous sample.
    previous: Option<(u64, Instant)>,
}

impl Default for ProcessSampler {
    fn default() -> Self {
        Self::new()
    }
}

impl ProcessSampler {
    pub fn new() -> Self {
        Self { previous: None }
    }

    pub fn sample(&mut self) -> Option<ProcessStats> {
        let rss_bytes = process_rss_bytes()?;
        let ticks = process_cpu_ticks()?;
        let now = Instant::now();

        let cpu_percent = match self.previous {
            Some((prev_ticks, prev_at)) => {
                let elapsed = now.duration_since(prev_at).as_secs_f64();
                if elapsed <= 0.0 {
                    0.0
                } else {
                    let cpu_seconds = ticks.saturating_sub(prev_ticks) as f64 / USER_HZ;
                    (cpu_seconds / elapsed * 100.0) as f32
                }
            }
            None => 0.0,
        };
        self.previous = Some((ticks, now));

        Some(ProcessStats {
            rss_bytes,
            cpu_percent,
        })
    }
}

/// Resident set size from `/proc/self/status`, which reports it in kB and so
/// needs no page-size lookup.
#[cfg(target_os = "linux")]
fn process_rss_bytes() -> Option<u64> {
    let status = fs::read_to_string("/proc/self/status").ok()?;
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("VmRSS:") {
            return parse_leading_u64(rest).map(|kb| kb * 1024);
        }
    }
    None
}

/// Combined user + system time from `/proc/self/stat`, in USER_HZ ticks.
#[cfg(target_os = "linux")]
fn process_cpu_ticks() -> Option<u64> {
    let stat = fs::read_to_string("/proc/self/stat").ok()?;
    // Field 2 is the executable name in parentheses and may itself contain
    // spaces or parentheses, so fields are counted from the last ')'.
    let after_comm = &stat[stat.rfind(')')? + 1..];
    let fields: Vec<&str> = after_comm.split_whitespace().collect();
    // After comm, index 0 is `state` (field 3), so utime (field 14) is index 11
    // and stime (field 15) is index 12.
    let utime: u64 = fields.get(11)?.parse().ok()?;
    let stime: u64 = fields.get(12)?.parse().ok()?;
    Some(utime + stime)
}

#[cfg(not(target_os = "linux"))]
fn process_rss_bytes() -> Option<u64> {
    None
}

#[cfg(not(target_os = "linux"))]
fn process_cpu_ticks() -> Option<u64> {
    None
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn memory_reports_plausible_totals() {
        let stats = memory().expect("/proc/meminfo should be readable on Linux");
        assert!(stats.total_bytes > 0);
        assert!(stats.used_bytes <= stats.total_bytes);
    }

    #[test]
    fn cpu_sampler_first_sample_has_no_baseline() {
        let mut sampler = CpuSampler::new();
        assert_eq!(sampler.sample(), Some(0.0));
        let second = sampler.sample().expect("/proc/stat should be readable");
        assert!((0.0..=1.0).contains(&second));
    }

    #[test]
    fn process_sampler_reports_resident_memory() {
        let mut sampler = ProcessSampler::new();
        let first = sampler.sample().expect("/proc/self should be readable");
        assert!(first.rss_bytes > 0);
        assert_eq!(first.cpu_percent, 0.0);

        // Burn a little CPU so the second sample has a non-negative delta.
        let mut sink = 0u64;
        for i in 0..500_000u64 {
            sink = sink.wrapping_add(i);
        }
        assert!(sink > 0);

        let second = sampler.sample().unwrap();
        assert!(second.cpu_percent >= 0.0);
    }

    #[test]
    fn cpu_cores_is_at_least_one() {
        assert!(cpu_cores() >= 1);
    }

    #[test]
    fn host_name_is_not_empty_when_present() {
        if let Some(name) = host_name() {
            assert!(!name.is_empty());
        }
    }
}

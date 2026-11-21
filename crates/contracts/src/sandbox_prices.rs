//! Sandbox list prices: what a provider charges for a sandbox's reserved
//! resources while it runs or is stopped, so sandbox time can carry dollars
//! the way model calls do.
//!
//! Decisions:
//! - No sandbox provider reports a per-sandbox cost through its API
//!   (Daytona's `usage` is quota, E2B points at its calculator; Modal's
//!   billing report is per workspace object and lags by hours), so a
//!   sandbox's cost is computed the way the providers bill it: reserved
//!   resources x time in each state x the published rate. Results are
//!   labelled estimates. An org's price list (`cost_rates`, meter
//!   `sandbox.time`) overrides them for discounts, credits or flat plans.
//! - Rates are per resource-hour in USD and carry the date they were read
//!   from the provider's pricing page. A price change is a new entry with a
//!   later `effective_from`, never an edit, so old sessions keep their rate.
//! - Resources come from the sandbox's pinned profile
//!   (`target.options`: `size`, `cpu`, `memory_mb`/`memory`, `disk`), which is
//!   fixed when the session is created, falling back to the provider's default
//!   sandbox. A provider without a public per-resource rate (boxd quotes team
//!   plans by email; self-hosted Kubernetes or Docker) has no list price.
//! - Free tiers and plan credits are not applied: they are per account, not
//!   per sandbox. Modal bills the larger of request and actual use, so its
//!   estimate is a floor.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Resources a sandbox reserves, in the units its provider bills.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SandboxResources {
    /// CPUs in the provider's billing unit: vCPUs, or physical cores on
    /// Modal.
    #[cfg_attr(feature = "openapi", schema(example = 1.0))]
    pub cpu: f64,
    /// Memory in GiB.
    #[cfg_attr(feature = "openapi", schema(example = 1.0))]
    pub memory_gib: f64,
    /// Disk in GiB.
    #[cfg_attr(feature = "openapi", schema(example = 3.0))]
    pub disk_gib: f64,
}

/// A provider's published per-resource rates, in USD per hour.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SandboxListPrice {
    /// Sandbox provider id (`daytona`, `e2b`, `modal`).
    pub provider: &'static str,
    /// Date (UTC, `YYYY-MM-DD`) the rate took effect, as read from the
    /// provider's pricing page.
    pub effective_from: &'static str,
    /// Per CPU unit per hour while running.
    pub cpu_hour: f64,
    /// Per GiB of memory per hour while running.
    pub memory_gib_hour: f64,
    /// Per GiB of disk per hour while running.
    pub disk_gib_hour: f64,
    /// Per GiB of disk per hour while stopped or paused (0 when stopped
    /// sandboxes are free).
    pub stopped_disk_gib_hour: f64,
    /// Where the rate was read.
    pub source: &'static str,
}

/// Every list price, newest last within a provider.
pub const SANDBOX_LIST_PRICES: &[SandboxListPrice] = &[
    // Per second on reserved vCPU, memory and disk; stopped sandboxes keep
    // paying for disk.
    SandboxListPrice {
        provider: "daytona",
        effective_from: "2026-11-20",
        cpu_hour: 0.0504,
        memory_gib_hour: 0.0162,
        disk_gib_hour: 0.000108,
        stopped_disk_gib_hour: 0.000108,
        source: "https://www.daytona.io/pricing",
    },
    // Per second on vCPU and memory while running (disk included); paused
    // and killed sandboxes are free.
    SandboxListPrice {
        provider: "e2b",
        effective_from: "2026-11-20",
        cpu_hour: 0.0504,
        memory_gib_hour: 0.0162,
        disk_gib_hour: 0.0,
        stopped_disk_gib_hour: 0.0,
        source: "https://e2b.dev/pricing",
    },
    // Sandbox rates per physical core ($0.00003942/s) and GiB of memory
    // ($0.00000672/s); a sandbox does not pause, it ends.
    SandboxListPrice {
        provider: "modal",
        effective_from: "2026-11-20",
        cpu_hour: 0.141_912,
        memory_gib_hour: 0.024_192,
        disk_gib_hour: 0.0,
        stopped_disk_gib_hour: 0.0,
        source: "https://modal.com/pricing",
    },
];

/// The list price of `provider` in effect on `date` (`YYYY-MM-DD`), or the
/// earliest one for a date before any.
pub fn list_price(provider: &str, date: &str) -> Option<&'static SandboxListPrice> {
    let provider = provider.to_ascii_lowercase();
    let mut matching = SANDBOX_LIST_PRICES
        .iter()
        .filter(|p| p.provider == provider);
    let first = matching.clone().next()?;
    Some(
        matching
            .rfind(|p| p.effective_from <= date)
            .unwrap_or(first),
    )
}

/// Resources of a sandbox of `provider` whose profile options are
/// `options` (the pinned profile's `target.options`; `Null` for none).
pub fn sandbox_resources(provider: &str, options: &Value) -> Option<SandboxResources> {
    let number = |key: &str| options.get(key).and_then(Value::as_f64);
    let memory_gib = number("memory_mb")
        .map(|mb| mb / 1024.0)
        .or_else(|| number("memory"));
    match provider.to_ascii_lowercase().as_str() {
        "daytona" => {
            let (cpu, memory, disk) = match options.get("size").and_then(Value::as_str) {
                Some("medium") => (2.0, 4.0, 8.0),
                Some("large") => (4.0, 8.0, 10.0),
                _ => (1.0, 1.0, 3.0),
            };
            Some(SandboxResources {
                cpu: number("cpu").unwrap_or(cpu),
                memory_gib: memory_gib.unwrap_or(memory),
                disk_gib: number("disk").unwrap_or(disk),
            })
        }
        // The default template: 2 vCPU, 512 MiB.
        "e2b" => Some(SandboxResources {
            cpu: number("cpu").unwrap_or(2.0),
            memory_gib: memory_gib.unwrap_or(0.5),
            disk_gib: number("disk").unwrap_or(0.0),
        }),
        // Modal's sandbox default: 0.125 cores, 128 MiB.
        "modal" => Some(SandboxResources {
            cpu: number("cpu").unwrap_or(0.125),
            memory_gib: memory_gib.unwrap_or(0.125),
            disk_gib: 0.0,
        }),
        _ => None,
    }
}

/// List-price cost in USD of `resources` running for `running_seconds` and
/// stopped for `stopped_seconds`.
pub fn list_price_usd(
    price: &SandboxListPrice,
    resources: SandboxResources,
    running_seconds: f64,
    stopped_seconds: f64,
) -> f64 {
    let running_hour = resources.cpu * price.cpu_hour
        + resources.memory_gib * price.memory_gib_hour
        + resources.disk_gib * price.disk_gib_hour;
    let stopped_hour = resources.disk_gib * price.stopped_disk_gib_hour;
    (running_seconds.max(0.0) * running_hour + stopped_seconds.max(0.0) * stopped_hour) / 3600.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn daytona_small_costs_its_reserved_resources_per_hour() {
        let price = list_price("daytona", "2026-11-20").unwrap();
        let small = sandbox_resources("daytona", &Value::Null).unwrap();
        assert_eq!(small.cpu, 1.0);
        let usd = list_price_usd(price, small, 3600.0, 0.0);
        assert!((usd - (0.0504 + 0.0162 + 3.0 * 0.000108)).abs() < 1e-9);
        // Stopped, only the disk is billed.
        let stopped = list_price_usd(price, small, 0.0, 3600.0);
        assert!((stopped - 3.0 * 0.000108).abs() < 1e-12);
    }

    #[test]
    fn sizes_and_explicit_options_choose_resources() {
        let large = sandbox_resources("daytona", &json!({"size": "large"})).unwrap();
        assert_eq!(
            (large.cpu, large.memory_gib, large.disk_gib),
            (4.0, 8.0, 10.0)
        );
        let modal = sandbox_resources("modal", &json!({"cpu": 0.5, "memory_mb": 1024})).unwrap();
        assert_eq!((modal.cpu, modal.memory_gib), (0.5, 1.0));
    }

    #[test]
    fn paused_e2b_sandboxes_are_free() {
        let price = list_price("e2b", "2026-12-01").unwrap();
        let r = sandbox_resources("e2b", &Value::Null).unwrap();
        assert_eq!(list_price_usd(price, r, 0.0, 86_400.0), 0.0);
    }

    #[test]
    fn providers_without_a_public_rate_have_no_list_price() {
        assert!(list_price("boxd", "2026-11-20").is_none());
        assert!(sandbox_resources("kubernetes", &Value::Null).is_none());
    }

    #[test]
    fn an_older_date_takes_the_earliest_rate() {
        assert_eq!(
            list_price("modal", "2020-01-01").unwrap().effective_from,
            "2026-11-20"
        );
    }
}

//! Temporary latency accounting (CELL_PROF=1): total time and calls per name.
use std::collections::HashMap;
use std::sync::LazyLock;
use std::time::{Duration, Instant};

/// When the table was last logged, and calls and microseconds per name.
type Table = (Instant, HashMap<&'static str, (u64, u64)>);

static TABLE: LazyLock<parking_lot::Mutex<Table>> =
    LazyLock::new(|| parking_lot::Mutex::new((Instant::now(), HashMap::new())));
static ON: LazyLock<bool> = LazyLock::new(|| std::env::var_os("CELL_PROF").is_some());

pub fn add(name: &'static str, started: Instant, n: u64) {
    if !*ON {
        return;
    }
    let us = started.elapsed().as_micros() as u64;
    let mut t = TABLE.lock();
    let e = t.1.entry(name).or_default();
    e.0 += n;
    e.1 += us;
    if t.0.elapsed() > Duration::from_secs(20) {
        t.0 = Instant::now();
        let mut v: Vec<_> =
            t.1.drain()
                .map(|(k, (n, us))| (k, n, us / 1000, us.checked_div(n).unwrap_or(0)))
                .collect();
        v.sort_by_key(|v| std::cmp::Reverse(v.2));
        tracing::info!(target: "everruns_cell::prof", ?v, "latency (name, calls, total ms, avg us)");
    }
}

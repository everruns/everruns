// Per-org and per-destination admission for the egress proxy.
//
// Decision: counters live in this process. Per-minute windows are fixed
// (they reset on the minute), which is coarse but cheap and has no shared
// state to fail. With several proxy replicas each one enforces
// ceil(cap / replicas) (`EGRESS_PROXY_REPLICAS`), so the fleet as a whole
// stays near the configured cap without a shared store.
//
// Decision: the daily byte cap is the one limit that must hold across
// replicas and restarts, so it can be anchored on the control endpoint: each
// limits fetch may report the bytes already booked today
// (`OrgLimits::bytes_used_today`), and this process adds what it has counted
// since that fetch. Without a control endpoint the cap is per replica and
// resets on restart.
//
// Decision: a refused request is not counted, so a client hammering a closed
// window does not keep pushing the window's count up; the window still resets
// on the minute.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use super::limits::OrgLimits;

/// Why a request was refused. The text is what the agent reads in the 429.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    OrgRate,
    OrgHostRate,
    GlobalHostRate,
    Concurrency,
    DailyBytes,
}

impl Refusal {
    pub fn reason(self) -> &'static str {
        match self {
            Self::OrgRate => {
                "egress rate limit: too many requests per minute for this organization"
            }
            Self::OrgHostRate => {
                "egress rate limit: too many requests per minute to this host for this organization"
            }
            Self::GlobalHostRate => "egress rate limit: too many requests per minute to this host",
            Self::Concurrency => {
                "egress limit: too many concurrent connections for this organization"
            }
            Self::DailyBytes => {
                "egress limit: daily data transfer cap reached for this organization"
            }
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::OrgRate => "org_rate",
            Self::OrgHostRate => "org_host_rate",
            Self::GlobalHostRate => "global_host_rate",
            Self::Concurrency => "concurrency",
            Self::DailyBytes => "daily_bytes",
        }
    }
}

#[derive(Default)]
struct State {
    minute: i64,
    org_requests: HashMap<String, u64>,
    org_host_requests: HashMap<(String, String), u64>,
    host_requests: HashMap<String, u64>,
    day: i64,
    /// Bytes counted here since the org's limits were last fetched.
    bytes_since_refresh: HashMap<String, u64>,
    concurrent: HashMap<String, u64>,
}

#[derive(Clone)]
pub struct Limiter {
    replicas: u64,
    global_per_host_rpm: u64,
    state: Arc<Mutex<State>>,
}

/// Holds one concurrent-connection slot for an org until dropped.
pub struct Admission {
    org: String,
    state: Arc<Mutex<State>>,
}

impl Drop for Admission {
    fn drop(&mut self) {
        let mut state = lock(&self.state);
        if let Some(count) = state.concurrent.get_mut(&self.org) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                state.concurrent.remove(&self.org);
            }
        }
    }
}

fn lock(state: &Mutex<State>) -> std::sync::MutexGuard<'_, State> {
    // A panic while holding the lock leaves plain counters behind, which are
    // still valid; keep serving rather than poisoning every later request.
    state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// This replica's share of a fleet-wide cap; never zero.
fn share(cap: u64, replicas: u64) -> u64 {
    cap.div_ceil(replicas.max(1)).max(1)
}

impl Limiter {
    pub fn new(replicas: u64, global_per_host_rpm: u64) -> Self {
        Self {
            replicas: replicas.max(1),
            global_per_host_rpm,
            state: Arc::new(Mutex::new(State::default())),
        }
    }

    /// Admit one request from `org` (an org id, or `-` for unscoped traffic)
    /// to `host`, or say why not. `now_unix` drives the windows.
    pub fn admit(
        &self,
        org: &str,
        host: &str,
        limits: &OrgLimits,
        now_unix: i64,
    ) -> Result<Admission, Refusal> {
        let host = host.to_ascii_lowercase();
        let mut state = lock(&self.state);
        roll(&mut state, now_unix);

        let since = state.bytes_since_refresh.get(org).copied().unwrap_or(0);
        if limits.bytes_used_today.saturating_add(since) >= limits.bytes_per_day {
            return Err(Refusal::DailyBytes);
        }
        let org_count = state.org_requests.get(org).copied().unwrap_or(0);
        if org_count >= share(limits.requests_per_minute, self.replicas) {
            return Err(Refusal::OrgRate);
        }
        let key = (org.to_string(), host.clone());
        let org_host_count = state.org_host_requests.get(&key).copied().unwrap_or(0);
        if org_host_count >= share(limits.per_host_requests_per_minute, self.replicas) {
            return Err(Refusal::OrgHostRate);
        }
        let host_count = state.host_requests.get(&host).copied().unwrap_or(0);
        if host_count >= share(self.global_per_host_rpm, self.replicas) {
            return Err(Refusal::GlobalHostRate);
        }
        let concurrent = state.concurrent.get(org).copied().unwrap_or(0);
        if concurrent >= share(limits.max_concurrent, self.replicas) {
            return Err(Refusal::Concurrency);
        }

        *state.org_requests.entry(org.to_string()).or_default() += 1;
        *state.org_host_requests.entry(key).or_default() += 1;
        *state.host_requests.entry(host).or_default() += 1;
        *state.concurrent.entry(org.to_string()).or_default() += 1;
        Ok(Admission {
            org: org.to_string(),
            state: self.state.clone(),
        })
    }

    /// Count bytes moved for `org`, toward the daily cap.
    pub fn record_bytes(&self, org: &str, bytes: u64, now_unix: i64) {
        let mut state = lock(&self.state);
        roll(&mut state, now_unix);
        *state
            .bytes_since_refresh
            .entry(org.to_string())
            .or_default() += bytes;
    }

    /// The org's limits were just fetched, and with them the bytes the ledger
    /// has booked: what was counted here before now is either in that number
    /// or in the next flush, so start counting afresh.
    ///
    /// The bytes counted but not yet flushed drop out of view until the next
    /// flush and fetch, at most about two minutes. That is the accepted slack.
    pub fn limits_refreshed(&self, org: &str) {
        lock(&self.state).bytes_since_refresh.remove(org);
    }

    pub fn concurrent(&self, org: &str) -> u64 {
        lock(&self.state).concurrent.get(org).copied().unwrap_or(0)
    }
}

fn roll(state: &mut State, now_unix: i64) {
    let minute = now_unix.div_euclid(60);
    if minute != state.minute {
        state.minute = minute;
        state.org_requests.clear();
        state.org_host_requests.clear();
        state.host_requests.clear();
    }
    let day = now_unix.div_euclid(86_400);
    if day != state.day {
        state.day = day;
        state.bytes_since_refresh.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: i64 = 1_800_000_000 - (1_800_000_000 % 86_400) + 3_600;

    fn limits(rpm: u64, per_host: u64, concurrent: u64, bytes: u64) -> OrgLimits {
        OrgLimits {
            label: None,
            requests_per_minute: rpm,
            max_concurrent: concurrent,
            bytes_per_day: bytes,
            per_host_requests_per_minute: per_host,
            suspended: false,
            bytes_used_today: 0,
        }
    }

    #[test]
    fn the_org_rate_resets_on_the_minute() {
        let limiter = Limiter::new(1, 1_000);
        let l = limits(2, 100, 100, u64::MAX);
        drop(limiter.admit("org_a", "a.example", &l, T0));
        drop(limiter.admit("org_a", "b.example", &l, T0 + 1));
        assert_eq!(
            limiter.admit("org_a", "c.example", &l, T0 + 2).err(),
            Some(Refusal::OrgRate)
        );
        // Another org is unaffected.
        assert!(limiter.admit("org_b", "a.example", &l, T0 + 2).is_ok());
        // Next minute.
        assert!(limiter.admit("org_a", "c.example", &l, T0 + 60).is_ok());
    }

    #[test]
    fn per_host_caps_apply_per_org_and_globally() {
        let limiter = Limiter::new(1, 3);
        let l = limits(100, 2, 100, u64::MAX);
        assert!(limiter.admit("org_a", "Target.example", &l, T0).is_ok());
        assert!(limiter.admit("org_a", "target.example", &l, T0).is_ok());
        assert_eq!(
            limiter.admit("org_a", "TARGET.example", &l, T0).err(),
            Some(Refusal::OrgHostRate)
        );
        // A second org reaches the global cap for the host.
        assert!(limiter.admit("org_b", "target.example", &l, T0).is_ok());
        assert_eq!(
            limiter.admit("org_c", "target.example", &l, T0).err(),
            Some(Refusal::GlobalHostRate)
        );
        assert!(limiter.admit("org_c", "other.example", &l, T0).is_ok());
    }

    #[test]
    fn concurrency_is_released_when_the_admission_drops() {
        let limiter = Limiter::new(1, 1_000);
        let l = limits(100, 100, 2, u64::MAX);
        let first = limiter.admit("org_a", "a.example", &l, T0).expect("first");
        let _second = limiter.admit("org_a", "a.example", &l, T0).expect("second");
        assert_eq!(
            limiter.admit("org_a", "a.example", &l, T0).err(),
            Some(Refusal::Concurrency)
        );
        drop(first);
        assert_eq!(limiter.concurrent("org_a"), 1);
        assert!(limiter.admit("org_a", "a.example", &l, T0).is_ok());
    }

    #[test]
    fn the_daily_byte_cap_counts_the_ledger_and_local_bytes() {
        let limiter = Limiter::new(1, 1_000);
        let mut l = limits(100, 100, 100, 1_000);
        l.bytes_used_today = 600;
        assert!(limiter.admit("org_a", "a.example", &l, T0).is_ok());
        limiter.record_bytes("org_a", 400, T0);
        assert_eq!(
            limiter.admit("org_a", "a.example", &l, T0).err(),
            Some(Refusal::DailyBytes)
        );
        // A refresh hands the counting back to the ledger number.
        limiter.limits_refreshed("org_a");
        assert!(limiter.admit("org_a", "a.example", &l, T0).is_ok());
        l.bytes_used_today = 1_000;
        assert_eq!(
            limiter.admit("org_a", "a.example", &l, T0).err(),
            Some(Refusal::DailyBytes)
        );
    }

    #[test]
    fn replicas_split_the_caps_but_never_to_zero() {
        let limiter = Limiter::new(2, 1_000);
        let l = limits(3, 100, 100, u64::MAX);
        assert!(limiter.admit("org_a", "a.example", &l, T0).is_ok());
        assert!(limiter.admit("org_a", "a.example", &l, T0).is_ok());
        assert_eq!(
            limiter.admit("org_a", "a.example", &l, T0).err(),
            Some(Refusal::OrgRate)
        );
        assert_eq!(share(1, 4), 1);
        assert_eq!(share(600, 2), 300);
        // An unset cap stays unset however many replicas share it.
        assert!(
            limiter
                .admit("org_b", "a.example", &OrgLimits::unlimited(), T0)
                .is_ok()
        );
    }
}

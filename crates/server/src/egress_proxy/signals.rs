// Abuse signals: per org and minute, what a control plane's abuse rules read.
//
// Decision: the proxy counts, a control plane decides. The proxy sees every
// tenant connection but holds no history beyond the current minutes and no
// account state, so it reports facts (refusals by kind, distinct destination
// hosts) and the control plane applies rules over a window and acts (the
// hosted one suspends the org). Without a control endpoint the facts are log
// lines, like usage.
//
// Decision: the signals are the ones that separate abuse from heavy use.
// - `refused_port`: attempts on blocked ports, which are the SMTP ports. An
//   agent has no legitimate reason to speak SMTP through the proxy; repeated
//   attempts are spam delivery.
// - `refused_private`: destinations that resolve to non-public addresses,
//   repeatedly: probing for internal services or metadata endpoints.
// - `refused_rate`: requests refused by a cap. A loop that keeps hammering
//   past its caps is a flood the caps contain but the operator should see.
// - `distinct_hosts`: how many different hosts the org reached in the minute.
//   Scanning and spam fan out across hosts; agents, even crawling ones,
//   revisit a few. Counted per org-minute (not per session) so many sessions
//   using the same few APIs do not add up.
//
// Decision: the host set per org-minute is bounded; past the bound the count
// saturates, which is already far above any rule's threshold.

use std::collections::{BTreeMap, HashSet};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

/// Most distinct hosts remembered per org-minute; the count stops there.
pub const MAX_TRACKED_HOSTS: usize = 10_000;

/// Most (org, minute) rows held while the sink is unreachable.
pub const MAX_PENDING_SIGNALS: usize = 50_000;

/// One org's abuse signals for one minute.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AbuseSignals {
    pub org: String,
    /// Unix seconds at the start of the minute (a multiple of 60).
    pub minute: i64,
    #[serde(default)]
    pub distinct_hosts: u64,
    #[serde(default)]
    pub refused_port: u64,
    #[serde(default)]
    pub refused_private: u64,
    #[serde(default)]
    pub refused_rate: u64,
}

/// A refusal kind counted as a signal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    Port,
    Private,
    Rate,
}

#[derive(Debug, Default)]
struct Counts {
    hosts: HashSet<String>,
    /// Hosts carried over from a failed flush, whose names are gone.
    requeued_hosts: u64,
    refused_port: u64,
    refused_private: u64,
    refused_rate: u64,
}

type Key = (i64, String);

#[derive(Debug, Default)]
pub struct SignalCounter {
    pending: Mutex<BTreeMap<Key, Counts>>,
}

impl SignalCounter {
    /// Count an admitted request to `host`.
    pub fn host(&self, org: &str, host: &str, now_unix: i64) {
        let host = host.trim_end_matches('.').to_ascii_lowercase();
        self.with(org, now_unix, |counts| {
            if counts.hosts.len() < MAX_TRACKED_HOSTS {
                counts.hosts.insert(host);
            }
        });
    }

    /// Count a refusal.
    pub fn refused(&self, org: &str, refusal: Refusal, now_unix: i64) {
        self.with(org, now_unix, |counts| match refusal {
            Refusal::Port => counts.refused_port += 1,
            Refusal::Private => counts.refused_private += 1,
            Refusal::Rate => counts.refused_rate += 1,
        });
    }

    fn with(&self, org: &str, now_unix: i64, update: impl FnOnce(&mut Counts)) {
        let minute = now_unix - now_unix.rem_euclid(60);
        let mut pending = self.lock();
        update(pending.entry((minute, org.to_string())).or_default());
        if pending.len() > MAX_PENDING_SIGNALS {
            let over = pending.len() - MAX_PENDING_SIGNALS;
            let dropped: Vec<Key> = pending.keys().take(over).cloned().collect();
            for key in &dropped {
                pending.remove(key);
            }
            tracing::error!(
                dropped = over,
                "egress abuse signal backlog full; dropped oldest minutes"
            );
        }
    }

    /// Take the signals for minutes before `before_minute` (all when `None`).
    pub fn drain(&self, before_minute: Option<i64>) -> Vec<AbuseSignals> {
        let mut pending = self.lock();
        let keep = match before_minute {
            Some(cutoff) => pending.split_off(&(cutoff, String::new())),
            None => BTreeMap::new(),
        };
        std::mem::replace(&mut *pending, keep)
            .into_iter()
            .map(|((minute, org), counts)| AbuseSignals {
                org,
                minute,
                distinct_hosts: counts.hosts.len() as u64 + counts.requeued_hosts,
                refused_port: counts.refused_port,
                refused_private: counts.refused_private,
                refused_rate: counts.refused_rate,
            })
            .collect()
    }

    /// Put back signals whose flush failed.
    pub fn requeue(&self, signals: Vec<AbuseSignals>) {
        let mut pending = self.lock();
        for signal in signals {
            let counts = pending.entry((signal.minute, signal.org)).or_default();
            counts.requeued_hosts += signal.distinct_hosts;
            counts.refused_port += signal.refused_port;
            counts.refused_private += signal.refused_private;
            counts.refused_rate += signal.refused_rate;
        }
    }

    pub fn pending_len(&self) -> usize {
        self.lock().len()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<Key, Counts>> {
        self.pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ORG: &str = "org_0123456789abcdef0123456789abcdef";
    const OTHER: &str = "org_fedcba9876543210fedcba9876543210";
    const M0: i64 = 1_800_000_000;

    #[test]
    fn counts_distinct_hosts_and_refusals_per_org_minute() {
        let counter = SignalCounter::default();
        for host in ["a.example", "A.Example.", "b.example", "a.example"] {
            counter.host(ORG, host, M0 + 5);
        }
        counter.host(ORG, "c.example", M0 + 60);
        counter.host(OTHER, "a.example", M0);
        counter.refused(ORG, Refusal::Port, M0 + 1);
        counter.refused(ORG, Refusal::Port, M0 + 2);
        counter.refused(ORG, Refusal::Private, M0 + 3);
        counter.refused(ORG, Refusal::Rate, M0 + 4);

        let due = counter.drain(Some(M0 + 60));
        assert_eq!(due.len(), 2);
        let org = due.iter().find(|s| s.org == ORG).unwrap();
        assert_eq!(
            (
                org.minute,
                org.distinct_hosts,
                org.refused_port,
                org.refused_private,
                org.refused_rate
            ),
            (M0, 2, 2, 1, 1)
        );
        assert_eq!(counter.pending_len(), 1, "the current minute stays");
    }

    #[test]
    fn the_host_set_saturates_and_requeue_keeps_counts() {
        let counter = SignalCounter::default();
        for n in 0..(MAX_TRACKED_HOSTS + 50) {
            counter.host(ORG, &format!("h{n}.example"), M0);
        }
        counter.refused(ORG, Refusal::Rate, M0);
        let drained = counter.drain(None);
        assert_eq!(drained[0].distinct_hosts, MAX_TRACKED_HOSTS as u64);

        counter.requeue(drained);
        counter.host(ORG, "late.example", M0);
        let again = counter.drain(None);
        assert_eq!(again[0].distinct_hosts, MAX_TRACKED_HOSTS as u64 + 1);
        assert_eq!(again[0].refused_rate, 1);
    }
}

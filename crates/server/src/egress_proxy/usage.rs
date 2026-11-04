// Network usage counted by the proxy, flushed to a `UsageSink`.
//
// Decision: the proxy aggregates per (org, session, minute) and flushes every
// minute, so a busy org costs the sink one item per minute, not one per
// request. Only completed minutes are flushed on the timer; the shutdown flush
// sends everything, including the current minute.
//
// Decision: each process start gets its own `instance` label, carried on every
// batch. A minute is flushed once per instance (completed minutes receive no
// new counts), so a sink that keys on (instance, org, session, minute) can
// make retried flushes idempotent and never collides with a previous run.
//
// Decision: a failed flush is put back and retried on the next tick. The
// backlog is bounded; past the bound the oldest minutes are dropped with an
// error log, because unbounded memory in the egress path is the worse failure.
//
// Decision: without a control endpoint the sink is a log line per item. A
// self-hosted operator reads usage from the logs; a hosted control plane
// books it (`HttpUsageSink`).

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use super::signals::{AbuseSignals, SignalCounter};

/// Path of the usage endpoint on the control endpoint.
pub const USAGE_PATH: &str = "/internal/egress-proxy/usage";

/// Most items in one POST.
pub const MAX_ITEMS_PER_BATCH: usize = 500;

/// Most (org, session, minute) rows held while the sink is unreachable.
pub const MAX_PENDING: usize = 200_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageItem {
    pub org: String,
    #[serde(default)]
    pub session: Option<String>,
    /// Unix seconds at the start of the minute (a multiple of 60).
    pub minute: i64,
    pub egress_bytes: u64,
    pub ingress_bytes: u64,
    pub requests: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageBatch {
    pub instance: String,
    pub items: Vec<UsageItem>,
    /// Abuse signals for the same completed minutes (see `signals`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub signals: Vec<AbuseSignals>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Counts {
    egress_bytes: u64,
    ingress_bytes: u64,
    requests: u64,
}

type Key = (i64, String, Option<String>);

pub struct UsageAggregator {
    instance: String,
    pending: Mutex<BTreeMap<Key, Counts>>,
    signals: SignalCounter,
}

impl UsageAggregator {
    pub fn new(instance: impl Into<String>) -> Self {
        Self {
            instance: instance.into(),
            pending: Mutex::new(BTreeMap::new()),
            signals: SignalCounter::default(),
        }
    }

    /// Abuse signals, flushed with usage.
    pub fn signals(&self) -> &SignalCounter {
        &self.signals
    }

    pub fn instance(&self) -> &str {
        &self.instance
    }

    /// Count usage for an org at `now_unix`. Unscoped traffic (no org) is
    /// logged by the caller but has no account to book to.
    pub fn record(
        &self,
        org: &str,
        session: Option<&str>,
        now_unix: i64,
        egress_bytes: u64,
        ingress_bytes: u64,
        requests: u64,
    ) {
        if egress_bytes == 0 && ingress_bytes == 0 && requests == 0 {
            return;
        }
        let minute = now_unix - now_unix.rem_euclid(60);
        let mut pending = self.lock();
        let counts = pending
            .entry((minute, org.to_string(), session.map(str::to_string)))
            .or_default();
        counts.egress_bytes += egress_bytes;
        counts.ingress_bytes += ingress_bytes;
        counts.requests += requests;
        if pending.len() > MAX_PENDING {
            let over = pending.len() - MAX_PENDING;
            let dropped: Vec<Key> = pending.keys().take(over).cloned().collect();
            for key in &dropped {
                pending.remove(key);
            }
            tracing::error!(
                dropped = over,
                "egress usage backlog full; dropped oldest minutes"
            );
        }
    }

    /// Take the items for minutes before `before_minute` (all of them when
    /// `None`).
    pub fn drain(&self, before_minute: Option<i64>) -> Vec<UsageItem> {
        let mut pending = self.lock();
        let keep = match before_minute {
            Some(cutoff) => pending.split_off(&(cutoff, String::new(), None)),
            None => BTreeMap::new(),
        };
        let taken = std::mem::replace(&mut *pending, keep);
        taken
            .into_iter()
            .map(|((minute, org, session), counts)| UsageItem {
                org,
                session,
                minute,
                egress_bytes: counts.egress_bytes,
                ingress_bytes: counts.ingress_bytes,
                requests: counts.requests,
            })
            .collect()
    }

    /// Put back items whose flush failed.
    pub fn requeue(&self, items: Vec<UsageItem>) {
        let mut pending = self.lock();
        for item in items {
            let counts = pending
                .entry((item.minute, item.org, item.session))
                .or_default();
            counts.egress_bytes += item.egress_bytes;
            counts.ingress_bytes += item.ingress_bytes;
            counts.requests += item.requests;
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

    /// Send what is due to `sink`, in batches. Anything not accepted goes back.
    pub async fn flush(&self, sink: &dyn UsageSink, before_minute: Option<i64>) -> FlushReport {
        let items = self.drain(before_minute);
        let signals = self.signals.drain(before_minute);
        let mut report = FlushReport::default();
        let mut start = 0;
        while start < items.len() || start < signals.len() {
            let window = |len: usize| start.min(len)..(start + MAX_ITEMS_PER_BATCH).min(len);
            let batch = UsageBatch {
                instance: self.instance.clone(),
                items: items[window(items.len())].to_vec(),
                signals: signals[window(signals.len())].to_vec(),
            };
            if let Err(err) = sink.send(&batch).await {
                tracing::warn!(error = %err, "egress usage flush failed; will retry");
                let rest = items[start.min(items.len())..].to_vec();
                report.requeued = rest.len();
                self.requeue(rest);
                self.signals
                    .requeue(signals[start.min(signals.len())..].to_vec());
                break;
            }
            report.batches += 1;
            report.items += batch.items.len();
            report.signals += batch.signals.len();
            start += MAX_ITEMS_PER_BATCH;
        }
        report
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct FlushReport {
    pub batches: usize,
    pub items: usize,
    pub signals: usize,
    pub requeued: usize,
}

#[async_trait]
pub trait UsageSink: Send + Sync {
    async fn send(&self, batch: &UsageBatch) -> anyhow::Result<()>;
}

/// Posts to the control endpoint (`EGRESS_PROXY_CONTROL_URL`).
pub struct HttpUsageSink {
    client: reqwest::Client,
    url: String,
    secret: String,
}

impl HttpUsageSink {
    pub fn new(base_url: &str, secret: impl Into<String>) -> anyhow::Result<Self> {
        Ok(Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(20))
                .no_proxy()
                .build()?,
            url: format!("{}{USAGE_PATH}", base_url.trim_end_matches('/')),
            secret: secret.into(),
        })
    }
}

#[async_trait]
impl UsageSink for HttpUsageSink {
    async fn send(&self, batch: &UsageBatch) -> anyhow::Result<()> {
        self.client
            .post(&self.url)
            .bearer_auth(&self.secret)
            .json(batch)
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }
}

/// No control endpoint: each item is a structured log line.
pub struct LogSink;

#[async_trait]
impl UsageSink for LogSink {
    async fn send(&self, batch: &UsageBatch) -> anyhow::Result<()> {
        for item in &batch.items {
            tracing::info!(
                target: "everruns::egress::usage",
                instance = %batch.instance,
                org = %item.org,
                session = item.session.as_deref().unwrap_or("-"),
                minute = item.minute,
                egress_bytes = item.egress_bytes,
                ingress_bytes = item.ingress_bytes,
                requests = item.requests,
                "egress usage"
            );
        }
        for signal in &batch.signals {
            tracing::info!(
                target: "everruns::egress::signals",
                instance = %batch.instance,
                org = %signal.org,
                minute = signal.minute,
                distinct_hosts = signal.distinct_hosts,
                refused_port = signal.refused_port,
                refused_private = signal.refused_private,
                refused_rate = signal.refused_rate,
                "egress abuse signals"
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::*;

    const ORG: &str = "org_0123456789abcdef0123456789abcdef";
    // A minute boundary (1_800_000_000 is divisible by 60).
    const M0: i64 = 1_800_000_000;

    #[derive(Default)]
    struct Recording {
        batches: Mutex<Vec<UsageBatch>>,
        fail: AtomicBool,
    }

    #[async_trait]
    impl UsageSink for Arc<Recording> {
        async fn send(&self, batch: &UsageBatch) -> anyhow::Result<()> {
            if self.fail.load(Ordering::SeqCst) {
                anyhow::bail!("control endpoint down");
            }
            self.batches.lock().expect("lock").push(batch.clone());
            Ok(())
        }
    }

    #[test]
    fn usage_aggregates_per_org_session_and_minute() {
        let agg = UsageAggregator::new("i1");
        agg.record(ORG, Some("s1"), M0 + 1, 100, 1_000, 1);
        agg.record(ORG, Some("s1"), M0 + 59, 50, 500, 1);
        agg.record(ORG, None, M0 + 2, 1, 2, 1);
        agg.record(ORG, Some("s1"), M0 + 60, 7, 7, 1);
        agg.record(ORG, Some("s1"), M0 + 61, 0, 0, 0);
        let items = agg.drain(None);
        assert_eq!(items.len(), 3);
        let first = items
            .iter()
            .find(|i| i.minute == M0 && i.session.as_deref() == Some("s1"))
            .expect("aggregated row");
        assert_eq!(
            (first.egress_bytes, first.ingress_bytes, first.requests),
            (150, 1_500, 2)
        );
        assert_eq!(agg.pending_len(), 0);
    }

    #[test]
    fn only_completed_minutes_drain_on_the_timer() {
        let agg = UsageAggregator::new("i1");
        agg.record(ORG, None, M0, 1, 1, 1);
        agg.record(ORG, None, M0 + 60, 1, 1, 1);
        let due = agg.drain(Some(M0 + 60));
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].minute, M0);
        assert_eq!(agg.pending_len(), 1);
    }

    #[tokio::test]
    async fn flushes_batch_and_failed_batches_are_retried() {
        let sink = Arc::new(Recording::default());
        let agg = UsageAggregator::new("i1");
        for n in 0..(MAX_ITEMS_PER_BATCH as i64 + 5) {
            agg.record(ORG, Some(&format!("s{n}")), M0, 10, 20, 1);
        }

        sink.fail.store(true, Ordering::SeqCst);
        let report = agg.flush(&sink, None).await;
        assert_eq!(report.batches, 0);
        assert_eq!(report.requeued, MAX_ITEMS_PER_BATCH + 5);
        assert_eq!(agg.pending_len(), MAX_ITEMS_PER_BATCH + 5);

        sink.fail.store(false, Ordering::SeqCst);
        let report = agg.flush(&sink, None).await;
        assert_eq!(report.batches, 2);
        assert_eq!(report.items, MAX_ITEMS_PER_BATCH + 5);
        let batches = sink.batches.lock().expect("lock");
        assert_eq!(batches[0].items.len(), MAX_ITEMS_PER_BATCH);
        assert_eq!(batches[1].items.len(), 5);
        assert!(batches.iter().all(|b| b.instance == "i1"));
        // Retrying did not double anything.
        assert!(
            batches
                .iter()
                .flat_map(|b| &b.items)
                .all(|i| i.egress_bytes == 10)
        );
    }

    #[tokio::test]
    async fn signals_ride_with_usage_and_survive_a_failed_flush() {
        use super::super::signals::Refusal;

        let sink = Arc::new(Recording::default());
        let agg = UsageAggregator::new("i1");
        agg.signals().host(ORG, "a.example", M0);
        agg.signals().refused(ORG, Refusal::Port, M0);
        // A minute with signals and no usage still flushes.
        agg.signals().refused(ORG, Refusal::Private, M0 + 60);

        sink.fail.store(true, Ordering::SeqCst);
        assert_eq!(agg.flush(&sink, None).await.batches, 0);
        assert_eq!(agg.signals().pending_len(), 2);

        sink.fail.store(false, Ordering::SeqCst);
        let report = agg.flush(&sink, None).await;
        assert_eq!((report.batches, report.items, report.signals), (1, 0, 2));
        let batches = sink.batches.lock().expect("lock");
        let first = &batches[0].signals[0];
        assert_eq!(
            (first.minute, first.distinct_hosts, first.refused_port),
            (M0, 1, 1)
        );
        // An old control plane that does not know signals still reads the
        // batch: the field is skipped when empty and defaulted when absent.
        let plain: UsageBatch =
            serde_json::from_str(r#"{"instance":"i1","items":[]}"#).expect("parse");
        assert!(plain.signals.is_empty());
        assert!(!serde_json::to_string(&plain).unwrap().contains("signals"));
    }
}

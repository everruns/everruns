// Per-org limits and where the proxy learns them.
//
// Decision: limits are plain numbers with `u64::MAX` meaning "no cap", so the
// wire shape a control endpoint answers with is the shape the limiter reads.
// Every cap is off unless configured: on a self-hosted deployment the operator
// owns the traffic, and a hosted control plane sends its own numbers.
//
// Decision: without a control endpoint every org gets the configured static
// limits (`EGRESS_PROXY_ORG_*`), with no lookup and no per-request log.
//
// Decision: with a control endpoint, answers are cached for `ttl` (60 s in the
// binary). A suspension therefore takes effect within a minute, and the proxy
// asks at most once a minute per active org. When the endpoint cannot be
// asked, the org gets the fallback limits for a shorter time, so a recovered
// endpoint is picked up quickly: an outage of the control plane does not
// become an outage of tenant egress, and the fallback is whatever the
// operator configured, never something looser.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Per-org limits as a control endpoint reports them and the proxy enforces
/// them. `u64::MAX` is no cap.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrgLimits {
    /// Free-form name for the limits (for example a trust tier), logged and
    /// otherwise ignored. A hosted control plane sends it as `tier`.
    #[serde(default, alias = "tier", skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub requests_per_minute: u64,
    pub max_concurrent: u64,
    pub bytes_per_day: u64,
    pub per_host_requests_per_minute: u64,
    /// Refuse everything for this org (403).
    #[serde(default)]
    pub suspended: bool,
    /// Bytes already booked for this org today (UTC) elsewhere. Lets the
    /// daily cap hold across proxy replicas and restarts; each replica adds
    /// what it has counted since.
    #[serde(default)]
    pub bytes_used_today: u64,
}

impl OrgLimits {
    /// No caps at all.
    pub const fn unlimited() -> Self {
        Self {
            label: None,
            requests_per_minute: u64::MAX,
            max_concurrent: u64::MAX,
            bytes_per_day: u64::MAX,
            per_host_requests_per_minute: u64::MAX,
            suspended: false,
            bytes_used_today: 0,
        }
    }
}

/// Path of the limits endpoint on the control endpoint.
pub fn limits_path(org: &str) -> String {
    format!("/internal/egress-proxy/orgs/{org}/limits")
}

#[async_trait]
pub trait LimitsSource: Send + Sync {
    async fn fetch(&self, org: &str) -> anyhow::Result<OrgLimits>;
}

/// Asks the control endpoint (`EGRESS_PROXY_CONTROL_URL`).
pub struct HttpLimitsSource {
    client: reqwest::Client,
    base_url: String,
    secret: String,
}

impl HttpLimitsSource {
    pub fn new(base_url: impl Into<String>, secret: impl Into<String>) -> anyhow::Result<Self> {
        Ok(Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(5))
                // The control endpoint is reached directly; never through an
                // environment proxy.
                .no_proxy()
                .build()?,
            base_url: base_url.into().trim_end_matches('/').to_string(),
            secret: secret.into(),
        })
    }
}

#[async_trait]
impl LimitsSource for HttpLimitsSource {
    async fn fetch(&self, org: &str) -> anyhow::Result<OrgLimits> {
        let response = self
            .client
            .get(format!("{}{}", self.base_url, limits_path(org)))
            .bearer_auth(&self.secret)
            .send()
            .await?
            .error_for_status()?;
        Ok(response.json().await?)
    }
}

pub struct LimitsCache {
    /// `None`: every org gets `fallback`, no lookup.
    source: Option<Box<dyn LimitsSource>>,
    fallback: OrgLimits,
    ttl: Duration,
    fallback_ttl: Duration,
    entries: Mutex<HashMap<String, (Instant, OrgLimits)>>,
}

/// One lookup's result.
pub struct Lookup {
    pub limits: OrgLimits,
    /// A fresh answer from the source (not cache, not fallback).
    pub refreshed: bool,
}

impl LimitsCache {
    /// Look limits up in `source`, falling back to `fallback` when it fails.
    pub fn new(source: Box<dyn LimitsSource>, ttl: Duration, fallback: OrgLimits) -> Self {
        Self {
            source: Some(source),
            fallback,
            ttl,
            fallback_ttl: ttl.min(Duration::from_secs(10)),
            entries: Mutex::new(HashMap::new()),
        }
    }

    /// The same limits for every org.
    pub fn fixed(limits: OrgLimits) -> Self {
        Self {
            source: None,
            fallback: limits,
            ttl: Duration::ZERO,
            fallback_ttl: Duration::ZERO,
            entries: Mutex::new(HashMap::new()),
        }
    }

    /// What unscoped traffic (no org) and unreachable sources get.
    pub fn fallback(&self) -> &OrgLimits {
        &self.fallback
    }

    pub async fn get(&self, org: &str) -> Lookup {
        let Some(source) = &self.source else {
            return Lookup {
                limits: self.fallback.clone(),
                refreshed: false,
            };
        };
        let now = Instant::now();
        if let Some((until, limits)) = self.lock().get(org)
            && *until > now
        {
            return Lookup {
                limits: limits.clone(),
                refreshed: false,
            };
        }
        let (limits, ttl, refreshed) = match source.fetch(org).await {
            Ok(limits) => (limits, self.ttl, true),
            Err(err) => {
                tracing::warn!(org, error = %err, "egress limits unavailable; applying fallback limits");
                (self.fallback.clone(), self.fallback_ttl, false)
            }
        };
        let mut entries = self.lock();
        // Bound the map: entries expire on read, so sweep when it grows.
        if entries.len() > 50_000 {
            entries.retain(|_, (until, _)| *until > now);
        }
        entries.insert(org.to_string(), (now + ttl, limits.clone()));
        Lookup { limits, refreshed }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, (Instant, OrgLimits)>> {
        self.entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    struct Counting {
        calls: Arc<AtomicUsize>,
        answer: Option<OrgLimits>,
    }

    #[async_trait]
    impl LimitsSource for Counting {
        async fn fetch(&self, _org: &str) -> anyhow::Result<OrgLimits> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.answer.clone().ok_or_else(|| anyhow::anyhow!("down"))
        }
    }

    fn capped(rpm: u64) -> OrgLimits {
        OrgLimits {
            requests_per_minute: rpm,
            ..OrgLimits::unlimited()
        }
    }

    #[tokio::test]
    async fn answers_are_cached_per_org() {
        let calls = Arc::new(AtomicUsize::new(0));
        let cache = LimitsCache::new(
            Box::new(Counting {
                calls: calls.clone(),
                answer: Some(capped(600)),
            }),
            Duration::from_secs(60),
            capped(1),
        );
        let first = cache.get("org_a").await;
        assert!(first.refreshed);
        assert_eq!(first.limits.requests_per_minute, 600);
        let second = cache.get("org_a").await;
        assert!(!second.refreshed);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        cache.get("org_b").await;
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn an_unreachable_source_yields_the_fallback() {
        let cache = LimitsCache::new(
            Box::new(Counting {
                calls: Arc::new(AtomicUsize::new(0)),
                answer: None,
            }),
            Duration::from_secs(60),
            capped(120),
        );
        let lookup = cache.get("org_a").await;
        assert!(!lookup.refreshed);
        assert_eq!(lookup.limits, capped(120));
    }

    #[tokio::test]
    async fn fixed_limits_apply_to_every_org_without_refreshing() {
        let cache = LimitsCache::fixed(OrgLimits::unlimited());
        let lookup = cache.get("org_a").await;
        assert!(!lookup.refreshed);
        assert_eq!(lookup.limits, OrgLimits::unlimited());
    }

    #[test]
    fn a_hosted_tier_answer_reads_as_a_label() {
        // The shape a hosted control plane answers with today.
        let limits: OrgLimits = serde_json::from_value(serde_json::json!({
            "tier": "verified", "requests_per_minute": 600, "max_concurrent": 50,
            "bytes_per_day": 5_000_000_000u64, "per_host_requests_per_minute": 1_800,
            "suspended": false, "bytes_used_today": 7
        }))
        .expect("hosted limits");
        assert_eq!(limits.label.as_deref(), Some("verified"));
        assert_eq!(limits.bytes_used_today, 7);
        let minimal: OrgLimits = serde_json::from_value(serde_json::json!({
            "requests_per_minute": 1, "max_concurrent": 1,
            "bytes_per_day": 1, "per_host_requests_per_minute": 1
        }))
        .expect("minimal limits");
        assert!(!minimal.suspended);
        assert_eq!(minimal.label, None);
    }
}

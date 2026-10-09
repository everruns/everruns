//! Domain reputation for open reads.
//!
//! In `curated-writes`, reads may reach hosts outside the allowlist. Before the
//! first read of a host, the boundary asks a filtering DNS resolver whether the
//! domain is known malware, phishing, or otherwise blocked. Decision: a
//! security resolver (Cloudflare `security.cloudflare-dns.com`) rather than a
//! paid URL-reputation API, because it costs nothing, only the hostname leaves
//! the deployment, and it answers in one DNS-over-HTTPS round trip. A lookup
//! failure fails open: reputation narrows open reads, it is not what makes
//! them safe (the deny list, URL cap, rate limit, and audit log are).

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Selects the reputation source: `off` (default) or `cloudflare-security`.
pub const EGRESS_REPUTATION_ENV: &str = "EVERRUNS_EGRESS_REPUTATION";

const CLOUDFLARE_SECURITY_DOH: &str = "https://security.cloudflare-dns.com/dns-query";
const LOOKUP_TIMEOUT: Duration = Duration::from_secs(2);
const CACHE_TTL: Duration = Duration::from_secs(60 * 60);
const CACHE_MAX_ENTRIES: usize = 10_000;

/// Filtering-resolver reputation check with a per-host verdict cache.
pub struct DomainReputation {
    client: reqwest::Client,
    endpoint: String,
    cache: Mutex<HashMap<String, (bool, Instant)>>,
}

impl std::fmt::Debug for DomainReputation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DomainReputation")
            .field("endpoint", &self.endpoint)
            .finish_non_exhaustive()
    }
}

impl DomainReputation {
    /// Query a DNS-over-HTTPS endpoint speaking the `application/dns-json`
    /// dialect (Cloudflare, Google).
    pub fn with_endpoint(endpoint: impl Into<String>) -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(LOOKUP_TIMEOUT)
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .expect("build reputation HTTP client"),
            endpoint: endpoint.into(),
            cache: Mutex::new(HashMap::new()),
        }
    }

    /// Cloudflare's malware-blocking resolver (the 1.1.1.2 service).
    pub fn cloudflare_security() -> Self {
        Self::with_endpoint(CLOUDFLARE_SECURITY_DOH)
    }

    /// The reputation source named by [`EGRESS_REPUTATION_ENV`], if any. An
    /// unrecognized value is logged and treated as `off`.
    pub fn from_env() -> Option<Self> {
        match std::env::var(EGRESS_REPUTATION_ENV).ok().as_deref() {
            None | Some("off") => None,
            Some("cloudflare-security") => Some(Self::cloudflare_security()),
            Some(value) => {
                tracing::error!(
                    value,
                    "unrecognized {EGRESS_REPUTATION_ENV}; reputation off"
                );
                None
            }
        }
    }

    /// Whether the resolver blocks `host`. Fails open on lookup errors.
    pub async fn is_flagged(&self, host: &str) -> bool {
        let host = host.trim_end_matches('.').to_ascii_lowercase();
        if let Some(verdict) = self.cached(&host) {
            return verdict;
        }
        match self.lookup(&host).await {
            Ok(flagged) => {
                self.remember(host, flagged);
                flagged
            }
            Err(error) => {
                tracing::warn!(host = %host, error = %error, "domain reputation lookup failed; allowing");
                false
            }
        }
    }

    fn cached(&self, host: &str) -> Option<bool> {
        let cache = self.cache.lock().unwrap_or_else(|p| p.into_inner());
        cache
            .get(host)
            .filter(|(_, at)| at.elapsed() < CACHE_TTL)
            .map(|(verdict, _)| *verdict)
    }

    fn remember(&self, host: String, flagged: bool) {
        let mut cache = self.cache.lock().unwrap_or_else(|p| p.into_inner());
        if cache.len() >= CACHE_MAX_ENTRIES {
            cache.retain(|_, (_, at)| at.elapsed() < CACHE_TTL);
            if cache.len() >= CACHE_MAX_ENTRIES {
                cache.clear();
            }
        }
        cache.insert(host, (flagged, Instant::now()));
    }

    async fn lookup(&self, host: &str) -> Result<bool, String> {
        let url = reqwest::Url::parse_with_params(&self.endpoint, &[("name", host), ("type", "A")])
            .map_err(|error| error.to_string())?;
        let response = self
            .client
            .get(url)
            .header("accept", "application/dns-json")
            .send()
            .await
            .map_err(|error| error.to_string())?;
        if !response.status().is_success() {
            return Err(format!("status {}", response.status()));
        }
        let bytes = response.bytes().await.map_err(|error| error.to_string())?;
        let body: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
        Ok(is_blocked_answer(&body))
    }
}

/// A filtering resolver answers a blocked name with `0.0.0.0` (or `::`) and
/// an extended DNS error 15-17 ("Blocked", "Censored", "Filtered").
fn is_blocked_answer(body: &serde_json::Value) -> bool {
    let sinkholed = body
        .get("Answer")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|answers| {
            answers.iter().any(|answer| {
                matches!(
                    answer.get("data").and_then(serde_json::Value::as_str),
                    Some("0.0.0.0" | "::")
                )
            })
        });
    let filtered = body
        .get("Comment")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|comments| {
            comments
                .iter()
                .filter_map(serde_json::Value::as_str)
                .any(|comment| {
                    ["EDE(15)", "EDE(16)", "EDE(17)"]
                        .iter()
                        .any(|code| comment.starts_with(code))
                })
        });
    sinkholed || filtered
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{header, method, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn recognizes_blocked_answers() {
        assert!(is_blocked_answer(&json!({
            "Status": 0,
            "Answer": [{"name": "malware.testcategory.com", "type": 1, "data": "0.0.0.0"}],
            "Comment": ["EDE(16): Censored"]
        })));
        assert!(is_blocked_answer(
            &json!({"Status": 0, "Comment": ["EDE(17): Filtered"]})
        ));
        assert!(!is_blocked_answer(&json!({
            "Status": 0,
            "Answer": [{"name": "docs.rs", "type": 1, "data": "151.101.1.91"}]
        })));
        assert!(!is_blocked_answer(&json!({"Status": 3})));
    }

    #[tokio::test]
    async fn flags_sinkholed_hosts_caches_verdicts_and_fails_open() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(query_param("name", "bad.example"))
            .and(header("accept", "application/dns-json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "Status": 0,
                "Answer": [{"name": "bad.example", "type": 1, "data": "0.0.0.0"}]
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(query_param("name", "good.example"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "Status": 0,
                "Answer": [{"name": "good.example", "type": 1, "data": "93.184.216.34"}]
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(query_param("name", "broken.example"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;

        let reputation = DomainReputation::with_endpoint(format!("{}/dns-query", server.uri()));
        assert!(reputation.is_flagged("Bad.Example.").await);
        // Second lookup is served from cache (the mock expects one call).
        assert!(reputation.is_flagged("bad.example").await);
        assert!(!reputation.is_flagged("good.example").await);
        assert!(!reputation.is_flagged("broken.example").await);
    }

    #[tokio::test]
    async fn boundary_denies_open_reads_of_flagged_hosts_only() {
        use crate::{EgressError, EgressRequest, EgressRequestKind, EgressService};
        use everruns_contracts::runtime::{EgressPolicyMode, SystemEgressPolicy};
        use std::sync::Arc;

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "Status": 0,
                "Answer": [{"name": "x", "type": 1, "data": "0.0.0.0"}]
            })))
            .mount(&server)
            .await;
        let service = super::super::DirectEgressService::new()
            .with_system_policy(Some(Arc::new(SystemEgressPolicy::embedded(
                EgressPolicyMode::CuratedWrites,
            ))))
            .with_reputation(Some(Arc::new(DomainReputation::with_endpoint(format!(
                "{}/dns-query",
                server.uri()
            )))));
        let error = service
            .send(EgressRequest::new(
                "GET",
                "https://flagged.example.net/",
                EgressRequestKind::Capability,
            ))
            .await
            .unwrap_err();
        assert!(
            matches!(&error, EgressError::NetworkAccessDenied { url } if url.contains("reputation")),
            "{error}"
        );
        // Allowlisted hosts never consult the reputation source.
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
        let _ = service
            .send(EgressRequest::new(
                "GET",
                "https://127.0.0.1.invalid.crates.io/",
                EgressRequestKind::Capability,
            ))
            .await;
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }
}

//! Optional 10-second detail for Monitoring, read from a Prometheus-compatible
//! store (VictoriaMetrics, Prometheus) that scrapes the shards' `/metrics`.
//!
//! Decisions (knowledge/operations/monitoring.md):
//! - PostgreSQL minute history is the record and always present; this source
//!   only adds per-node detail for the last hour, so Monitoring works without it.
//! - The address comes from the environment only (`MONITORING_DETAIL_URL`),
//!   never from the API or the database: a settable URL would let anyone with
//!   platform access point the server at an internal host.
//! - Shard names reach PromQL only after [`valid_shard`]: label values are quoted,
//!   and a name with a quote or backslash would end the string.

use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::sync::OnceLock;
use std::time::Duration;

/// Seconds between detail points.
pub const STEP_SECONDS: u32 = 10;
/// The most detail one read covers.
pub const MAX_MINUTES: u32 = 60;

/// Where detail comes from, as configured.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DetailConfig {
    /// `MONITORING_DETAIL` unset or `off`.
    Off,
    /// A Prometheus-compatible query API at `url` (no trailing slash).
    Prometheus { url: String },
    /// Set, but not usable; the reason is shown to operators.
    Invalid(String),
}

impl DetailConfig {
    pub fn from_values(kind: Option<&str>, url: Option<&str>) -> Self {
        let kind = kind.map(str::trim).unwrap_or("").to_ascii_lowercase();
        match kind.as_str() {
            "" | "off" | "none" => Self::Off,
            "prometheus" | "victoriametrics" => {
                match url.map(str::trim).filter(|u| !u.is_empty()) {
                    None => Self::Invalid("MONITORING_DETAIL_URL is not set".into()),
                    Some(url) => match reqwest::Url::parse(url) {
                        Ok(parsed) if matches!(parsed.scheme(), "http" | "https") => {
                            Self::Prometheus {
                                url: url.trim_end_matches('/').to_owned(),
                            }
                        }
                        _ => Self::Invalid("MONITORING_DETAIL_URL must be an http(s) URL".into()),
                    },
                }
            }
            other => Self::Invalid(format!(
                "MONITORING_DETAIL={other} is not known; use prometheus or off"
            )),
        }
    }

    /// Read once per process.
    pub fn from_env() -> &'static Self {
        static CONFIG: OnceLock<DetailConfig> = OnceLock::new();
        CONFIG.get_or_init(|| {
            Self::from_values(
                std::env::var("MONITORING_DETAIL").ok().as_deref(),
                std::env::var("MONITORING_DETAIL_URL").ok().as_deref(),
            )
        })
    }

    /// The host shown to operators: scheme and authority only, so a URL
    /// carrying credentials or a token never reaches a page.
    pub fn display_host(&self) -> Option<String> {
        let Self::Prometheus { url } = self else {
            return None;
        };
        let parsed = reqwest::Url::parse(url).ok()?;
        let host = parsed.host_str()?;
        Some(match parsed.port() {
            Some(port) => format!("{}://{host}:{port}", parsed.scheme()),
            None => format!("{}://{host}", parsed.scheme()),
        })
    }
}

/// A shard name safe to put in a PromQL label matcher.
pub fn valid_shard(shard: &str) -> bool {
    !shard.is_empty()
        && shard.len() <= 128
        && shard
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

/// One node's series, keyed by unix second.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct NodeSeries {
    pub running: BTreeMap<i64, f64>,
    pub rss_bytes: BTreeMap<i64, f64>,
    pub queued: BTreeMap<i64, f64>,
    pub admit_p99_seconds: BTreeMap<i64, f64>,
}

/// The queries behind each series, one per node.
fn queries(shard: &str) -> [(&'static str, String); 4] {
    let m = |metric: &str, agg: &str| format!("{agg} by (node) ({metric}{{shard=\"{shard}\"}})");
    [
        ("running", m("agentd_shard_turns_running", "sum")),
        ("rss_bytes", m("agentd_shard_rss_bytes", "max")),
        ("queued", m("agentd_shard_queued_inputs", "max")),
        ("admit", m("agentd_shard_slo_admit_p99_seconds", "max")),
    ]
}

#[derive(Deserialize)]
struct QueryResponse {
    status: String,
    #[serde(default)]
    data: Option<QueryData>,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Deserialize)]
struct QueryData {
    #[serde(default)]
    result: Vec<RangeSeries>,
}

#[derive(Deserialize)]
struct RangeSeries {
    #[serde(default)]
    metric: BTreeMap<String, String>,
    #[serde(default)]
    values: Vec<(f64, String)>,
}

fn client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap_or_default()
    })
}

/// Fold one `query_range` response into `nodes`.
fn fold(nodes: &mut BTreeMap<String, NodeSeries>, which: &str, body: &str) -> Result<(), String> {
    let parsed: QueryResponse =
        serde_json::from_str(body).map_err(|e| format!("unreadable answer: {e}"))?;
    if parsed.status != "success" {
        return Err(parsed.error.unwrap_or_else(|| "query failed".into()));
    }
    for series in parsed.data.map(|d| d.result).unwrap_or_default() {
        let node = series.metric.get("node").cloned().unwrap_or_default();
        let entry = nodes.entry(node).or_default();
        let target = match which {
            "running" => &mut entry.running,
            "rss_bytes" => &mut entry.rss_bytes,
            "queued" => &mut entry.queued,
            _ => &mut entry.admit_p99_seconds,
        };
        for (t, v) in series.values {
            if let Ok(v) = v.parse::<f64>()
                && v.is_finite()
            {
                target.insert(t as i64, v);
            }
        }
    }
    Ok(())
}

/// Read `shard`'s per-node detail over `[from, to]`.
pub async fn read_shard(
    url: &str,
    shard: &str,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Result<BTreeMap<String, NodeSeries>, String> {
    if !valid_shard(shard) {
        return Err("invalid shard name".into());
    }
    let mut nodes = BTreeMap::new();
    let endpoint = format!("{url}/api/v1/query_range");
    let reads = queries(shard).map(|(which, query)| {
        let request = client().get(&endpoint).query(&[
            ("query", query),
            ("start", from.timestamp().to_string()),
            ("end", to.timestamp().to_string()),
            ("step", format!("{STEP_SECONDS}s")),
        ]);
        async move {
            let response = request
                .send()
                .await
                .map_err(|e| e.without_url().to_string())?;
            let status = response.status();
            let body = response
                .text()
                .await
                .map_err(|e| e.without_url().to_string())?;
            if !status.is_success() {
                return Err(format!("answered {status}"));
            }
            Ok::<_, String>((which, body))
        }
    });
    for result in futures::future::join_all(reads).await {
        let (which, body) = result?;
        fold(&mut nodes, which, &body)?;
    }
    Ok(nodes)
}

/// Whether the store answers a trivial query; for the system doctor.
pub async fn probe(url: &str) -> Result<(), String> {
    let response = client()
        .get(format!("{url}/api/v1/query"))
        .query(&[("query", "1")])
        .send()
        .await
        .map_err(|e| e.without_url().to_string())?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|e| e.without_url().to_string())?;
    if !status.is_success() {
        return Err(format!("answered {status}"));
    }
    let parsed: QueryResponse =
        serde_json::from_str(&body).map_err(|e| format!("unreadable answer: {e}"))?;
    if parsed.status == "success" {
        Ok(())
    } else {
        Err(parsed.error.unwrap_or_else(|| "query failed".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_comes_from_two_variables() {
        assert_eq!(DetailConfig::from_values(None, None), DetailConfig::Off);
        assert_eq!(
            DetailConfig::from_values(Some("off"), Some("http://x")),
            DetailConfig::Off
        );
        assert_eq!(
            DetailConfig::from_values(Some("VictoriaMetrics"), Some("http://vm:8428/")),
            DetailConfig::Prometheus {
                url: "http://vm:8428".into()
            }
        );
        assert!(matches!(
            DetailConfig::from_values(Some("prometheus"), None),
            DetailConfig::Invalid(_)
        ));
        assert!(matches!(
            DetailConfig::from_values(Some("prometheus"), Some("file:///etc/passwd")),
            DetailConfig::Invalid(_)
        ));
        assert!(matches!(
            DetailConfig::from_values(Some("graphite"), Some("http://x")),
            DetailConfig::Invalid(_)
        ));
    }

    #[test]
    fn the_shown_host_drops_credentials_and_path() {
        let config = DetailConfig::from_values(
            Some("prometheus"),
            Some("https://user:secret@vm.internal:8443/select/0/prometheus?token=t"),
        );
        assert_eq!(
            config.display_host().as_deref(),
            Some("https://vm.internal:8443")
        );
    }

    #[test]
    fn only_plain_shard_names_reach_promql() {
        assert!(valid_shard("shard-07"));
        assert!(valid_shard("dp1.shard_2"));
        assert!(!valid_shard(""));
        assert!(!valid_shard("a\"}or vector(1)"));
        assert!(!valid_shard("a\\b"));
    }

    #[test]
    fn range_answers_fold_by_node() {
        let mut nodes = BTreeMap::new();
        let body = r#"{"status":"success","data":{"resultType":"matrix","result":[
            {"metric":{"node":"1"},"values":[[1700000000,"3"],[1700000010,"5"]]},
            {"metric":{"node":"2"},"values":[[1700000000,"NaN"],[1700000010,"1"]]}]}}"#;
        fold(&mut nodes, "running", body).unwrap();
        assert_eq!(nodes["1"].running.get(&1_700_000_010), Some(&5.0));
        assert_eq!(nodes["2"].running.len(), 1, "NaN points are dropped");
        let err = fold(
            &mut nodes,
            "running",
            r#"{"status":"error","error":"bad query"}"#,
        );
        assert_eq!(err.unwrap_err(), "bad query");
    }

    #[tokio::test]
    async fn a_shard_is_read_from_a_prometheus_api() {
        use axum::{Router, extract::Query, routing::get};
        async fn range(Query(q): Query<BTreeMap<String, String>>) -> String {
            assert_eq!(q["step"], "10s");
            assert!(q["query"].contains("shard=\"shard-07\""), "{}", q["query"]);
            let value = if q["query"].contains("agentd_shard_rss_bytes") {
                "1024"
            } else {
                "2"
            };
            format!(
                r#"{{"status":"success","data":{{"result":[{{"metric":{{"node":"3"}},"values":[[{},"{value}"]]}}]}}}}"#,
                q["start"]
            )
        }
        async fn instant() -> &'static str {
            r#"{"status":"success","data":{"result":[]}}"#
        }
        let app = Router::new()
            .route("/api/v1/query_range", get(range))
            .route("/api/v1/query", get(instant));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        probe(&url).await.unwrap();
        let to = Utc::now();
        let from = to - chrono::TimeDelta::minutes(15);
        let nodes = read_shard(&url, "shard-07", from, to).await.unwrap();
        let node = &nodes["3"];
        assert_eq!(node.rss_bytes.get(&from.timestamp()), Some(&1024.0));
        assert_eq!(node.running.get(&from.timestamp()), Some(&2.0));
        assert!(read_shard(&url, "x\"", from, to).await.is_err());
        assert!(probe("http://127.0.0.1:1").await.is_err());
    }
}

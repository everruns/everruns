//! Monitoring → Shards → one shard's last hour, per node, every 10 seconds,
//! from the optional Prometheus-compatible detail store
//! (`domains::system::monitoring_detail`). Platform only.
//!
//! Decision: a missing or unreachable store is a normal answer
//! (`available: false` with the reason), not an error, so the page keeps the
//! minute history and says why the detail is absent.

use axum::{
    Json,
    extract::{Path, Query},
    http::StatusCode,
};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::{
    api::common::{ApiResult, ErrorResponse},
    auth::PlatformUser,
    domains::system::monitoring_detail::{
        self as detail, DetailConfig, MAX_MINUTES, NodeSeries, STEP_SECONDS,
    },
};

/// How much detail to read.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ShardDetailQuery {
    /// Minutes ending now, 5 to 60. Defaults to 60.
    pub minutes: Option<u32>,
}

/// One point of a node's detail.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ShardNodePoint {
    /// UTC time of the point.
    #[schema(example = "2026-11-15T19:59:50Z")]
    pub t: DateTime<Utc>,
    /// Turns running on the node.
    #[schema(example = 81)]
    pub running: Option<f64>,
    /// Resident memory in bytes.
    #[schema(example = 1073741824_u64)]
    pub rss_bytes: Option<f64>,
    /// Messages waiting for a turn, as the node sees them.
    #[schema(example = 0)]
    pub queued: Option<f64>,
    /// One-minute p99 time to admit a turn, in milliseconds.
    #[schema(example = 12.5)]
    pub admit_p99_ms: Option<f64>,
}

/// One node's detail.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ShardNodeDetail {
    /// Node id within the shard.
    #[schema(example = "2")]
    pub node: String,
    /// Points, oldest first.
    pub points: Vec<ShardNodePoint>,
}

/// A shard's per-node detail, or why there is none.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct MonitoringShardDetail {
    /// Shard name.
    #[schema(example = "shard-07")]
    pub shard: String,
    /// Whether detail could be read.
    #[schema(example = true)]
    pub available: bool,
    /// Why not, when `available` is false.
    #[schema(example = "No detail store is configured (MONITORING_DETAIL).")]
    pub reason: Option<String>,
    /// The detail store's scheme and host, without credentials or path.
    #[schema(example = "http://victoriametrics:8428")]
    pub source: Option<String>,
    /// Seconds between points.
    #[schema(example = 10)]
    pub step_seconds: u32,
    /// Inclusive UTC start.
    #[schema(example = "2026-11-15T19:00:00Z")]
    pub from: DateTime<Utc>,
    /// UTC end.
    #[schema(example = "2026-11-15T20:00:00Z")]
    pub to: DateTime<Utc>,
    /// Nodes, by id.
    pub nodes: Vec<ShardNodeDetail>,
}

fn points(series: NodeSeries) -> Vec<ShardNodePoint> {
    let mut times: Vec<i64> = series
        .running
        .keys()
        .chain(series.rss_bytes.keys())
        .chain(series.queued.keys())
        .chain(series.admit_p99_seconds.keys())
        .copied()
        .collect();
    times.sort_unstable();
    times.dedup();
    times
        .into_iter()
        .filter_map(|t| {
            Some(ShardNodePoint {
                t: DateTime::from_timestamp(t, 0)?,
                running: series.running.get(&t).copied(),
                rss_bytes: series.rss_bytes.get(&t).copied(),
                queued: series.queued.get(&t).copied(),
                admit_p99_ms: series.admit_p99_seconds.get(&t).map(|s| s * 1000.0),
            })
        })
        .collect()
}

/// Read one shard's per-node detail for the last hour. Requires platform-user access.
#[utoipa::path(get, path = "/v1/durable/monitoring/shards/{shard}/detail", tag = "durable",
    params(("shard" = String, Path, description = "Shard name"), ShardDetailQuery),
    responses((status = 200, description = "Per-node detail, or why there is none", body = MonitoringShardDetail),
        (status = 400, description = "Invalid shard name or minutes", body = ErrorResponse),
        (status = 403, description = "Platform access required", body = ErrorResponse)))]
pub async fn get_fleet_monitoring_shard_detail(
    _platform: PlatformUser,
    Path(shard): Path<String>,
    Query(query): Query<ShardDetailQuery>,
) -> ApiResult<MonitoringShardDetail> {
    let bad = |m: &str| ErrorResponse::new(m).into_response(StatusCode::BAD_REQUEST);
    if !detail::valid_shard(&shard) {
        return Err(bad("shard must be letters, digits, '-', '_' or '.'"));
    }
    let minutes = query.minutes.unwrap_or(MAX_MINUTES);
    if !(5..=MAX_MINUTES).contains(&minutes) {
        return Err(bad("minutes must be between 5 and 60"));
    }
    let to = Utc::now();
    let from = to - Duration::minutes(i64::from(minutes));
    let config = DetailConfig::from_env();
    Ok(Json(answer(config, &shard, from, to).await))
}

pub(super) async fn answer(
    config: &DetailConfig,
    shard: &str,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> MonitoringShardDetail {
    let mut out = MonitoringShardDetail {
        shard: shard.to_owned(),
        available: false,
        reason: None,
        source: config.display_host(),
        step_seconds: STEP_SECONDS,
        from,
        to,
        nodes: Vec::new(),
    };
    match config {
        DetailConfig::Off => {
            out.reason = Some("No detail store is configured (MONITORING_DETAIL).".into());
        }
        DetailConfig::Invalid(why) => out.reason = Some(why.clone()),
        DetailConfig::Prometheus { url } => match detail::read_shard(url, shard, from, to).await {
            Ok(nodes) => {
                out.available = true;
                out.nodes = nodes
                    .into_iter()
                    .map(|(node, series)| ShardNodeDetail {
                        node,
                        points: points(series),
                    })
                    .collect();
            }
            Err(error) => {
                tracing::warn!(%error, shard, "Monitoring detail store did not answer");
                out.reason = Some(format!("The detail store did not answer: {error}"));
            }
        },
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn without_a_store_the_answer_says_why() {
        let now = Utc::now();
        let off = answer(&DetailConfig::Off, "shard-01", now, now).await;
        assert!(!off.available);
        assert!(off.reason.unwrap().contains("MONITORING_DETAIL"));
        let down = answer(
            &DetailConfig::Prometheus {
                url: "http://127.0.0.1:1".into(),
            },
            "shard-01",
            now - Duration::minutes(5),
            now,
        )
        .await;
        assert!(!down.available);
        assert_eq!(down.source.as_deref(), Some("http://127.0.0.1:1"));
        assert!(
            down.reason
                .unwrap()
                .starts_with("The detail store did not answer")
        );
    }

    #[test]
    fn series_merge_into_points_by_time() {
        let mut s = NodeSeries::default();
        s.running.insert(20, 3.0);
        s.rss_bytes.insert(10, 100.0);
        s.admit_p99_seconds.insert(20, 0.012);
        let p = points(s);
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].rss_bytes, Some(100.0));
        assert_eq!(p[0].running, None);
        assert_eq!(p[1].running, Some(3.0));
        assert_eq!(p[1].admit_p99_ms, Some(12.0));
    }
}

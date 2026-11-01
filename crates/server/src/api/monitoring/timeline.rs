//! Monitoring → Timeline: recorded minute history aggregated into buckets.
//! Authorization matches the live snapshot: organization adoption plus
//! session-read authority, or platform access for the fleet view.

use axum::{
    Json,
    extract::{Query, State},
    http::StatusCode,
};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use super::{AppState, authorize_org};
use crate::{
    api::common::{ApiResult, ApiResultExt, ErrorResponse},
    auth::{PlatformUser, ResolvedOrg},
    storage::{PLATFORM_ORG, TimelineBucketRow, TimelineWorkerBucket, timeline_minute},
};

const MAX_RANGE_DAYS: i64 = 31;
const MAX_WINDOW_DAYS: i64 = 7;
const MAX_BUCKETS: i64 = 1500;
const BUCKET_CHOICES: &[i64] = &[60, 300, 900, 3600, 21600, 86400];

/// Range, explicit bounds, and bucket size for a timeline read.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct TimelineQuery {
    /// Range ending now: 15m, 1h, 6h, 24h, 7d, or 30d. Ignored when `from` is set. Defaults to 24h.
    #[param(example = "24h")]
    pub range: Option<String>,
    /// Inclusive UTC start; use with `to` for a custom range of up to 31 days.
    pub from: Option<DateTime<Utc>>,
    /// Exclusive UTC end; defaults to now.
    pub to: Option<DateTime<Utc>>,
    /// Bucket size in seconds: 60, 300, 900, 3600, 21600, or 86400. Chosen from the range when omitted.
    #[param(example = 300)]
    pub bucket: Option<i64>,
}

/// Bounds of a drill-down window.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct TimelineWindowQuery {
    /// Inclusive UTC start of the selected window.
    pub from: DateTime<Utc>,
    /// Exclusive UTC end of the selected window; at most 7 days after `from`.
    pub to: DateTime<Utc>,
}

/// Load of one worker over a bucket (platform scope).
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct TimelineWorkerLoad {
    /// Worker identifier from the registry.
    #[schema(example = "worker-01")]
    pub worker_id: String,
    /// Average reported load over the bucket's samples.
    #[schema(example = 31.5)]
    pub load_avg: f64,
    /// Reported maximum concurrency.
    #[schema(example = 50)]
    pub capacity: u64,
}

/// One time bucket. Averages are over 10-second samples; counters are exact.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct TimelineBucket {
    /// UTC start of the bucket.
    #[schema(example = "2026-11-01T05:00:00Z")]
    pub start: DateTime<Utc>,
    /// Whether population was sampled; false for empty buckets and history rebuilt from turn records.
    pub sampled: bool,
    /// Whether this bucket includes history rebuilt from turn records (no phase split or queue data).
    pub backfilled: bool,
    /// Average sessions with an active, runnable, or held actor.
    #[schema(example = 96.4)]
    pub active_avg: f64,
    /// Highest sampled count of such sessions.
    #[schema(example = 112)]
    pub active_peak: u64,
    /// Average sessions in a model phase.
    pub model_avg: f64,
    /// Average sessions running tools.
    pub tools_avg: f64,
    /// Average sessions waiting for input or external tool results.
    pub waiting_avg: f64,
    /// Average sessions ready and waiting for a worker lease.
    pub runnable_avg: f64,
    /// Average sessions in turn setup or a phase handoff.
    pub setup_avg: f64,
    /// Highest sampled count of sessions waiting for a worker.
    pub runnable_peak: u64,
    /// Longest wait for a worker lease observed in the bucket, in milliseconds.
    pub oldest_runnable_ms: Option<u64>,
    /// Highest sampled count of valid actor leases.
    pub leases_peak: u64,
    /// Expired leases another worker took over.
    pub lease_takeovers: u64,
    /// Turns started.
    pub turns_started: u64,
    /// Turns completed.
    pub turns_completed: u64,
    /// Turns failed.
    pub turns_failed: u64,
    /// Turns cancelled.
    pub turns_cancelled: u64,
    /// Model calls started.
    pub model_calls: u64,
    /// Tool calls started.
    pub tool_calls: u64,
    /// Tool calls that ended in an error or timeout.
    pub tool_failures: u64,
    /// Model input tokens.
    pub input_tokens: u64,
    /// Model output tokens.
    pub output_tokens: u64,
    /// Model spend in USD, actual where reconciled, otherwise estimated.
    #[schema(example = 0.29)]
    pub cost_usd: f64,
    /// Highest per-minute p95 model call duration in the bucket, in milliseconds.
    pub model_latency_p95_ms: Option<u64>,
    /// Platform only: highest number of organizations with active sessions at once.
    pub organizations_peak: Option<u64>,
    /// Platform only: registered workers with a fresh heartbeat.
    pub workers: Option<u64>,
    /// Platform only: total worker slots.
    pub worker_capacity: Option<u64>,
    /// Platform only: average reported worker load across all workers.
    pub worker_load_avg: Option<f64>,
    /// Platform only: highest reported total worker load.
    pub worker_load_peak: Option<u64>,
    /// Platform only: per-worker average load.
    pub worker_loads: Vec<TimelineWorkerLoad>,
}

/// Totals over a range, for the summary row and the previous-period comparison.
#[derive(Debug, Clone, Default, Serialize, ToSchema)]
pub struct TimelineSummary {
    /// Highest sampled parallel sessions.
    pub peak_active: u64,
    /// Minute of that peak.
    pub peak_at: Option<DateTime<Utc>>,
    /// Turns started.
    pub turns_started: u64,
    /// Turns that finished: completed, failed, or cancelled.
    pub turns_finished: u64,
    /// Turns failed.
    pub turns_failed: u64,
    /// Failed turns divided by finished turns; null when none finished.
    pub failure_rate: Option<f64>,
    /// Tool calls started.
    pub tool_calls: u64,
    /// Longest wait for a worker lease, in milliseconds.
    pub longest_wait_ms: Option<u64>,
    /// Start of the bucket with the longest wait.
    pub longest_wait_at: Option<DateTime<Utc>>,
    /// Model spend in USD.
    pub cost_usd: f64,
    /// Platform only: seconds of buckets whose peak load exceeded 90% of capacity.
    pub seconds_above_90: Option<u64>,
}

/// Deploy or worker change on the timeline.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct TimelineMarker {
    /// UTC time of the change.
    pub at: DateTime<Utc>,
    /// deploy, worker_joined, or worker_left (worker changes are platform only).
    #[schema(example = "deploy")]
    pub kind: String,
    /// Server version for a deploy, worker id otherwise.
    #[schema(example = "0.50.1")]
    pub label: String,
}

/// Recorded execution history for one organization or the platform.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct MonitoringTimeline {
    /// UTC time of this read.
    pub generated_at: DateTime<Utc>,
    /// organization or fleet.
    #[schema(example = "organization")]
    pub scope: String,
    /// Inclusive UTC start, aligned to the bucket size.
    pub from: DateTime<Utc>,
    /// Exclusive UTC end.
    pub to: DateTime<Utc>,
    /// Bucket size in seconds.
    #[schema(example = 300)]
    pub bucket_seconds: u32,
    /// Every bucket in the range, oldest first; buckets without history are zero.
    pub buckets: Vec<TimelineBucket>,
    /// Totals for this range.
    pub summary: TimelineSummary,
    /// Totals for the same length of time immediately before.
    pub previous: TimelineSummary,
    /// Deploys and worker changes inside the range.
    pub markers: Vec<TimelineMarker>,
}

/// One agent (organization) or organization (platform) ranked in a window.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct TimelineRank {
    /// Agent or organization public identifier; null for sessions without an agent.
    pub id: Option<String>,
    /// Agent name; null in platform scope.
    pub name: Option<String>,
    /// Sessions that started a turn in the window.
    pub sessions: u64,
    /// Turns started in the window.
    pub turns: u64,
}

/// Failed turns of one classified error code.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct TimelineFailure {
    /// Classified error code, or other.
    #[schema(example = "provider_rate_limited")]
    pub code: String,
    /// Failed turns with this code.
    pub turns: u64,
}

/// Breakdown of a selected window.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct MonitoringTimelineWindow {
    /// Inclusive UTC start.
    pub from: DateTime<Utc>,
    /// Exclusive UTC end.
    pub to: DateTime<Utc>,
    /// Up to 5 busiest agents (organization) or organizations (platform).
    pub busiest: Vec<TimelineRank>,
    /// Up to 6 failure codes, most frequent first.
    pub failures: Vec<TimelineFailure>,
}

type Bad = (StatusCode, Json<ErrorResponse>);

fn bad(message: &str) -> Bad {
    ErrorResponse::new(message).into_response(StatusCode::BAD_REQUEST)
}

fn range_length(range: &str) -> Option<Duration> {
    Some(match range {
        "15m" => Duration::minutes(15),
        "1h" => Duration::hours(1),
        "6h" => Duration::hours(6),
        "24h" => Duration::hours(24),
        "7d" => Duration::days(7),
        "30d" => Duration::days(30),
        _ => return None,
    })
}

fn default_bucket(length: Duration) -> i64 {
    if length <= Duration::hours(6) {
        60
    } else if length <= Duration::hours(24) {
        300
    } else {
        3600
    }
}

fn align_down(at: DateTime<Utc>, secs: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(at.timestamp().div_euclid(secs) * secs, 0).unwrap_or(at)
}

/// Validated `(from, to, bucket)` for a timeline read.
pub(super) fn resolve(
    query: &TimelineQuery,
    now: DateTime<Utc>,
) -> Result<(DateTime<Utc>, DateTime<Utc>, i64), Bad> {
    let end = timeline_minute(now) + Duration::minutes(1);
    let (from, to) = match query.from {
        Some(from) => (from, query.to.unwrap_or(end).min(end)),
        None => {
            let length = range_length(query.range.as_deref().unwrap_or("24h"))
                .ok_or_else(|| bad("range must be one of 15m, 1h, 6h, 24h, 7d, 30d"))?;
            (end - length, end)
        }
    };
    if from >= to {
        return Err(bad("from must be before to"));
    }
    if to - from > Duration::days(MAX_RANGE_DAYS) {
        return Err(bad("range is limited to 31 days"));
    }
    let bucket = match query.bucket {
        Some(b) if BUCKET_CHOICES.contains(&b) => b,
        Some(_) => {
            return Err(bad(
                "bucket must be one of 60, 300, 900, 3600, 21600, 86400",
            ));
        }
        None => default_bucket(to - from),
    };
    if (to - from).num_seconds() / bucket > MAX_BUCKETS {
        return Err(bad(
            "too many buckets for this range; choose a larger bucket",
        ));
    }
    Ok((align_down(from, bucket), to, bucket))
}

fn ratio(sum: i64, samples: i64) -> f64 {
    if samples <= 0 {
        0.0
    } else {
        // One decimal is all a chart needs.
        (sum as f64 / samples as f64 * 10.0).round() / 10.0
    }
}

fn nonneg(v: i64) -> u64 {
    v.max(0) as u64
}

pub(super) fn bucket(
    start: DateTime<Utc>,
    row: Option<&TimelineBucketRow>,
    workers: Option<&TimelineWorkerBucket>,
    platform: bool,
) -> TimelineBucket {
    let Some(r) = row else {
        return TimelineBucket {
            start,
            sampled: false,
            backfilled: false,
            active_avg: 0.0,
            active_peak: 0,
            model_avg: 0.0,
            tools_avg: 0.0,
            waiting_avg: 0.0,
            runnable_avg: 0.0,
            setup_avg: 0.0,
            runnable_peak: 0,
            oldest_runnable_ms: None,
            leases_peak: 0,
            lease_takeovers: 0,
            turns_started: 0,
            turns_completed: 0,
            turns_failed: 0,
            turns_cancelled: 0,
            model_calls: 0,
            tool_calls: 0,
            tool_failures: 0,
            input_tokens: 0,
            output_tokens: 0,
            cost_usd: 0.0,
            model_latency_p95_ms: None,
            organizations_peak: platform.then_some(0),
            workers: None,
            worker_capacity: None,
            worker_load_avg: None,
            worker_load_peak: None,
            worker_loads: vec![],
        };
    };
    let s = r.samples;
    TimelineBucket {
        start,
        sampled: s > 0 && !r.backfilled,
        backfilled: r.backfilled,
        active_avg: ratio(r.active_sum, s),
        active_peak: nonneg(r.active_peak.into()),
        model_avg: ratio(r.model_sum, s),
        tools_avg: ratio(r.tools_sum, s),
        waiting_avg: ratio(r.waiting_sum, s),
        runnable_avg: ratio(r.runnable_sum, s),
        setup_avg: ratio(r.setup_sum, s),
        runnable_peak: nonneg(r.runnable_peak.into()),
        oldest_runnable_ms: r.oldest_runnable_ms.map(nonneg),
        leases_peak: nonneg(r.leases_peak.into()),
        lease_takeovers: nonneg(r.lease_takeovers),
        turns_started: nonneg(r.turns_started),
        turns_completed: nonneg(r.turns_completed),
        turns_failed: nonneg(r.turns_failed),
        turns_cancelled: nonneg(r.turns_cancelled),
        model_calls: nonneg(r.model_calls),
        tool_calls: nonneg(r.tool_calls),
        tool_failures: nonneg(r.tool_failures),
        input_tokens: nonneg(r.input_tokens),
        output_tokens: nonneg(r.output_tokens),
        cost_usd: r.cost_usd.max(0.0),
        model_latency_p95_ms: r.model_latency_p95_ms.map(|v| nonneg(v.into())),
        organizations_peak: platform.then(|| nonneg(r.organizations_peak.into())),
        workers: platform
            .then(|| r.workers.map(|v| nonneg(v.into())))
            .flatten(),
        worker_capacity: platform
            .then(|| r.worker_capacity.map(|v| nonneg(v.into())))
            .flatten(),
        worker_load_avg: platform
            .then(|| r.worker_load_sum.map(|sum| ratio(sum, s)))
            .flatten(),
        worker_load_peak: platform
            .then(|| r.worker_load_peak.map(|v| nonneg(v.into())))
            .flatten(),
        worker_loads: workers
            .filter(|_| platform)
            .map(|w| {
                w.iter()
                    .map(|(id, (load, samples, capacity))| TimelineWorkerLoad {
                        worker_id: id.clone(),
                        load_avg: ratio(*load, *samples),
                        capacity: nonneg(*capacity),
                    })
                    .collect()
            })
            .unwrap_or_default(),
    }
}

pub(super) fn summarize(rows: &[TimelineBucketRow], bucket_secs: i64) -> TimelineSummary {
    let mut s = TimelineSummary::default();
    let mut above = 0u64;
    for r in rows {
        let peak = nonneg(r.active_peak.into());
        if peak > s.peak_active || s.peak_at.is_none() && peak > 0 {
            s.peak_active = peak;
            s.peak_at = r.peak_at;
        }
        s.turns_started += nonneg(r.turns_started);
        s.turns_finished += nonneg(r.turns_completed + r.turns_failed + r.turns_cancelled);
        s.turns_failed += nonneg(r.turns_failed);
        s.tool_calls += nonneg(r.tool_calls);
        s.cost_usd += r.cost_usd.max(0.0);
        if let Some(wait) = r.oldest_runnable_ms.map(nonneg)
            && s.longest_wait_ms.is_none_or(|w| wait > w)
        {
            s.longest_wait_ms = Some(wait);
            s.longest_wait_at = Some(r.bucket);
        }
        if let (Some(load), Some(cap)) = (r.worker_load_peak, r.worker_capacity)
            && cap > 0
            && f64::from(load) > 0.9 * f64::from(cap)
        {
            above += bucket_secs as u64;
        }
    }
    s.failure_rate =
        (s.turns_finished > 0).then(|| s.turns_failed as f64 / s.turns_finished as f64);
    s.seconds_above_90 = rows
        .iter()
        .any(|r| r.worker_capacity.is_some())
        .then_some(above);
    s
}

async fn timeline(
    state: &AppState,
    org: Option<i64>,
    query: TimelineQuery,
) -> ApiResult<MonitoringTimeline> {
    let now = Utc::now();
    let (from, to, step) = resolve(&query, now)?;
    let key = org.unwrap_or(PLATFORM_ORG);
    let platform = org.is_none();
    let previous_from = from - (to - from);
    let (rows, previous, markers) = tokio::try_join!(
        state.db.timeline_buckets(key, from, to, step),
        state.db.timeline_buckets(key, previous_from, from, step),
        state.db.timeline_markers(platform, from, to),
    )
    .log_internal_error_json("load monitoring timeline")?;
    let workers = if platform {
        state
            .db
            .timeline_worker_buckets(from, to, step)
            .await
            .log_internal_error_json("load monitoring timeline workers")?
    } else {
        Default::default()
    };
    let by_start: std::collections::HashMap<_, _> = rows.iter().map(|r| (r.bucket, r)).collect();
    let mut buckets = Vec::new();
    let mut at = from;
    while at < to {
        buckets.push(bucket(
            at,
            by_start.get(&at).copied(),
            workers.get(&at),
            platform,
        ));
        at += Duration::seconds(step);
    }
    Ok(Json(MonitoringTimeline {
        generated_at: now,
        scope: if platform { "fleet" } else { "organization" }.into(),
        from,
        to,
        bucket_seconds: step as u32,
        buckets,
        summary: summarize(&rows, step),
        previous: summarize(&previous, step),
        markers: markers
            .into_iter()
            .map(|m| TimelineMarker {
                at: m.at,
                kind: m.kind,
                label: m.label,
            })
            .collect(),
    }))
}

async fn window(
    state: &AppState,
    org: Option<i64>,
    query: TimelineWindowQuery,
) -> ApiResult<MonitoringTimelineWindow> {
    if query.from >= query.to {
        return Err(bad("from must be before to"));
    }
    if query.to - query.from > Duration::days(MAX_WINDOW_DAYS) {
        return Err(bad("window is limited to 7 days"));
    }
    let (busiest, failures) = tokio::try_join!(
        state.db.timeline_busiest(org, query.from, query.to),
        state.db.timeline_failures(org, query.from, query.to),
    )
    .log_internal_error_json("load monitoring timeline window")?;
    Ok(Json(MonitoringTimelineWindow {
        from: query.from,
        to: query.to,
        busiest: busiest
            .into_iter()
            .map(|r| TimelineRank {
                id: r.key,
                name: r.label,
                sessions: nonneg(r.sessions),
                turns: nonneg(r.turns),
            })
            .collect(),
        failures: failures
            .into_iter()
            .map(|r| TimelineFailure {
                code: r.code,
                turns: nonneg(r.turns),
            })
            .collect(),
    }))
}

/// Get recorded execution history for the organization.
#[utoipa::path(get, path = "/v1/monitoring/timeline", tag = "monitoring", params(TimelineQuery),
    responses((status = 200, description = "Organization execution history", body = MonitoringTimeline),
        (status = 400, description = "Invalid range", body = ErrorResponse),
        (status = 403, description = "Forbidden", body = ErrorResponse), (status = 404, description = "Not enrolled", body = ErrorResponse)))]
pub async fn get_monitoring_timeline(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Query(query): Query<TimelineQuery>,
) -> ApiResult<MonitoringTimeline> {
    authorize_org(&org, &state)?;
    timeline(&state, Some(org.org_id), query).await
}

/// Get recorded execution history across all organizations. Requires platform-user access.
#[utoipa::path(get, path = "/v1/durable/monitoring/timeline", tag = "durable", params(TimelineQuery),
    responses((status = 200, description = "Platform execution history", body = MonitoringTimeline),
        (status = 400, description = "Invalid range", body = ErrorResponse),
        (status = 403, description = "Platform access required", body = ErrorResponse)))]
pub async fn get_fleet_monitoring_timeline(
    _platform: PlatformUser,
    State(state): State<AppState>,
    Query(query): Query<TimelineQuery>,
) -> ApiResult<MonitoringTimeline> {
    timeline(&state, None, query).await
}

/// Break down a selected window: busiest agents and failure causes.
#[utoipa::path(get, path = "/v1/monitoring/timeline/window", tag = "monitoring", params(TimelineWindowQuery),
    responses((status = 200, description = "Organization window breakdown", body = MonitoringTimelineWindow),
        (status = 400, description = "Invalid window", body = ErrorResponse),
        (status = 403, description = "Forbidden", body = ErrorResponse), (status = 404, description = "Not enrolled", body = ErrorResponse)))]
pub async fn get_monitoring_timeline_window(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Query(query): Query<TimelineWindowQuery>,
) -> ApiResult<MonitoringTimelineWindow> {
    authorize_org(&org, &state)?;
    window(&state, Some(org.org_id), query).await
}

/// Break down a selected window across organizations. Requires platform-user access.
#[utoipa::path(get, path = "/v1/durable/monitoring/timeline/window", tag = "durable", params(TimelineWindowQuery),
    responses((status = 200, description = "Platform window breakdown", body = MonitoringTimelineWindow),
        (status = 400, description = "Invalid window", body = ErrorResponse),
        (status = 403, description = "Platform access required", body = ErrorResponse)))]
pub async fn get_fleet_monitoring_timeline_window(
    _platform: PlatformUser,
    State(state): State<AppState>,
    Query(query): Query<TimelineWindowQuery>,
) -> ApiResult<MonitoringTimelineWindow> {
    window(&state, None, query).await
}

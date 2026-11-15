//! Monitoring → Timeline: recorded minute history aggregated into buckets.
//! Authorization matches the live snapshot: organization adoption plus
//! session-read authority, or platform access for the fleet view.
//!
//! Decision: worker (v1) and cell (data plane v2) history arrive in one
//! bucket, side by side: the cell part is `cells`, absent while no cell
//! reported. The page chooses what to stack; the API never adds numbers
//! measured differently (sampled every 10 seconds versus reported once a
//! minute). Node, CPU and memory detail is platform only: an organization
//! shares cells with others.

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
    storage::{
        PLATFORM_ORG, TimelineBucketRow, TimelineCellBucketRow, TimelineCellsRow, TimelineFleetRow,
        TimelineWorkerBucket, timeline_minute,
    },
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
    /// Sessions not archived, on any data plane: the newest count in or before the bucket.
    pub open_sessions: Option<u64>,
    /// Agents defined, not archived: the newest count in or before the bucket.
    pub agents: Option<u64>,
    /// Average distinct agents with an active session on workers.
    pub agents_working_avg: f64,
    /// Highest sampled count of such agents.
    pub agents_working_peak: u64,
    /// Platform only: server replicas running.
    pub servers: Option<u64>,
    /// Data plane v2 cells; absent when no cell reported and no cell turn finished in the bucket.
    pub cells: Option<TimelineCellsBucket>,
}

/// What data plane v2 cells reported over a bucket. Population comes from
/// each cell's one-minute report; counters from the cells' feed.
#[derive(Debug, Clone, Default, Serialize, ToSchema)]
pub struct TimelineCellsBucket {
    /// Highest number of cells that reported in one minute.
    #[schema(example = 12)]
    pub reporting: u64,
    /// Average sessions resident on cells.
    pub sessions_avg: f64,
    /// Highest per-minute sessions resident on cells.
    pub sessions_peak: u64,
    /// Average turns running on cells.
    pub running_avg: f64,
    /// Highest per-minute turns running on cells.
    pub running_peak: u64,
    /// Highest per-minute messages waiting for a turn on cells.
    pub queued_peak: u64,
    /// Average distinct agents with a running turn on cells.
    pub agents_working_avg: f64,
    /// Highest per-minute agents with a running turn on cells.
    pub agents_working_peak: u64,
    /// Turns completed on cells.
    pub turns_completed: u64,
    /// Turns failed on cells.
    pub turns_failed: u64,
    /// Turns cancelled on cells.
    pub turns_cancelled: u64,
    /// Model calls made on cells.
    pub model_calls: u64,
    /// Model input tokens on cells.
    pub input_tokens: u64,
    /// Model output tokens on cells.
    pub output_tokens: u64,
    /// Model spend on cells in USD.
    pub cost_usd: f64,
    /// Platform only: highest total of cell nodes (pods) in one minute.
    pub nodes: Option<u64>,
    /// Platform only: lowest total of nodes that answered their leader in one minute.
    pub nodes_answering: Option<u64>,
    /// Platform only: the fullest node's memory use as a percentage of its limit.
    pub memory_peak_pct: Option<f64>,
    /// Platform only: the busiest node's CPU use as a percentage of its limit.
    pub cpu_peak_pct: Option<f64>,
    /// Platform only: worst node's p99 time to admit a turn, in milliseconds.
    pub admit_p99_ms: Option<u64>,
    /// Platform only: worst node's p99 log shipping lag, in milliseconds.
    pub ship_lag_p99_ms: Option<u64>,
    /// Platform only: leader elections across cells.
    pub elections: Option<u64>,
    /// Platform only: sessions moved to another cell.
    pub moves: Option<u64>,
    /// Platform only: highest per-minute turns running on a second or later attempt.
    pub resumed_peak: Option<u64>,
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
    /// Highest per-minute turns running on cells; null when no cell reported.
    pub cells_peak_running: Option<u64>,
    /// Platform only: leader elections across cells; null when no cell reported.
    pub cells_elections: Option<u64>,
}

/// Deploy or worker change on the timeline.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct TimelineMarker {
    /// UTC time of the change.
    pub at: DateTime<Utc>,
    /// deploy, worker_joined, worker_left, or leader_change (all but deploy are platform only).
    #[schema(example = "deploy")]
    pub kind: String,
    /// Server version for a deploy, worker id or cell name otherwise.
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
    /// Deploys, worker changes and cell leader changes inside the range.
    pub markers: Vec<TimelineMarker>,
    /// Sessions not archived right now, on any data plane.
    pub open_sessions_now: u64,
    /// Agents defined right now.
    pub agents_now: u64,
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
    /// Up to 6 tools whose calls failed most often; `code` is the tool name.
    pub failed_tools: Vec<TimelineFailure>,
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
            open_sessions: None,
            agents: None,
            agents_working_avg: 0.0,
            agents_working_peak: 0,
            servers: None,
            cells: None,
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
        open_sessions: None,
        agents: None,
        agents_working_avg: 0.0,
        agents_working_peak: 0,
        servers: None,
        cells: None,
    }
}

/// Last known open-session and agent counts, carried across buckets
/// without a count.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct Carry {
    pub sessions: Option<i64>,
    pub agents: Option<i32>,
}

fn pct(share: Option<f64>) -> Option<f64> {
    share.map(|v| (v * 1000.0).round() / 10.0)
}

/// Fold the fleet columns of one bucket into `b`.
pub(super) fn apply_fleet(
    b: &mut TimelineBucket,
    fleet: Option<&TimelineFleetRow>,
    cells: Option<&TimelineCellsRow>,
    samples: i64,
    platform: bool,
    carry: &mut Carry,
) {
    if let Some(f) = fleet {
        carry.sessions = f.sessions.or(carry.sessions);
        carry.agents = f.agents.or(carry.agents);
        b.agents_working_avg = ratio(f.agents_working_sum, samples);
        b.agents_working_peak = nonneg(f.agents_working_peak.into());
        if platform {
            b.servers = f.servers.map(|v| nonneg(v.into()));
        }
        let has_cells = f.dp_minutes > 0
            || f.dp_turns_completed + f.dp_turns_failed + f.dp_turns_cancelled + f.dp_model_calls
                > 0;
        if has_cells {
            let m = f.dp_minutes;
            b.cells = Some(TimelineCellsBucket {
                reporting: nonneg(f.dp_reports.into()),
                sessions_avg: ratio(f.dp_sessions_sum, m),
                sessions_peak: nonneg(f.dp_sessions_peak),
                running_avg: ratio(f.dp_running_sum, m),
                running_peak: nonneg(f.dp_running_peak),
                queued_peak: nonneg(f.dp_queued_peak),
                agents_working_avg: ratio(f.dp_agents_working_sum, m),
                agents_working_peak: nonneg(f.dp_agents_working_peak),
                turns_completed: nonneg(f.dp_turns_completed),
                turns_failed: nonneg(f.dp_turns_failed),
                turns_cancelled: nonneg(f.dp_turns_cancelled),
                model_calls: nonneg(f.dp_model_calls),
                input_tokens: nonneg(f.dp_input_tokens),
                output_tokens: nonneg(f.dp_output_tokens),
                cost_usd: f.dp_cost_usd.max(0.0),
                ..Default::default()
            });
        }
    }
    b.open_sessions = carry.sessions.map(nonneg);
    b.agents = carry.agents.map(|v| nonneg(v.into()));
    if let (true, Some(c)) = (platform, cells) {
        let out = b.cells.get_or_insert_with(Default::default);
        out.reporting = out.reporting.max(nonneg(c.cells));
        out.nodes = Some(nonneg(c.nodes));
        out.nodes_answering = Some(nonneg(c.nodes_answering));
        out.memory_peak_pct = pct(c.memory_peak);
        out.cpu_peak_pct = pct(c.cpu_peak);
        out.admit_p99_ms = c.admit_p99_ms.map(|v| nonneg(v.into()));
        out.ship_lag_p99_ms = c.ship_lag_p99_ms.map(|v| nonneg(v.into()));
        out.elections = Some(nonneg(c.elections));
        out.moves = Some(nonneg(c.moves));
        out.resumed_peak = Some(nonneg(c.resumed_peak));
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

/// Cell totals of a range folded into its summary.
fn summarize_cells(
    s: &mut TimelineSummary,
    fleet: &[TimelineFleetRow],
    cells: &[TimelineCellsRow],
) {
    for f in fleet {
        let finished = f.dp_turns_completed + f.dp_turns_failed + f.dp_turns_cancelled;
        s.turns_finished += nonneg(finished);
        s.turns_failed += nonneg(f.dp_turns_failed);
        s.cost_usd += f.dp_cost_usd.max(0.0);
        if f.dp_minutes > 0 {
            let peak = nonneg(f.dp_running_peak);
            s.cells_peak_running = Some(s.cells_peak_running.map_or(peak, |p| p.max(peak)));
        }
    }
    if !cells.is_empty() {
        s.cells_elections = Some(cells.iter().map(|c| nonneg(c.elections)).sum());
    }
    s.failure_rate =
        (s.turns_finished > 0).then(|| s.turns_failed as f64 / s.turns_finished as f64);
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
    let (rows, previous, mut markers) = tokio::try_join!(
        state.db.timeline_buckets(key, from, to, step),
        state.db.timeline_buckets(key, previous_from, from, step),
        state.db.timeline_markers(platform, from, to),
    )
    .log_internal_error_json("load monitoring timeline")?;
    let (fleet, previous_fleet, seed, (open_now, agents_now)) = tokio::try_join!(
        state.db.timeline_fleet_buckets(key, from, to, step),
        state
            .db
            .timeline_fleet_buckets(key, previous_from, from, step),
        state.db.timeline_counts_before(key, from),
        state.db.timeline_live_counts(org),
    )
    .log_internal_error_json("load monitoring timeline fleet")?;
    let (workers, cells, previous_cells) = if platform {
        let (workers, cells, previous_cells, cell_markers) = tokio::try_join!(
            state.db.timeline_worker_buckets(from, to, step),
            state.db.timeline_cells_buckets(from, to, step),
            state.db.timeline_cells_buckets(previous_from, from, step),
            state.db.timeline_cell_markers(from, to),
        )
        .log_internal_error_json("load monitoring timeline workers")?;
        markers.extend(cell_markers);
        markers.sort_by_key(|m| m.at);
        (workers, cells, previous_cells)
    } else {
        Default::default()
    };
    let by_start: std::collections::HashMap<_, _> = rows.iter().map(|r| (r.bucket, r)).collect();
    let fleet_by: std::collections::HashMap<_, _> = fleet.iter().map(|r| (r.bucket, r)).collect();
    let cells_by: std::collections::HashMap<_, _> = cells.iter().map(|r| (r.bucket, r)).collect();
    let mut carry = Carry {
        sessions: seed.0,
        agents: seed.1,
    };
    let mut buckets = Vec::new();
    let mut at = from;
    while at < to {
        let row = by_start.get(&at).copied();
        let mut b = bucket(at, row, workers.get(&at), platform);
        apply_fleet(
            &mut b,
            fleet_by.get(&at).copied(),
            cells_by.get(&at).copied(),
            row.map_or(0, |r| r.samples),
            platform,
            &mut carry,
        );
        buckets.push(b);
        at += Duration::seconds(step);
    }
    // The newest bucket shows the count right now, which the minute job
    // records only for organizations with a row that minute.
    if let Some(last) = buckets.last_mut() {
        last.open_sessions = Some(nonneg(open_now));
        last.agents = Some(nonneg(agents_now));
    }
    let mut summary = summarize(&rows, step);
    summarize_cells(&mut summary, &fleet, &cells);
    let mut previous_summary = summarize(&previous, step);
    summarize_cells(&mut previous_summary, &previous_fleet, &previous_cells);
    Ok(Json(MonitoringTimeline {
        generated_at: now,
        scope: if platform { "fleet" } else { "organization" }.into(),
        from,
        to,
        bucket_seconds: step as u32,
        buckets,
        summary,
        previous: previous_summary,
        markers: markers
            .into_iter()
            .map(|m| TimelineMarker {
                at: m.at,
                kind: m.kind,
                label: m.label,
            })
            .collect(),
        open_sessions_now: nonneg(open_now),
        agents_now: nonneg(agents_now),
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
    let (busiest, failures, failed_tools) = tokio::try_join!(
        state.db.timeline_busiest(org, query.from, query.to),
        state.db.timeline_failures(org, query.from, query.to),
        state.db.timeline_failed_tools(org, query.from, query.to),
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
        failed_tools: failed_tools
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

/// Break down a window by agent and failure cause.
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

/// How old a cell's newest report may be before the cell shows as silent.
const CELL_SILENT_AFTER_SECS: i64 = 180;

/// One cell over one bucket.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct MonitoringCellBucket {
    /// UTC start of the bucket.
    pub start: DateTime<Utc>,
    /// Minute reports received in the bucket.
    pub reports: u64,
    /// Highest node count.
    pub nodes: Option<u64>,
    /// Lowest count of nodes that answered the leader.
    pub nodes_answering: Option<u64>,
    /// Highest sessions resident.
    pub sessions_peak: Option<u64>,
    /// Highest turns running.
    pub running_peak: Option<u64>,
    /// Highest messages waiting for a turn.
    pub queued_peak: Option<u64>,
    /// The fullest node's memory use as a percentage of its limit.
    pub memory_peak_pct: Option<f64>,
    /// The busiest node's CPU use as a percentage of its limit.
    pub cpu_peak_pct: Option<f64>,
    /// Worst node's p99 time to admit a turn, in milliseconds.
    pub admit_p99_ms: Option<u64>,
    /// Worst node's p99 log shipping lag, in milliseconds.
    pub ship_lag_p99_ms: Option<u64>,
    /// Leader elections.
    pub elections: u64,
    /// Sessions moved to another cell.
    pub moves: u64,
    /// Turns completed.
    pub turns_completed: u64,
    /// Turns failed.
    pub turns_failed: u64,
    /// Turns cancelled.
    pub turns_cancelled: u64,
}

/// One data plane v2 cell: its newest minute report and its history.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct MonitoringCell {
    /// Cell name.
    #[schema(example = "cell-07")]
    pub cell: String,
    /// Data plane the cell belongs to.
    pub data_plane: String,
    /// Minute of the newest report.
    pub reported_at: DateTime<Utc>,
    /// No report for the last three minutes.
    pub silent: bool,
    /// Leader node at the newest report.
    pub leader: Option<String>,
    /// Members.
    pub nodes: u64,
    /// Members that answered the leader.
    pub nodes_answering: u64,
    /// Sessions resident.
    pub sessions: u64,
    /// Turns running.
    pub running: u64,
    /// Messages waiting for a turn.
    pub queued: u64,
    /// Turns running on a second or later attempt.
    pub resumed: u64,
    /// The fullest node's resident memory, in bytes.
    pub memory_bytes: u64,
    /// Its memory limit in bytes; null when unknown.
    pub memory_limit_bytes: Option<u64>,
    /// The busiest node's CPU, in cores.
    pub cpu_cores: f64,
    /// Its CPU limit in cores; null when unknown.
    pub cpu_limit_cores: Option<f64>,
    /// Worst node's p99 time to admit a turn, in milliseconds.
    pub admit_p99_ms: u64,
    /// Worst node's p99 log shipping lag, in milliseconds.
    pub ship_lag_p99_ms: u64,
    /// Raft term.
    pub term: u64,
    /// History over the range, oldest first; buckets without a report are absent.
    pub buckets: Vec<MonitoringCellBucket>,
}

/// Data plane v2 cells as their minute reports show them.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct MonitoringCells {
    /// UTC time of this read.
    pub generated_at: DateTime<Utc>,
    /// Inclusive UTC start of the history.
    pub from: DateTime<Utc>,
    /// Exclusive UTC end.
    pub to: DateTime<Utc>,
    /// Bucket size in seconds.
    pub bucket_seconds: u32,
    /// Cells that reported in the last week, by name.
    pub cells: Vec<MonitoringCell>,
}

fn share_pct(used: Option<f64>, limit: Option<f64>) -> Option<f64> {
    match (used, limit) {
        (Some(u), Some(l)) if l > 0.0 => pct(Some(u / l)),
        _ => None,
    }
}

fn cell_bucket(r: &TimelineCellBucketRow) -> MonitoringCellBucket {
    let opt = |v: Option<i64>| v.map(nonneg);
    let opt32 = |v: Option<i32>| v.map(|v| nonneg(v.into()));
    MonitoringCellBucket {
        start: r.bucket,
        reports: nonneg(r.reports),
        nodes: opt32(r.nodes),
        nodes_answering: opt32(r.nodes_answering),
        sessions_peak: opt(r.sessions_peak),
        running_peak: opt(r.running_peak),
        queued_peak: opt(r.queued_peak),
        memory_peak_pct: share_pct(
            r.rss_peak_bytes.map(|v| v as f64),
            r.memory_limit_bytes.map(|v| v as f64),
        ),
        cpu_peak_pct: share_pct(
            r.cpu_peak_cores.map(f64::from),
            r.cpu_limit_cores.map(f64::from),
        ),
        admit_p99_ms: opt32(r.admit_p99_ms),
        ship_lag_p99_ms: opt32(r.ship_lag_p99_ms),
        elections: nonneg(r.elections),
        moves: nonneg(r.moves),
        turns_completed: nonneg(r.turns_completed),
        turns_failed: nonneg(r.turns_failed),
        turns_cancelled: nonneg(r.turns_cancelled),
    }
}

/// List data plane v2 cells with their newest report and history. Requires platform-user access.
#[utoipa::path(get, path = "/v1/durable/monitoring/cells", tag = "durable", params(TimelineQuery),
    responses((status = 200, description = "Cells and their history", body = MonitoringCells),
        (status = 400, description = "Invalid range", body = ErrorResponse),
        (status = 403, description = "Platform access required", body = ErrorResponse)))]
pub async fn get_fleet_monitoring_cells(
    _platform: PlatformUser,
    State(state): State<AppState>,
    Query(query): Query<TimelineQuery>,
) -> ApiResult<MonitoringCells> {
    let now = Utc::now();
    let (from, to, step) = resolve(&query, now)?;
    let (latest, history) = tokio::try_join!(
        state.db.monitoring_cells_latest(),
        state.db.timeline_cell_buckets(from, to, step),
    )
    .log_internal_error_json("load monitoring cells")?;
    let mut by_cell: std::collections::HashMap<&str, Vec<MonitoringCellBucket>> =
        std::collections::HashMap::new();
    for r in &history {
        by_cell
            .entry(r.cell.as_str())
            .or_default()
            .push(cell_bucket(r));
    }
    let limit = |v: Option<f64>| v.filter(|l| *l > 0.0);
    let cells = latest
        .iter()
        .map(|c| MonitoringCell {
            cell: c.cell.clone(),
            data_plane: c.data_plane.clone(),
            reported_at: c.minute,
            silent: now - c.minute > Duration::seconds(CELL_SILENT_AFTER_SECS),
            leader: c.leader.clone().filter(|l| !l.is_empty()),
            nodes: c.nodes.map_or(0, |v| nonneg(v.into())),
            nodes_answering: c.nodes_answering.map_or(0, |v| nonneg(v.into())),
            sessions: c.sessions.map_or(0, nonneg),
            running: c.running.map_or(0, nonneg),
            queued: c.queued.map_or(0, nonneg),
            resumed: c.resumed.map_or(0, nonneg),
            memory_bytes: c.rss_peak_bytes.map_or(0, nonneg),
            memory_limit_bytes: limit(c.memory_limit_bytes.map(|v| v as f64)).map(|v| v as u64),
            cpu_cores: c.cpu_peak_cores.map_or(0.0, f64::from),
            cpu_limit_cores: limit(c.cpu_limit_cores.map(f64::from)),
            admit_p99_ms: c.admit_p99_ms.map_or(0, |v| nonneg(v.into())),
            ship_lag_p99_ms: c.ship_lag_p99_ms.map_or(0, |v| nonneg(v.into())),
            term: c.raft_term.map_or(0, nonneg),
            buckets: by_cell.remove(c.cell.as_str()).unwrap_or_default(),
        })
        .collect();
    Ok(Json(MonitoringCells {
        generated_at: now,
        from,
        to,
        bucket_seconds: step as u32,
        cells,
    }))
}

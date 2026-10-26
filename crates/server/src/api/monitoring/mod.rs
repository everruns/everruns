//! Monitoring is a platform read model, separate from the generic durable engine.
//! Every refresh is authorized before consulting the per-scope one-second cache.

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::get,
};
use chrono::{DateTime, Utc};
use everruns_contracts::typed_id::SessionId;
use everruns_core::{Caller, ResourceConfigResponse};
use everruns_durable::persistence::WORKER_HEARTBEAT_TIMEOUT_SECS;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::{Mutex, Semaphore};
use utoipa::ToSchema;

use super::common::{ApiResult, ApiResultExt, ErrorResponse, impl_auth_state};
use crate::{
    auth::{AuthState, PlatformUser, ResolvedOrg},
    domains::sessions::SESSION_VIEW,
    storage::{MONITORING_ACTOR_PAGE, MONITORING_COHORT_LIMIT, MonitoringActorRow, StorageBackend},
};

const MAX_SCOPES: usize = 64;
const MAX_WORKERS: usize = 2048;
const PHASES: &[&str] = &["runnable", "model", "tools", "waiting", "setup"];
type CachedSnapshot = (Instant, MonitoringSnapshot);
type SnapshotEntry = Arc<Mutex<Option<CachedSnapshot>>>;
type SnapshotCache = Arc<Mutex<HashMap<Option<i64>, SnapshotEntry>>>;

#[derive(Clone)]
pub struct AppState {
    db: Arc<StorageBackend>,
    auth: AuthState,
    cache: SnapshotCache,
    reads: Arc<Semaphore>,
}
impl_auth_state!(AppState);

impl AppState {
    pub fn new(db: Arc<StorageBackend>, auth: AuthState) -> Self {
        Self {
            db,
            auth,
            cache: Arc::default(),
            reads: Arc::new(Semaphore::new(4)),
        }
    }

    async fn snapshot(&self, org: Option<i64>) -> anyhow::Result<MonitoringSnapshot> {
        let entry = {
            let mut cache = self.cache.lock().await;
            if !cache.contains_key(&org) && cache.len() >= MAX_SCOPES {
                // An idle entry is replaceable. Never evict a scope
                // being refreshed: that would fan out duplicate DB reads.
                let idle = cache
                    .iter()
                    .find(|(_, entry)| Arc::strong_count(entry) == 1)
                    .map(|(key, _)| *key);
                if let Some(key) = idle {
                    cache.remove(&key);
                } else {
                    anyhow::bail!("monitoring capacity reached");
                }
            }
            cache.entry(org).or_default().clone()
        };
        let mut cached = entry.lock().await;
        if let Some((at, snapshot)) = cached.as_ref()
            && at.elapsed() < Duration::from_secs(1)
        {
            return Ok(snapshot.clone());
        }
        let now = Utc::now();
        let _permit = self.reads.acquire().await?;
        let (total, mut cohorts, rates, lease_rates) = tokio::try_join!(
            self.db.monitoring_totals(org, now),
            self.db.monitoring_cohorts(org, now),
            self.db.monitoring_rates(org, now),
            self.db.monitoring_lease_rates(org, now),
        )?;
        let cohorts_truncated = cohorts.len() > MONITORING_COHORT_LIMIT as usize;
        cohorts.truncate(MONITORING_COHORT_LIMIT as usize);
        let mut phases = [
            ("runnable", total.runnable),
            ("model", total.model),
            ("tools", total.tools),
            ("waiting", total.waiting),
            ("setup", total.setup),
        ]
        .into_iter()
        .map(|(phase, count)| MonitoringPhase {
            phase: phase.into(),
            count: positive(count),
            oldest_ms: 0,
        })
        .collect::<Vec<_>>();
        let mut workers = BTreeMap::<String, MonitoringWorker>::new();
        let mut ownership = BTreeMap::<(String, String), u64>::new();
        for c in &cohorts {
            if let Some(p) = phases.iter_mut().find(|p| p.phase == c.phase) {
                p.oldest_ms = p.oldest_ms.max(positive(c.oldest_phase_ms));
            }
            if let Some(holder) = &c.holder {
                let w = workers
                    .entry(holder.clone())
                    .or_insert_with(|| MonitoringWorker::owned(holder.clone()));
                w.actor_leases += positive(c.leased);
                *w.phases.entry(c.phase.clone()).or_default() += positive(c.actors);
                *ownership
                    .entry((holder.clone(), c.org_id.clone()))
                    .or_default() += positive(c.leased);
            }
        }
        // Organization callers receive only their own actor-to-holder mapping.
        // Hostnames, global load, other orgs, and worker capacity are fleet-only.
        if org.is_none() {
            for row in self.db.monitoring_workers().await? {
                let w = workers
                    .entry(row.id.clone())
                    .or_insert_with(|| MonitoringWorker::owned(row.id));
                w.worker_group = row.worker_group;
                w.status = Some(
                    if row.last_heartbeat_at
                        < now - chrono::Duration::seconds(WORKER_HEARTBEAT_TIMEOUT_SECS)
                    {
                        "stale".into()
                    } else {
                        row.status
                    },
                );
                w.current_load = Some(positive(row.current_load));
                w.capacity = Some(positive(row.max_concurrency));
                w.accepting_work = Some(row.accepting_tasks);
                w.last_heartbeat_at = Some(row.last_heartbeat_at);
            }
        }
        let workers_truncated = workers.len() > MAX_WORKERS;
        let workers = workers.into_values().take(MAX_WORKERS).collect();
        let events = rates
            .into_iter()
            .map(|r| (r.event_type, positive(r.count)))
            .collect();
        let snapshot = MonitoringSnapshot {
            generated_at: now,
            scope: if org.is_some() {
                "organization"
            } else {
                "fleet"
            }
            .into(),
            window_seconds: 60,
            actors: positive(total.actors),
            organizations: positive(total.organizations),
            actor_leases: positive(total.leased),
            expired_leases: positive(total.expired),
            oldest_runnable_ms: total.oldest_runnable_ms.map(positive),
            phases,
            cohorts: cohorts
                .into_iter()
                .map(|r| MonitoringCohort {
                    phase: r.phase,
                    org_id: r.org_id,
                    worker_id: r.holder,
                    count: positive(r.actors),
                })
                .collect(),
            ownership: ownership
                .into_iter()
                .map(|((worker_id, org_id), actor_leases)| MonitoringOwnership {
                    worker_id,
                    org_id,
                    actor_leases,
                })
                .collect(),
            workers,
            events,
            lease_events: lease_rates
                .into_iter()
                .map(|r| (r.event_type, positive(r.count)))
                .collect(),
            cohorts_truncated,
            workers_truncated,
        };
        *cached = Some((Instant::now(), snapshot.clone()));
        Ok(snapshot)
    }
}

fn positive(value: i64) -> u64 {
    value.max(0) as u64
}

/// Population and age of one committed execution state.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct MonitoringPhase {
    /// Execution state: runnable, model, tools, waiting, or setup.
    #[schema(example = "model")]
    pub phase: String,
    /// Complete number of sessions observed in this execution state.
    #[schema(example = 2800)]
    pub count: u64,
    /// Age of the oldest committed phase observation in milliseconds; may be partial when cohorts are truncated.
    #[schema(example = 3200)]
    pub oldest_ms: u64,
}

/// Bounded execution breakdown for one organization and live lease holder.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct MonitoringCohort {
    /// Execution state shared by this organization and live holder cohort.
    #[schema(example = "model")]
    pub phase: String,
    /// Public identifier of the organization owning these sessions.
    #[schema(example = "org_00000000000000000000000000000001")]
    pub org_id: String,
    /// Live lease holder; null for unleased or expired ownership.
    #[schema(example = "worker-01")]
    pub worker_id: Option<String>,
    /// Sessions in this phase, organization, and holder cohort.
    #[schema(example = 32)]
    pub count: u64,
}

/// Valid actor ownership linking an organization to a worker.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct MonitoringOwnership {
    /// Worker currently holding the valid actor leases.
    #[schema(example = "worker-01")]
    pub worker_id: String,
    /// Public identifier of the sessions' owning organization.
    #[schema(example = "org_00000000000000000000000000000001")]
    pub org_id: String,
    /// Valid actor leases held by this worker within this organization.
    #[schema(example = 32)]
    pub actor_leases: u64,
}

/// Scope-aware actor ownership with platform-only worker telemetry.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct MonitoringWorker {
    /// Worker identifier recorded by actor ownership or the fleet registry.
    #[schema(example = "worker-01")]
    pub id: String,
    /// Registered worker group; null in organization scope.
    #[schema(example = "execution")]
    pub worker_group: Option<String>,
    /// Valid actor leases in this monitoring scope; not occupied work slots.
    #[schema(example = 32)]
    pub actor_leases: u64,
    /// Leased session counts by execution state within this scope.
    #[schema(example = json!({"model": 20, "tools": 12}))]
    pub phases: BTreeMap<String, u64>,
    /// Fleet-only worker status, including stale heartbeats; null in organization scope.
    #[schema(example = "active")]
    pub status: Option<String>,
    /// Reported worker activity load, distinct from actor leases; null in organization scope.
    #[schema(example = 8)]
    pub current_load: Option<u64>,
    /// Reported maximum worker concurrency; null in organization scope.
    #[schema(example = 50)]
    pub capacity: Option<u64>,
    /// Last reported admission flag; null in organization scope.
    #[schema(example = true)]
    pub accepting_work: Option<bool>,
    /// Last worker heartbeat in UTC; null in organization scope.
    #[schema(example = "2026-10-14T12:00:00Z")]
    pub last_heartbeat_at: Option<DateTime<Utc>>,
}
impl MonitoringWorker {
    fn owned(id: String) -> Self {
        Self {
            id,
            worker_group: None,
            actor_leases: 0,
            phases: BTreeMap::new(),
            status: None,
            current_load: None,
            capacity: None,
            accepting_work: None,
            last_heartbeat_at: None,
        }
    }
}

/// Live execution population, ownership, and event-window summary.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct MonitoringSnapshot {
    /// UTC observation time used to classify lease validity and event windows.
    #[schema(example = "2026-10-14T12:00:03Z")]
    pub generated_at: DateTime<Utc>,
    /// Monitoring boundary: organization or fleet.
    #[schema(example = "fleet")]
    pub scope: String,
    /// Lookback window for lifecycle and ownership event counts, in seconds.
    #[schema(example = 60)]
    pub window_seconds: u32,
    /// Complete population of live sessions and runnable or held session actors.
    #[schema(example = 5000)]
    pub actors: u64,
    /// Distinct organizations represented in the current population.
    #[schema(example = 12)]
    pub organizations: u64,
    /// Complete count of actor leases valid at the observation time.
    #[schema(example = 3700)]
    pub actor_leases: u64,
    /// Held leases whose deadline has passed, regardless of worker heartbeat.
    #[schema(example = 3)]
    pub expired_leases: u64,
    /// Age of the oldest work awaiting a valid lease, in milliseconds; null when none is runnable.
    #[schema(example = 3000)]
    pub oldest_runnable_ms: Option<u64>,
    /// Complete counts by execution state, with observed phase ages.
    pub phases: Vec<MonitoringPhase>,
    /// Bounded phase, organization, and live holder breakdown; consult cohorts_truncated.
    pub cohorts: Vec<MonitoringCohort>,
    /// Organization-to-worker mapping derived only from valid actor leases.
    pub ownership: Vec<MonitoringOwnership>,
    /// Bounded worker observations; global telemetry is redacted for organization callers.
    pub workers: Vec<MonitoringWorker>,
    /// Committed lifecycle event counts in the lookback window, not lifetime totals.
    #[schema(example = json!({"turn.started": 120, "reason.started": 350, "tool.started": 240}))]
    pub events: BTreeMap<String, u64>,
    /// Acquisition, reacquisition, and release counts in the lookback window; renewals are excluded.
    #[schema(example = json!({"acquired": 120, "released": 110, "reacquired": 3}))]
    pub lease_events: BTreeMap<String, u64>,
    /// Whether ownership cohort detail exceeded its limit; population and phase counts remain complete.
    #[schema(example = false)]
    pub cohorts_truncated: bool,
    /// Whether worker detail exceeded its limit; population and phase counts remain complete.
    #[schema(example = false)]
    pub workers_truncated: bool,
}

/// Session actor ownership, lease validity, and durable progress cursors.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct MonitoringActor {
    /// Canonical session identifier whose actor is being inspected.
    #[schema(example = "session_01933b5a000070008000000000000001")]
    pub session_id: String,
    /// Public identifier of the owning session's organization.
    #[schema(example = "org_00000000000000000000000000000001")]
    pub org_id: String,
    /// Persisted session lifecycle status.
    #[schema(example = "active")]
    pub session_status: String,
    /// Observed execution state; idle is also possible for a released actor detail.
    #[schema(example = "model")]
    pub phase: String,
    /// UTC timestamp of the committed observation used to classify the phase.
    #[schema(example = "2026-10-14T12:00:01Z")]
    pub phase_since: DateTime<Utc>,
    /// Observed lease validity: held, expired, or unleased.
    #[schema(example = "held")]
    pub lease_state: String,
    /// Current recorded holder, or previous holder for an expired lease; null when unleased.
    #[schema(example = "worker-01")]
    pub holder: Option<String>,
    /// Monotonic ownership fence; null when no actor row exists.
    #[schema(example = 17)]
    pub fence: Option<i64>,
    /// UTC deadline of the recorded lease; null when unleased.
    #[schema(example = "2026-10-14T12:00:10Z")]
    pub lease_expires_at: Option<DateTime<Utc>>,
    /// Last persisted acquisition or renewal acknowledgement in UTC; null before an acknowledgement or after release.
    #[schema(example = "2026-10-14T12:00:00Z")]
    pub lease_renewed_at: Option<DateTime<Utc>>,
    /// Positive interval between renewal acknowledgement and expiry in milliseconds; null when unavailable.
    #[schema(example = 10000)]
    pub lease_ttl_ms: Option<u64>,
    /// UTC timestamp when the actor became runnable; null for parked work.
    #[schema(example = "2026-10-14T11:59:59Z")]
    pub runnable_since: Option<DateTime<Utc>>,
    /// Milliseconds awaiting a valid lease; null for held or parked actors.
    #[schema(example = 3000)]
    pub runnable_age_ms: Option<u64>,
    /// Durable inbox progress cursor; it is not a message count.
    #[schema(example = 42)]
    pub inbox_seq: Option<i64>,
    /// Persisted consumed event cursor; its difference from inbox_seq is not a pending-message count.
    #[schema(example = 39)]
    pub consumed_seq: Option<i64>,
    /// Persisted next-input progress cursor.
    #[schema(example = 12)]
    pub next_input_seq: Option<i64>,
    /// Lease attempts accumulated since the last release.
    #[schema(example = 1)]
    pub attempts_since_release: Option<i32>,
    /// UTC cancellation request timestamp; null when cancellation is not requested.
    #[schema(example = "2026-10-14T12:00:02Z")]
    pub cancel_requested_at: Option<DateTime<Utc>>,
}
impl From<MonitoringActorRow> for MonitoringActor {
    fn from(r: MonitoringActorRow) -> Self {
        Self {
            session_id: SessionId::from_uuid(r.session_id).to_string(),
            org_id: r.org_id,
            session_status: r.session_status,
            phase: r.phase,
            phase_since: r.phase_since,
            lease_state: if r.expired == Some(true) {
                "expired"
            } else if r.leased == Some(true) {
                "held"
            } else {
                "unleased"
            }
            .into(),
            holder: r.lease_holder,
            fence: r.lease_fence,
            lease_expires_at: r.lease_expires_at,
            lease_renewed_at: r.lease_renewed_at,
            lease_ttl_ms: r
                .lease_expires_at
                .zip(r.lease_renewed_at)
                .map(|(expires, renewed)| (expires - renewed).num_milliseconds())
                .filter(|ms| *ms > 0)
                .map(positive),
            runnable_since: r.runnable_since,
            runnable_age_ms: r.runnable_age_ms.map(positive),
            inbox_seq: r.inbox_seq,
            consumed_seq: r.consumed_seq,
            next_input_seq: r.next_input_seq,
            attempts_since_release: r.attempts,
            cancel_requested_at: r.cancel_requested_at,
        }
    }
}

/// Bounded page of current session actor observations.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct MonitoringActorPage {
    /// UTC observation time for this actor page.
    #[schema(example = "2026-10-14T12:00:03Z")]
    pub generated_at: DateTime<Utc>,
    /// Up to 200 matching actors, ordered by expired leases and oldest runnable work.
    pub actors: Vec<MonitoringActor>,
    /// Offset for the next page; null when there are no further matching actors.
    #[schema(example = 200)]
    pub next_offset: Option<u32>,
}
/// Persisted actor ownership transition, excluding renewal noise.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct MonitoringLeaseEvent {
    /// UTC time of the persisted ownership transition.
    #[schema(example = "2026-10-14T12:00:00Z")]
    pub occurred_at: DateTime<Utc>,
    /// Ownership transition: acquired, reacquired, or released; renewals produce no history entry.
    #[schema(example = "reacquired")]
    pub kind: String,
    /// Holder acquiring ownership, or the holder relinquishing a released lease.
    #[schema(example = "worker-02")]
    pub holder: String,
    /// Ownership fence associated with this transition.
    #[schema(example = 18)]
    pub fence: i64,
    /// Holder preceding acquisition or reacquisition, when recorded.
    #[schema(example = "worker-01")]
    pub previous_holder: Option<String>,
    /// Previous lease deadline in UTC, when recorded.
    #[schema(example = "2026-10-14T11:59:59Z")]
    pub previous_expires_at: Option<DateTime<Utc>>,
    /// New lease deadline in UTC; null for a release.
    #[schema(example = "2026-10-14T12:00:10Z")]
    pub expires_at: Option<DateTime<Utc>>,
}
/// Payload-free timing projection of one step in the latest turn.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct MonitoringStep {
    /// Projected turn ordinal for this trace step.
    #[schema(example = 2)]
    pub turn_no: i32,
    /// Projected step ordinal within the turn.
    #[schema(example = 3)]
    pub step_no: i32,
    /// Trace-step category, without tool arguments, results, or agent names.
    #[schema(example = "tool")]
    pub kind: String,
    /// Projected execution status of the trace step.
    #[schema(example = "completed")]
    pub status: String,
    /// UTC time at which the projected step started.
    #[schema(example = "2026-10-14T12:00:01Z")]
    pub started_at: DateTime<Utc>,
    /// Recorded step duration in milliseconds; null until duration is available.
    #[schema(example = 820)]
    pub duration_ms: Option<i64>,
}
/// Authorized actor inspection with bounded ownership history and trace timing.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct MonitoringActorDetail {
    /// UTC observation time for actor ownership and timing.
    #[schema(example = "2026-10-14T12:00:03Z")]
    pub generated_at: DateTime<Utc>,
    /// Actor observation after validating the owning session's organization.
    pub actor: MonitoringActor,
    /// Up to 50 newest ownership changes from the retained one-day history.
    pub ownership_history: Vec<MonitoringLeaseEvent>,
    /// Up to 100 projected steps from the latest turn, newest first; no conversation payloads.
    pub steps: Vec<MonitoringStep>,
}
/// Validated filters and offset for bounded current-actor pages.
#[derive(Debug, Default, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ActorQuery {
    /// Zero-based offset for fixed 200-actor pages; maximum 1000000.
    #[param(example = 0)]
    pub offset: Option<u32>,
    /// Filter by a valid current lease holder, up to 256 UTF-8 bytes.
    #[param(example = "worker-01")]
    pub worker_id: Option<String>,
    /// Filter by runnable, model, tools, waiting, or setup execution state.
    #[param(example = "model")]
    pub phase: Option<String>,
}

pub fn routes(state: AppState) -> Router {
    Router::new()
        .route("/v1/monitoring/config", get(monitoring_config))
        .route("/v1/monitoring", get(get_monitoring))
        .route("/v1/monitoring/actors", get(list_monitoring_actors))
        .route(
            "/v1/monitoring/actors/{session_id}",
            get(get_monitoring_actor),
        )
        .route("/v1/durable/monitoring", get(get_fleet_monitoring))
        .route(
            "/v1/durable/monitoring/actors",
            get(list_fleet_monitoring_actors),
        )
        .route(
            "/v1/durable/monitoring/actors/{session_id}",
            get(get_fleet_monitoring_actor),
        )
        .with_state(state)
}

fn authorize_org(
    org: &ResolvedOrg,
    state: &AppState,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    // THREAT[TM-TENANT-001]: deployment availability and platform access never
    // substitute for organization adoption or session-read authority.
    if !org.feature_flags.monitoring {
        return Err(
            ErrorResponse::new("Monitoring is not enabled for this organization")
                .into_response(StatusCode::NOT_FOUND),
        );
    }
    SESSION_VIEW
        .evaluate_with(state.auth.permission_resolver.as_ref(), &Caller::from(org))
        .map_err(|e| ErrorResponse::new(e.message).into_response(StatusCode::FORBIDDEN))
}

/// Resolve organization monitoring read authority.
#[utoipa::path(get, path = "/v1/monitoring/config", tag = "monitoring",
    responses((status = 200, description = "Monitoring authorization", body = ResourceConfigResponse)))]
pub async fn monitoring_config(
    org: ResolvedOrg,
    State(state): State<AppState>,
) -> Json<ResourceConfigResponse> {
    let allowed = SESSION_VIEW
        .evaluate_with(state.auth.permission_resolver.as_ref(), &Caller::from(&org))
        .is_ok();
    Json(ResourceConfigResponse {
        policies: [("monitoring.view".into(), allowed)].into_iter().collect(),
    })
}

/// Get organization execution monitoring.
#[utoipa::path(get, path = "/v1/monitoring", tag = "monitoring",
    responses((status = 200, description = "Organization execution monitoring", body = MonitoringSnapshot),
        (status = 403, description = "Forbidden", body = ErrorResponse), (status = 404, description = "Not enrolled", body = ErrorResponse)))]
pub async fn get_monitoring(
    org: ResolvedOrg,
    State(state): State<AppState>,
) -> ApiResult<MonitoringSnapshot> {
    authorize_org(&org, &state)?;
    Ok(Json(
        state
            .snapshot(Some(org.org_id))
            .await
            .log_internal_error_json("load organization monitoring")?,
    ))
}

/// Get platform-wide execution monitoring. Requires platform-user access.
#[utoipa::path(get, path = "/v1/durable/monitoring", tag = "durable",
    responses((status = 200, description = "Platform-wide execution monitoring", body = MonitoringSnapshot),
        (status = 403, description = "Platform access required", body = ErrorResponse)))]
pub async fn get_fleet_monitoring(
    _platform: PlatformUser,
    State(state): State<AppState>,
) -> ApiResult<MonitoringSnapshot> {
    Ok(Json(
        state
            .snapshot(None)
            .await
            .log_internal_error_json("load fleet monitoring")?,
    ))
}

async fn actor_page(
    state: &AppState,
    org: Option<i64>,
    query: ActorQuery,
) -> ApiResult<MonitoringActorPage> {
    let offset = query.offset.unwrap_or(0);
    if offset > 1_000_000 {
        return Err(
            ErrorResponse::new("Actor offset exceeds the supported range")
                .into_response(StatusCode::BAD_REQUEST),
        );
    }
    if query.worker_id.as_ref().is_some_and(|id| id.len() > 256)
        || query
            .phase
            .as_ref()
            .is_some_and(|phase| !PHASES.contains(&phase.as_str()))
    {
        return Err(
            ErrorResponse::new("Invalid actor filter").into_response(StatusCode::BAD_REQUEST)
        );
    }
    let now = Utc::now();
    let mut rows = state
        .db
        .monitoring_actors(
            org,
            now,
            i64::from(offset),
            query.worker_id.as_deref(),
            query.phase.as_deref(),
        )
        .await
        .log_internal_error_json("list monitoring actors")?;
    let next_offset = (rows.len() > MONITORING_ACTOR_PAGE as usize)
        .then_some(offset + MONITORING_ACTOR_PAGE as u32);
    rows.truncate(MONITORING_ACTOR_PAGE as usize);
    Ok(Json(MonitoringActorPage {
        generated_at: now,
        actors: rows.into_iter().map(Into::into).collect(),
        next_offset,
    }))
}

/// List organization session actors.
#[utoipa::path(get, path = "/v1/monitoring/actors", tag = "monitoring", params(ActorQuery),
    responses((status = 200, description = "Organization actor page", body = MonitoringActorPage)))]
pub async fn list_monitoring_actors(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Query(query): Query<ActorQuery>,
) -> ApiResult<MonitoringActorPage> {
    authorize_org(&org, &state)?;
    actor_page(&state, Some(org.org_id), query).await
}

/// List a bounded platform-wide page of session actors.
#[utoipa::path(get, path = "/v1/durable/monitoring/actors", tag = "durable", params(ActorQuery),
    responses((status = 200, description = "Platform actor page", body = MonitoringActorPage)))]
pub async fn list_fleet_monitoring_actors(
    _platform: PlatformUser,
    State(state): State<AppState>,
    Query(query): Query<ActorQuery>,
) -> ApiResult<MonitoringActorPage> {
    actor_page(&state, None, query).await
}

async fn actor_detail(
    state: &AppState,
    org: Option<i64>,
    session: SessionId,
) -> ApiResult<MonitoringActorDetail> {
    let now = Utc::now();
    let actor = state
        .db
        .monitoring_actor(org, now, session.uuid())
        .await
        .log_internal_error_json("load monitoring actor")?
        .ok_or_else(|| {
            ErrorResponse::new("Actor not found").into_response(StatusCode::NOT_FOUND)
        })?;
    // Authorization is complete before the unscoped trace index is read. The
    // write path projects a turn only when it ends, so bring the open turn's
    // steps in first.
    state
        .db
        .database()
        .catch_up_session_trace_fully(session.uuid())
        .await
        .log_internal_error_json("project monitoring actor steps")?;
    let (history, steps) = tokio::try_join!(
        state.db.monitoring_lease_events(org, session.uuid()),
        state.db.monitoring_steps(session.uuid()),
    )
    .log_internal_error_json("load monitoring actor history")?;
    Ok(Json(MonitoringActorDetail {
        generated_at: now,
        actor: actor.into(),
        ownership_history: history
            .into_iter()
            .map(|r| MonitoringLeaseEvent {
                occurred_at: r.occurred_at,
                kind: r.kind,
                holder: r.holder,
                fence: r.fence,
                previous_holder: r.previous_holder,
                previous_expires_at: r.previous_expires_at,
                expires_at: r.expires_at,
            })
            .collect(),
        steps: steps
            .into_iter()
            .map(|r| MonitoringStep {
                turn_no: r.turn_no,
                step_no: r.step_no,
                kind: r.kind,
                status: r.status,
                started_at: r.started_at,
                duration_ms: r.duration_ms,
            })
            .collect(),
    }))
}

/// Inspect an organization actor and its lease history.
#[utoipa::path(get, path = "/v1/monitoring/actors/{session_id}", tag = "monitoring",
    params(("session_id" = String, Path, description = "Session identifier")),
    responses((status = 200, description = "Organization actor diagnostics", body = MonitoringActorDetail), (status = 404, description = "Actor not found", body = ErrorResponse)))]
pub async fn get_monitoring_actor(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(session): Path<SessionId>,
) -> ApiResult<MonitoringActorDetail> {
    authorize_org(&org, &state)?;
    actor_detail(&state, Some(org.org_id), session).await
}

/// Inspect a session actor across organizations. Requires platform-user access.
#[utoipa::path(get, path = "/v1/durable/monitoring/actors/{session_id}", tag = "durable",
    params(("session_id" = String, Path, description = "Session identifier")),
    responses((status = 200, description = "Platform actor diagnostics", body = MonitoringActorDetail), (status = 404, description = "Actor not found", body = ErrorResponse)))]
pub async fn get_fleet_monitoring_actor(
    _platform: PlatformUser,
    State(state): State<AppState>,
    Path(session): Path<SessionId>,
) -> ApiResult<MonitoringActorDetail> {
    actor_detail(&state, None, session).await
}

#[cfg(test)]
mod tests;

use crate::auth::config::AuthConfig;
use crate::background::supervised_task::TaskSupervisor;
use crate::domains::health_issues::service::SlackHealthService;
use crate::domains::system::doctor::{self, DoctorEnv, DoctorExtensions, SystemDoctor, instances};
use crate::live_updates::event_delivery::EventDelivery;
use crate::storage::{EncryptionService, StorageBackend};
use axum::http::StatusCode;
use axum::{Json, extract::State};
use everruns_core::host::HostComposition;
use everruns_durable::WorkflowEventStore;
use serde::Serialize;
use std::sync::Arc;

#[derive(Serialize)]
pub(super) struct HealthResponse {
    /// `ok`, or `degraded` while the server runs below its configuration
    /// (live session events polled from PostgreSQL). Still HTTP 200: a
    /// restart would not fix it, and liveness must not restart healthy pods.
    status: &'static str,
    version: &'static str,
    auth_mode: String,
    /// How live session events reach clients: `nats`, `in_memory` (one
    /// server only) or `polling`.
    event_delivery: &'static str,
}

#[derive(Clone)]
pub(super) struct HealthState {
    pub(super) auth_mode: String,
    pub(super) db: Arc<StorageBackend>,
    event_delivery: &'static str,
    degraded: bool,
}

impl HealthState {
    pub(super) fn new(
        auth_config: &AuthConfig,
        db: &Arc<StorageBackend>,
        event_delivery: &EventDelivery,
    ) -> Self {
        Self {
            auth_mode: format!("{:?}", auth_config.mode),
            db: db.clone(),
            event_delivery: event_delivery.backend_name(),
            degraded: event_delivery.is_degraded(),
        }
    }
}

#[derive(Serialize)]
pub(super) struct ReadyResponse {
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<&'static str>,
}

/// `GET /health/ready`: the database answers and its schema matches this
/// build. Orchestrators route traffic on it; liveness stays on `/health` so
/// a database outage does not get healthy servers restarted.
async fn ready(State(state): State<HealthState>) -> (StatusCode, Json<ReadyResponse>) {
    match doctor::readiness(state.db.pool()).await {
        Ok(()) => (
            StatusCode::OK,
            Json(ReadyResponse {
                status: "ready",
                reason: None,
            }),
        ),
        Err(reason) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ReadyResponse {
                status: "not_ready",
                reason: Some(reason),
            }),
        ),
    }
}

/// The system doctor over this server's own services, exporting its
/// findings as Prometheus gauges.
pub(super) fn system_doctor(
    db: &Arc<StorageBackend>,
    workflow_store: &Option<Arc<dyn WorkflowEventStore + Send + Sync>>,
    encryption: &Option<Arc<EncryptionService>>,
    auth_config: &AuthConfig,
    host_composition: &HostComposition,
) -> Arc<SystemDoctor> {
    let utility_llm_configured = host_composition.utility_llm_service().is_configured();
    let mut doctor = SystemDoctor::new(
        db.clone(),
        workflow_store.clone(),
        encryption.clone(),
        DoctorEnv::from_env(auth_config, utility_llm_configured),
    );
    // A host's own checks (see `doctor::extension`).
    if let Some(extensions) = host_composition.extension::<DoctorExtensions>() {
        doctor = doctor.with_extensions(extensions.as_ref().clone());
    }
    let doctor = Arc::new(doctor);
    // Its gauges are how alerting learns about a problem; see the sweep.
    doctor::spawn_metrics_sweep(doctor.clone());
    doctor
}

/// `/health` (liveness) and `/health/ready` (readiness).
pub(super) fn routes(state: HealthState) -> axum::Router {
    axum::Router::new()
        .route("/health", axum::routing::get(endpoint))
        .route("/health/ready", axum::routing::get(ready))
        .with_state(state)
}

/// `GET /health`: liveness plus the build version, auth mode and event
/// delivery backend.
async fn endpoint(State(state): State<HealthState>) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: if state.degraded { "degraded" } else { "ok" },
        version: env!("CARGO_PKG_VERSION"),
        auth_mode: state.auth_mode.clone(),
        event_delivery: state.event_delivery,
    })
}

/// Health monitors started with the background loops.
pub(super) async fn start(
    supervisor: &mut TaskSupervisor,
    db: Arc<StorageBackend>,
    encryption: Option<Arc<EncryptionService>>,
) {
    // A fresh process has no local check in flight. Fail interrupted advisory
    // runs so the UI cannot retain a perpetual running/pending spinner.
    match db.reap_running_agent_health_check_runs().await {
        Ok(0) => {}
        Ok(count) => tracing::info!(count, "Reaped interrupted agent health-check runs"),
        Err(error) => {
            tracing::error!(%error, "Failed to reap interrupted agent health-check runs")
        }
    }
    // Every replica beats, so a doctor run on any of them sees all servers.
    supervisor.track("server_heartbeat", instances::spawn_heartbeat(db.clone()));
    supervisor.track(
        "slack_health_reconciliation",
        SlackHealthService::new(db.clone(), encryption).spawn(),
    );
    supervisor.track(
        "active_turn_limit_health_sweep",
        crate::domains::health_issues::active_turns::spawn_sweep(db.clone()),
    );
    supervisor.track(
        "background_work_health_sweep",
        crate::domains::health_issues::background_work::spawn_sweep(db),
    );
}

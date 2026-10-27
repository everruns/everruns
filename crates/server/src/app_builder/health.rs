use crate::auth::config::AuthConfig;
use crate::background::supervised_task::TaskSupervisor;
use crate::domains::health_issues::service::SlackHealthService;
use crate::domains::system::doctor::{self, DoctorEnv, SystemDoctor};
use crate::storage::{EncryptionService, StorageBackend};
use axum::http::StatusCode;
use axum::{Json, extract::State};
use everruns_durable::WorkflowEventStore;
use serde::Serialize;
use std::sync::Arc;

#[derive(Serialize)]
pub(super) struct HealthResponse {
    status: &'static str,
    version: &'static str,
    auth_mode: String,
}

#[derive(Clone)]
pub(super) struct HealthState {
    pub(super) auth_mode: String,
    pub(super) db: Arc<StorageBackend>,
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

/// The system doctor over this server's own services.
pub(super) fn system_doctor(
    db: &Arc<StorageBackend>,
    workflow_store: &Option<Arc<dyn WorkflowEventStore + Send + Sync>>,
    encryption: &Option<Arc<EncryptionService>>,
    auth_config: &AuthConfig,
    utility_llm_configured: bool,
) -> Arc<SystemDoctor> {
    Arc::new(SystemDoctor::new(
        db.clone(),
        workflow_store.clone(),
        encryption.clone(),
        DoctorEnv::from_env(auth_config, utility_llm_configured),
    ))
}

/// `/health` (liveness) and `/health/ready` (readiness).
pub(super) fn routes(state: HealthState) -> axum::Router {
    axum::Router::new()
        .route("/health", axum::routing::get(endpoint))
        .route("/health/ready", axum::routing::get(ready))
        .with_state(state)
}

/// `GET /health`: liveness plus the build version and auth mode.
async fn endpoint(State(state): State<HealthState>) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
        auth_mode: state.auth_mode.clone(),
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
    supervisor.track(
        "slack_health_reconciliation",
        SlackHealthService::new(db.clone(), encryption).spawn(),
    );
    supervisor.track(
        "active_turn_limit_health_sweep",
        crate::domains::health_issues::active_turns::spawn_sweep(db),
    );
}

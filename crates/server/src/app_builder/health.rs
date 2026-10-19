use crate::background::supervised_task::TaskSupervisor;
use crate::domains::health_issues::service::SlackHealthService;
use crate::storage::{EncryptionService, StorageBackend};
use axum::{Json, extract::State};
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
}

/// `GET /health`: liveness plus the build version and auth mode.
pub(super) async fn endpoint(State(state): State<HealthState>) -> Json<HealthResponse> {
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

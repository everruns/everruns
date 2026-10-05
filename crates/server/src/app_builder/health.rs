use crate::domains::health_issues::service::SlackHealthService;
use crate::storage::{EncryptionService, StorageBackend};
use crate::supervised_task::TaskSupervisor;
use std::sync::Arc;

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
        SlackHealthService::new(db, encryption).spawn(),
    );
}

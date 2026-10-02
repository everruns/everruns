//! Helpers shared by the PostgreSQL benchmarks.

use chrono::Utc;

use crate::persistence::{WorkerInfo, WorkflowEventStore};

/// Register a benchmark worker so the store lets it claim tasks.
///
/// The PostgreSQL store only hands tasks to a registered, non-draining worker,
/// so a benchmark that claims under an unregistered id spins on empty claims
/// forever.
pub async fn register_bench_worker<S: WorkflowEventStore + ?Sized>(
    store: &S,
    worker_id: &str,
    activity_type: &str,
) {
    store
        .register_worker(WorkerInfo {
            id: worker_id.to_string(),
            worker_group: Some("bench".to_string()),
            activity_types: vec![activity_type.to_string()],
            max_concurrency: 1,
            current_load: 0,
            status: "active".to_string(),
            accepting_tasks: true,
            backpressure_reason: None,
            started_at: Utc::now(),
            last_heartbeat_at: Utc::now(),
            hostname: None,
            version: None,
            metadata: None,
            tasks_completed: 0,
            tasks_failed: 0,
            avg_task_duration_ms: None,
        })
        .await
        .expect("failed to register benchmark worker");
}

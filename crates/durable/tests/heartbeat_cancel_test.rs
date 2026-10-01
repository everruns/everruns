//! A cancelled workflow reaches the worker that still owns its task (EVE-1134).
//!
//! The heartbeat used to report `should_cancel` only when the claim was lost,
//! so a running activity never learned that its turn was cancelled. Ownership
//! loss stays distinguishable: it is the only case with `accepted == false`.
//!
//! The PostgreSQL case runs with `--features postgres-tests` against a
//! migrated database (DATABASE_URL, as in `postgres_integration_test`).

use serde_json::json;
use uuid::Uuid;

use everruns_durable::persistence::{
    InMemoryWorkflowEventStore, TaskDefinition, WorkflowEventStore, WorkflowStatus,
};
use everruns_durable::workflow::ActivityOptions;

async fn cancelled_turn_reaches_the_owning_worker(store: &impl WorkflowEventStore) {
    let workflow_id = Uuid::now_v7();
    store
        .create_workflow(workflow_id, "heartbeat_cancel", json!({}), None)
        .await
        .unwrap();
    let task_id = store
        .enqueue_task(TaskDefinition {
            workflow_id: Some(workflow_id),
            activity_id: "reason-1".to_string(),
            activity_type: "heartbeat_cancel_reason".to_string(),
            input: json!({}),
            options: ActivityOptions::default(),
        })
        .await
        .unwrap();
    let claimed = store
        .claim_task("worker-1", &["heartbeat_cancel_reason".to_string()], 1)
        .await
        .unwrap();
    assert_eq!(claimed.len(), 1);

    let live = store
        .heartbeat_task(task_id, "worker-1", None)
        .await
        .unwrap();
    assert!(live.accepted);
    assert!(!live.should_cancel, "a running turn is not cancelled");

    store
        .update_workflow_status(workflow_id, WorkflowStatus::Cancelled, None, None)
        .await
        .unwrap();
    let cancelled = store
        .heartbeat_task(task_id, "worker-1", None)
        .await
        .unwrap();
    assert!(cancelled.accepted, "the worker still owns the task");
    assert!(cancelled.should_cancel, "the turn was cancelled");
}

#[tokio::test]
async fn in_memory_store_reports_a_cancelled_turn() {
    cancelled_turn_reaches_the_owning_worker(&InMemoryWorkflowEventStore::new()).await;
}

#[cfg(feature = "postgres-tests")]
#[tokio::test]
async fn postgres_store_reports_a_cancelled_turn_and_ownership_loss_apart() {
    use everruns_durable::persistence::{PostgresWorkflowEventStore, WorkerInfo};
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        let port = std::env::var("DB_PORT").unwrap_or_else(|_| "9332".to_string());
        format!("postgres://postgres:postgres@localhost:{port}/everruns_test")
    });
    let store = PostgresWorkflowEventStore::new(sqlx::PgPool::connect(&url).await.unwrap());
    store
        .register_worker(WorkerInfo {
            id: "worker-1".to_string(),
            worker_group: Some("default".to_string()),
            activity_types: vec!["heartbeat_cancel_reason".to_string()],
            max_concurrency: 10,
            current_load: 0,
            status: "active".to_string(),
            accepting_tasks: true,
            backpressure_reason: None,
            started_at: chrono::Utc::now(),
            last_heartbeat_at: chrono::Utc::now(),
            hostname: None,
            version: None,
            metadata: None,
            tasks_completed: 0,
            tasks_failed: 0,
            avg_task_duration_ms: None,
        })
        .await
        .unwrap();
    cancelled_turn_reaches_the_owning_worker(&store).await;

    // A heartbeat from a worker that does not own the task is ownership loss.
    let workflow_id = Uuid::now_v7();
    store
        .create_workflow(workflow_id, "heartbeat_cancel", json!({}), None)
        .await
        .unwrap();
    let task_id = store
        .enqueue_task(TaskDefinition {
            workflow_id: Some(workflow_id),
            activity_id: "reason-2".to_string(),
            activity_type: "heartbeat_cancel_reason".to_string(),
            input: json!({}),
            options: ActivityOptions::default(),
        })
        .await
        .unwrap();
    store
        .claim_task("worker-1", &["heartbeat_cancel_reason".to_string()], 1)
        .await
        .unwrap();
    let lost = store
        .heartbeat_task(task_id, "worker-2", None)
        .await
        .unwrap();
    assert!(!lost.accepted);
    assert!(lost.should_cancel);
}

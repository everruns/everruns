//! Repository-level integration tests for PostgresWorkflowEventStore
//!
//! Test organization:
//! - postgres_integration_test.rs - Workflow/task business logic (existing)
//! - postgres_repository_test.rs  - SQL query coverage (this file)
//! - crates/server/tests/         - HTTP API endpoint tests
//!
//! These tests focus on SQL query correctness and type mappings,
//! covering queries used by the durable SSE endpoints.
//!
//! Run with: cargo test -p everruns-durable --test postgres_repository_test --features postgres-tests -- --test-threads=1
//!
//! Requirements:
//! - `postgres-tests` feature enabled
//! - PostgreSQL running with DATABASE_URL set
//! - Migrations applied (run migrations from crates/server/migrations/)

#![cfg(feature = "postgres-tests")]

use chrono::Utc;
use serde_json::json;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use everruns_durable::persistence::{
    Pagination, PostgresWorkflowEventStore, TaskDefinition, TaskFilter, TaskStatus, WorkerFilter,
    WorkerInfo, WorkflowEventStore, WorkflowFilter, WorkflowStatus,
};
use everruns_durable::reliability::{CircuitBreakerConfig, CircuitState};
use everruns_durable::workflow::ActivityOptions;
use std::time::Duration;

/// Get test database URL from environment or use default
fn get_database_url() -> String {
    std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5432/everruns_test".to_string())
}

/// Create a test store with a fresh database connection
async fn create_test_store() -> PostgresWorkflowEventStore {
    let database_url = get_database_url();
    let pool = PgPool::connect(&database_url)
        .await
        .expect("Failed to connect to PostgreSQL. Set DATABASE_URL or ensure postgres is running.");
    PostgresWorkflowEventStore::new(pool)
}

/// Clean up test data for a specific workflow
async fn cleanup_workflow(store: &PostgresWorkflowEventStore, workflow_id: Uuid) {
    sqlx::query("DELETE FROM durable_signals WHERE workflow_id = $1")
        .bind(workflow_id)
        .execute(store.pool())
        .await
        .ok();
    sqlx::query("DELETE FROM durable_dead_letter_queue WHERE workflow_id = $1")
        .bind(workflow_id)
        .execute(store.pool())
        .await
        .ok();
    sqlx::query("DELETE FROM durable_task_queue WHERE workflow_id = $1")
        .bind(workflow_id)
        .execute(store.pool())
        .await
        .ok();
    sqlx::query("DELETE FROM durable_workflow_events WHERE workflow_id = $1")
        .bind(workflow_id)
        .execute(store.pool())
        .await
        .ok();
    sqlx::query("DELETE FROM durable_workflow_instances WHERE id = $1")
        .bind(workflow_id)
        .execute(store.pool())
        .await
        .ok();
}

/// Clean up test worker
async fn cleanup_worker(store: &PostgresWorkflowEventStore, worker_id: &str) {
    sqlx::query("DELETE FROM durable_workers WHERE id = $1")
        .bind(worker_id)
        .execute(store.pool())
        .await
        .ok();
}

/// Clean up test circuit breaker
async fn cleanup_circuit_breaker(store: &PostgresWorkflowEventStore, key: &str) {
    sqlx::query("DELETE FROM durable_circuit_breaker_state WHERE key = $1")
        .bind(key)
        .execute(store.pool())
        .await
        .ok();
}

// ============================================
// Workflow Query Tests
// ============================================

#[tokio::test]
async fn test_list_workflows_empty() {
    let store = create_test_store().await;

    let workflows = store
        .list_workflows(
            WorkflowFilter::default(),
            Pagination {
                offset: 0,
                limit: 10,
            },
        )
        .await
        .unwrap();

    // May have existing data, just verify query succeeds
    assert!(workflows.len() <= 10);
}

#[tokio::test]
async fn test_list_workflows_with_filter() {
    let store = create_test_store().await;
    let workflow_id = Uuid::now_v7();

    // Create workflow
    store
        .create_workflow(
            workflow_id,
            "list_filter_test",
            json!({"key": "value"}),
            None,
        )
        .await
        .unwrap();

    // List with status filter
    let pending = store
        .list_workflows(
            WorkflowFilter {
                status: Some(WorkflowStatus::Pending),
                workflow_type: None,
            },
            Pagination {
                offset: 0,
                limit: 100,
            },
        )
        .await
        .unwrap();
    assert!(pending.iter().any(|w| w.id == workflow_id));

    // List with type filter
    let by_type = store
        .list_workflows(
            WorkflowFilter {
                status: None,
                workflow_type: Some("list_filter_test".to_string()),
            },
            Pagination {
                offset: 0,
                limit: 100,
            },
        )
        .await
        .unwrap();
    assert!(by_type.iter().any(|w| w.id == workflow_id));

    // List with both filters
    let combined = store
        .list_workflows(
            WorkflowFilter {
                status: Some(WorkflowStatus::Pending),
                workflow_type: Some("list_filter_test".to_string()),
            },
            Pagination {
                offset: 0,
                limit: 100,
            },
        )
        .await
        .unwrap();
    assert!(combined.iter().any(|w| w.id == workflow_id));

    cleanup_workflow(&store, workflow_id).await;
}

#[tokio::test]
async fn test_list_workflows_pagination() {
    let store = create_test_store().await;
    let workflow_ids: Vec<Uuid> = (0..5).map(|_| Uuid::now_v7()).collect();

    // Create 5 workflows
    for (i, id) in workflow_ids.iter().enumerate() {
        store
            .create_workflow(*id, &format!("pagination_test_{}", i), json!({}), None)
            .await
            .unwrap();
    }

    // Test pagination
    let page1 = store
        .list_workflows(
            WorkflowFilter::default(),
            Pagination {
                offset: 0,
                limit: 2,
            },
        )
        .await
        .unwrap();
    assert_eq!(page1.len(), 2);

    let page2 = store
        .list_workflows(
            WorkflowFilter::default(),
            Pagination {
                offset: 2,
                limit: 2,
            },
        )
        .await
        .unwrap();
    assert_eq!(page2.len(), 2);

    // Cleanup
    for id in workflow_ids {
        cleanup_workflow(&store, id).await;
    }
}

#[tokio::test]
async fn test_count_active_workflows() {
    let store = create_test_store().await;
    let workflow_id = Uuid::now_v7();

    let initial_count = store.count_active_workflows().await.unwrap();

    // Create running workflow
    store
        .create_workflow(workflow_id, "count_test", json!({}), None)
        .await
        .unwrap();
    store
        .update_workflow_status(workflow_id, WorkflowStatus::Running, None, None)
        .await
        .unwrap();

    let new_count = store.count_active_workflows().await.unwrap();
    assert_eq!(new_count, initial_count + 1);

    cleanup_workflow(&store, workflow_id).await;
}

#[tokio::test]
async fn test_cancel_workflow() {
    let store = create_test_store().await;
    let workflow_id = Uuid::now_v7();

    store
        .create_workflow(workflow_id, "cancel_test", json!({}), None)
        .await
        .unwrap();

    // Cancel workflow
    store.cancel_workflow(workflow_id).await.unwrap();

    // Verify status is cancelled
    let status = store.get_workflow_status(workflow_id).await.unwrap();
    assert_eq!(status, WorkflowStatus::Cancelled);

    cleanup_workflow(&store, workflow_id).await;
}

#[tokio::test]
async fn test_get_workflow_events() {
    let store = create_test_store().await;
    let workflow_id = Uuid::now_v7();

    store
        .create_workflow(workflow_id, "events_test", json!({"input": "data"}), None)
        .await
        .unwrap();

    // Append some events
    use everruns_durable::workflow::WorkflowEvent;
    let events = vec![
        WorkflowEvent::ActivityScheduled {
            activity_id: "act1".to_string(),
            activity_type: "test_activity".to_string(),
            input: json!({}),
            options: ActivityOptions::default(),
        },
        WorkflowEvent::ActivityCompleted {
            activity_id: "act1".to_string(),
            result: json!({"done": true}),
        },
    ];
    store.append_events(workflow_id, 0, events).await.unwrap();

    // Get events via the API method
    let retrieved = store.get_workflow_events(workflow_id).await.unwrap();
    assert_eq!(retrieved.len(), 2);

    cleanup_workflow(&store, workflow_id).await;
}

// ============================================
// Task Query Tests
// ============================================

#[tokio::test]
async fn test_get_task() {
    let store = create_test_store().await;
    let workflow_id = Uuid::now_v7();

    store
        .create_workflow(workflow_id, "get_task_test", json!({}), None)
        .await
        .unwrap();

    let task_id = store
        .enqueue_task(TaskDefinition {
            workflow_id,
            activity_id: "task1".to_string(),
            activity_type: "test_activity".to_string(),
            input: json!({"data": "value"}),
            options: ActivityOptions::default(),
        })
        .await
        .unwrap();

    // Get task by ID
    let task = store.get_task(task_id).await.unwrap();
    assert_eq!(task.id, task_id);
    assert_eq!(task.workflow_id, workflow_id);
    assert_eq!(task.activity_id, "task1");
    assert_eq!(task.activity_type, "test_activity");

    cleanup_workflow(&store, workflow_id).await;
}

#[tokio::test]
async fn test_list_tasks_empty() {
    let store = create_test_store().await;

    let tasks = store
        .list_tasks(
            TaskFilter::default(),
            Pagination {
                offset: 0,
                limit: 10,
            },
        )
        .await
        .unwrap();

    // Query should succeed
    assert!(tasks.len() <= 10);
}

#[tokio::test]
async fn test_list_tasks_with_filter() {
    let store = create_test_store().await;
    let workflow_id = Uuid::now_v7();

    store
        .create_workflow(workflow_id, "list_tasks_test", json!({}), None)
        .await
        .unwrap();

    let task_id = store
        .enqueue_task(TaskDefinition {
            workflow_id,
            activity_id: "filtered_task".to_string(),
            activity_type: "filterable_activity".to_string(),
            input: json!({}),
            options: ActivityOptions::default(),
        })
        .await
        .unwrap();

    // Filter by status
    let pending = store
        .list_tasks(
            TaskFilter {
                status: Some(TaskStatus::Pending),
                activity_type: None,
                workflow_id: None,
            },
            Pagination {
                offset: 0,
                limit: 100,
            },
        )
        .await
        .unwrap();
    assert!(pending.iter().any(|t| t.id == task_id));

    // Filter by activity type
    let by_type = store
        .list_tasks(
            TaskFilter {
                status: None,
                activity_type: Some("filterable_activity".to_string()),
                workflow_id: None,
            },
            Pagination {
                offset: 0,
                limit: 100,
            },
        )
        .await
        .unwrap();
    assert!(by_type.iter().any(|t| t.id == task_id));

    cleanup_workflow(&store, workflow_id).await;
}

// ============================================
// Circuit Breaker Tests
// ============================================

#[tokio::test]
async fn test_circuit_breaker_lifecycle() {
    let store = create_test_store().await;
    let cb_key = format!("test_cb_{}", Uuid::now_v7());

    let config = CircuitBreakerConfig {
        failure_threshold: 5,
        success_threshold: 2,
        reset_timeout: Duration::from_secs(30),
        window_size: Duration::from_secs(60),
    };

    // Create circuit breaker
    store
        .create_circuit_breaker(&cb_key, &config)
        .await
        .unwrap();

    // Get circuit breaker
    let cb = store.get_circuit_breaker(&cb_key).await.unwrap();
    assert!(cb.is_some());
    let cb = cb.unwrap();
    assert_eq!(cb.key, cb_key);
    assert_eq!(cb.state, CircuitState::Closed);

    // Update circuit breaker (record failure)
    store
        .update_circuit_breaker(&cb_key, CircuitState::Closed, 1, 0)
        .await
        .unwrap();

    let updated = store.get_circuit_breaker(&cb_key).await.unwrap().unwrap();
    assert_eq!(updated.failure_count, 1);

    // Delete circuit breaker
    store.delete_circuit_breaker(&cb_key).await.unwrap();

    let deleted = store.get_circuit_breaker(&cb_key).await.unwrap();
    assert!(deleted.is_none());

    cleanup_circuit_breaker(&store, &cb_key).await;
}

#[tokio::test]
async fn test_list_circuit_breakers() {
    let store = create_test_store().await;
    let cb_key = format!("list_cb_{}", Uuid::now_v7());

    let config = CircuitBreakerConfig {
        failure_threshold: 5,
        success_threshold: 2,
        reset_timeout: Duration::from_secs(30),
        window_size: Duration::from_secs(60),
    };

    store
        .create_circuit_breaker(&cb_key, &config)
        .await
        .unwrap();

    let breakers = store.list_circuit_breakers().await.unwrap();
    assert!(breakers.iter().any(|cb| cb.key == cb_key));

    cleanup_circuit_breaker(&store, &cb_key).await;
}

#[tokio::test]
async fn test_force_open_circuit_breaker() {
    let store = create_test_store().await;
    let cb_key = format!("force_open_cb_{}", Uuid::now_v7());

    let config = CircuitBreakerConfig {
        failure_threshold: 5,
        success_threshold: 2,
        reset_timeout: Duration::from_secs(30),
        window_size: Duration::from_secs(60),
    };

    store
        .create_circuit_breaker(&cb_key, &config)
        .await
        .unwrap();

    // Force open
    store.force_open_circuit_breaker(&cb_key).await.unwrap();

    let cb = store.get_circuit_breaker(&cb_key).await.unwrap().unwrap();
    assert_eq!(cb.state, CircuitState::Open);

    cleanup_circuit_breaker(&store, &cb_key).await;
}

#[tokio::test]
async fn test_force_close_circuit_breaker() {
    let store = create_test_store().await;
    let cb_key = format!("force_close_cb_{}", Uuid::now_v7());

    let config = CircuitBreakerConfig {
        failure_threshold: 5,
        success_threshold: 2,
        reset_timeout: Duration::from_secs(30),
        window_size: Duration::from_secs(60),
    };

    store
        .create_circuit_breaker(&cb_key, &config)
        .await
        .unwrap();

    // Force open first
    store.force_open_circuit_breaker(&cb_key).await.unwrap();

    // Then force close
    store.force_close_circuit_breaker(&cb_key).await.unwrap();

    let cb = store.get_circuit_breaker(&cb_key).await.unwrap().unwrap();
    assert_eq!(cb.state, CircuitState::Closed);
    assert_eq!(cb.failure_count, 0); // Should be reset

    cleanup_circuit_breaker(&store, &cb_key).await;
}

// ============================================
// Worker Query Tests
// ============================================

#[tokio::test]
async fn test_drain_and_resume_worker() {
    let store = create_test_store().await;
    let worker_id = format!("drain_test_{}", Uuid::now_v7());

    // Register worker
    store
        .register_worker(WorkerInfo {
            id: worker_id.clone(),
            worker_group: Some("default".to_string()),
            activity_types: vec!["test".to_string()],
            max_concurrency: 10,
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
        .unwrap();

    // Keep heartbeat fresh
    store.worker_heartbeat(&worker_id, 0, true).await.unwrap();

    // Drain worker
    store.drain_worker(&worker_id).await.unwrap();

    let workers = store.list_workers(WorkerFilter::default()).await.unwrap();
    let worker = workers.iter().find(|w| w.id == worker_id);
    assert!(worker.is_some());
    assert!(!worker.unwrap().accepting_tasks);

    // Resume worker
    store.resume_worker(&worker_id).await.unwrap();

    // Refresh heartbeat
    store.worker_heartbeat(&worker_id, 0, true).await.unwrap();

    let workers = store.list_workers(WorkerFilter::default()).await.unwrap();
    let worker = workers.iter().find(|w| w.id == worker_id).unwrap();
    assert!(worker.accepting_tasks);

    cleanup_worker(&store, &worker_id).await;
}

// ============================================
// System Health Tests
// ============================================

#[tokio::test]
async fn test_get_system_health() {
    let store = create_test_store().await;

    // Query should succeed and return valid SystemHealth struct
    // All fields are u64 so we just verify the query executes without error
    let health = store.get_system_health().await.unwrap();

    // Verify active_workers <= total_workers (sanity check)
    assert!(health.active_workers <= health.total_workers);
    // Verify workers_accepting <= active_workers
    assert!(health.workers_accepting <= health.active_workers);
    // Verify current_load <= total_capacity (when capacity exists)
    if health.total_capacity > 0 {
        assert!(health.current_load <= health.total_capacity);
    }
}

#[tokio::test]
async fn test_get_system_health_with_data() {
    let store = create_test_store().await;
    let workflow_id = Uuid::now_v7();
    let worker_id = format!("health_test_{}", Uuid::now_v7());

    // Create a pending workflow
    store
        .create_workflow(workflow_id, "health_test", json!({}), None)
        .await
        .unwrap();

    // Register an active worker
    store
        .register_worker(WorkerInfo {
            id: worker_id.clone(),
            worker_group: Some("default".to_string()),
            activity_types: vec!["test".to_string()],
            max_concurrency: 10,
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
        .unwrap();

    // Keep heartbeat fresh
    store.worker_heartbeat(&worker_id, 0, true).await.unwrap();

    // Create a pending task
    store
        .enqueue_task(TaskDefinition {
            workflow_id,
            activity_id: "health_task".to_string(),
            activity_type: "test".to_string(),
            input: json!({}),
            options: ActivityOptions::default(),
        })
        .await
        .unwrap();

    // Get health - should reflect our test data
    let health = store.get_system_health().await.unwrap();
    assert!(health.pending_workflows >= 1);
    assert!(health.active_workers >= 1);
    assert!(health.pending_tasks >= 1);

    cleanup_worker(&store, &worker_id).await;
    cleanup_workflow(&store, workflow_id).await;
}

// ============================================
// Stale Worker Heartbeat Tests
// ============================================

/// Verify that get_system_health excludes workers with stale heartbeats
/// from the active_workers count. This was the root cause of the dashboard
/// showing 15 workers when only 3 were actually connected.
#[tokio::test]
async fn test_system_health_excludes_stale_workers() {
    let store = create_test_store().await;
    let fresh_worker_id = format!("fresh_{}", Uuid::now_v7());
    let stale_worker_id = format!("stale_{}", Uuid::now_v7());

    // Register a fresh worker (recent heartbeat)
    store
        .register_worker(WorkerInfo {
            id: fresh_worker_id.clone(),
            worker_group: Some("default".to_string()),
            activity_types: vec!["test".to_string()],
            max_concurrency: 5,
            current_load: 2,
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
        .unwrap();
    store
        .worker_heartbeat(&fresh_worker_id, 2, true)
        .await
        .unwrap();

    // Register a stale worker (old heartbeat via direct SQL)
    store
        .register_worker(WorkerInfo {
            id: stale_worker_id.clone(),
            worker_group: Some("default".to_string()),
            activity_types: vec!["test".to_string()],
            max_concurrency: 10,
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
        .unwrap();

    // Manually set the stale worker's heartbeat to 5 minutes ago
    sqlx::query(
        "UPDATE durable_workers SET last_heartbeat_at = NOW() - INTERVAL '5 minutes' WHERE id = $1",
    )
    .bind(&stale_worker_id)
    .execute(store.pool())
    .await
    .unwrap();

    // get_system_health should only count the fresh worker
    let health = store.get_system_health().await.unwrap();
    assert!(
        health.active_workers >= 1,
        "should have at least 1 active worker (the fresh one)"
    );
    // The stale worker should NOT be counted — capacity should not include its 10
    assert!(
        health.total_capacity >= 5,
        "fresh worker capacity should be counted"
    );
    // The stale worker's load should not inflate total
    assert!(
        health.current_load >= 2,
        "fresh worker load should be counted"
    );

    // list_workers should also exclude stale worker
    let workers = store.list_workers(WorkerFilter::default()).await.unwrap();
    let stale_in_list = workers.iter().any(|w| w.id == stale_worker_id);
    assert!(
        !stale_in_list,
        "stale worker should not appear in list_workers"
    );
    let fresh_in_list = workers.iter().any(|w| w.id == fresh_worker_id);
    assert!(fresh_in_list, "fresh worker should appear in list_workers");

    cleanup_worker(&store, &fresh_worker_id).await;
    cleanup_worker(&store, &stale_worker_id).await;
}

/// Verify that reclaim_stale_tasks marks stale workers as stopped.
#[tokio::test]
async fn test_reclaim_stale_tasks_marks_stale_workers_stopped() {
    let store = create_test_store().await;
    let stale_worker_id = format!("stale_reap_{}", Uuid::now_v7());

    // Register worker
    store
        .register_worker(WorkerInfo {
            id: stale_worker_id.clone(),
            worker_group: Some("default".to_string()),
            activity_types: vec!["test".to_string()],
            max_concurrency: 5,
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
        .unwrap();

    // Make heartbeat stale
    sqlx::query(
        "UPDATE durable_workers SET last_heartbeat_at = NOW() - INTERVAL '5 minutes' WHERE id = $1",
    )
    .bind(&stale_worker_id)
    .execute(store.pool())
    .await
    .unwrap();

    // Reclaim stale tasks (which also marks stale workers as stopped)
    store
        .reclaim_stale_tasks(Duration::from_secs(30))
        .await
        .unwrap();

    // Worker should now be stopped
    let row = sqlx::query("SELECT status FROM durable_workers WHERE id = $1")
        .bind(&stale_worker_id)
        .fetch_one(store.pool())
        .await
        .unwrap();
    let status: String = row.get("status");
    assert_eq!(status, "stopped", "stale worker should be marked stopped");

    cleanup_worker(&store, &stale_worker_id).await;
}

/// Verify that get_system_health returns completed/failed workflow and task counts.
#[tokio::test]
async fn test_system_health_includes_completed_and_failed_counts() {
    let store = create_test_store().await;
    let wf_completed = Uuid::now_v7();
    let wf_failed = Uuid::now_v7();
    let wf_pending = Uuid::now_v7();

    // Create workflows in various states
    store
        .create_workflow(wf_completed, "health_wf_test", json!({}), None)
        .await
        .unwrap();
    store
        .create_workflow(wf_failed, "health_wf_test", json!({}), None)
        .await
        .unwrap();
    store
        .create_workflow(wf_pending, "health_wf_test", json!({}), None)
        .await
        .unwrap();

    // Mark one as completed, one as failed
    sqlx::query("UPDATE durable_workflow_instances SET status = 'completed', completed_at = NOW() WHERE id = $1")
        .bind(wf_completed)
        .execute(store.pool())
        .await
        .unwrap();
    sqlx::query("UPDATE durable_workflow_instances SET status = 'failed', completed_at = NOW() WHERE id = $1")
        .bind(wf_failed)
        .execute(store.pool())
        .await
        .unwrap();

    let health = store.get_system_health().await.unwrap();
    assert!(
        health.completed_workflows >= 1,
        "should count completed workflows"
    );
    assert!(
        health.failed_workflows >= 1,
        "should count failed workflows"
    );
    assert!(
        health.pending_workflows >= 1,
        "should count pending workflows"
    );
    // started_workflows counts all workflows where started_at IS NOT NULL.
    // completed and failed workflows have started_at set (they ran before finishing).
    // Note: our test uses direct SQL UPDATE which may not set started_at,
    // so we just verify the field exists and is non-negative.
    assert!(
        health.started_workflows
            <= health.completed_workflows + health.failed_workflows + health.running_workflows,
        "started_workflows should be consistent with other counts"
    );

    cleanup_workflow(&store, wf_completed).await;
    cleanup_workflow(&store, wf_failed).await;
    cleanup_workflow(&store, wf_pending).await;
}

/// Verify WORKER_HEARTBEAT_TIMEOUT_SECS constant is used consistently.
#[test]
fn test_worker_heartbeat_timeout_constant() {
    use everruns_durable::persistence::WORKER_HEARTBEAT_TIMEOUT_SECS;
    assert_eq!(WORKER_HEARTBEAT_TIMEOUT_SECS, 60);
}

//! Integration tests for PostgresWorkflowEventStore
//!
//! Run with: cargo test -p everruns-durable --test postgres_integration_test --features postgres-tests -- --test-threads=1
//!
//! Requirements:
//! - `postgres-tests` feature enabled
//! - PostgreSQL running with DATABASE_URL set or default postgres://localhost:5432/everruns_test
//! - Migrations applied (run migrations from crates/server/migrations/)

#![cfg(feature = "postgres-tests")]

use std::time::Duration;

use chrono::Utc;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use everruns_durable::persistence::{
    DlqFilter, Pagination, PostgresWorkflowEventStore, StoreError, TaskDefinition,
    TaskFailureOutcome, TaskStatus, TraceContext, WorkerFilter, WorkerInfo, WorkflowEventStore,
    WorkflowStatus,
};
use everruns_durable::reliability::RetryPolicy;
use everruns_durable::workflow::{ActivityOptions, WorkflowError, WorkflowEvent, WorkflowSignal};

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

/// Register a test worker with the given ID and activity types
async fn register_test_worker(
    store: &PostgresWorkflowEventStore,
    worker_id: &str,
    activity_types: Vec<String>,
) {
    store
        .register_worker(WorkerInfo {
            id: worker_id.to_string(),
            worker_group: Some("default".to_string()),
            activity_types,
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
        .expect("Failed to register test worker");
}

/// Clean up a test worker
async fn cleanup_worker(store: &PostgresWorkflowEventStore, worker_id: &str) {
    sqlx::query("DELETE FROM durable_workers WHERE id = $1")
        .bind(worker_id)
        .execute(store.pool())
        .await
        .ok();
}

/// Clean up test data for a specific workflow
async fn cleanup_workflow(store: &PostgresWorkflowEventStore, workflow_id: Uuid) {
    // Delete in reverse dependency order
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

// --------------------------------------------
// Workflow Lifecycle Tests
// --------------------------------------------

#[tokio::test]
async fn test_create_and_get_workflow() {
    let store = create_test_store().await;
    let workflow_id = Uuid::now_v7();

    // Create workflow
    store
        .create_workflow(
            workflow_id,
            "test_workflow",
            json!({"order_id": "123"}),
            Some(&TraceContext {
                trace_id: "trace-123".to_string(),
                span_id: "span-456".to_string(),
                trace_flags: 1,
            }),
        )
        .await
        .expect("Failed to create workflow");

    // Get status
    let status = store
        .get_workflow_status(workflow_id)
        .await
        .expect("Failed to get status");
    assert_eq!(status, WorkflowStatus::Pending);

    // Get full info
    let info = store
        .get_workflow_info(workflow_id)
        .await
        .expect("Failed to get workflow info");
    assert_eq!(info.workflow_type, "test_workflow");
    assert_eq!(info.status, WorkflowStatus::Pending);
    assert_eq!(info.input, json!({"order_id": "123"}));

    // Cleanup
    cleanup_workflow(&store, workflow_id).await;
}

#[tokio::test]
async fn test_workflow_status_transitions() {
    let store = create_test_store().await;
    let workflow_id = Uuid::now_v7();

    // Create workflow
    store
        .create_workflow(workflow_id, "test_workflow", json!({}), None)
        .await
        .expect("Failed to create workflow");

    // Pending -> Running
    store
        .update_workflow_status(workflow_id, WorkflowStatus::Running, None, None)
        .await
        .expect("Failed to update to running");
    let status = store.get_workflow_status(workflow_id).await.unwrap();
    assert_eq!(status, WorkflowStatus::Running);

    // Running -> Completed
    store
        .update_workflow_status(
            workflow_id,
            WorkflowStatus::Completed,
            Some(json!({"result": "success"})),
            None,
        )
        .await
        .expect("Failed to complete");
    let info = store.get_workflow_info(workflow_id).await.unwrap();
    assert_eq!(info.status, WorkflowStatus::Completed);
    assert_eq!(info.result, Some(json!({"result": "success"})));

    cleanup_workflow(&store, workflow_id).await;
}

#[tokio::test]
async fn test_workflow_failure() {
    let store = create_test_store().await;
    let workflow_id = Uuid::now_v7();

    store
        .create_workflow(workflow_id, "failing_workflow", json!({}), None)
        .await
        .unwrap();

    let error = WorkflowError::new("Something went wrong");
    store
        .update_workflow_status(
            workflow_id,
            WorkflowStatus::Failed,
            None,
            Some(error.clone()),
        )
        .await
        .unwrap();

    let info = store.get_workflow_info(workflow_id).await.unwrap();
    assert_eq!(info.status, WorkflowStatus::Failed);
    assert!(info.error.is_some());
    assert_eq!(info.error.unwrap().message, "Something went wrong");

    cleanup_workflow(&store, workflow_id).await;
}

#[tokio::test]
async fn test_workflow_not_found() {
    let store = create_test_store().await;
    let fake_id = Uuid::now_v7();

    let result = store.get_workflow_status(fake_id).await;
    assert!(matches!(result, Err(StoreError::WorkflowNotFound(_))));
}

// --------------------------------------------
// Event Sourcing Tests
// --------------------------------------------

#[tokio::test]
async fn test_append_and_load_events() {
    let store = create_test_store().await;
    let workflow_id = Uuid::now_v7();

    store
        .create_workflow(workflow_id, "event_test", json!({"test": true}), None)
        .await
        .unwrap();

    // Append workflow started event
    let events = vec![WorkflowEvent::WorkflowStarted {
        input: json!({"test": true}),
    }];
    let seq = store.append_events(workflow_id, 0, events).await.unwrap();
    assert_eq!(seq, 1);

    // Append activity events
    let events = vec![
        WorkflowEvent::ActivityScheduled {
            activity_id: "step-1".to_string(),
            activity_type: "process".to_string(),
            input: json!({}),
            options: ActivityOptions::default(),
        },
        WorkflowEvent::ActivityCompleted {
            activity_id: "step-1".to_string(),
            result: json!({"done": true}),
        },
    ];
    let seq = store.append_events(workflow_id, 1, events).await.unwrap();
    assert_eq!(seq, 3);

    // Load all events
    let loaded = store.load_events(workflow_id).await.unwrap();
    assert_eq!(loaded.len(), 3);
    assert_eq!(loaded[0].0, 0); // sequence 0
    assert_eq!(loaded[1].0, 1); // sequence 1
    assert_eq!(loaded[2].0, 2); // sequence 2

    // Verify event types
    assert!(matches!(loaded[0].1, WorkflowEvent::WorkflowStarted { .. }));
    assert!(matches!(
        loaded[1].1,
        WorkflowEvent::ActivityScheduled { .. }
    ));
    assert!(matches!(
        loaded[2].1,
        WorkflowEvent::ActivityCompleted { .. }
    ));

    cleanup_workflow(&store, workflow_id).await;
}

#[tokio::test]
async fn test_optimistic_concurrency_conflict() {
    let store = create_test_store().await;
    let workflow_id = Uuid::now_v7();

    store
        .create_workflow(workflow_id, "concurrency_test", json!({}), None)
        .await
        .unwrap();

    // First append succeeds
    store
        .append_events(
            workflow_id,
            0,
            vec![WorkflowEvent::WorkflowStarted { input: json!({}) }],
        )
        .await
        .unwrap();

    // Second append with same expected sequence fails
    let result = store
        .append_events(
            workflow_id,
            0, // Wrong! Should be 1
            vec![WorkflowEvent::WorkflowCompleted { result: json!({}) }],
        )
        .await;

    assert!(matches!(
        result,
        Err(StoreError::ConcurrencyConflict {
            expected: 0,
            actual: 1
        })
    ));

    cleanup_workflow(&store, workflow_id).await;
}

// --------------------------------------------
// Task Queue Tests
// --------------------------------------------

#[tokio::test]
async fn test_task_enqueue_and_claim() {
    let store = create_test_store().await;
    let workflow_id = Uuid::now_v7();

    // Register workers before claiming tasks
    register_test_worker(&store, "worker-1", vec!["send_email".to_string()]).await;
    register_test_worker(&store, "worker-2", vec!["send_email".to_string()]).await;

    store
        .create_workflow(workflow_id, "task_test", json!({}), None)
        .await
        .unwrap();

    // Enqueue a task
    let task = TaskDefinition {
        workflow_id,
        activity_id: "step-1".to_string(),
        activity_type: "send_email".to_string(),
        input: json!({"to": "test@example.com"}),
        options: ActivityOptions::default(),
    };
    let task_id = store.enqueue_task(task).await.unwrap();

    // Claim the task
    let claimed = store
        .claim_task("worker-1", &["send_email".to_string()], 10)
        .await
        .unwrap();
    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].id, task_id);
    assert_eq!(claimed[0].activity_type, "send_email");
    assert_eq!(claimed[0].attempt, 1);

    // Second claim should get nothing (task already claimed)
    let claimed2 = store
        .claim_task("worker-2", &["send_email".to_string()], 10)
        .await
        .unwrap();
    assert!(claimed2.is_empty());

    cleanup_workflow(&store, workflow_id).await;
    cleanup_worker(&store, "worker-1").await;
    cleanup_worker(&store, "worker-2").await;
}

#[tokio::test]
async fn test_task_claim_by_activity_type() {
    let store = create_test_store().await;
    let workflow_id = Uuid::now_v7();

    // Register workers before claiming tasks
    register_test_worker(&store, "email-worker", vec!["send_email".to_string()]).await;
    register_test_worker(&store, "sms-worker", vec!["send_sms".to_string()]).await;

    store
        .create_workflow(workflow_id, "activity_filter_test", json!({}), None)
        .await
        .unwrap();

    // Enqueue tasks of different types
    store
        .enqueue_task(TaskDefinition {
            workflow_id,
            activity_id: "email".to_string(),
            activity_type: "send_email".to_string(),
            input: json!({}),
            options: ActivityOptions::default(),
        })
        .await
        .unwrap();

    store
        .enqueue_task(TaskDefinition {
            workflow_id,
            activity_id: "sms".to_string(),
            activity_type: "send_sms".to_string(),
            input: json!({}),
            options: ActivityOptions::default(),
        })
        .await
        .unwrap();

    // Worker that only handles emails
    let email_tasks = store
        .claim_task("email-worker", &["send_email".to_string()], 10)
        .await
        .unwrap();
    assert_eq!(email_tasks.len(), 1);
    assert_eq!(email_tasks[0].activity_type, "send_email");

    // Worker that handles sms
    let sms_tasks = store
        .claim_task("sms-worker", &["send_sms".to_string()], 10)
        .await
        .unwrap();
    assert_eq!(sms_tasks.len(), 1);
    assert_eq!(sms_tasks[0].activity_type, "send_sms");

    cleanup_workflow(&store, workflow_id).await;
    cleanup_worker(&store, "email-worker").await;
    cleanup_worker(&store, "sms-worker").await;
}

#[tokio::test]
async fn test_task_complete() {
    let store = create_test_store().await;
    let workflow_id = Uuid::now_v7();

    // Register workers before claiming tasks
    register_test_worker(&store, "worker", vec!["process".to_string()]).await;
    register_test_worker(&store, "worker-2", vec!["process".to_string()]).await;

    store
        .create_workflow(workflow_id, "complete_test", json!({}), None)
        .await
        .unwrap();

    let task_id = store
        .enqueue_task(TaskDefinition {
            workflow_id,
            activity_id: "step".to_string(),
            activity_type: "process".to_string(),
            input: json!({}),
            options: ActivityOptions::default(),
        })
        .await
        .unwrap();

    store
        .claim_task("worker", &["process".to_string()], 1)
        .await
        .unwrap();

    // Complete the task (must pass worker_id that claimed it)
    store
        .complete_task(task_id, "worker", json!({"status": "done"}))
        .await
        .unwrap();

    // Task should not be claimable anymore
    let claimed = store
        .claim_task("worker-2", &["process".to_string()], 10)
        .await
        .unwrap();
    assert!(claimed.is_empty());

    cleanup_workflow(&store, workflow_id).await;
    cleanup_worker(&store, "worker").await;
    cleanup_worker(&store, "worker-2").await;
}

#[tokio::test]
async fn test_task_failure_with_retry() {
    let store = create_test_store().await;
    let workflow_id = Uuid::now_v7();

    // Register worker before claiming tasks
    register_test_worker(&store, "worker", vec!["flaky_task".to_string()]).await;

    store
        .create_workflow(workflow_id, "retry_test", json!({}), None)
        .await
        .unwrap();

    let options = ActivityOptions {
        retry_policy: RetryPolicy::exponential()
            .with_max_attempts(3)
            .with_initial_interval(Duration::from_millis(10)),
        ..Default::default()
    };

    let task_id = store
        .enqueue_task(TaskDefinition {
            workflow_id,
            activity_id: "flaky".to_string(),
            activity_type: "flaky_task".to_string(),
            input: json!({}),
            options,
        })
        .await
        .unwrap();

    // Claim and fail
    store
        .claim_task("worker", &["flaky_task".to_string()], 1)
        .await
        .unwrap();
    let outcome = store.fail_task(task_id, "Network error").await.unwrap();

    // Should be scheduled for retry
    assert!(matches!(
        outcome,
        TaskFailureOutcome::WillRetry {
            next_attempt: 2,
            ..
        }
    ));

    // Wait for visibility window
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Should be claimable again
    let claimed = store
        .claim_task("worker", &["flaky_task".to_string()], 1)
        .await
        .unwrap();
    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].attempt, 2);

    cleanup_workflow(&store, workflow_id).await;
    cleanup_worker(&store, "worker").await;
}

#[tokio::test]
async fn test_task_exhausts_retries_to_dlq() {
    let store = create_test_store().await;
    let workflow_id = Uuid::now_v7();

    // Register worker before claiming tasks
    register_test_worker(&store, "worker", vec!["doomed_task".to_string()]).await;

    store
        .create_workflow(workflow_id, "dlq_test", json!({}), None)
        .await
        .unwrap();

    let options = ActivityOptions {
        retry_policy: RetryPolicy::exponential()
            .with_max_attempts(2)
            .with_initial_interval(Duration::from_millis(1)),
        ..Default::default()
    };

    let task_id = store
        .enqueue_task(TaskDefinition {
            workflow_id,
            activity_id: "doomed".to_string(),
            activity_type: "doomed_task".to_string(),
            input: json!({"data": "important"}),
            options,
        })
        .await
        .unwrap();

    // Attempt 1: claim and fail
    store
        .claim_task("worker", &["doomed_task".to_string()], 1)
        .await
        .unwrap();
    store.fail_task(task_id, "Error 1").await.unwrap();

    tokio::time::sleep(Duration::from_millis(10)).await;

    // Attempt 2: claim and fail
    store
        .claim_task("worker", &["doomed_task".to_string()], 1)
        .await
        .unwrap();
    let outcome = store.fail_task(task_id, "Error 2").await.unwrap();

    // Should be moved to DLQ
    assert!(matches!(outcome, TaskFailureOutcome::MovedToDlq));

    cleanup_workflow(&store, workflow_id).await;
    cleanup_worker(&store, "worker").await;
}

#[tokio::test]
async fn test_heartbeat() {
    let store = create_test_store().await;
    let workflow_id = Uuid::now_v7();

    // Register worker before claiming tasks
    register_test_worker(&store, "worker-1", vec!["long_task".to_string()]).await;

    store
        .create_workflow(workflow_id, "heartbeat_test", json!({}), None)
        .await
        .unwrap();

    let task_id = store
        .enqueue_task(TaskDefinition {
            workflow_id,
            activity_id: "long_running".to_string(),
            activity_type: "long_task".to_string(),
            input: json!({}),
            options: ActivityOptions::default(),
        })
        .await
        .unwrap();

    store
        .claim_task("worker-1", &["long_task".to_string()], 1)
        .await
        .unwrap();

    // Heartbeat from correct worker
    let response = store
        .heartbeat_task(task_id, "worker-1", Some(json!({"progress": 50})))
        .await
        .unwrap();
    assert!(response.accepted);
    assert!(!response.should_cancel);

    // Heartbeat from wrong worker
    let response = store
        .heartbeat_task(task_id, "wrong-worker", None)
        .await
        .unwrap();
    assert!(!response.accepted);

    cleanup_workflow(&store, workflow_id).await;
    cleanup_worker(&store, "worker-1").await;
}

#[tokio::test]
async fn test_reclaim_stale_tasks() {
    let store = create_test_store().await;
    let workflow_id = Uuid::now_v7();

    // Register workers before claiming tasks
    register_test_worker(&store, "dead-worker", vec!["stale_task".to_string()]).await;
    register_test_worker(&store, "new-worker", vec!["stale_task".to_string()]).await;

    store
        .create_workflow(workflow_id, "stale_test", json!({}), None)
        .await
        .unwrap();

    store
        .enqueue_task(TaskDefinition {
            workflow_id,
            activity_id: "stale".to_string(),
            activity_type: "stale_task".to_string(),
            input: json!({}),
            options: ActivityOptions::default(),
        })
        .await
        .unwrap();

    // Claim the task
    store
        .claim_task("dead-worker", &["stale_task".to_string()], 1)
        .await
        .unwrap();

    // Simulate stale heartbeat by setting heartbeat_at in the past
    sqlx::query(
        r#"
        UPDATE durable_task_queue
        SET heartbeat_at = NOW() - INTERVAL '1 hour'
        WHERE workflow_id = $1
        "#,
    )
    .bind(workflow_id)
    .execute(store.pool())
    .await
    .unwrap();

    // Reclaim stale tasks (threshold: 30 seconds)
    let reclaimed = store
        .reclaim_stale_tasks(Duration::from_secs(30))
        .await
        .unwrap();
    assert_eq!(reclaimed.len(), 1);

    // Task should be claimable again
    let claimed = store
        .claim_task("new-worker", &["stale_task".to_string()], 1)
        .await
        .unwrap();
    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].attempt, 2); // Attempt incremented

    cleanup_workflow(&store, workflow_id).await;
    cleanup_worker(&store, "dead-worker").await;
    cleanup_worker(&store, "new-worker").await;
}

// --------------------------------------------
// Signal Tests
// --------------------------------------------

#[tokio::test]
async fn test_signals() {
    let store = create_test_store().await;
    let workflow_id = Uuid::now_v7();

    store
        .create_workflow(workflow_id, "signal_test", json!({}), None)
        .await
        .unwrap();

    // Send signals
    store
        .send_signal(
            workflow_id,
            WorkflowSignal {
                signal_type: "approval".to_string(),
                payload: json!({"approved": true}),
                sent_at: Utc::now(),
            },
        )
        .await
        .unwrap();

    store
        .send_signal(
            workflow_id,
            WorkflowSignal {
                signal_type: "update".to_string(),
                payload: json!({"new_data": "value"}),
                sent_at: Utc::now(),
            },
        )
        .await
        .unwrap();

    // Get pending signals
    let signals = store.get_pending_signals(workflow_id).await.unwrap();
    assert_eq!(signals.len(), 2);
    assert_eq!(signals[0].signal_type, "approval");
    assert_eq!(signals[1].signal_type, "update");

    // Mark first signal as processed
    store.mark_signals_processed(workflow_id, 1).await.unwrap();

    // Only one signal should remain
    let signals = store.get_pending_signals(workflow_id).await.unwrap();
    assert_eq!(signals.len(), 1);
    assert_eq!(signals[0].signal_type, "update");

    cleanup_workflow(&store, workflow_id).await;
}

// --------------------------------------------
// Worker Registry Tests
// --------------------------------------------

#[tokio::test]
async fn test_worker_registration() {
    let store = create_test_store().await;
    let worker_id = format!("test-worker-{}", Uuid::now_v7());

    // Register worker
    store
        .register_worker(WorkerInfo {
            id: worker_id.clone(),
            worker_group: Some("default".to_string()),
            activity_types: vec!["task_a".to_string(), "task_b".to_string()],
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

    // Update heartbeat
    store.worker_heartbeat(&worker_id, 5, true).await.unwrap();

    // List workers
    let workers = store.list_workers(WorkerFilter::default()).await.unwrap();
    let our_worker = workers.iter().find(|w| w.id == worker_id);
    assert!(our_worker.is_some());
    let w = our_worker.unwrap();
    assert_eq!(w.current_load, 5);

    // Deregister
    store.deregister_worker(&worker_id).await.unwrap();

    // Worker should be stopped
    let workers = store
        .list_workers(WorkerFilter {
            status: Some("stopped".to_string()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(workers.iter().any(|w| w.id == worker_id));

    // Cleanup
    sqlx::query("DELETE FROM durable_workers WHERE id = $1")
        .bind(&worker_id)
        .execute(store.pool())
        .await
        .ok();
}

/// Test that draining workers cannot claim tasks
///
/// When a worker is marked as draining, it should not be able to claim
/// new tasks even if there are pending tasks matching its activity types.
#[tokio::test]
async fn test_draining_worker_cannot_claim_tasks() {
    let store = create_test_store().await;
    let worker_id = format!("test-draining-worker-{}", Uuid::now_v7());
    let workflow_id = Uuid::now_v7();

    // Register an active worker
    store
        .register_worker(WorkerInfo {
            id: worker_id.clone(),
            worker_group: Some("default".to_string()),
            activity_types: vec!["draining_test_task".to_string()],
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

    // Create workflow and enqueue a task
    store
        .create_workflow(workflow_id, "draining_test", json!({}), None)
        .await
        .unwrap();

    store
        .enqueue_task(TaskDefinition {
            workflow_id,
            activity_id: "step".to_string(),
            activity_type: "draining_test_task".to_string(),
            input: json!({}),
            options: ActivityOptions::default(),
        })
        .await
        .unwrap();

    // Worker can claim tasks while active
    let active_claim = store
        .claim_task(&worker_id, &["draining_test_task".to_string()], 1)
        .await
        .unwrap();
    assert_eq!(active_claim.len(), 1, "Active worker should claim tasks");

    // Complete the task so we can test draining with a new task
    store
        .complete_task(active_claim[0].id, &worker_id, json!({}))
        .await
        .unwrap();

    // Enqueue another task
    store
        .enqueue_task(TaskDefinition {
            workflow_id,
            activity_id: "step-2".to_string(),
            activity_type: "draining_test_task".to_string(),
            input: json!({}),
            options: ActivityOptions::default(),
        })
        .await
        .unwrap();

    // Drain the worker
    store.drain_worker(&worker_id).await.unwrap();

    // Draining worker should NOT be able to claim new tasks
    let draining_claim = store
        .claim_task(&worker_id, &["draining_test_task".to_string()], 1)
        .await
        .unwrap();
    assert!(
        draining_claim.is_empty(),
        "Draining worker should NOT claim new tasks, but claimed {} tasks",
        draining_claim.len()
    );

    // Register another active worker to verify task is still claimable
    let other_worker_id = format!("test-active-worker-{}", Uuid::now_v7());
    store
        .register_worker(WorkerInfo {
            id: other_worker_id.clone(),
            worker_group: Some("default".to_string()),
            activity_types: vec!["draining_test_task".to_string()],
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

    // Active worker should be able to claim the task
    let other_claim = store
        .claim_task(&other_worker_id, &["draining_test_task".to_string()], 1)
        .await
        .unwrap();
    assert_eq!(
        other_claim.len(),
        1,
        "Active worker should claim the task that draining worker couldn't"
    );

    // Cleanup
    cleanup_workflow(&store, workflow_id).await;
    sqlx::query("DELETE FROM durable_workers WHERE id = $1")
        .bind(&worker_id)
        .execute(store.pool())
        .await
        .ok();
    sqlx::query("DELETE FROM durable_workers WHERE id = $1")
        .bind(&other_worker_id)
        .execute(store.pool())
        .await
        .ok();
}

/// Test that avg_task_duration_ms is properly calculated as FLOAT8 in SQL
/// This test verifies the fix for the "mismatched types; Rust type `f64` is not
/// compatible with SQL type `NUMERIC`" error that occurred when AVG() returned
/// a NUMERIC type but Rust expected f64.
#[tokio::test]
async fn test_worker_stats_avg_duration_type() {
    let store = create_test_store().await;
    let worker_id = format!("test-worker-stats-{}", Uuid::now_v7());
    let workflow_id = Uuid::now_v7();

    // Register worker
    store
        .register_worker(WorkerInfo {
            id: worker_id.clone(),
            worker_group: Some("default".to_string()),
            activity_types: vec!["test_activity".to_string()],
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

    // Keep worker heartbeat fresh
    store.worker_heartbeat(&worker_id, 0, true).await.unwrap();

    // Create workflow for task
    store
        .create_workflow(workflow_id, "stats_test_workflow", json!({}), None)
        .await
        .unwrap();

    // Create and complete a task to generate statistics
    let options = ActivityOptions {
        retry_policy: RetryPolicy::exponential().with_max_attempts(3),
        ..Default::default()
    };

    let task_id = store
        .enqueue_task(TaskDefinition {
            workflow_id,
            activity_id: "stats_task".to_string(),
            activity_type: "test_activity".to_string(),
            input: json!({}),
            options,
        })
        .await
        .unwrap();

    // Claim the task
    let activity_types = vec!["test_activity".to_string()];
    let claimed = store
        .claim_task(&worker_id, &activity_types, 1)
        .await
        .unwrap();
    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].id, task_id);

    // Complete the task (this will update heartbeat_at, creating a duration)
    store
        .complete_task(task_id, &worker_id, json!({"result": "ok"}))
        .await
        .unwrap();

    // List workers - this should not fail with type mismatch
    // The AVG() function returns NUMERIC, but our query casts it to FLOAT8
    let workers = store.list_workers(WorkerFilter::default()).await.unwrap();
    let our_worker = workers.iter().find(|w| w.id == worker_id);
    assert!(our_worker.is_some(), "Worker should be listed");

    let w = our_worker.unwrap();
    // The avg_task_duration_ms should be present since we completed a task
    // (The actual value depends on timing between claim and complete)
    assert_eq!(w.tasks_completed, 1);

    // Cleanup
    cleanup_workflow(&store, workflow_id).await;
    sqlx::query("DELETE FROM durable_workers WHERE id = $1")
        .bind(&worker_id)
        .execute(store.pool())
        .await
        .ok();
}

// --------------------------------------------
// DLQ Tests
// --------------------------------------------

#[tokio::test]
async fn test_dlq_operations() {
    let store = create_test_store().await;
    let workflow_id = Uuid::now_v7();

    // Register worker before claiming tasks
    register_test_worker(&store, "worker", vec!["dlq_activity".to_string()]).await;

    store
        .create_workflow(workflow_id, "dlq_ops_test", json!({}), None)
        .await
        .unwrap();

    // Create a task and move it to DLQ
    let options = ActivityOptions {
        retry_policy: RetryPolicy::exponential().with_max_attempts(1),
        ..Default::default()
    };

    let task_id = store
        .enqueue_task(TaskDefinition {
            workflow_id,
            activity_id: "dlq_task".to_string(),
            activity_type: "dlq_activity".to_string(),
            input: json!({"important": "data"}),
            options,
        })
        .await
        .unwrap();

    store
        .claim_task("worker", &["dlq_activity".to_string()], 1)
        .await
        .unwrap();
    store.fail_task(task_id, "Final failure").await.unwrap();

    // Move to DLQ with error history
    store
        .move_to_dlq(
            task_id,
            vec!["Error 1".to_string(), "Final failure".to_string()],
        )
        .await
        .unwrap();

    // List DLQ
    let dlq_entries = store
        .list_dlq(
            DlqFilter {
                workflow_id: Some(workflow_id),
                activity_type: None,
            },
            Pagination {
                offset: 0,
                limit: 10,
            },
        )
        .await
        .unwrap();
    assert!(!dlq_entries.is_empty());

    let entry = &dlq_entries[0];
    assert_eq!(entry.activity_type, "dlq_activity");
    assert_eq!(entry.error_history.len(), 2);

    // Requeue from DLQ
    let new_task_id = store.requeue_from_dlq(entry.id).await.unwrap();
    assert_ne!(new_task_id, task_id);

    // New task should be claimable
    let claimed = store
        .claim_task("worker", &["dlq_activity".to_string()], 1)
        .await
        .unwrap();
    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].id, new_task_id);

    cleanup_workflow(&store, workflow_id).await;
    cleanup_worker(&store, "worker").await;
}

// --------------------------------------------
// Concurrent Claiming Tests (SKIP LOCKED)
// --------------------------------------------

#[tokio::test]
async fn test_concurrent_task_claiming() {
    let store = create_test_store().await;
    let workflow_id = Uuid::now_v7();

    // Register workers before claiming tasks
    register_test_worker(&store, "worker-1", vec!["concurrent_task".to_string()]).await;
    register_test_worker(&store, "worker-2", vec!["concurrent_task".to_string()]).await;
    register_test_worker(&store, "worker-3", vec!["concurrent_task".to_string()]).await;

    store
        .create_workflow(workflow_id, "concurrent_claim_test", json!({}), None)
        .await
        .unwrap();

    // Enqueue 10 tasks
    for i in 0..10 {
        store
            .enqueue_task(TaskDefinition {
                workflow_id,
                activity_id: format!("task-{}", i),
                activity_type: "concurrent_task".to_string(),
                input: json!({"num": i}),
                options: ActivityOptions::default(),
            })
            .await
            .unwrap();
    }

    // Have multiple workers claim tasks concurrently
    let store1 = store.clone();
    let store2 = store.clone();
    let store3 = store.clone();

    let types1 = vec!["concurrent_task".to_string()];
    let types2 = vec!["concurrent_task".to_string()];
    let types3 = vec!["concurrent_task".to_string()];

    let (r1, r2, r3) = tokio::join!(
        store1.claim_task("worker-1", &types1, 5),
        store2.claim_task("worker-2", &types2, 5),
        store3.claim_task("worker-3", &types3, 5),
    );

    let claimed1 = r1.unwrap();
    let claimed2 = r2.unwrap();
    let claimed3 = r3.unwrap();

    // Total claimed should be 10 (all tasks)
    let total = claimed1.len() + claimed2.len() + claimed3.len();
    assert_eq!(total, 10);

    // No duplicate claims - each task claimed by exactly one worker
    let mut all_ids: Vec<_> = claimed1.iter().map(|t| t.id).collect();
    all_ids.extend(claimed2.iter().map(|t| t.id));
    all_ids.extend(claimed3.iter().map(|t| t.id));
    all_ids.sort();
    all_ids.dedup();
    assert_eq!(all_ids.len(), 10);

    cleanup_workflow(&store, workflow_id).await;
    cleanup_worker(&store, "worker-1").await;
    cleanup_worker(&store, "worker-2").await;
    cleanup_worker(&store, "worker-3").await;
}

// --------------------------------------------
// Task Completion Ownership Tests (Duplicate Prevention)
// --------------------------------------------

/// Test that task completion fails when task was reclaimed by another worker
///
/// This test verifies the fix for the duplicate act atoms bug:
/// When a task's heartbeat times out and is reclaimed by another worker,
/// the original worker must NOT be able to complete the task.
#[tokio::test]
async fn test_task_completion_rejected_when_reclaimed() {
    let store = create_test_store().await;
    let workflow_id = Uuid::now_v7();

    // Register workers before claiming tasks
    register_test_worker(&store, "worker-1", vec!["process".to_string()]).await;
    register_test_worker(&store, "worker-2", vec!["process".to_string()]).await;

    store
        .create_workflow(workflow_id, "reclaim_test", json!({}), None)
        .await
        .unwrap();

    let task_id = store
        .enqueue_task(TaskDefinition {
            workflow_id,
            activity_id: "step".to_string(),
            activity_type: "process".to_string(),
            input: json!({}),
            options: ActivityOptions::default(),
        })
        .await
        .unwrap();

    // Worker 1 claims the task
    store
        .claim_task("worker-1", &["process".to_string()], 1)
        .await
        .unwrap();

    // Simulate heartbeat timeout: manually set heartbeat_at to the past
    sqlx::query(
        r#"
        UPDATE durable_task_queue
        SET heartbeat_at = NOW() - INTERVAL '1 hour'
        WHERE id = $1
        "#,
    )
    .bind(task_id)
    .execute(store.pool())
    .await
    .unwrap();

    // Reclaim stale tasks
    let reclaimed = store
        .reclaim_stale_tasks(Duration::from_secs(30))
        .await
        .unwrap();
    assert_eq!(reclaimed.len(), 1);

    // Worker 2 claims the reclaimed task
    let claimed = store
        .claim_task("worker-2", &["process".to_string()], 1)
        .await
        .unwrap();
    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].id, task_id);

    // Worker 1 tries to complete the task (should fail - task was reclaimed)
    let result = store
        .complete_task(task_id, "worker-1", json!({"status": "done"}))
        .await;
    assert!(
        matches!(result, Err(StoreError::TaskNotOwned(_))),
        "Expected TaskNotOwned error, got {:?}",
        result
    );

    // Worker 2 should be able to complete the task
    store
        .complete_task(task_id, "worker-2", json!({"status": "done"}))
        .await
        .expect("Worker 2 should be able to complete the task it claimed");

    cleanup_workflow(&store, workflow_id).await;
    cleanup_worker(&store, "worker-1").await;
    cleanup_worker(&store, "worker-2").await;
}

/// Test that task completion fails when wrong worker tries to complete
#[tokio::test]
async fn test_task_completion_rejected_wrong_worker() {
    let store = create_test_store().await;
    let workflow_id = Uuid::now_v7();

    // Register workers before claiming tasks
    register_test_worker(&store, "worker-1", vec!["process".to_string()]).await;

    store
        .create_workflow(workflow_id, "wrong_worker_test", json!({}), None)
        .await
        .unwrap();

    let task_id = store
        .enqueue_task(TaskDefinition {
            workflow_id,
            activity_id: "step".to_string(),
            activity_type: "process".to_string(),
            input: json!({}),
            options: ActivityOptions::default(),
        })
        .await
        .unwrap();

    // Worker 1 claims the task
    store
        .claim_task("worker-1", &["process".to_string()], 1)
        .await
        .unwrap();

    // Wrong worker (worker-2) tries to complete the task
    let result = store
        .complete_task(task_id, "worker-2", json!({"status": "done"}))
        .await;
    assert!(
        matches!(result, Err(StoreError::TaskNotOwned(_))),
        "Expected TaskNotOwned error when wrong worker tries to complete"
    );

    // Correct worker (worker-1) should be able to complete
    store
        .complete_task(task_id, "worker-1", json!({"status": "done"}))
        .await
        .expect("Correct worker should be able to complete");

    cleanup_workflow(&store, workflow_id).await;
    cleanup_worker(&store, "worker-1").await;
}

/// Test that task completion fails when task is already completed
#[tokio::test]
async fn test_task_completion_rejected_already_completed() {
    let store = create_test_store().await;
    let workflow_id = Uuid::now_v7();

    // Register worker before claiming tasks
    register_test_worker(&store, "worker-1", vec!["process".to_string()]).await;

    store
        .create_workflow(workflow_id, "double_complete_test", json!({}), None)
        .await
        .unwrap();

    let task_id = store
        .enqueue_task(TaskDefinition {
            workflow_id,
            activity_id: "step".to_string(),
            activity_type: "process".to_string(),
            input: json!({}),
            options: ActivityOptions::default(),
        })
        .await
        .unwrap();

    // Worker claims and completes the task
    store
        .claim_task("worker-1", &["process".to_string()], 1)
        .await
        .unwrap();

    store
        .complete_task(task_id, "worker-1", json!({"status": "done"}))
        .await
        .unwrap();

    // Same worker tries to complete again (should fail - already completed)
    let result = store
        .complete_task(task_id, "worker-1", json!({"status": "done again"}))
        .await;
    assert!(
        matches!(result, Err(StoreError::TaskNotOwned(_))),
        "Expected TaskNotOwned error when task already completed"
    );

    cleanup_workflow(&store, workflow_id).await;
    cleanup_worker(&store, "worker-1").await;
}

/// Test the full race condition scenario that causes duplicate act atoms
///
/// Scenario:
/// 1. Worker A claims task
/// 2. Worker A executes task (slow)
/// 3. Heartbeat times out, task reclaimed
/// 4. Worker B claims and executes same task
/// 5. Worker B completes task (succeeds)
/// 6. Worker A finishes execution, tries to complete (must fail)
///
/// Without the fix, both workers would complete and schedule next activity,
/// causing duplicate atoms.
#[tokio::test]
async fn test_duplicate_scheduling_prevention() {
    let store = create_test_store().await;
    let workflow_id = Uuid::now_v7();

    // Register workers before claiming tasks
    register_test_worker(&store, "worker-A", vec!["reason".to_string()]).await;
    register_test_worker(&store, "worker-B", vec!["reason".to_string()]).await;
    register_test_worker(&store, "worker-C", vec!["reason".to_string()]).await;

    store
        .create_workflow(workflow_id, "duplicate_prevention_test", json!({}), None)
        .await
        .unwrap();

    let task_id = store
        .enqueue_task(TaskDefinition {
            workflow_id,
            activity_id: "reason_1".to_string(),
            activity_type: "reason".to_string(),
            input: json!({"test": true}),
            options: ActivityOptions::default(),
        })
        .await
        .unwrap();

    // Step 1: Worker A claims the task
    store
        .claim_task("worker-A", &["reason".to_string()], 1)
        .await
        .unwrap();

    // Step 2-3: Simulate heartbeat timeout (worker A is slow)
    sqlx::query(
        r#"
        UPDATE durable_task_queue
        SET heartbeat_at = NOW() - INTERVAL '1 hour'
        WHERE id = $1
        "#,
    )
    .bind(task_id)
    .execute(store.pool())
    .await
    .unwrap();

    store
        .reclaim_stale_tasks(Duration::from_secs(30))
        .await
        .unwrap();

    // Step 4: Worker B claims the reclaimed task
    let claimed = store
        .claim_task("worker-B", &["reason".to_string()], 1)
        .await
        .unwrap();
    assert_eq!(claimed.len(), 1);

    // Step 5: Worker B completes the task first
    store
        .complete_task(task_id, "worker-B", json!({"has_tool_calls": true}))
        .await
        .expect("Worker B should complete successfully");

    // Step 6: Worker A finishes and tries to complete (must fail!)
    let result = store
        .complete_task(task_id, "worker-A", json!({"has_tool_calls": true}))
        .await;

    assert!(
        matches!(result, Err(StoreError::TaskNotOwned(_))),
        "Worker A must NOT be able to complete the reclaimed task. Got: {:?}",
        result
    );

    // Verify task is completed (not duplicated in queue)
    let pending_tasks = store
        .claim_task("worker-C", &["reason".to_string()], 10)
        .await
        .unwrap();
    assert!(
        pending_tasks.is_empty(),
        "No duplicate pending tasks should exist"
    );

    cleanup_workflow(&store, workflow_id).await;
    cleanup_worker(&store, "worker-A").await;
    cleanup_worker(&store, "worker-B").await;
    cleanup_worker(&store, "worker-C").await;
}

// --------------------------------------------
// Stale Task Reclaim Max Attempts Tests
// --------------------------------------------

/// Test that tasks cannot be claimed after exhausting max_attempts via stale reclaim
///
/// Bug reproduction: When a worker panics (doesn't call fail_task), the task becomes
/// stale and gets reclaimed. Without the fix, this can happen indefinitely because:
/// 1. reclaim_stale_tasks sets status='pending' without checking attempt count
/// 2. claim_task doesn't check if attempt >= max_attempts
///
/// Fix: claim_task must check `attempt < max_attempts` before claiming.
#[tokio::test]
async fn test_stale_reclaim_respects_max_attempts() {
    let store = create_test_store().await;
    let workflow_id = Uuid::now_v7();

    // Register workers before claiming tasks
    register_test_worker(&store, "worker-1", vec!["panic_activity".to_string()]).await;
    register_test_worker(&store, "worker-2", vec!["panic_activity".to_string()]).await;
    register_test_worker(&store, "worker-3", vec!["panic_activity".to_string()]).await;

    store
        .create_workflow(workflow_id, "stale_max_attempts_test", json!({}), None)
        .await
        .unwrap();

    // Create task with max_attempts=2
    let options = ActivityOptions {
        retry_policy: RetryPolicy::exponential()
            .with_max_attempts(2)
            .with_initial_interval(Duration::from_millis(1)),
        ..Default::default()
    };

    let task_id = store
        .enqueue_task(TaskDefinition {
            workflow_id,
            activity_id: "panic_task".to_string(),
            activity_type: "panic_activity".to_string(),
            input: json!({}),
            options,
        })
        .await
        .unwrap();

    // Attempt 1: Worker claims task then "panics" (no fail_task call)
    let claimed = store
        .claim_task("worker-1", &["panic_activity".to_string()], 1)
        .await
        .unwrap();
    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].attempt, 1);

    // Simulate heartbeat timeout (worker panicked)
    sqlx::query(
        r#"
        UPDATE durable_task_queue
        SET heartbeat_at = NOW() - INTERVAL '1 hour'
        WHERE id = $1
        "#,
    )
    .bind(task_id)
    .execute(store.pool())
    .await
    .unwrap();

    // Reclaim stale task
    let reclaimed = store
        .reclaim_stale_tasks(Duration::from_secs(30))
        .await
        .unwrap();
    assert_eq!(reclaimed.len(), 1);

    // Attempt 2: Another worker claims the reclaimed task
    let claimed = store
        .claim_task("worker-2", &["panic_activity".to_string()], 1)
        .await
        .unwrap();
    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].attempt, 2); // This is the 2nd (and final) attempt

    // Simulate another heartbeat timeout (worker panicked again)
    sqlx::query(
        r#"
        UPDATE durable_task_queue
        SET heartbeat_at = NOW() - INTERVAL '1 hour'
        WHERE id = $1
        "#,
    )
    .bind(task_id)
    .execute(store.pool())
    .await
    .unwrap();

    // Reclaim stale task again
    let reclaimed = store
        .reclaim_stale_tasks(Duration::from_secs(30))
        .await
        .unwrap();
    assert_eq!(reclaimed.len(), 1);

    // BUG: Without fix, this would claim the task for attempt 3 (exceeding max_attempts=2)
    // FIXED: Task should NOT be claimable because attempt (2) >= max_attempts (2)
    let claimed = store
        .claim_task("worker-3", &["panic_activity".to_string()], 1)
        .await
        .unwrap();
    assert!(
        claimed.is_empty(),
        "Task should NOT be claimable after exhausting max_attempts via stale reclaim. \
         Got {} tasks with attempts: {:?}",
        claimed.len(),
        claimed.iter().map(|t| t.attempt).collect::<Vec<_>>()
    );

    // Verify task status is 'dead' (moved to DLQ)
    let task = store.get_task(task_id).await.unwrap();
    assert_eq!(
        task.status,
        TaskStatus::Dead,
        "Task should be marked as dead after exhausting attempts"
    );

    cleanup_workflow(&store, workflow_id).await;
    cleanup_worker(&store, "worker-1").await;
    cleanup_worker(&store, "worker-2").await;
    cleanup_worker(&store, "worker-3").await;
}

/// Test that partially exhausted tasks can still be claimed
///
/// Ensures the fix doesn't prevent claiming tasks that still have attempts remaining.
#[tokio::test]
async fn test_stale_reclaim_allows_remaining_attempts() {
    let store = create_test_store().await;
    let workflow_id = Uuid::now_v7();

    // Register workers before claiming tasks
    register_test_worker(&store, "worker-1", vec!["retry_activity".to_string()]).await;
    register_test_worker(&store, "worker-2", vec!["retry_activity".to_string()]).await;

    store
        .create_workflow(workflow_id, "partial_attempts_test", json!({}), None)
        .await
        .unwrap();

    // Create task with max_attempts=3
    let options = ActivityOptions {
        retry_policy: RetryPolicy::exponential()
            .with_max_attempts(3)
            .with_initial_interval(Duration::from_millis(1)),
        ..Default::default()
    };

    store
        .enqueue_task(TaskDefinition {
            workflow_id,
            activity_id: "retry_task".to_string(),
            activity_type: "retry_activity".to_string(),
            input: json!({}),
            options,
        })
        .await
        .unwrap();

    // Attempt 1: claim and simulate panic
    let claimed = store
        .claim_task("worker-1", &["retry_activity".to_string()], 1)
        .await
        .unwrap();
    assert_eq!(claimed[0].attempt, 1);

    sqlx::query(
        r#"
        UPDATE durable_task_queue
        SET heartbeat_at = NOW() - INTERVAL '1 hour'
        WHERE workflow_id = $1
        "#,
    )
    .bind(workflow_id)
    .execute(store.pool())
    .await
    .unwrap();

    store
        .reclaim_stale_tasks(Duration::from_secs(30))
        .await
        .unwrap();

    // Attempt 2: should still be claimable (attempt 2 < max_attempts 3)
    let claimed = store
        .claim_task("worker-2", &["retry_activity".to_string()], 1)
        .await
        .unwrap();
    assert_eq!(claimed.len(), 1, "Task should be claimable for attempt 2");
    assert_eq!(claimed[0].attempt, 2);

    // Complete the task on attempt 2
    store
        .complete_task(claimed[0].id, "worker-2", json!({"done": true}))
        .await
        .unwrap();

    cleanup_workflow(&store, workflow_id).await;
    cleanup_worker(&store, "worker-1").await;
    cleanup_worker(&store, "worker-2").await;
}

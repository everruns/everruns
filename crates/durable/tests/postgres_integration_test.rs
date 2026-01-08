//! Integration tests for PostgresWorkflowEventStore
//!
//! Run with: cargo test -p everruns-durable --test postgres_integration_test -- --test-threads=1
//!
//! Requirements:
//! - PostgreSQL running with DATABASE_URL set or postgres://localhost:5432/everruns_test
//! - Migrations applied (run migrations from crates/control-plane/migrations/)

use std::time::Duration;

use chrono::Utc;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use everruns_durable::persistence::{
    DlqFilter, Pagination, PostgresWorkflowEventStore, StoreError, TaskDefinition,
    TaskFailureOutcome, TraceContext, WorkerFilter, WorkerInfo, WorkflowEventStore, WorkflowStatus,
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

// ============================================
// Workflow Lifecycle Tests
// ============================================

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

// ============================================
// Event Sourcing Tests
// ============================================

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

// ============================================
// Task Queue Tests
// ============================================

#[tokio::test]
async fn test_task_enqueue_and_claim() {
    let store = create_test_store().await;
    let workflow_id = Uuid::now_v7();

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
}

#[tokio::test]
async fn test_task_claim_by_activity_type() {
    let store = create_test_store().await;
    let workflow_id = Uuid::now_v7();

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
}

#[tokio::test]
async fn test_task_complete() {
    let store = create_test_store().await;
    let workflow_id = Uuid::now_v7();

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

    // Complete the task
    store
        .complete_task(task_id, json!({"status": "done"}))
        .await
        .unwrap();

    // Task should not be claimable anymore
    let claimed = store
        .claim_task("worker-2", &["process".to_string()], 10)
        .await
        .unwrap();
    assert!(claimed.is_empty());

    cleanup_workflow(&store, workflow_id).await;
}

#[tokio::test]
async fn test_task_failure_with_retry() {
    let store = create_test_store().await;
    let workflow_id = Uuid::now_v7();

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
}

#[tokio::test]
async fn test_task_exhausts_retries_to_dlq() {
    let store = create_test_store().await;
    let workflow_id = Uuid::now_v7();

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
}

#[tokio::test]
async fn test_heartbeat() {
    let store = create_test_store().await;
    let workflow_id = Uuid::now_v7();

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
}

#[tokio::test]
async fn test_reclaim_stale_tasks() {
    let store = create_test_store().await;
    let workflow_id = Uuid::now_v7();

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
}

// ============================================
// Signal Tests
// ============================================

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

// ============================================
// Worker Registry Tests
// ============================================

#[tokio::test]
async fn test_worker_registration() {
    let store = create_test_store().await;
    let worker_id = format!("test-worker-{}", Uuid::now_v7());

    // Register worker
    store
        .register_worker(WorkerInfo {
            id: worker_id.clone(),
            worker_group: "default".to_string(),
            activity_types: vec!["task_a".to_string(), "task_b".to_string()],
            max_concurrency: 10,
            current_load: 0,
            status: "active".to_string(),
            accepting_tasks: true,
            started_at: Utc::now(),
            last_heartbeat_at: Utc::now(),
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

// ============================================
// DLQ Tests
// ============================================

#[tokio::test]
async fn test_dlq_operations() {
    let store = create_test_store().await;
    let workflow_id = Uuid::now_v7();

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
}

// ============================================
// Concurrent Claiming Tests (SKIP LOCKED)
// ============================================

#[tokio::test]
async fn test_concurrent_task_claiming() {
    let store = create_test_store().await;
    let workflow_id = Uuid::now_v7();

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
}

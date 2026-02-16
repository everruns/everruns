//! Failure injection tests using fail-rs
//!
//! These tests verify system behavior under various failure conditions.
//! Run with: cargo test -p everruns-durable --test failure_injection_test --features failpoints -- --test-threads=1
//!
//! Requires PostgreSQL running with DATABASE_URL set.

#![cfg(feature = "failpoints")]

use fail::FailScenario;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use chrono::Utc;
use everruns_durable::persistence::{
    PostgresWorkflowEventStore, StoreError, TaskDefinition, WorkerInfo, WorkflowEventStore,
};
use everruns_durable::workflow::{ActivityOptions, WorkflowEvent};

// ============================================
// Test Helpers
// ============================================

fn get_database_url() -> String {
    std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5432/everruns_test".to_string())
}

async fn create_test_store() -> PostgresWorkflowEventStore {
    let database_url = get_database_url();
    let pool = PgPool::connect(&database_url)
        .await
        .expect("Failed to connect to PostgreSQL. Set DATABASE_URL or ensure postgres is running.");
    PostgresWorkflowEventStore::new(pool)
}

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

async fn cleanup_circuit_breaker(store: &PostgresWorkflowEventStore, key: &str) {
    sqlx::query("DELETE FROM durable_circuit_breaker_state WHERE key = $1")
        .bind(key)
        .execute(store.pool())
        .await
        .ok();
}

async fn register_test_worker(store: &PostgresWorkflowEventStore, worker_id: &str) {
    let now = Utc::now();
    store
        .register_worker(WorkerInfo {
            id: worker_id.to_string(),
            worker_group: Some("default".to_string()),
            activity_types: vec!["failpoint_test".to_string()],
            max_concurrency: 10,
            current_load: 0,
            status: "active".to_string(),
            accepting_tasks: true,
            backpressure_reason: None,
            started_at: now,
            last_heartbeat_at: now,
            hostname: None,
            version: None,
            metadata: None,
            tasks_completed: 0,
            tasks_failed: 0,
            avg_task_duration_ms: None,
        })
        .await
        .unwrap();
}

async fn cleanup_worker(store: &PostgresWorkflowEventStore, worker_id: &str) {
    sqlx::query("DELETE FROM durable_workers WHERE id = $1")
        .bind(worker_id)
        .execute(store.pool())
        .await
        .ok();
}

// ============================================
// Transaction Failure Tests
// ============================================

#[tokio::test]
async fn test_append_events_failure_after_insert_rolls_back() {
    let scenario = FailScenario::setup();
    let store = create_test_store().await;
    let workflow_id = Uuid::now_v7();

    // Create workflow
    store
        .create_workflow(workflow_id, "failure_test", json!({}), None)
        .await
        .unwrap();

    // Enable fail point - fail after insert but before commit
    fail::cfg("postgres_append_events_after_insert", "return").unwrap();

    // Attempt to append events - should fail
    let result = store
        .append_events(
            workflow_id,
            0,
            vec![WorkflowEvent::WorkflowStarted { input: json!({}) }],
        )
        .await;

    assert!(result.is_err());
    assert!(matches!(result, Err(StoreError::Database(_))));

    // Disable fail point
    fail::cfg("postgres_append_events_after_insert", "off").unwrap();

    // Verify no partial writes - events should be empty (rolled back)
    let events = store.load_events(workflow_id).await.unwrap();
    assert!(events.is_empty(), "Expected no events after rollback");

    cleanup_workflow(&store, workflow_id).await;
    scenario.teardown();
}

#[tokio::test]
async fn test_append_events_failure_before_commit_rolls_back() {
    let scenario = FailScenario::setup();
    let store = create_test_store().await;
    let workflow_id = Uuid::now_v7();

    store
        .create_workflow(workflow_id, "commit_failure_test", json!({}), None)
        .await
        .unwrap();

    // Enable fail point - fail just before commit
    fail::cfg("postgres_append_events_before_commit", "return").unwrap();

    let result = store
        .append_events(
            workflow_id,
            0,
            vec![WorkflowEvent::WorkflowStarted { input: json!({}) }],
        )
        .await;

    assert!(result.is_err());

    fail::cfg("postgres_append_events_before_commit", "off").unwrap();

    // Verify rollback
    let events = store.load_events(workflow_id).await.unwrap();
    assert!(events.is_empty());

    cleanup_workflow(&store, workflow_id).await;
    scenario.teardown();
}

#[tokio::test]
async fn test_append_events_single_failure_then_success() {
    let scenario = FailScenario::setup();
    let store = create_test_store().await;
    let workflow_id = Uuid::now_v7();

    store
        .create_workflow(workflow_id, "retry_test", json!({}), None)
        .await
        .unwrap();

    // Fail once, then succeed (1* prefix means one-shot)
    fail::cfg("postgres_append_events_after_insert", "1*return").unwrap();

    // First attempt fails
    let result1 = store
        .append_events(
            workflow_id,
            0,
            vec![WorkflowEvent::WorkflowStarted { input: json!({}) }],
        )
        .await;
    assert!(result1.is_err());

    // Second attempt succeeds (fail point auto-disabled after one failure)
    let result2 = store
        .append_events(
            workflow_id,
            0,
            vec![WorkflowEvent::WorkflowStarted { input: json!({}) }],
        )
        .await;
    assert!(result2.is_ok());
    assert_eq!(result2.unwrap(), 1);

    cleanup_workflow(&store, workflow_id).await;
    scenario.teardown();
}

// ============================================
// Task Claiming Failure Tests
// ============================================

#[tokio::test]
async fn test_claim_task_failure_after_db_commit_preserves_state() {
    // Test: Fail point fires AFTER successful DB claim.
    // Verifies DB state is consistent even when code returns error.
    let scenario = FailScenario::setup();
    let store = create_test_store().await;
    let workflow_id = Uuid::now_v7();

    store
        .create_workflow(workflow_id, "claim_failure_test", json!({}), None)
        .await
        .unwrap();

    // Register workers (required by claim_task's worker_check CTE)
    register_test_worker(&store, "worker-1").await;
    register_test_worker(&store, "worker-2").await;

    // Enqueue task
    let task_id = store
        .enqueue_task(TaskDefinition {
            workflow_id,
            activity_id: "step-1".to_string(),
            activity_type: "failpoint_test".to_string(),
            input: json!({}),
            options: ActivityOptions::default(),
        })
        .await
        .unwrap();

    // Enable fail point (fires AFTER successful DB update)
    fail::cfg("postgres_claim_task_after_query", "return").unwrap();

    // Claim returns error, but DB operation succeeded
    let result = store
        .claim_task("worker-1", &["failpoint_test".to_string()], 10)
        .await;
    assert!(result.is_err());

    // Disable fail point
    fail::cfg("postgres_claim_task_after_query", "off").unwrap();

    // Task was claimed by worker-1 in DB (despite error return)
    // So no pending tasks for worker-2
    let claimed = store
        .claim_task("worker-2", &["failpoint_test".to_string()], 10)
        .await
        .unwrap();
    assert_eq!(claimed.len(), 0, "Task already claimed by worker-1");

    // Worker-1 can complete the task (DB state is consistent)
    let complete_result = store
        .complete_task(task_id, "worker-1", json!({"done": true}))
        .await;
    assert!(complete_result.is_ok());

    cleanup_workflow(&store, workflow_id).await;
    cleanup_worker(&store, "worker-1").await;
    cleanup_worker(&store, "worker-2").await;
    scenario.teardown();
}

// ============================================
// Task Completion Failure Tests
// ============================================

#[tokio::test]
async fn test_complete_task_failure_after_db_commit_is_idempotent() {
    // Test: Fail point fires AFTER successful completion in DB.
    // Verifies task is completed even when code returns error.
    // Retry returns TaskNotOwned (task no longer claimed - it's completed).
    let scenario = FailScenario::setup();
    let store = create_test_store().await;
    let workflow_id = Uuid::now_v7();

    store
        .create_workflow(workflow_id, "complete_failure_test", json!({}), None)
        .await
        .unwrap();

    register_test_worker(&store, "worker-1").await;

    store
        .enqueue_task(TaskDefinition {
            workflow_id,
            activity_id: "step-1".to_string(),
            activity_type: "failpoint_test".to_string(),
            input: json!({}),
            options: ActivityOptions::default(),
        })
        .await
        .unwrap();

    // Claim task
    let claimed = store
        .claim_task("worker-1", &["failpoint_test".to_string()], 10)
        .await
        .unwrap();
    assert_eq!(claimed.len(), 1);
    let task_id = claimed[0].id;

    // Enable fail point (fires AFTER successful DB update)
    fail::cfg("postgres_complete_task_after_update", "return").unwrap();

    // Complete returns error, but DB operation succeeded
    let result = store
        .complete_task(task_id, "worker-1", json!({"done": true}))
        .await;
    assert!(result.is_err());

    // Disable fail point
    fail::cfg("postgres_complete_task_after_update", "off").unwrap();

    // Retry returns TaskNotOwned (task already completed in DB)
    let result = store
        .complete_task(task_id, "worker-1", json!({"done": true}))
        .await;
    assert!(
        matches!(result, Err(StoreError::TaskNotOwned(_))),
        "Task already completed, should return TaskNotOwned"
    );

    cleanup_workflow(&store, workflow_id).await;
    cleanup_worker(&store, "worker-1").await;
    scenario.teardown();
}

// ============================================
// Heartbeat Failure Tests
// ============================================

#[tokio::test]
async fn test_heartbeat_failure_does_not_lose_task() {
    let scenario = FailScenario::setup();
    let store = create_test_store().await;
    let workflow_id = Uuid::now_v7();

    store
        .create_workflow(workflow_id, "heartbeat_failure_test", json!({}), None)
        .await
        .unwrap();

    register_test_worker(&store, "worker-1").await;

    store
        .enqueue_task(TaskDefinition {
            workflow_id,
            activity_id: "step-1".to_string(),
            activity_type: "failpoint_test".to_string(),
            input: json!({}),
            options: ActivityOptions::default(),
        })
        .await
        .unwrap();

    let claimed = store
        .claim_task("worker-1", &["failpoint_test".to_string()], 10)
        .await
        .unwrap();
    let task_id = claimed[0].id;

    // Enable fail point
    fail::cfg("postgres_heartbeat_update", "return").unwrap();

    // Heartbeat fails
    let result = store.heartbeat_task(task_id, "worker-1", None).await;
    assert!(result.is_err());

    // Disable fail point
    fail::cfg("postgres_heartbeat_update", "off").unwrap();

    // Retry heartbeat works
    let result = store.heartbeat_task(task_id, "worker-1", None).await;
    assert!(result.is_ok());
    assert!(result.unwrap().accepted);

    cleanup_workflow(&store, workflow_id).await;
    cleanup_worker(&store, "worker-1").await;
    scenario.teardown();
}

// ============================================
// Circuit Breaker Failure Tests
// ============================================

#[tokio::test]
async fn test_circuit_breaker_state_fetch_failure() {
    use everruns_durable::reliability::{
        CircuitBreakerConfig, CircuitBreakerError, DistributedCircuitBreaker,
    };
    use std::sync::Arc;

    let scenario = FailScenario::setup();
    let store = create_test_store().await;
    let store = Arc::new(store);
    let cb_key = format!("test_service_failpoint_{}", Uuid::now_v7());

    let breaker =
        DistributedCircuitBreaker::new(&cb_key, CircuitBreakerConfig::default(), store.clone());

    // Enable fail point
    fail::cfg("circuit_breaker_get_state_db_fetch", "return").unwrap();

    // State fetch should fail
    let result = breaker.state().await;
    assert!(matches!(result, Err(CircuitBreakerError::Store(_))));

    // Disable fail point
    fail::cfg("circuit_breaker_get_state_db_fetch", "off").unwrap();

    // Should work now
    let result = breaker.state().await;
    assert!(result.is_ok());

    cleanup_circuit_breaker(&store, &cb_key).await;
    scenario.teardown();
}

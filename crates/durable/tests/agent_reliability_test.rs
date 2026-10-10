//! Agent reliability tests
//!
//! Store-level reliability tests: queued work survives infrastructure failures.
//! Tests four failure domains:
//!   1. Worker crashes mid-task (stale reclamation path)
//!   2. Control plane restart (tasks persist in PostgreSQL)
//!   3. Network failure between control plane and worker
//!   4. Network failure between control plane and database
//!
//! Turn-level recovery, which drives these store paths through the Everruns
//! turn driver, is covered in the worker and server suites.
//!
//! Run with:
//!   cargo test -p everruns-durable --test agent_reliability_test \
//!     --features "failpoints,postgres-tests" -- --test-threads=1

#![cfg(all(feature = "failpoints", feature = "postgres-tests"))]

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use fail::FailScenario;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use everruns_durable::persistence::{
    EventLog, PostgresWorkflowEventStore, StoreError, TaskDefinition, TaskQueue, WorkerInfo,
    WorkerRegistry,
};
use everruns_durable::reliability::{CircuitBreakerConfig, DistributedCircuitBreaker};
use everruns_durable::workflow::{ActivityOptions, WorkflowEvent};

// ============================================
// Test Helpers
// ============================================

fn get_database_url() -> String {
    std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        let port = std::env::var("DB_PORT").unwrap_or_else(|_| "9332".to_string());
        format!("postgres://postgres:postgres@localhost:{port}/everruns_test")
    })
}

async fn create_test_store() -> PostgresWorkflowEventStore {
    let database_url = get_database_url();
    let pool = PgPool::connect(&database_url)
        .await
        .expect("Failed to connect to PostgreSQL");
    reset_durable_tables(&pool).await;
    PostgresWorkflowEventStore::new(pool)
}

async fn reset_durable_tables(pool: &PgPool) {
    let mut tx = pool
        .begin()
        .await
        .expect("Failed to begin durable test table reset");

    sqlx::query(
        r#"
        TRUNCATE TABLE
            durable_signals,
            durable_dead_letter_queue,
            durable_task_queue,
            durable_workflow_events,
            durable_workflow_instances,
            durable_workers,
            durable_circuit_breaker_state,
            durable_schedule_executions,
            durable_schedules,
            durable_scheduler_instances
        RESTART IDENTITY CASCADE
        "#,
    )
    .execute(&mut *tx)
    .await
    .expect("Failed to reset durable test tables");

    // Keep this in the same transaction as TRUNCATE so PostgreSQL holds the
    // table locks until commit; otherwise a concurrent test can insert rows after
    // TRUNCATE commits and have its trigger increment overwritten back to zero.
    // TRUNCATE bypasses the row-level triggers that maintain `durable_stat_counters`
    // (migration 082), so the cumulative health counters would otherwise stay
    // non-zero while the tables they track are now empty. That stale drift made
    // `postgres_repository_test`'s counter==COUNT(*) health check flaky when this
    // binary ran first against the shared DB. The triggers UPDATE pre-seeded rows,
    // so the rows must remain — zero the values in place instead of truncating.
    sqlx::query("UPDATE durable_stat_counters SET value = 0")
        .execute(&mut *tx)
        .await
        .expect("Failed to reset durable stat counters");

    tx.commit()
        .await
        .expect("Failed to commit durable test table reset");
}

async fn register_worker(store: &PostgresWorkflowEventStore, worker_id: &str) {
    let now = Utc::now();
    store
        .register_worker(WorkerInfo {
            id: worker_id.to_string(),
            worker_group: Some("default".to_string()),
            activity_types: vec!["pipeline_step".to_string()],
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

/// Make a task's heartbeat stale so it can be reclaimed
async fn make_task_stale(store: &PostgresWorkflowEventStore, workflow_id: Uuid) {
    sqlx::query(
        r#"
        UPDATE durable_task_queue
        SET heartbeat_at = NOW() - INTERVAL '1 hour'
        WHERE workflow_id = $1 AND status = 'claimed'
        "#,
    )
    .bind(workflow_id)
    .execute(store.pool())
    .await
    .unwrap();
}

// ============================================
// Scenario 1: Worker Killed Mid-Task
// ============================================

/// Repeated worker crashes exhaust retry attempts. Task goes to dead state.
#[tokio::test]
async fn test_repeated_worker_crashes_exhaust_retries() {
    let store = create_test_store().await;

    register_worker(&store, "flaky-worker").await;

    let workflow_id = Uuid::now_v7();
    store
        .create_workflow(
            workflow_id,
            "reliability_test_pipeline",
            json!({"total_steps": 1}),
            None,
        )
        .await
        .unwrap();

    // Enqueue with max 2 attempts
    let options = ActivityOptions {
        retry_policy: everruns_durable::reliability::RetryPolicy::exponential()
            .with_max_attempts(2)
            .with_initial_interval(Duration::from_millis(1)),
        ..Default::default()
    };

    store
        .enqueue_task(TaskDefinition {
            workflow_id: Some(workflow_id),
            activity_id: "step-1".to_string(),
            activity_type: "pipeline_step".to_string(),
            input: json!({"step": 1}),
            options,
        })
        .await
        .unwrap();

    // Crash 1: claim, go stale, reclaim
    store
        .claim_task("flaky-worker", &["pipeline_step".to_string()], 1)
        .await
        .unwrap();
    make_task_stale(&store, workflow_id).await;
    let result = store
        .reclaim_stale_tasks(Duration::from_secs(30))
        .await
        .unwrap();
    assert_eq!(result.reclaimed_ids.len(), 1);

    // Crash 2: claim again (attempt 2), go stale again
    tokio::time::sleep(Duration::from_millis(10)).await;
    store
        .claim_task("flaky-worker", &["pipeline_step".to_string()], 1)
        .await
        .unwrap();
    make_task_stale(&store, workflow_id).await;

    // This time reclaim should mark it dead (attempts exhausted)
    let result = store
        .reclaim_stale_tasks(Duration::from_secs(30))
        .await
        .unwrap();
    assert!(result.reclaimed_ids.is_empty(), "no more reclaimable tasks");
    assert_eq!(result.dead_tasks.len(), 1, "task should be marked dead");
}

// ============================================
// Scenario 2: Control Plane Restart
// ============================================

/// Tasks survive control plane restart (they're in PostgreSQL).
#[tokio::test]
async fn test_pending_tasks_survive_restart() {
    let store = create_test_store().await;

    register_worker(&store, "worker-1").await;

    let workflow_id = Uuid::now_v7();
    store
        .create_workflow(
            workflow_id,
            "reliability_test_pipeline",
            json!({"total_steps": 1}),
            None,
        )
        .await
        .unwrap();

    store
        .enqueue_task(TaskDefinition {
            workflow_id: Some(workflow_id),
            activity_id: "step-1".to_string(),
            activity_type: "pipeline_step".to_string(),
            input: json!({"step": 1}),
            options: ActivityOptions::default(),
        })
        .await
        .unwrap();

    // "Restart": create new store connection (same DB)
    let store2 = create_test_store_no_reset().await;
    register_worker(&store2, "worker-2").await;

    // Task should still be claimable
    let claimed = store2
        .claim_task("worker-2", &["pipeline_step".to_string()], 1)
        .await
        .unwrap();
    assert_eq!(claimed.len(), 1, "task should survive restart");
    assert_eq!(claimed[0].activity_id, "step-1");
}

/// Helper: create store without resetting tables (for restart simulation)
async fn create_test_store_no_reset() -> PostgresWorkflowEventStore {
    let database_url = get_database_url();
    let pool = PgPool::connect(&database_url)
        .await
        .expect("Failed to connect to PostgreSQL");
    PostgresWorkflowEventStore::new(pool)
}

// ============================================
// Scenario 3: Network CP ↔ Worker Failure
// ============================================

/// Worker claims task, network dies during completion (complete_task fails).
/// The failpoint fires AFTER the DB update succeeds, so the task is actually
/// completed in the database. A retry returns TaskNotOwned because the row's
/// status is already 'completed' and no longer matches 'claimed'.
#[tokio::test]
async fn test_network_failure_during_task_completion() {
    let scenario = FailScenario::setup();
    let store = create_test_store().await;

    register_worker(&store, "worker-a").await;
    register_worker(&store, "worker-b").await;

    let workflow_id = Uuid::now_v7();
    store
        .create_workflow(workflow_id, "network_test", json!({}), None)
        .await
        .unwrap();
    store
        .enqueue_task(TaskDefinition {
            workflow_id: Some(workflow_id),
            activity_id: "step-1".to_string(),
            activity_type: "pipeline_step".to_string(),
            input: json!({"step": 1}),
            options: ActivityOptions::default(),
        })
        .await
        .unwrap();

    // Worker-A claims task
    let claimed = store
        .claim_task("worker-a", &["pipeline_step".to_string()], 1)
        .await
        .unwrap();
    let task_id = claimed[0].id;

    // Network dies: complete_task fails
    fail::cfg("postgres_complete_task_after_update", "return").unwrap();

    let result = store
        .complete_task(
            task_id,
            "worker-a",
            json!({"step": 1, "output": "result-1"}),
        )
        .await;
    assert!(
        result.is_err(),
        "completion should fail during network outage"
    );

    fail::cfg("postgres_complete_task_after_update", "off").unwrap();

    // Task was actually completed in DB (failpoint fires AFTER update).
    // Retry returns TaskNotOwned.
    let result = store
        .complete_task(
            task_id,
            "worker-a",
            json!({"step": 1, "output": "result-1"}),
        )
        .await;
    assert!(
        matches!(result, Err(StoreError::TaskNotOwned(_))),
        "late completion should return TaskNotOwned"
    );

    scenario.teardown();
}

/// Transient network failure: one claim fails, retry succeeds.
#[tokio::test]
async fn test_transient_network_failure_claim_retry() {
    let scenario = FailScenario::setup();
    let store = create_test_store().await;

    register_worker(&store, "worker-1").await;

    let workflow_id = Uuid::now_v7();
    store
        .create_workflow(workflow_id, "network_test", json!({}), None)
        .await
        .unwrap();

    store
        .enqueue_task(TaskDefinition {
            workflow_id: Some(workflow_id),
            activity_id: "step-1".to_string(),
            activity_type: "pipeline_step".to_string(),
            input: json!({"step": 1}),
            options: ActivityOptions::default(),
        })
        .await
        .unwrap();

    // One-shot failure: first claim fails, second succeeds
    fail::cfg("postgres_claim_task_after_query", "1*return").unwrap();

    let result = store
        .claim_task("worker-1", &["pipeline_step".to_string()], 1)
        .await;
    assert!(result.is_err(), "first claim should fail");

    // Retry succeeds (failpoint auto-disabled after one shot).
    // But the first claim already updated DB, so the task is already claimed.
    // This is the "claim after DB success" scenario - task is claimed in DB.
    let claimed = store
        .claim_task("worker-1", &["pipeline_step".to_string()], 1)
        .await
        .unwrap();
    // Task was claimed by the first call (DB succeeded), so no pending tasks
    assert_eq!(
        claimed.len(),
        0,
        "task already claimed by first call (DB succeeded despite error return)"
    );

    scenario.teardown();
}

/// Extended network outage: heartbeat fails, task becomes stale, reclaimed.
#[tokio::test]
async fn test_extended_network_outage_heartbeat_fails() {
    let scenario = FailScenario::setup();
    let store = create_test_store().await;

    register_worker(&store, "worker-a").await;
    register_worker(&store, "worker-b").await;

    let workflow_id = Uuid::now_v7();
    store
        .create_workflow(workflow_id, "heartbeat_outage_test", json!({}), None)
        .await
        .unwrap();

    let task_id = store
        .enqueue_task(TaskDefinition {
            workflow_id: Some(workflow_id),
            activity_id: "step-1".to_string(),
            activity_type: "pipeline_step".to_string(),
            input: json!({"step": 1}),
            options: ActivityOptions::default(),
        })
        .await
        .unwrap();

    // Worker-A claims task
    store
        .claim_task("worker-a", &["pipeline_step".to_string()], 1)
        .await
        .unwrap();

    // Network dies: heartbeat fails repeatedly
    fail::cfg("postgres_heartbeat_update", "return").unwrap();

    let result = store.heartbeat_task(task_id, "worker-a", None).await;
    assert!(result.is_err());

    fail::cfg("postgres_heartbeat_update", "off").unwrap();

    // Simulate time passing: heartbeat goes stale
    make_task_stale(&store, workflow_id).await;

    // Task reclaimed
    let reclaim = store
        .reclaim_stale_tasks(Duration::from_secs(30))
        .await
        .unwrap();
    assert_eq!(reclaim.reclaimed_ids.len(), 1);

    // Worker-B picks it up
    let claimed = store
        .claim_task("worker-b", &["pipeline_step".to_string()], 1)
        .await
        .unwrap();
    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].attempt, 2, "should be second attempt");

    // Worker-A tries late completion → TaskNotOwned
    let result = store
        .complete_task(task_id, "worker-a", json!({"late": true}))
        .await;
    assert!(
        matches!(result, Err(StoreError::TaskNotOwned(_))),
        "late completion from worker-a should fail"
    );

    scenario.teardown();
}

// ============================================
// Scenario 4: Network CP ↔ DB Failure
// ============================================

/// DB failure during event append rolls back cleanly. No partial writes.
#[tokio::test]
async fn test_db_failure_during_event_append_rolls_back() {
    let scenario = FailScenario::setup();
    let store = create_test_store().await;

    let workflow_id = Uuid::now_v7();
    store
        .create_workflow(workflow_id, "db_failure_test", json!({}), None)
        .await
        .unwrap();

    // Fail both phases to ensure no partial writes
    fail::cfg("postgres_append_events_after_insert", "return").unwrap();

    let result = store
        .append_events(workflow_id, 0, vec![WorkflowEvent::started(json!({}))])
        .await;
    assert!(result.is_err());

    fail::cfg("postgres_append_events_after_insert", "off").unwrap();

    // Verify zero events (clean rollback)
    let events = store.load_events(workflow_id).await.unwrap();
    assert!(events.is_empty(), "no partial writes after rollback");

    // Recovery: same operation now succeeds
    let seq = store
        .append_events(workflow_id, 0, vec![WorkflowEvent::started(json!({}))])
        .await
        .unwrap();
    assert_eq!(seq, 1);

    scenario.teardown();
}

/// DB failure during event loading doesn't corrupt cached state.
#[tokio::test]
async fn test_db_failure_during_event_load_recovers() {
    let scenario = FailScenario::setup();
    let store = create_test_store().await;

    let workflow_id = Uuid::now_v7();
    store
        .create_workflow(workflow_id, "load_failure_test", json!({}), None)
        .await
        .unwrap();

    // Write some events successfully
    store
        .append_events(workflow_id, 0, vec![WorkflowEvent::started(json!({}))])
        .await
        .unwrap();

    // DB fails during load
    fail::cfg("postgres_load_events_after_query", "return").unwrap();

    let result = store.load_events(workflow_id).await;
    assert!(result.is_err(), "load should fail during DB outage");

    fail::cfg("postgres_load_events_after_query", "off").unwrap();

    // Recovery: load succeeds, data intact
    let events = store.load_events(workflow_id).await.unwrap();
    assert_eq!(events.len(), 1, "events should be intact after recovery");
    assert!(matches!(events[0].1, WorkflowEvent::WorkflowStarted { .. }));

    scenario.teardown();
}

/// Brief DB blip during task enqueue. Failpoint fires after INSERT succeeds,
/// so the task is persisted but the caller sees an error. A naive retry
/// creates a ghost duplicate. This test demonstrates the ghost task hazard
/// and verifies both tasks are visible in the queue.
#[tokio::test]
async fn test_db_blip_during_enqueue_creates_ghost_task() {
    let scenario = FailScenario::setup();
    let store = create_test_store().await;

    register_worker(&store, "worker-1").await;

    let workflow_id = Uuid::now_v7();
    store
        .create_workflow(workflow_id, "enqueue_blip_test", json!({}), None)
        .await
        .unwrap();

    // Fail after INSERT succeeds
    fail::cfg("postgres_enqueue_task_after_insert", "1*return").unwrap();

    let result = store
        .enqueue_task(TaskDefinition {
            workflow_id: Some(workflow_id),
            activity_id: "step-1".to_string(),
            activity_type: "pipeline_step".to_string(),
            input: json!({"step": 1}),
            options: ActivityOptions::default(),
        })
        .await;
    assert!(result.is_err(), "enqueue should report failure");

    // The task was actually inserted (failpoint fires after INSERT).
    // A second enqueue creates a duplicate task (different UUID).
    let result2 = store
        .enqueue_task(TaskDefinition {
            workflow_id: Some(workflow_id),
            activity_id: "step-1-retry".to_string(),
            activity_type: "pipeline_step".to_string(),
            input: json!({"step": 1}),
            options: ActivityOptions::default(),
        })
        .await;
    assert!(result2.is_ok(), "retry enqueue should succeed");

    // Both tasks are claimable (the ghost task from first call + retry)
    let claimed = store
        .claim_task("worker-1", &["pipeline_step".to_string()], 10)
        .await
        .unwrap();
    assert_eq!(claimed.len(), 2, "ghost task + retry task both exist");

    scenario.teardown();
}

/// DB failure during stale task reclamation. Tasks are actually reclaimed
/// in DB (failpoint fires after UPDATE). Verify consistency.
#[tokio::test]
async fn test_db_failure_during_reclamation() {
    let scenario = FailScenario::setup();
    let store = create_test_store().await;

    register_worker(&store, "worker-a").await;
    register_worker(&store, "worker-b").await;

    let workflow_id = Uuid::now_v7();
    store
        .create_workflow(workflow_id, "reclaim_failure_test", json!({}), None)
        .await
        .unwrap();

    store
        .enqueue_task(TaskDefinition {
            workflow_id: Some(workflow_id),
            activity_id: "step-1".to_string(),
            activity_type: "pipeline_step".to_string(),
            input: json!({"step": 1}),
            options: ActivityOptions::default(),
        })
        .await
        .unwrap();

    // Worker claims, then goes stale
    store
        .claim_task("worker-a", &["pipeline_step".to_string()], 1)
        .await
        .unwrap();
    make_task_stale(&store, workflow_id).await;

    // Reclaim fails (but DB update already happened)
    fail::cfg("postgres_reclaim_stale_after_update", "return").unwrap();

    let result = store.reclaim_stale_tasks(Duration::from_secs(30)).await;
    assert!(result.is_err(), "reclaim should report failure");

    fail::cfg("postgres_reclaim_stale_after_update", "off").unwrap();

    // Despite error, task was actually reclaimed in DB.
    // Another worker can pick it up.
    let claimed = store
        .claim_task("worker-b", &["pipeline_step".to_string()], 1)
        .await
        .unwrap();
    assert_eq!(claimed.len(), 1, "reclaimed task should be claimable");

    scenario.teardown();
}

/// Circuit breaker opens under sustained failures, protecting the system
/// from cascading failures. No failpoints needed — the breaker's own
/// failure recording API drives state transitions.
#[tokio::test]
async fn test_circuit_breaker_opens_under_sustained_failures() {
    let store = create_test_store().await;
    let store = Arc::new(store);
    let cb_key = format!("test_reliability_{}", Uuid::now_v7());

    let config = CircuitBreakerConfig {
        failure_threshold: 3,
        success_threshold: 1,
        reset_timeout: Duration::from_secs(60),
        ..Default::default()
    };
    let breaker = DistributedCircuitBreaker::new(&cb_key, config, store.clone());

    // Record failures to open the circuit
    for _ in 0..3 {
        let permit = breaker.allow().await.unwrap();
        permit.failure().await.unwrap();
    }

    // Circuit should be open: requests rejected immediately
    let result = breaker.allow().await;
    assert!(result.is_err(), "circuit should be open after 3 failures");

    // Cleanup
    sqlx::query("DELETE FROM durable_circuit_breaker_state WHERE key = $1")
        .bind(&cb_key)
        .execute(store.pool())
        .await
        .ok();
}

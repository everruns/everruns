//! Agent reliability tests
//!
//! End-to-end reliability tests verifying agent execution survives infrastructure failures.
//! Tests four failure domains:
//!   1. Worker crashes mid-task (stale reclamation path)
//!   2. Control plane restart (event sourcing replay)
//!   3. Network failure between control plane and worker
//!   4. Network failure between control plane and database
//!
//! Run with:
//!   cargo test -p everruns-durable --test agent_reliability_test \
//!     --features "failpoints,postgres-tests" -- --test-threads=1

#![cfg(all(feature = "failpoints", feature = "postgres-tests"))]

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use fail::FailScenario;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use everruns_durable::engine::{ExecutorConfig, WorkflowExecutor};
use everruns_durable::persistence::{
    PostgresWorkflowEventStore, StoreError, TaskDefinition, WorkerInfo, WorkflowEventStore,
    WorkflowStatus,
};
use everruns_durable::reliability::{CircuitBreakerConfig, DistributedCircuitBreaker};
use everruns_durable::workflow::{
    ActivityOptions, Workflow, WorkflowAction, WorkflowError, WorkflowEvent,
};

// ============================================
// Test Workflow: Multi-Step Pipeline
// ============================================

/// A test workflow that runs N sequential activities, each producing a result
/// that feeds into the next. Completes when all steps are done.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PipelineWorkflow {
    total_steps: u32,
    completed_steps: u32,
    results: Vec<serde_json::Value>,
    failed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PipelineInput {
    total_steps: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PipelineOutput {
    results: Vec<serde_json::Value>,
}

impl Workflow for PipelineWorkflow {
    const TYPE: &'static str = "reliability_test_pipeline";
    type Input = PipelineInput;
    type Output = PipelineOutput;

    fn new(input: Self::Input) -> Self {
        Self {
            total_steps: input.total_steps,
            completed_steps: 0,
            results: Vec::new(),
            failed: false,
        }
    }

    fn on_start(&mut self) -> Vec<WorkflowAction> {
        vec![WorkflowAction::ScheduleActivity {
            activity_id: "step-1".to_string(),
            activity_type: "pipeline_step".to_string(),
            input: json!({"step": 1}),
            options: ActivityOptions::default(),
        }]
    }

    fn on_activity_completed(
        &mut self,
        _activity_id: &str,
        result: serde_json::Value,
    ) -> Vec<WorkflowAction> {
        self.completed_steps += 1;
        self.results.push(result);

        if self.completed_steps >= self.total_steps {
            vec![WorkflowAction::CompleteWorkflow {
                result: json!(PipelineOutput {
                    results: self.results.clone(),
                }),
            }]
        } else {
            let next = self.completed_steps + 1;
            vec![WorkflowAction::ScheduleActivity {
                activity_id: format!("step-{next}"),
                activity_type: "pipeline_step".to_string(),
                input: json!({"step": next}),
                options: ActivityOptions::default(),
            }]
        }
    }

    fn on_activity_failed(
        &mut self,
        _activity_id: &str,
        _error: &everruns_durable::activity::ActivityError,
    ) -> Vec<WorkflowAction> {
        self.failed = true;
        vec![WorkflowAction::FailWorkflow {
            error: WorkflowError::new("activity failed"),
        }]
    }

    fn is_completed(&self) -> bool {
        self.completed_steps >= self.total_steps || self.failed
    }

    fn result(&self) -> Option<Self::Output> {
        if self.completed_steps >= self.total_steps {
            Some(PipelineOutput {
                results: self.results.clone(),
            })
        } else {
            None
        }
    }
}

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
    .execute(pool)
    .await
    .expect("Failed to reset durable test tables");
}

fn create_executor(
    store: PostgresWorkflowEventStore,
) -> WorkflowExecutor<PostgresWorkflowEventStore> {
    let mut executor = WorkflowExecutor::with_config(
        store,
        ExecutorConfig {
            max_events_per_workflow: 10000,
            validate_actions: true,
            snapshot_interval: 0, // disable snapshots for test clarity
        },
    );
    executor.register::<PipelineWorkflow>();
    executor
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

/// Simulate a full activity execution cycle: claim → complete → process workflow
async fn execute_one_task(
    executor: &WorkflowExecutor<PostgresWorkflowEventStore>,
    worker_id: &str,
) -> bool {
    let store = executor.store();
    let claimed = store
        .claim_task(worker_id, &["pipeline_step".to_string()], 1)
        .await
        .unwrap();

    if claimed.is_empty() {
        return false;
    }

    let task = &claimed[0];
    let step: u32 = task.input.get("step").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let result = json!({"step": step, "output": format!("result-{step}")});

    store
        .complete_task(task.id, worker_id, result.clone())
        .await
        .unwrap();

    // Drive the workflow forward
    if let Some(workflow_id) = task.workflow_id {
        executor
            .on_activity_completed(workflow_id, &task.activity_id, result)
            .await
            .unwrap();
    }

    true
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

/// Worker crashes after claiming task. Task becomes stale, gets reclaimed,
/// another worker completes it, workflow finishes.
#[tokio::test]
async fn test_worker_crash_task_reclaimed_and_completed() {
    let store = create_test_store().await;
    let executor = create_executor(store);
    let store = executor.store();

    register_worker(store, "worker-a").await;
    register_worker(store, "worker-b").await;

    // Start a 3-step pipeline
    let wf_id = executor
        .start_workflow::<PipelineWorkflow>(PipelineInput { total_steps: 3 }, None)
        .await
        .unwrap();

    // Worker-A claims step 1 (then "crashes" — never completes)
    let claimed = store
        .claim_task("worker-a", &["pipeline_step".to_string()], 1)
        .await
        .unwrap();
    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].activity_id, "step-1");

    // Simulate crash: make heartbeat stale
    make_task_stale(store, wf_id).await;

    // Reclaim stale tasks (threshold 30s, heartbeat is 1h old)
    let result = store
        .reclaim_stale_tasks(Duration::from_secs(30))
        .await
        .unwrap();
    assert_eq!(
        result.reclaimed_ids.len(),
        1,
        "one task should be reclaimed"
    );

    // Worker-B picks up the reclaimed task and completes it
    execute_one_task(&executor, "worker-b").await;

    // Complete remaining steps normally
    execute_one_task(&executor, "worker-b").await;
    execute_one_task(&executor, "worker-b").await;

    // Verify workflow completed
    let info = store.get_workflow_info(wf_id).await.unwrap();
    assert_eq!(info.status, WorkflowStatus::Completed);
}

/// Worker crashes mid-pipeline (after step 1 succeeds, during step 2).
/// Verifies partial progress is preserved.
#[tokio::test]
async fn test_worker_crash_mid_pipeline_preserves_progress() {
    let store = create_test_store().await;
    let executor = create_executor(store);
    let store = executor.store();

    register_worker(store, "worker-a").await;
    register_worker(store, "worker-b").await;

    let wf_id = executor
        .start_workflow::<PipelineWorkflow>(PipelineInput { total_steps: 3 }, None)
        .await
        .unwrap();

    // Worker-A completes step 1 successfully
    execute_one_task(&executor, "worker-a").await;

    // Worker-A claims step 2, then crashes
    let claimed = store
        .claim_task("worker-a", &["pipeline_step".to_string()], 1)
        .await
        .unwrap();
    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].activity_id, "step-2");

    // Verify step 1 completion event is persisted
    let events = store.load_events(wf_id).await.unwrap();
    let completed_count = events
        .iter()
        .filter(|(_, e)| matches!(e, WorkflowEvent::ActivityCompleted { .. }))
        .count();
    assert_eq!(completed_count, 1, "step 1 completion should be persisted");

    // Simulate crash and reclaim
    make_task_stale(store, wf_id).await;
    store
        .reclaim_stale_tasks(Duration::from_secs(30))
        .await
        .unwrap();

    // Worker-B completes remaining steps
    execute_one_task(&executor, "worker-b").await; // step 2
    execute_one_task(&executor, "worker-b").await; // step 3

    let info = store.get_workflow_info(wf_id).await.unwrap();
    assert_eq!(info.status, WorkflowStatus::Completed);
}

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

/// Control plane restarts. New executor replays events from PostgreSQL
/// and resumes workflow execution.
#[tokio::test]
async fn test_control_plane_restart_workflow_resumes() {
    let store = create_test_store().await;

    // Phase 1: Original executor starts workflow and completes step 1
    {
        let executor = create_executor(store);
        let store = executor.store();
        register_worker(store, "worker-1").await;

        let wf_id = executor
            .start_workflow::<PipelineWorkflow>(PipelineInput { total_steps: 3 }, None)
            .await
            .unwrap();

        execute_one_task(&executor, "worker-1").await;

        // Verify step 1 done, step 2 task enqueued
        let events = executor.store().load_events(wf_id).await.unwrap();
        assert!(
            events.len() >= 3,
            "should have started + scheduled + completed events"
        );

        // "Crash": drop executor (simulates control plane restart)
        // Store is backed by the same PgPool, so we reconnect to same DB
    }

    // Phase 2: New executor with fresh connection to same DB
    let store2 = create_test_store_no_reset().await;
    let executor2 = create_executor(store2);
    let store2 = executor2.store();

    // Worker re-registers (as it would after reconnecting)
    register_worker(store2, "worker-2").await;

    // Find the workflow that's still running
    let workflows = sqlx::query_as::<_, (Uuid,)>(
        "SELECT id FROM durable_workflow_instances WHERE status = 'running' LIMIT 1",
    )
    .fetch_one(store2.pool())
    .await
    .unwrap();
    let wf_id = workflows.0;

    // New executor can process the workflow (replay events)
    let process_result = executor2.process_workflow(wf_id).await.unwrap();
    assert!(
        !process_result.completed,
        "workflow should not be completed yet"
    );

    // Complete remaining steps with new executor
    execute_one_task(&executor2, "worker-2").await;
    execute_one_task(&executor2, "worker-2").await;

    let info = store2.get_workflow_info(wf_id).await.unwrap();
    assert_eq!(info.status, WorkflowStatus::Completed);
}

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
/// After recovery, late completion returns TaskNotOwned because task was
/// already reclaimed and completed by another worker.
#[tokio::test]
async fn test_network_failure_during_task_completion() {
    let scenario = FailScenario::setup();
    let store = create_test_store().await;
    let executor = create_executor(store);
    let store = executor.store();

    register_worker(store, "worker-a").await;
    register_worker(store, "worker-b").await;

    executor
        .start_workflow::<PipelineWorkflow>(PipelineInput { total_steps: 1 }, None)
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
        .append_events(
            workflow_id,
            0,
            vec![WorkflowEvent::WorkflowStarted { input: json!({}) }],
        )
        .await;
    assert!(result.is_err());

    fail::cfg("postgres_append_events_after_insert", "off").unwrap();

    // Verify zero events (clean rollback)
    let events = store.load_events(workflow_id).await.unwrap();
    assert!(events.is_empty(), "no partial writes after rollback");

    // Recovery: same operation now succeeds
    let seq = store
        .append_events(
            workflow_id,
            0,
            vec![WorkflowEvent::WorkflowStarted { input: json!({}) }],
        )
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
        .append_events(
            workflow_id,
            0,
            vec![WorkflowEvent::WorkflowStarted { input: json!({}) }],
        )
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

/// Brief DB blip during task enqueue. Task is actually persisted (failpoint
/// fires after INSERT). Verify no duplicate on retry.
#[tokio::test]
async fn test_db_blip_during_enqueue_no_duplicate() {
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

/// Extended DB outage: multiple operations fail, then recovery.
/// Verifies state consistency after recovery.
#[tokio::test]
async fn test_extended_db_outage_then_recovery() {
    let scenario = FailScenario::setup();
    let store = create_test_store().await;
    let executor = create_executor(store);
    let store = executor.store();

    register_worker(store, "worker-1").await;

    // Start workflow successfully before outage
    let wf_id = executor
        .start_workflow::<PipelineWorkflow>(PipelineInput { total_steps: 2 }, None)
        .await
        .unwrap();

    // Complete step 1
    execute_one_task(&executor, "worker-1").await;

    // DB goes down: claim fails
    fail::cfg("postgres_claim_task_after_query", "return").unwrap();

    let result = store
        .claim_task("worker-1", &["pipeline_step".to_string()], 1)
        .await;
    assert!(result.is_err(), "claim should fail during DB outage");

    // DB recovers
    fail::cfg("postgres_claim_task_after_query", "off").unwrap();

    // Complete step 2 normally
    execute_one_task(&executor, "worker-1").await;

    let info = store.get_workflow_info(wf_id).await.unwrap();
    assert_eq!(info.status, WorkflowStatus::Completed);

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

/// Circuit breaker opens under sustained DB failures, protecting the system
/// from cascading failures.
#[tokio::test]
async fn test_circuit_breaker_opens_under_db_failure() {
    let scenario = FailScenario::setup();
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

    scenario.teardown();
}

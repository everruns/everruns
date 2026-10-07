//! PostgreSQL-only run-start race: a start that waits on the workflow lock
//! behind a hand-off completing the run.
//!
//! Run with: cargo test -p everruns-durable --test postgres_run_start_race_test --features postgres-tests
//!
//! Uses its own workflow ids and leaves other rows alone, so it can share a
//! database with other tests.

#![cfg(feature = "postgres-tests")]

use std::time::Duration;

use chrono::Utc;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use everruns_durable::persistence::{
    EventLog, PostgresWorkflowEventStore, RunStart, RunSteering, SignalStore, TaskDefinition,
    TaskQueue, WorkerInfo, WorkerRegistry,
};
use everruns_durable::workflow::ActivityOptions;

fn database_url() -> String {
    std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        let port = std::env::var("DB_PORT").unwrap_or_else(|_| "9332".to_string());
        format!("postgres://postgres:postgres@localhost:{port}/everruns_test")
    })
}

fn task(activity_type: &str) -> TaskDefinition {
    TaskDefinition {
        workflow_id: None,
        activity_id: format!("{activity_type}-{}", Uuid::now_v7()),
        activity_type: activity_type.to_string(),
        input: json!({}),
        options: ActivityOptions::default(),
    }
}

fn worker(id: &str, activity_type: &str) -> WorkerInfo {
    WorkerInfo {
        id: id.to_string(),
        worker_group: Some("default".to_string()),
        activity_types: vec![activity_type.to_string()],
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
    }
}

/// The load-test race: a run start that queued on the workflow lock while
/// the run's last step was handing off must see the run as ended once the
/// hand-off commits, and start a new run. It saw the step's task as still
/// claimed (from the snapshot its statement took before waiting), reported
/// the run active, and steered a completed run nothing would ever read.
#[tokio::test]
async fn a_start_queued_behind_the_completing_hand_off_starts_a_new_run() {
    let pool = PgPool::connect(&database_url())
        .await
        .expect("Failed to connect to PostgreSQL. Set DATABASE_URL.");
    let store = PostgresWorkflowEventStore::new(pool.clone());
    let activity_type = format!("race_{}", Uuid::now_v7().simple());
    let worker_id = format!("race-worker-{}", Uuid::now_v7().simple());
    store
        .register_worker(worker(&worker_id, &activity_type))
        .await
        .unwrap();

    let workflow_id = Uuid::now_v7();
    let started = store
        .start_run_with_task(workflow_id, "race", json!({}), task(&activity_type), None)
        .await
        .unwrap();
    assert!(matches!(started, RunStart::Started { created: true, .. }));
    let claimed = store
        .claim_task(&worker_id, std::slice::from_ref(&activity_type), 1)
        .await
        .unwrap();
    assert_eq!(claimed.len(), 1);

    // The last step's hand-off: lock, complete the task and the workflow,
    // and hold the commit while a run start queues on the lock.
    let mut hand_off = pool.begin().await.unwrap();
    sqlx::query("SELECT 1 FROM durable_workflow_instances WHERE id = $1 FOR UPDATE")
        .bind(workflow_id)
        .execute(&mut *hand_off)
        .await
        .unwrap();
    sqlx::query("UPDATE durable_task_queue SET status = 'completed' WHERE id = $1")
        .bind(claimed[0].id)
        .execute(&mut *hand_off)
        .await
        .unwrap();
    sqlx::query(
        "UPDATE durable_workflow_instances SET status = 'completed', completed_at = NOW() WHERE id = $1",
    )
    .bind(workflow_id)
    .execute(&mut *hand_off)
    .await
    .unwrap();

    let start = tokio::spawn({
        let store = store.clone();
        let next = task(&activity_type);
        async move {
            let steering = RunSteering {
                signal_type: "steer".into(),
                payload: Some(json!({ "message": 2 })),
            };
            store
                .start_run_with_task(workflow_id, "race", json!({}), next, Some(steering))
                .await
        }
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        !start.is_finished(),
        "the start waits on the hand-off's lock"
    );
    hand_off.commit().await.unwrap();

    let started = start.await.unwrap().unwrap();
    assert!(
        matches!(started, RunStart::Started { created: false, .. }),
        "the run ended, so the start begins a new one: {started:?}"
    );
    assert!(
        store
            .get_pending_signals(workflow_id)
            .await
            .unwrap()
            .is_empty(),
        "nothing steers the ended run"
    );
    store.deregister_worker(&worker_id).await.unwrap();
}

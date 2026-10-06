// Poll-loop wake-up tests: a finished task cuts the poll backoff short, so the
// next turn phase is claimed at once instead of after `poll_backoff_max`.

use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::durable::{ActivityOptions, InMemoryWorkflowEventStore, TaskDefinition, TaskQueue};
use uuid::Uuid;

use crate::unified_worker::{TaskWorker, TaskWorkerConfig};
use crate::unified_worker_test_adapters::NoopAdapters;

/// Far longer than the claim should take once woken, so a pass cannot be a
/// lucky backoff tick.
const BACKOFF: Duration = Duration::from_secs(5);

async fn enqueue(store: &InMemoryWorkflowEventStore) -> Uuid {
    TaskQueue::enqueue_task(
        store,
        TaskDefinition {
            workflow_id: None,
            activity_id: Uuid::now_v7().to_string(),
            // Unparseable input fails the task at once; the test only needs
            // the task to finish, not to succeed.
            activity_type: "process_input".to_string(),
            input: serde_json::json!({}),
            options: ActivityOptions::default(),
        },
    )
    .await
    .expect("enqueue")
}

async fn wait_claimed(store: &InMemoryWorkflowEventStore, task_id: Uuid, within: Duration) -> bool {
    let deadline = Instant::now() + within;
    while Instant::now() < deadline {
        let task = TaskQueue::get_task(store, task_id).await.expect("get task");
        // A failed attempt goes back to pending with `claimed_at` cleared, so
        // the attempt count is what records that a worker took it.
        if task.attempt > 0 || task.claimed_at.is_some() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    false
}

fn single_slot_config() -> TaskWorkerConfig {
    TaskWorkerConfig {
        // One slot: the second task can only be claimed after the first
        // finishes, which is exactly the reason -> act -> reason hand-off.
        max_concurrent_tasks: 1,
        claim_batch_size: 1,
        poll_interval: BACKOFF,
        poll_backoff_max: BACKOFF,
        heartbeat_interval: Duration::from_secs(60),
        ..Default::default()
    }
}

#[tokio::test]
async fn finished_task_wakes_the_poll_loop_for_the_next_one() {
    let store = Arc::new(InMemoryWorkflowEventStore::new());
    let first = enqueue(&store).await;
    let second = enqueue(&store).await;

    let mut worker = TaskWorker::new(single_slot_config(), store.clone(), NoopAdapters);
    let shutdown = worker.shutdown_handle();
    let run = tokio::spawn(async move { worker.run().await });

    assert!(
        wait_claimed(&store, first, Duration::from_secs(2)).await,
        "first task should be claimed by the initial poll"
    );
    assert!(
        wait_claimed(&store, second, Duration::from_secs(1)).await,
        "second task waited for the {BACKOFF:?} poll backoff instead of the completion wake-up"
    );

    shutdown.shutdown();
    run.await.expect("join").expect("worker run");
}

/// Guards the test above: without a wake-up, an idle worker really does sit
/// out the backoff, so the fast claim there is the wake-up's doing.
#[tokio::test]
async fn idle_worker_waits_for_backoff_without_a_wakeup() {
    let store = Arc::new(InMemoryWorkflowEventStore::new());
    let mut worker = TaskWorker::new(single_slot_config(), store.clone(), NoopAdapters);
    let shutdown = worker.shutdown_handle();
    let run = tokio::spawn(async move { worker.run().await });

    // Let the first (empty) poll happen so the loop is asleep in its backoff.
    tokio::time::sleep(Duration::from_millis(200)).await;
    let task = enqueue(&store).await;
    assert!(
        !wait_claimed(&store, task, Duration::from_millis(500)).await,
        "an idle worker with no push channel should only claim on its next poll"
    );

    shutdown.shutdown();
    run.await.expect("join").expect("worker run");
}

/// An operator drain reaches the worker through its heartbeat: it stops
/// claiming, and a resume wakes it to claim at once rather than after its
/// poll backoff.
#[tokio::test]
async fn drained_worker_claims_again_as_soon_as_it_is_resumed() {
    use crate::durable::WorkerRegistry;

    let store = Arc::new(InMemoryWorkflowEventStore::new());
    let config = TaskWorkerConfig {
        heartbeat_interval: Duration::from_millis(20),
        ..single_slot_config()
    };
    let worker_id = config.worker_id.clone();
    let mut worker = TaskWorker::new(config, store.clone(), NoopAdapters);
    let shutdown = worker.shutdown_handle();
    let run = tokio::spawn(async move { worker.run().await });

    // Let the worker register and settle into its (long) poll backoff.
    tokio::time::sleep(Duration::from_millis(100)).await;
    WorkerRegistry::drain_worker(&*store, &worker_id)
        .await
        .expect("drain");
    let task = enqueue(&store).await;
    assert!(
        !wait_claimed(&store, task, Duration::from_millis(300)).await,
        "a drained worker must not claim"
    );

    WorkerRegistry::resume_worker(&*store, &worker_id)
        .await
        .expect("resume");
    assert!(
        wait_claimed(&store, task, Duration::from_secs(1)).await,
        "a resumed worker should claim on the next heartbeat, not after the {BACKOFF:?} backoff"
    );

    shutdown.shutdown();
    run.await.expect("join").expect("worker run");
}

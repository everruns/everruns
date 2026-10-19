//! One behavioural contract, run against every `WorkflowEventStore`.
//!
//! Decision: the in-memory store is the test double for PostgreSQL, so the two
//! must agree on what a caller can observe. Each case below is written once,
//! generic over a [`Harness`], and the `conformance!` macro instantiates it for
//! the in-memory store (always) and for PostgreSQL (with `postgres-tests`).
//! When the stores disagree, a case fails for one of them, which is how drift
//! like "the in-memory store ignores priority" gets caught.
//!
//! Run the PostgreSQL half with:
//! `cargo test -p everruns-durable --test store_conformance_test --features postgres-tests -- --test-threads=1`
//!
//! Every case uses fresh workflow ids and its own activity type, so cases do
//! not see each other's tasks in a shared database.

use std::sync::Arc;
use std::time::Duration;

use everruns_durable::persistence::{
    DEFAULT_MAX_PENDING_TASKS_PER_WORKFLOW, InMemoryWorkflowEventStore, StoreError, TaskDefinition,
    TaskFailureOutcome, TaskStatus, WorkerFilter, WorkerInfo, WorkflowEventStore, WorkflowStatus,
};
use everruns_durable::reliability::RetryPolicy;
use everruns_durable::workflow::{ActivityOptions, WorkflowEvent};
use everruns_durable::{DeadLetters, EventLog, TaskQueue, WorkerRegistry};
use serde_json::json;
use uuid::Uuid;

/// A store under test plus the one thing the trait cannot express: making a
/// claim look abandoned, as a worker that stopped heartbeating would.
trait Harness {
    type Store: WorkflowEventStore;
    fn store(&self) -> &Self::Store;
    /// The same store behind an `Arc`, for components that own one.
    fn shared(&self) -> Arc<dyn WorkflowEventStore>;
    async fn expire_claim(&self, task_id: Uuid);
}

struct MemoryHarness(Arc<InMemoryWorkflowEventStore>);

impl Harness for MemoryHarness {
    type Store = InMemoryWorkflowEventStore;
    fn store(&self) -> &Self::Store {
        &self.0
    }
    fn shared(&self) -> Arc<dyn WorkflowEventStore> {
        self.0.clone()
    }
    async fn expire_claim(&self, task_id: Uuid) {
        self.0.expire_claim(task_id);
    }
}

#[cfg(feature = "postgres-tests")]
struct Postgres(everruns_durable::persistence::PostgresWorkflowEventStore);

#[cfg(feature = "postgres-tests")]
impl Postgres {
    async fn connect() -> Self {
        let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
            "postgres://postgres:postgres@localhost:5432/everruns_test".to_string()
        });
        let pool = sqlx::PgPool::connect(&url)
            .await
            .expect("Failed to connect to PostgreSQL. Set DATABASE_URL.");
        Self(everruns_durable::persistence::PostgresWorkflowEventStore::new(pool))
    }
}

#[cfg(feature = "postgres-tests")]
impl Harness for Postgres {
    type Store = everruns_durable::persistence::PostgresWorkflowEventStore;
    fn store(&self) -> &Self::Store {
        &self.0
    }
    fn shared(&self) -> Arc<dyn WorkflowEventStore> {
        Arc::new(self.0.clone())
    }
    async fn expire_claim(&self, task_id: Uuid) {
        sqlx::query(
            "UPDATE durable_task_queue SET heartbeat_at = NOW() - INTERVAL '1 hour' WHERE id = $1",
        )
        .bind(task_id)
        .execute(self.0.pool())
        .await
        .expect("expire claim");
    }
}

macro_rules! conformance {
    ($($case:ident),* $(,)?) => {
        mod memory {
            $(
                #[tokio::test]
                async fn $case() {
                    super::$case(super::MemoryHarness(std::sync::Arc::new(super::InMemoryWorkflowEventStore::new()))).await;
                }
            )*
        }

        #[cfg(feature = "postgres-tests")]
        mod postgres {
            $(
                #[tokio::test]
                async fn $case() {
                    super::$case(super::Postgres::connect().await).await;
                }
            )*
        }
    };
}

conformance!(
    claims_only_for_registered_active_workers,
    claims_by_priority_then_fifo,
    claim_respects_max_tasks,
    claims_take_one_queue_only,
    retry_waits_for_backoff,
    delayed_tasks_wait_for_their_start_delay,
    non_retryable_failure_is_dead,
    exhausted_retries_are_dead,
    only_the_owner_completes,
    failing_a_reclaimed_task_is_rejected,
    fresh_claims_are_not_reclaimed,
    abandoned_claims_are_reclaimed,
    heartbeat_reports_ownership,
    first_claim_records_activity_started,
    pending_tasks_per_workflow_are_capped,
    cancel_pending_leaves_claimed_tasks,
    events_append_in_order_with_optimistic_concurrency,
    dead_letters_can_be_requeued,
    requeued_dead_letters_keep_their_queue_and_options,
    cancel_workflow_cancels_pending_tasks_once,
    drained_workers_stop_claiming_until_resumed,
    claims_carry_the_workflow_status,
    ensure_schedule_converges_on_the_spec,
    concurrent_schedule_ensures_create_one_schedule,
    a_due_schedule_fires_once_across_schedulers,
    concurrent_reaps_settle_a_dead_task_once,
);

// Dead-letter cases live in their own file to keep this one under the size cap.
#[path = "store_conformance/dead_letters.rs"]
mod dead_letters;
use dead_letters::*;

// --- helpers ---------------------------------------------------------------

/// An activity type no other case uses.
fn activity_type() -> String {
    format!("conformance_{}", Uuid::now_v7().simple())
}

async fn worker<H: Harness>(h: &H, activity_type: &str) -> String {
    let id = format!("conformance-worker-{}", Uuid::now_v7().simple());
    h.store()
        .register_worker(WorkerInfo::new(id.clone(), [activity_type]))
        .await
        .expect("register worker");
    id
}

async fn workflow<H: Harness>(h: &H) -> Uuid {
    let id = Uuid::now_v7();
    h.store()
        .create_workflow(id, "conformance", json!({}), None)
        .await
        .expect("create workflow");
    id
}

fn task(workflow_id: Option<Uuid>, activity_type: &str, activity_id: &str) -> TaskDefinition {
    TaskDefinition {
        workflow_id,
        activity_id: activity_id.to_string(),
        activity_type: activity_type.to_string(),
        input: json!({ "activity": activity_id }),
        options: ActivityOptions::default(),
    }
}

fn with_options(mut task: TaskDefinition, options: ActivityOptions) -> TaskDefinition {
    task.options = options;
    task
}

async fn enqueue<H: Harness>(h: &H, task: TaskDefinition) -> Uuid {
    h.store().enqueue_task(task).await.expect("enqueue")
}

async fn claim<H: Harness>(h: &H, worker: &str, activity_type: &str, max: usize) -> Vec<Uuid> {
    h.store()
        .claim_task(worker, &[activity_type.to_string()], max)
        .await
        .expect("claim")
        .into_iter()
        .map(|t| t.id)
        .collect()
}

async fn status<H: Harness>(h: &H, task_id: Uuid) -> TaskStatus {
    h.store().get_task(task_id).await.expect("get task").status
}

// --- task queue --------------------------------------------------------------

async fn claims_only_for_registered_active_workers<H: Harness>(h: H) {
    let ty = activity_type();
    let id = enqueue(&h, task(None, &ty, "a")).await;

    assert!(claim(&h, "never-registered", &ty, 1).await.is_empty());

    let draining = format!("draining-{}", Uuid::now_v7().simple());
    let mut info = WorkerInfo::new(draining.clone(), [ty.as_str()]);
    info.status = "draining".to_string();
    h.store().register_worker(info).await.unwrap();
    assert!(claim(&h, &draining, &ty, 1).await.is_empty());

    let w = worker(&h, &ty).await;
    assert_eq!(claim(&h, &w, &ty, 1).await, vec![id]);
}

async fn claims_by_priority_then_fifo<H: Harness>(h: H) {
    let ty = activity_type();
    let w = worker(&h, &ty).await;
    let priority = |p| ActivityOptions {
        priority: p,
        ..ActivityOptions::default()
    };
    let low_1 = enqueue(&h, with_options(task(None, &ty, "low-1"), priority(0))).await;
    let high_1 = enqueue(&h, with_options(task(None, &ty, "high-1"), priority(5))).await;
    let low_2 = enqueue(&h, with_options(task(None, &ty, "low-2"), priority(0))).await;
    let high_2 = enqueue(&h, with_options(task(None, &ty, "high-2"), priority(5))).await;

    let mut order = vec![];
    for _ in 0..4 {
        order.extend(claim(&h, &w, &ty, 1).await);
    }
    assert_eq!(order, vec![high_1, high_2, low_1, low_2]);
    assert!(claim(&h, &w, &ty, 1).await.is_empty());
}

async fn claim_respects_max_tasks<H: Harness>(h: H) {
    let ty = activity_type();
    let w = worker(&h, &ty).await;
    for i in 0..5 {
        enqueue(&h, task(None, &ty, &format!("t{i}"))).await;
    }
    assert_eq!(claim(&h, &w, &ty, 3).await.len(), 3);
    assert_eq!(claim(&h, &w, &ty, 3).await.len(), 2);
    assert!(claim(&h, &w, &ty, 3).await.is_empty());
}

async fn claims_take_one_queue_only<H: Harness>(h: H) {
    let ty = activity_type();
    let w = worker(&h, &ty).await;
    let queue = format!("conformance-queue-{}", Uuid::now_v7().simple());
    let other = format!("{queue}-other");
    let default = enqueue(&h, task(None, &ty, "default")).await;
    let queued = enqueue(
        &h,
        with_options(
            task(None, &ty, "queued"),
            ActivityOptions::default().with_queue(queue.clone()),
        ),
    )
    .await;
    let claim_from = |queue: Option<String>| {
        let h = &h;
        let w = w.clone();
        let types = vec![ty.clone()];
        async move {
            h.store()
                .claim_queue_tasks(&w, queue.as_deref(), &types, 10)
                .await
                .expect("claim")
                .into_iter()
                .map(|t| (t.id, t.options.queue))
                .collect::<Vec<_>>()
        }
    };

    assert!(claim_from(Some(other)).await.is_empty());
    assert_eq!(
        claim_from(Some(queue.clone())).await,
        vec![(queued, Some(queue))]
    );
    assert_eq!(claim(&h, &w, &ty, 10).await, vec![default]);
}

async fn retry_waits_for_backoff<H: Harness>(h: H) {
    let ty = activity_type();
    let w = worker(&h, &ty).await;
    let slow_retry = ActivityOptions {
        retry_policy: RetryPolicy::exponential()
            .with_max_attempts(3)
            .with_initial_interval(Duration::from_secs(600))
            .with_jitter(0.0),
        ..ActivityOptions::default()
    };
    let id = enqueue(&h, with_options(task(None, &ty, "a"), slow_retry)).await;
    assert_eq!(claim(&h, &w, &ty, 1).await, vec![id]);

    let outcome = h
        .store()
        .fail_task_with_retry(id, "boom", true)
        .await
        .unwrap();
    assert!(matches!(
        outcome,
        TaskFailureOutcome::WillRetry {
            next_attempt: 2,
            ..
        }
    ));
    assert_eq!(status(&h, id).await, TaskStatus::Pending);
    assert!(
        claim(&h, &w, &ty, 1).await.is_empty(),
        "a task waiting out its backoff is not claimable"
    );
}

async fn delayed_tasks_wait_for_their_start_delay<H: Harness>(h: H) {
    let ty = activity_type();
    let w = worker(&h, &ty).await;
    // Seconds, not milliseconds: a slow CI runner can spend a few hundred
    // milliseconds between the enqueue and the first claim, making a short
    // delay due before the "not due yet" assertion runs.
    let later = ActivityOptions::default().with_start_delay(Duration::from_secs(3));
    let delayed = enqueue(&h, with_options(task(None, &ty, "later"), later)).await;
    let now = enqueue(&h, task(None, &ty, "now")).await;

    assert_eq!(
        claim(&h, &w, &ty, 2).await,
        vec![now],
        "the delayed task is not due"
    );
    tokio::time::sleep(Duration::from_millis(3200)).await;
    assert_eq!(claim(&h, &w, &ty, 2).await, vec![delayed]);
}

async fn non_retryable_failure_is_dead<H: Harness>(h: H) {
    let ty = activity_type();
    let w = worker(&h, &ty).await;
    let id = enqueue(&h, task(None, &ty, "a")).await;
    claim(&h, &w, &ty, 1).await;

    let outcome = h
        .store()
        .fail_task_with_retry(id, "bad input", false)
        .await
        .unwrap();
    assert!(matches!(outcome, TaskFailureOutcome::MovedToDlq));
    assert_eq!(status(&h, id).await, TaskStatus::Dead);
}

async fn exhausted_retries_are_dead<H: Harness>(h: H) {
    let ty = activity_type();
    let w = worker(&h, &ty).await;
    let once = ActivityOptions {
        retry_policy: RetryPolicy::exponential().with_max_attempts(1),
        ..ActivityOptions::default()
    };
    let id = enqueue(&h, with_options(task(None, &ty, "a"), once)).await;
    claim(&h, &w, &ty, 1).await;

    let outcome = h
        .store()
        .fail_task_with_retry(id, "boom", true)
        .await
        .unwrap();
    assert!(matches!(outcome, TaskFailureOutcome::MovedToDlq));
    assert_eq!(status(&h, id).await, TaskStatus::Dead);
}

async fn only_the_owner_completes<H: Harness>(h: H) {
    let ty = activity_type();
    let owner = worker(&h, &ty).await;
    let other = worker(&h, &ty).await;
    let id = enqueue(&h, task(None, &ty, "a")).await;
    claim(&h, &owner, &ty, 1).await;

    let err = h
        .store()
        .complete_task(id, &other, json!(1))
        .await
        .unwrap_err();
    assert!(matches!(err, StoreError::TaskNotOwned(_)));

    h.store().complete_task(id, &owner, json!(1)).await.unwrap();
    assert_eq!(status(&h, id).await, TaskStatus::Completed);

    let err = h
        .store()
        .complete_task(id, &owner, json!(1))
        .await
        .unwrap_err();
    assert!(
        matches!(err, StoreError::TaskNotOwned(_)),
        "completing twice"
    );
}

async fn failing_a_reclaimed_task_is_rejected<H: Harness>(h: H) {
    let ty = activity_type();
    let w = worker(&h, &ty).await;
    let id = enqueue(&h, task(None, &ty, "a")).await;
    claim(&h, &w, &ty, 1).await;
    h.expire_claim(id).await;
    let reclaimed = h
        .store()
        .reclaim_stale_tasks(Duration::from_secs(30))
        .await
        .unwrap();
    assert!(reclaimed.reclaimed_ids.contains(&id));

    let err = h
        .store()
        .fail_task_with_retry(id, "late", true)
        .await
        .unwrap_err();
    assert!(matches!(err, StoreError::TaskNotOwned(_)));
    assert_eq!(status(&h, id).await, TaskStatus::Pending);
}

async fn fresh_claims_are_not_reclaimed<H: Harness>(h: H) {
    let ty = activity_type();
    let w = worker(&h, &ty).await;
    let id = enqueue(&h, task(None, &ty, "a")).await;
    claim(&h, &w, &ty, 1).await;

    let reclaimed = h
        .store()
        .reclaim_stale_tasks(Duration::from_secs(30))
        .await
        .unwrap();
    assert!(!reclaimed.reclaimed_ids.contains(&id));
    assert_eq!(status(&h, id).await, TaskStatus::Claimed);
}

async fn abandoned_claims_are_reclaimed<H: Harness>(h: H) {
    let ty = activity_type();
    let first = worker(&h, &ty).await;
    let second = worker(&h, &ty).await;
    let id = enqueue(&h, task(None, &ty, "a")).await;
    claim(&h, &first, &ty, 1).await;
    h.expire_claim(id).await;

    let reclaimed = h
        .store()
        .reclaim_stale_tasks(Duration::from_secs(30))
        .await
        .unwrap();
    assert!(reclaimed.reclaimed_ids.contains(&id));

    let claimed = h
        .store()
        .claim_task(&second, std::slice::from_ref(&ty), 1)
        .await
        .unwrap();
    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].attempt, 2, "a reclaim spends an attempt");
    let err = h
        .store()
        .complete_task(id, &first, json!(1))
        .await
        .unwrap_err();
    assert!(
        matches!(err, StoreError::TaskNotOwned(_)),
        "old owner is fenced off"
    );
    h.store()
        .complete_task(id, &second, json!(1))
        .await
        .unwrap();
}

async fn heartbeat_reports_ownership<H: Harness>(h: H) {
    let ty = activity_type();
    let owner = worker(&h, &ty).await;
    let other = worker(&h, &ty).await;
    let id = enqueue(&h, task(None, &ty, "a")).await;
    claim(&h, &owner, &ty, 1).await;

    let beat = h.store().heartbeat_task(id, &owner, None).await.unwrap();
    assert!(beat.accepted && !beat.should_cancel);

    let beat = h.store().heartbeat_task(id, &other, None).await.unwrap();
    assert!(!beat.accepted && beat.should_cancel);

    h.store().complete_task(id, &owner, json!(1)).await.unwrap();
    let beat = h.store().heartbeat_task(id, &owner, None).await.unwrap();
    assert!(!beat.accepted, "a finished task takes no heartbeats");
}

async fn first_claim_records_activity_started<H: Harness>(h: H) {
    let ty = activity_type();
    let w = worker(&h, &ty).await;
    let wf = workflow(&h).await;
    let id = enqueue(&h, task(Some(wf), &ty, "step")).await;

    let started = |events: &[(i32, WorkflowEvent)]| {
        events
            .iter()
            .filter(|(_, e)| matches!(e, WorkflowEvent::ActivityStarted { activity_id, .. } if activity_id == "step"))
            .count()
    };

    claim(&h, &w, &ty, 1).await;
    assert_eq!(started(&h.store().load_events(wf).await.unwrap()), 1);

    // A reclaim is not a new start (EVE-639).
    h.expire_claim(id).await;
    h.store()
        .reclaim_stale_tasks(Duration::from_secs(30))
        .await
        .unwrap();
    claim(&h, &w, &ty, 1).await;
    assert_eq!(started(&h.store().load_events(wf).await.unwrap()), 1);
}

async fn pending_tasks_per_workflow_are_capped<H: Harness>(h: H) {
    let ty = activity_type();
    let wf = workflow(&h).await;
    for i in 0..DEFAULT_MAX_PENDING_TASKS_PER_WORKFLOW {
        enqueue(&h, task(Some(wf), &ty, &format!("t{i}"))).await;
    }
    let err = h
        .store()
        .enqueue_task(task(Some(wf), &ty, "one-too-many"))
        .await
        .unwrap_err();
    assert!(matches!(err, StoreError::TaskQueueLimitExceeded { .. }));

    // Another workflow is unaffected, and claiming frees room.
    let other = workflow(&h).await;
    enqueue(&h, task(Some(other), &ty, "elsewhere")).await;
    let w = worker(&h, &ty).await;
    claim(&h, &w, &ty, 2).await;
    enqueue(&h, task(Some(wf), &ty, "fits-now")).await;
}

async fn cancel_pending_leaves_claimed_tasks<H: Harness>(h: H) {
    let ty = activity_type();
    let w = worker(&h, &ty).await;
    let wf = workflow(&h).await;
    let running = enqueue(&h, task(Some(wf), &ty, "running")).await;
    claim(&h, &w, &ty, 1).await;
    let waiting_1 = enqueue(&h, task(Some(wf), &ty, "waiting-1")).await;
    let waiting_2 = enqueue(&h, task(Some(wf), &ty, "waiting-2")).await;

    let cancelled = h
        .store()
        .cancel_pending_tasks_for_workflow(wf)
        .await
        .unwrap();
    assert_eq!(cancelled, 2);
    assert_eq!(status(&h, waiting_1).await, TaskStatus::Cancelled);
    assert_eq!(status(&h, waiting_2).await, TaskStatus::Cancelled);
    assert_eq!(status(&h, running).await, TaskStatus::Claimed);
    assert!(claim(&h, &w, &ty, 10).await.is_empty());
}

// --- event log, signals, dead letters ----------------------------------------

async fn events_append_in_order_with_optimistic_concurrency<H: Harness>(h: H) {
    let wf = workflow(&h).await;
    let base = h.store().count_events(wf).await.unwrap() as i32;
    let completed = |id: &str| WorkflowEvent::ActivityCompleted {
        activity_id: id.to_string(),
        result: json!(id),
    };

    h.store()
        .append_events(wf, base, vec![completed("a"), completed("b")])
        .await
        .unwrap();
    let err = h
        .store()
        .append_events(wf, base, vec![completed("stale")])
        .await
        .unwrap_err();
    assert!(matches!(err, StoreError::ConcurrencyConflict { .. }));

    let events = h.store().load_events(wf).await.unwrap();
    let ids: Vec<_> = events
        .iter()
        .filter_map(|(_, e)| match e {
            WorkflowEvent::ActivityCompleted { activity_id, .. } => Some(activity_id.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(ids, ["a", "b"]);
    let sequences: Vec<i32> = events.iter().map(|(seq, _)| *seq).collect();
    assert!(
        sequences.windows(2).all(|w| w[0] < w[1]),
        "sequences ascend"
    );
}

// --- cancellation, draining ----------------------------------------------------

async fn cancel_workflow_cancels_pending_tasks_once<H: Harness>(h: H) {
    let ty = activity_type();
    let wf = workflow(&h).await;
    let pending = enqueue(&h, task(Some(wf), &ty, "pending")).await;

    h.store().cancel_workflow(wf).await.unwrap();
    assert_eq!(
        h.store().get_workflow_status(wf).await.unwrap(),
        WorkflowStatus::Cancelled
    );
    assert_eq!(status(&h, pending).await, TaskStatus::Cancelled);

    // A terminal workflow, or an unknown one, cannot be cancelled again.
    for id in [wf, Uuid::now_v7()] {
        let err = h.store().cancel_workflow(id).await.unwrap_err();
        assert!(matches!(err, StoreError::WorkflowNotFound(_)));
    }
}

async fn drained_workers_stop_claiming_until_resumed<H: Harness>(h: H) {
    let ty = activity_type();
    let w = worker(&h, &ty).await;
    enqueue(&h, task(None, &ty, "t")).await;

    let beat = h.store().worker_heartbeat(&w, 0, true).await.unwrap();
    assert!(!beat.draining, "an active worker is not draining");

    h.store().drain_worker(&w).await.unwrap();
    assert!(claim(&h, &w, &ty, 1).await.is_empty());
    let beat = h.store().worker_heartbeat(&w, 0, true).await.unwrap();
    assert!(beat.draining, "the heartbeat tells a drained worker so");
    let info = h
        .store()
        .list_workers(WorkerFilter::default())
        .await
        .unwrap()
        .into_iter()
        .find(|info| info.id == w)
        .expect("registered worker is listed");
    assert_eq!(info.status, "draining");
    assert!(!info.accepting_tasks);

    h.store().resume_worker(&w).await.unwrap();
    let beat = h.store().worker_heartbeat(&w, 0, true).await.unwrap();
    assert!(!beat.draining, "a resumed worker is told to claim again");
    assert_eq!(claim(&h, &w, &ty, 1).await.len(), 1);

    let beat = h
        .store()
        .worker_heartbeat("unknown", 0, true)
        .await
        .unwrap();
    assert!(!beat.draining, "an unknown worker is not draining");
}

async fn claims_carry_the_workflow_status<H: Harness>(h: H) {
    let ty = activity_type();
    let w = worker(&h, &ty).await;
    let pending = workflow(&h).await;
    let running = workflow(&h).await;
    h.store()
        .update_workflow_status(running, WorkflowStatus::Running, None, None)
        .await
        .unwrap();
    for (wf, id) in [(pending, "p"), (running, "r")] {
        enqueue(&h, task(Some(wf), &ty, id)).await;
    }
    enqueue(&h, task(None, &ty, "standalone")).await;

    let claimed = h
        .store()
        .claim_task(&w, std::slice::from_ref(&ty), 10)
        .await
        .unwrap();
    let status_of = |activity_id: &str| {
        claimed
            .iter()
            .find(|t| t.activity_id == activity_id)
            .expect("claimed")
            .workflow_status
    };
    assert_eq!(claimed.len(), 3);
    assert_eq!(status_of("p"), Some(WorkflowStatus::Pending));
    assert_eq!(status_of("r"), Some(WorkflowStatus::Running));
    assert_eq!(status_of("standalone"), None);
}

// --- schedules and maintenance ---------------------------------------------

fn interval_spec(name: &str, activity_type: &str) -> everruns_durable::ScheduleSpec {
    everruns_durable::ScheduleSpec::activity(
        name,
        activity_type,
        everruns_durable::Cadence::Every(Duration::from_secs(90)),
        json!({ "batch": 10 }),
    )
    .with_description("conformance sweep")
}

async fn schedules_named<H: Harness>(h: &H, name: &str) -> Vec<everruns_durable::ScheduleRow> {
    use everruns_durable::Schedules;
    let mut found = Vec::new();
    let mut offset = 0;
    loop {
        let page = h
            .store()
            .list_schedules(
                everruns_durable::ScheduleFilter::default(),
                everruns_durable::Pagination { offset, limit: 500 },
            )
            .await
            .expect("list schedules");
        let len = page.len();
        found.extend(page.into_iter().filter(|row| row.name == name));
        if len < 500 {
            return found;
        }
        offset += 500;
    }
}

async fn ensure_schedule_converges_on_the_spec<H: Harness>(h: H) {
    use everruns_durable::{EnsureOutcome, Schedules, UpdateSchedule, ensure_schedule};
    let name = activity_type();
    let spec = interval_spec(&name, &activity_type());

    let EnsureOutcome::Created(id) = ensure_schedule(h.store(), &spec).await.expect("create")
    else {
        panic!("first ensure creates");
    };
    assert_eq!(
        ensure_schedule(h.store(), &spec).await.expect("again"),
        EnsureOutcome::Unchanged(id)
    );

    h.store()
        .update_schedule(
            id,
            UpdateSchedule {
                enabled: Some(false),
                cron_expression: Some("0 0 * * * * *".into()),
                ..Default::default()
            },
        )
        .await
        .expect("drift");
    assert_eq!(
        ensure_schedule(h.store(), &spec).await.expect("reset"),
        EnsureOutcome::Updated(id)
    );
    let rows = schedules_named(&h, &name).await;
    assert_eq!(rows.len(), 1);
    assert!(spec.matches(&rows[0]), "row: {:?}", rows[0]);
    assert_eq!(rows[0].cron_expression, "@every 90s");
}

async fn concurrent_schedule_ensures_create_one_schedule<H: Harness>(h: H) {
    use everruns_durable::{EnsureOutcome, ensure_schedule};
    let name = activity_type();
    let spec = interval_spec(&name, &activity_type());

    let (a, b) = tokio::join!(
        ensure_schedule(h.store(), &spec),
        ensure_schedule(h.store(), &spec)
    );
    let outcomes = [a.expect("first ensure"), b.expect("second ensure")];
    let created = outcomes
        .iter()
        .filter(|o| matches!(o, EnsureOutcome::Created(_)))
        .count();
    assert_eq!(created, 1, "outcomes: {outcomes:?}");
    assert_eq!(schedules_named(&h, &name).await.len(), 1);
}

async fn a_due_schedule_fires_once_across_schedulers<H: Harness>(h: H) {
    use everruns_durable::{CreateScheduleRow, DurableScheduler, ScheduleTargetType, Schedules};
    let ty = activity_type();
    let schedule_id = h
        .store()
        .create_schedule(CreateScheduleRow {
            name: ty.clone(),
            description: None,
            cron_expression: "@every 600s".into(),
            timezone: "UTC".into(),
            target_type: ScheduleTargetType::Activity,
            target_name: ty.clone(),
            target_input: json!({}),
            enabled: true,
            max_concurrent: Some(1),
            catch_up_missed: false,
            max_catch_up: Some(1),
            retry_policy: None,
            next_trigger_at: Some(chrono::Utc::now() - chrono::Duration::seconds(1)),
        })
        .await
        .expect("create schedule");

    let first = DurableScheduler::with_defaults(h.shared(), format!("{ty}-a"));
    let second = DurableScheduler::with_defaults(h.shared(), format!("{ty}-b"));
    let (a, b) = tokio::join!(
        first.process_due_schedules(),
        second.process_due_schedules()
    );
    a.expect("first scheduler");
    b.expect("second scheduler");
    // A later poll finds nothing due: the next trigger is a period away.
    first.process_due_schedules().await.expect("re-poll");

    let stats = h
        .store()
        .get_schedule_stats(schedule_id)
        .await
        .expect("stats");
    assert_eq!(stats.total_executions, 1);
    let worker = worker(&h, &ty).await;
    assert_eq!(
        claim(&h, &worker, &ty, 10).await.len(),
        1,
        "one task enqueued"
    );
}

#[derive(Default)]
struct CountingReaper(std::sync::Mutex<Vec<Uuid>>);

#[async_trait::async_trait]
impl everruns_durable::ReapHandler for CountingReaper {
    async fn workflow_failed(&self, dead: &everruns_durable::DeadTaskInfo, _: &str) {
        self.0.lock().unwrap().push(dead.task_id);
    }
}

async fn concurrent_reaps_settle_a_dead_task_once<H: Harness>(h: H) {
    use everruns_durable::reap_stale_tasks;
    let ty = activity_type();
    let worker = worker(&h, &ty).await;
    let wf = workflow(&h).await;
    h.store()
        .update_workflow_status(wf, WorkflowStatus::Running, None, None)
        .await
        .expect("run workflow");
    let task_id = enqueue(
        &h,
        with_options(
            task(Some(wf), &ty, "a"),
            ActivityOptions::default().with_retry(RetryPolicy::no_retry()),
        ),
    )
    .await;
    assert_eq!(claim(&h, &worker, &ty, 1).await, vec![task_id]);
    h.expire_claim(task_id).await;

    let handler = CountingReaper::default();
    let threshold = Duration::from_secs(60);
    let (a, b) = tokio::join!(
        reap_stale_tasks(h.store(), threshold, &handler),
        reap_stale_tasks(h.store(), threshold, &handler)
    );
    a.expect("first reap");
    b.expect("second reap");

    assert_eq!(*handler.0.lock().unwrap(), vec![task_id]);
    assert_eq!(status(&h, task_id).await, TaskStatus::Dead);
    assert_eq!(
        h.store().get_workflow_status(wf).await.expect("status"),
        WorkflowStatus::Failed
    );
}

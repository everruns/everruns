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

use std::time::Duration;

use everruns_durable::persistence::{
    DEFAULT_MAX_PENDING_TASKS_PER_WORKFLOW, InMemoryWorkflowEventStore, StoreError, TaskDefinition,
    TaskFailureOutcome, TaskStatus, WorkerFilter, WorkerInfo, WorkflowEventStore, WorkflowStatus,
};
use everruns_durable::reliability::RetryPolicy;
use everruns_durable::workflow::{ActivityOptions, WorkflowEvent, WorkflowSignal};
use everruns_durable::{DeadLetters, EventLog, SignalStore, TaskQueue, WorkerRegistry};
use serde_json::json;
use uuid::Uuid;

/// A store under test plus the one thing the trait cannot express: making a
/// claim look abandoned, as a worker that stopped heartbeating would.
trait Harness {
    type Store: WorkflowEventStore;
    fn store(&self) -> &Self::Store;
    async fn expire_claim(&self, task_id: Uuid);
}

struct MemoryHarness(InMemoryWorkflowEventStore);

impl Harness for MemoryHarness {
    type Store = InMemoryWorkflowEventStore;
    fn store(&self) -> &Self::Store {
        &self.0
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
                    super::$case(super::MemoryHarness(super::InMemoryWorkflowEventStore::new())).await;
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
    dedupe_enqueue_returns_the_existing_task,
    dedupe_replay_is_exempt_from_the_pending_cap,
    concurrent_dedupe_enqueues_agree_on_one_task,
    cancel_pending_leaves_claimed_tasks,
    events_append_in_order_with_optimistic_concurrency,
    signals_are_consumed_once,
    dead_letters_can_be_requeued,
    cancel_workflow_cancels_pending_tasks_once,
    drained_workers_stop_claiming_until_resumed,
);

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
    let later = ActivityOptions::default().with_start_delay(Duration::from_millis(300));
    let delayed = enqueue(&h, with_options(task(None, &ty, "later"), later)).await;
    let now = enqueue(&h, task(None, &ty, "now")).await;

    assert_eq!(
        claim(&h, &w, &ty, 2).await,
        vec![now],
        "the delayed task is not due"
    );
    tokio::time::sleep(Duration::from_millis(400)).await;
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

fn dedupe(task: TaskDefinition) -> TaskDefinition {
    with_options(
        task,
        ActivityOptions::default().with_dedupe_by_activity_id(),
    )
}

async fn dedupe_enqueue_returns_the_existing_task<H: Harness>(h: H) {
    let ty = activity_type();
    let wf = workflow(&h).await;
    let first = enqueue(&h, dedupe(task(Some(wf), &ty, "resume-1"))).await;
    assert_eq!(
        enqueue(&h, dedupe(task(Some(wf), &ty, "resume-1"))).await,
        first
    );

    // In any status, not only while pending.
    let w = worker(&h, &ty).await;
    assert_eq!(claim(&h, &w, &ty, 1).await, vec![first]);
    h.store().complete_task(first, &w, json!(1)).await.unwrap();
    assert_eq!(
        enqueue(&h, dedupe(task(Some(wf), &ty, "resume-1"))).await,
        first
    );

    // Scoped to the workflow and opt-in per task.
    let other = workflow(&h).await;
    assert_ne!(
        enqueue(&h, dedupe(task(Some(other), &ty, "resume-1"))).await,
        first
    );
    let plain = enqueue(&h, task(Some(wf), &ty, "plain")).await;
    assert_ne!(enqueue(&h, task(Some(wf), &ty, "plain")).await, plain);

    // Standalone tasks have no workflow to dedupe within.
    let standalone = enqueue(&h, dedupe(task(None, &ty, "solo"))).await;
    assert_ne!(
        enqueue(&h, dedupe(task(None, &ty, "solo"))).await,
        standalone
    );
}

async fn dedupe_replay_is_exempt_from_the_pending_cap<H: Harness>(h: H) {
    let ty = activity_type();
    let wf = workflow(&h).await;
    let resume = enqueue(&h, dedupe(task(Some(wf), &ty, "resume"))).await;
    for i in 1..DEFAULT_MAX_PENDING_TASKS_PER_WORKFLOW {
        enqueue(&h, task(Some(wf), &ty, &format!("t{i}"))).await;
    }
    assert_eq!(
        enqueue(&h, dedupe(task(Some(wf), &ty, "resume"))).await,
        resume
    );
    let err = h
        .store()
        .enqueue_task(dedupe(task(Some(wf), &ty, "new-resume")))
        .await
        .unwrap_err();
    assert!(matches!(err, StoreError::TaskQueueLimitExceeded { .. }));
}

async fn concurrent_dedupe_enqueues_agree_on_one_task<H: Harness>(h: H) {
    // The schema ships a unique index over these ids (server migration 143),
    // so PostgreSQL settles the race in the database, not only in the
    // pre-insert check.
    let ty = activity_type();
    let wf = workflow(&h).await;
    let id = format!("waiting_turn_resolution_{}", Uuid::now_v7());
    let enqueue_one = || h.store().enqueue_task(dedupe(task(Some(wf), &ty, &id)));
    let (a, b, c, d) = tokio::join!(enqueue_one(), enqueue_one(), enqueue_one(), enqueue_one());
    let a = a.expect("enqueue");
    for other in [b, c, d] {
        assert_eq!(other.expect("enqueue"), a);
    }
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

async fn signals_are_consumed_once<H: Harness>(h: H) {
    let wf = workflow(&h).await;
    h.store()
        .send_signal(wf, WorkflowSignal::new("approve", json!({ "by": "ops" })))
        .await
        .unwrap();
    h.store()
        .send_signal(wf, WorkflowSignal::new("comment", json!("hi")))
        .await
        .unwrap();

    let approvals = h
        .store()
        .consume_pending_signals_by_type(wf, "approve")
        .await
        .unwrap();
    assert_eq!(approvals.len(), 1);
    assert_eq!(approvals[0].payload, json!({ "by": "ops" }));

    let rest = h.store().consume_pending_signals(wf).await.unwrap();
    assert_eq!(rest.len(), 1);
    assert_eq!(rest[0].signal_type, "comment");
    assert!(h.store().get_pending_signals(wf).await.unwrap().is_empty());
}

async fn dead_letters_can_be_requeued<H: Harness>(h: H) {
    let ty = activity_type();
    let w = worker(&h, &ty).await;
    let wf = workflow(&h).await;
    let id = enqueue(&h, task(Some(wf), &ty, "flaky")).await;
    claim(&h, &w, &ty, 1).await;
    h.store()
        .fail_task_with_retry(id, "boom", false)
        .await
        .unwrap();
    h.store()
        .move_to_dlq(id, vec!["boom".to_string()])
        .await
        .unwrap();

    let entries = h
        .store()
        .list_dlq(
            everruns_durable::persistence::DlqFilter {
                workflow_id: Some(wf),
                activity_type: None,
            },
            Default::default(),
        )
        .await
        .unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].original_task_id, id);

    let requeued = h.store().requeue_from_dlq(entries[0].id).await.unwrap();
    assert_ne!(requeued, id);
    assert_eq!(claim(&h, &w, &ty, 1).await, vec![requeued]);
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

    h.store().drain_worker(&w).await.unwrap();
    assert!(claim(&h, &w, &ty, 1).await.is_empty());
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
    assert_eq!(claim(&h, &w, &ty, 1).await.len(), 1);
}

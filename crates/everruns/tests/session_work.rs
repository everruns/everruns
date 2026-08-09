use std::sync::Arc;
use std::time::{Duration, SystemTime};

use everruns::work::{
    InMemoryWorkBackend, SessionWake, Task, TaskOutcome, TaskRequest, TaskState, WakePolicy,
    WakeReason, WakeRequest, WorkError, WorkQueue, WorkSchedule,
};
use serde_json::json;

const LEASE: Duration = Duration::from_secs(30);

fn instant(seconds: u64) -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(seconds)
}

#[tokio::test]
async fn immediate_wake_is_claimed_once_after_acknowledgment() {
    let queue = WorkQueue::in_memory();
    let session = queue.for_session("session_example");
    let now = instant(100);
    let wake = session
        .wake_at(
            WakeRequest::new(json!({ "reason": "refresh" })).idempotency_key("refresh-1"),
            now,
        )
        .await
        .unwrap();
    let duplicate = session
        .wake_at(
            WakeRequest::new(json!({ "reason": "refresh" })).idempotency_key("refresh-1"),
            now + Duration::from_secs(1),
        )
        .await
        .unwrap();
    assert_eq!(duplicate.id, wake.id);
    assert!(matches!(
        session
            .wake_at(
                WakeRequest::new(json!({ "reason": "different" })).idempotency_key("refresh-1"),
                now + Duration::from_secs(2),
            )
            .await
            .unwrap_err(),
        WorkError::IdempotencyConflict { .. }
    ));

    let first = queue.claim_wakes_at(now, LEASE, 1).await.unwrap().remove(0);
    assert!(!format!("{first:?}").contains(first.lease_token()));
    assert_eq!(first.wake.id, wake.id);
    assert!(matches!(
        first.wake.reason,
        WakeReason::Requested { ref payload } if payload == &json!({ "reason": "refresh" })
    ));
    assert!(
        queue
            .claim_wakes_at(now, LEASE, 1)
            .await
            .unwrap()
            .is_empty()
    );

    let retry = queue
        .claim_wakes_at(now + LEASE, LEASE, 1)
        .await
        .unwrap()
        .remove(0);
    assert_eq!(retry.wake.id, wake.id);
    assert_eq!(retry.attempt, 2);
    assert!(matches!(
        queue.acknowledge_wake(&first).await.unwrap_err(),
        WorkError::StaleDelivery { .. }
    ));
    queue.acknowledge_wake(&retry).await.unwrap();
    queue.acknowledge_wake(&retry).await.unwrap();
    assert!(
        queue
            .claim_wakes_at(now + LEASE + LEASE, LEASE, 1)
            .await
            .unwrap()
            .is_empty()
    );

    let other_session_wake = queue
        .for_session("session_other")
        .wake_at(
            WakeRequest::new(json!({ "reason": "refresh" })).idempotency_key("refresh-1"),
            now,
        )
        .await
        .unwrap();
    assert_ne!(other_session_wake.id, wake.id);
}

#[tokio::test]
async fn scheduled_work_is_not_claimed_early() {
    let queue = WorkQueue::in_memory();
    let session = queue.for_session("session_example");
    let submitted_at = instant(100);
    let due_at = instant(160);
    session
        .submit_at(
            TaskRequest::new("digest", json!({ "range": "daily" }))
                .schedule(WorkSchedule::At(due_at)),
            submitted_at,
        )
        .await
        .unwrap();

    assert!(
        queue
            .claim_due_at(instant(159), LEASE, 1)
            .await
            .unwrap()
            .is_empty()
    );
    let delivery = queue
        .claim_due_at(due_at, LEASE, 1)
        .await
        .unwrap()
        .remove(0);
    assert_eq!(delivery.task.kind, "digest");
}

#[tokio::test]
async fn cancellation_is_immediate_when_pending_and_cooperative_when_running() {
    let queue = WorkQueue::in_memory();
    let session = queue.for_session("session_example");
    let now = instant(100);
    let pending = session
        .submit_at(
            TaskRequest::new("pending", json!(null)).wake_policy(WakePolicy::OnCompletion),
            now,
        )
        .await
        .unwrap();
    let pending = session.cancel_at(&pending.id, now).await.unwrap();
    assert_eq!(pending.state, TaskState::Canceled);
    assert_eq!(pending.cancel_requested_at, Some(now));
    let cancel_wake = queue.claim_wakes_at(now, LEASE, 1).await.unwrap().remove(0);
    assert!(matches!(
        cancel_wake.wake.reason,
        WakeReason::TaskFinished {
            state: TaskState::Canceled,
            outcome: TaskOutcome::Canceled,
            ..
        }
    ));

    let running = session
        .submit_at(TaskRequest::new("running", json!(null)), now)
        .await
        .unwrap();
    let delivery = queue
        .claim_due_at(now, LEASE, 10)
        .await
        .unwrap()
        .into_iter()
        .find(|delivery| delivery.task.id == running.id)
        .unwrap();
    let running = session.cancel_at(&running.id, now).await.unwrap();
    assert_eq!(running.state, TaskState::Running);
    assert_eq!(running.cancel_requested_at, Some(now));

    let canceled = queue
        .finish_at(&delivery, TaskOutcome::Canceled, now)
        .await
        .unwrap();
    assert_eq!(canceled.state, TaskState::Canceled);
}

#[tokio::test]
async fn submission_keys_deduplicate_and_reject_changed_intent() {
    let queue = WorkQueue::in_memory();
    let session = queue.for_session("session_example");
    let now = instant(100);
    let request =
        TaskRequest::new("export", json!({ "format": "jsonl" })).idempotency_key("export-42");
    let first = session.submit_at(request.clone(), now).await.unwrap();
    let duplicate = session.submit_at(request, instant(101)).await.unwrap();
    assert_eq!(duplicate.id, first.id);
    let other_session_task = queue
        .for_session("session_other")
        .submit_at(
            TaskRequest::new("export", json!({ "format": "jsonl" })).idempotency_key("export-42"),
            now,
        )
        .await
        .unwrap();
    assert_ne!(other_session_task.id, first.id);

    let error = session
        .submit_at(
            TaskRequest::new("export", json!({ "format": "csv" })).idempotency_key("export-42"),
            instant(102),
        )
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        WorkError::IdempotencyConflict { ref key } if key == "export-42"
    ));
}

#[tokio::test]
async fn expired_lease_redelivers_and_fences_the_stale_attempt() {
    let queue = WorkQueue::in_memory();
    let session = queue.for_session("session_example");
    let now = instant(100);
    session
        .submit_at(TaskRequest::new("retryable", json!(null)), now)
        .await
        .unwrap();

    let first = queue.claim_due_at(now, LEASE, 1).await.unwrap().remove(0);
    assert!(!format!("{first:?}").contains(first.lease_token()));
    let second = queue
        .claim_due_at(now + LEASE, LEASE, 1)
        .await
        .unwrap()
        .remove(0);
    assert_eq!(first.task.id, second.task.id);
    assert_eq!(first.attempt, 1);
    assert_eq!(second.attempt, 2);

    let error = queue
        .finish_at(&first, TaskOutcome::success(json!({ "stale": true })), now)
        .await
        .unwrap_err();
    assert!(matches!(error, WorkError::StaleDelivery { .. }));
    let done = queue
        .finish_at(
            &second,
            TaskOutcome::success(json!({ "attempt": 2 })),
            now + LEASE,
        )
        .await
        .unwrap();
    assert_eq!(done.state, TaskState::Succeeded);
}

#[tokio::test]
async fn completion_creates_one_at_least_once_wake() {
    let queue = WorkQueue::in_memory();
    let session = queue.for_session("session_example");
    let now = instant(100);
    let task = session
        .submit_at(
            TaskRequest::new("index", json!({ "document": 7 }))
                .wake_policy(WakePolicy::OnCompletion),
            now,
        )
        .await
        .unwrap();
    let delivery = queue.claim_due_at(now, LEASE, 1).await.unwrap().remove(0);
    let outcome = TaskOutcome::success_with_summary("indexed document", json!({ "ok": true }));
    queue
        .finish_at(&delivery, outcome.clone(), now)
        .await
        .unwrap();
    queue.finish_at(&delivery, outcome, now).await.unwrap();

    let wakes = queue.claim_wakes_at(now, LEASE, 10).await.unwrap();
    assert_eq!(wakes.len(), 1);
    assert!(matches!(
        wakes[0].wake.reason,
        WakeReason::TaskFinished { ref task_id, state: TaskState::Succeeded, .. }
            if task_id == &task.id
    ));
}

#[tokio::test]
async fn queue_recreation_with_same_backend_preserves_unfinished_work() {
    let backend = Arc::new(InMemoryWorkBackend::new());
    let now = instant(100);
    let task_id = {
        let first_queue = WorkQueue::with_backend(backend.clone());
        first_queue
            .for_session("session_example")
            .submit_at(TaskRequest::new("resume", json!(null)), now)
            .await
            .unwrap()
            .id
    };

    let restarted_queue = WorkQueue::with_backend(backend);
    let delivery = restarted_queue
        .claim_due_at(now, LEASE, 1)
        .await
        .unwrap()
        .remove(0);
    assert_eq!(delivery.task.id, task_id);
}

#[tokio::test]
async fn replacing_the_in_memory_backend_drops_process_local_state() {
    let now = instant(100);
    let first_queue = WorkQueue::in_memory();
    let task = first_queue
        .for_session("session_example")
        .submit_at(TaskRequest::new("ephemeral", json!(null)), now)
        .await
        .unwrap();

    let replacement = WorkQueue::in_memory();
    assert!(
        replacement
            .for_session("session_example")
            .task(&task.id)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn invalid_claim_and_request_values_fail_at_the_facade() {
    let queue = WorkQueue::in_memory();
    let error = queue
        .for_session("session_example")
        .submit(TaskRequest::new("   ", json!(null)))
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        WorkError::InvalidRequest { field: "kind", .. }
    ));

    let error = queue.claim_due(Duration::ZERO, 1).await.unwrap_err();
    assert!(matches!(
        error,
        WorkError::InvalidRequest {
            field: "lease_for",
            ..
        }
    ));

    let error = queue
        .for_session("session_example")
        .cancel("   ")
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        WorkError::InvalidRequest {
            field: "task_id",
            ..
        }
    ));

    let request = TaskRequest::new("invalid-snapshot", json!(null));
    let mut task = Task::pending("task_invalid", "session_example", &request, instant(100));
    task.state = TaskState::Succeeded;
    task.outcome = Some(TaskOutcome::failure("mismatched outcome"));
    assert!(SessionWake::task_finished("wake_invalid", &task, instant(101)).is_none());
}

#[tokio::test]
async fn task_reads_and_cancellation_are_session_scoped() {
    let queue = WorkQueue::in_memory();
    let owner = queue.for_session("session_owner");
    let other = queue.for_session("session_other");
    let task = owner
        .submit(TaskRequest::new("private", json!({ "value": 7 })))
        .await
        .unwrap();

    assert!(other.task(&task.id).await.unwrap().is_none());
    assert!(matches!(
        other.cancel(&task.id).await.unwrap_err(),
        WorkError::TaskNotFound { .. }
    ));
    assert_eq!(owner.task(&task.id).await.unwrap().unwrap().id, task.id);
}

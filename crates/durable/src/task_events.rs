//! Task lifecycle event recording
//!
//! This module provides centralized logic for recording task lifecycle events
//! (ActivityStarted, ActivityCompleted, ActivityFailed) with automatic retry
//! on concurrency conflicts.
//!
//! Used by both DEV_MODE (in-process worker) and production mode (gRPC handlers)
//! to ensure consistent event recording.

use crate::{ActivityError, StoreError, WorkflowError, WorkflowEvent, WorkflowEventStore};
use tracing::{debug, warn};
use uuid::Uuid;

/// Maximum retry attempts for event appending on concurrency conflicts
const MAX_RETRY_ATTEMPTS: u32 = 5;

/// Append a workflow event with automatic retry on concurrency conflicts.
///
/// This function handles the common pattern of:
/// 1. Load current events to get sequence number
/// 2. Attempt to append with expected sequence
/// 3. Retry on concurrency conflict
///
/// Returns Ok(()) on success, or the last error after all retries exhausted.
pub async fn append_event<S: WorkflowEventStore + ?Sized>(
    store: &S,
    workflow_id: Uuid,
    event: WorkflowEvent,
) -> Result<(), StoreError> {
    for attempt in 0..MAX_RETRY_ATTEMPTS {
        // Get current sequence without materializing the workflow replay payload.
        let current_seq = match store.count_events(workflow_id).await {
            Ok(count) => count as i32,
            Err(StoreError::WorkflowNotFound(_)) if attempt < MAX_RETRY_ATTEMPTS - 1 => {
                // Workflow might not be visible yet due to race condition, retry
                debug!(
                    %workflow_id,
                    attempt = attempt + 1,
                    "Workflow not found when counting events, retrying"
                );
                tokio::time::sleep(std::time::Duration::from_millis(10 * (attempt as u64 + 1)))
                    .await;
                continue;
            }
            Err(e) => {
                warn!(%workflow_id, error = %e, "Failed to count events for workflow");
                return Err(e);
            }
        };

        match store
            .append_events(workflow_id, current_seq, vec![event.clone()])
            .await
        {
            Ok(_seq) => {
                debug!(
                    %workflow_id,
                    event_type = ?std::mem::discriminant(&event),
                    "Appended workflow event"
                );
                return Ok(());
            }
            Err(StoreError::ConcurrencyConflict { expected, actual }) => {
                debug!(
                    %workflow_id,
                    attempt = attempt + 1,
                    expected,
                    actual,
                    "Concurrency conflict appending event, retrying"
                );
                continue;
            }
            Err(e) => {
                warn!(%workflow_id, error = %e, "Failed to append workflow event");
                return Err(e);
            }
        }
    }

    warn!(
        %workflow_id,
        "Failed to append workflow event after {} retries",
        MAX_RETRY_ATTEMPTS
    );
    Err(StoreError::ConcurrencyConflict {
        expected: -1,
        actual: -1,
    })
}

/// Record that an activity has started execution.
///
/// This should be called when a worker claims a task and begins execution.
/// For standalone tasks (workflow_id is None), this is a no-op.
pub async fn record_activity_started<S: WorkflowEventStore>(
    store: &S,
    workflow_id: Option<Uuid>,
    activity_id: String,
    attempt: u32,
    worker_id: String,
) {
    let Some(wf_id) = workflow_id else {
        return; // Standalone task: no workflow events to record
    };

    let event = WorkflowEvent::ActivityStarted {
        activity_id,
        attempt,
        worker_id,
    };

    if let Err(e) = append_event(store, wf_id, event).await {
        warn!(
            workflow_id = %wf_id,
            error = %e,
            "Failed to record ActivityStarted event"
        );
    }
}

/// Record that an activity has completed successfully.
///
/// This should be called when a worker completes a task with a result.
/// For standalone tasks (workflow_id is None), this is a no-op.
pub async fn record_activity_completed<S: WorkflowEventStore>(
    store: &S,
    workflow_id: Option<Uuid>,
    activity_id: String,
    result: serde_json::Value,
) {
    let Some(wf_id) = workflow_id else {
        return; // Standalone task: no workflow events to record
    };

    let event = WorkflowEvent::ActivityCompleted {
        activity_id,
        result,
    };

    if let Err(e) = append_event(store, wf_id, event).await {
        warn!(
            workflow_id = %wf_id,
            error = %e,
            "Failed to record ActivityCompleted event"
        );
    }
}

/// Record that an activity has failed.
///
/// This should be called when a worker fails a task.
/// For standalone tasks (workflow_id is None), this is a no-op.
pub async fn record_activity_failed<S: WorkflowEventStore + ?Sized>(
    store: &S,
    workflow_id: Option<Uuid>,
    activity_id: String,
    error_message: String,
    will_retry: bool,
) {
    let Some(wf_id) = workflow_id else {
        return; // Standalone task: no workflow events to record
    };

    let error = if will_retry {
        ActivityError::retryable(error_message)
    } else {
        ActivityError::non_retryable(error_message)
    };

    let event = WorkflowEvent::ActivityFailed {
        activity_id,
        error,
        will_retry,
    };

    if let Err(e) = append_event(store, wf_id, event).await {
        warn!(
            workflow_id = %wf_id,
            error = %e,
            "Failed to record ActivityFailed event"
        );
    }
}

/// Record that a workflow has completed.
///
/// This should be called when a workflow finishes execution.
pub async fn record_workflow_completed<S: WorkflowEventStore>(
    store: &S,
    workflow_id: Uuid,
    result: serde_json::Value,
) {
    let event = WorkflowEvent::WorkflowCompleted { result };

    if let Err(e) = append_event(store, workflow_id, event).await {
        warn!(
            %workflow_id,
            error = %e,
            "Failed to record WorkflowCompleted event"
        );
    }
}

/// Record that a workflow has failed.
///
/// This should be called when a workflow fails permanently.
pub async fn record_workflow_failed<S: WorkflowEventStore + ?Sized>(
    store: &S,
    workflow_id: Uuid,
    error_message: String,
) {
    let event = WorkflowEvent::WorkflowFailed {
        error: WorkflowError::new(error_message),
    };

    if let Err(e) = append_event(store, workflow_id, event).await {
        warn!(
            %workflow_id,
            error = %e,
            "Failed to record WorkflowFailed event"
        );
    }
}

/// Record that a workflow has been cancelled.
///
/// This should be called when a workflow is cancelled by user request.
pub async fn record_workflow_cancelled<S: WorkflowEventStore>(
    store: &S,
    workflow_id: Uuid,
    reason: Option<String>,
) {
    let event = WorkflowEvent::WorkflowCancelled {
        reason: reason.unwrap_or_else(|| "Cancelled".to_string()),
    };

    if let Err(e) = append_event(store, workflow_id, event).await {
        warn!(
            %workflow_id,
            error = %e,
            "Failed to record WorkflowCancelled event"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persistence::{EventLog, InMemoryWorkflowEventStore};
    use std::sync::Arc;

    async fn workflow(store: &InMemoryWorkflowEventStore) -> Uuid {
        let id = Uuid::now_v7();
        store
            .create_workflow(id, "test", serde_json::json!({}), None)
            .await
            .unwrap();
        id
    }

    async fn events(store: &InMemoryWorkflowEventStore, id: Uuid) -> Vec<WorkflowEvent> {
        store
            .load_events(id)
            .await
            .unwrap()
            .into_iter()
            .map(|(_, event)| event)
            .collect()
    }

    #[tokio::test]
    async fn test_records_activity_lifecycle_in_order() {
        let store = InMemoryWorkflowEventStore::new();
        let id = workflow(&store).await;

        record_activity_started(&store, Some(id), "a".into(), 1, "w".into()).await;
        record_activity_failed(&store, Some(id), "a".into(), "timeout".into(), true).await;
        record_activity_completed(&store, Some(id), "a".into(), serde_json::json!(7)).await;

        let events = events(&store, id).await;
        assert!(matches!(
            &events[0],
            WorkflowEvent::ActivityStarted { activity_id, attempt: 1, worker_id }
                if activity_id == "a" && worker_id == "w"
        ));
        assert!(matches!(
            &events[1],
            WorkflowEvent::ActivityFailed { error, will_retry: true, .. }
                if error.retryable && error.message == "timeout"
        ));
        assert!(matches!(
            &events[2],
            WorkflowEvent::ActivityCompleted { result, .. } if result == &serde_json::json!(7)
        ));
    }

    #[tokio::test]
    async fn test_final_failure_is_recorded_non_retryable() {
        let store = InMemoryWorkflowEventStore::new();
        let id = workflow(&store).await;

        record_activity_failed(&store, Some(id), "a".into(), "bad input".into(), false).await;

        assert!(matches!(
            &events(&store, id).await[0],
            WorkflowEvent::ActivityFailed { error, will_retry: false, .. } if !error.retryable
        ));
    }

    #[tokio::test]
    async fn test_records_workflow_terminal_events() {
        let store = InMemoryWorkflowEventStore::new();
        let (done, failed, cancelled, cancelled_default) = (
            workflow(&store).await,
            workflow(&store).await,
            workflow(&store).await,
            workflow(&store).await,
        );

        record_workflow_completed(&store, done, serde_json::json!("ok")).await;
        record_workflow_failed(&store, failed, "boom".into()).await;
        record_workflow_cancelled(&store, cancelled, Some("user".into())).await;
        record_workflow_cancelled(&store, cancelled_default, None).await;

        assert!(matches!(
            &events(&store, done).await[0],
            WorkflowEvent::WorkflowCompleted { result } if result == "ok"
        ));
        assert!(matches!(
            &events(&store, failed).await[0],
            WorkflowEvent::WorkflowFailed { error } if error.message == "boom"
        ));
        assert!(matches!(
            &events(&store, cancelled).await[0],
            WorkflowEvent::WorkflowCancelled { reason } if reason == "user"
        ));
        assert!(matches!(
            &events(&store, cancelled_default).await[0],
            WorkflowEvent::WorkflowCancelled { reason } if reason == "Cancelled"
        ));
    }

    #[tokio::test]
    async fn test_standalone_tasks_record_nothing() {
        let store = InMemoryWorkflowEventStore::new();

        // No workflow exists; a `None` workflow id must not even look one up.
        record_activity_started(&store, None, "a".into(), 1, "w".into()).await;
        record_activity_completed(&store, None, "a".into(), serde_json::json!(1)).await;
        record_activity_failed(&store, None, "a".into(), "x".into(), false).await;
    }

    #[tokio::test]
    async fn test_append_event_gives_up_on_missing_workflow() {
        let store = InMemoryWorkflowEventStore::new();
        let missing = Uuid::now_v7();

        let result = append_event(
            &store,
            missing,
            WorkflowEvent::WorkflowCancelled { reason: "x".into() },
        )
        .await;

        assert!(matches!(result, Err(StoreError::WorkflowNotFound(id)) if id == missing));
        // The record_* wrappers log the error instead of propagating it.
        record_workflow_failed(&store, missing, "boom".into()).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn test_concurrent_appends_all_land_in_sequence() {
        let store = Arc::new(InMemoryWorkflowEventStore::new());
        let id = workflow(&store).await;

        let writers: Vec<_> = (0..4)
            .map(|i| {
                let store = Arc::clone(&store);
                tokio::spawn(async move {
                    append_event(
                        store.as_ref(),
                        id,
                        WorkflowEvent::ActivityStarted {
                            activity_id: format!("a{i}"),
                            attempt: 1,
                            worker_id: "w".into(),
                        },
                    )
                    .await
                })
            })
            .collect();
        for writer in writers {
            writer.await.unwrap().unwrap();
        }

        let loaded = store.load_events(id).await.unwrap();
        let sequences: Vec<i32> = loaded.iter().map(|(seq, _)| *seq).collect();
        assert_eq!(sequences, vec![0, 1, 2, 3]);
    }
}

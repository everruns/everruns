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
pub async fn append_event<S: WorkflowEventStore>(
    store: &S,
    workflow_id: Uuid,
    event: WorkflowEvent,
) -> Result<(), StoreError> {
    for attempt in 0..MAX_RETRY_ATTEMPTS {
        // Get current sequence by loading events
        let events = match store.load_events(workflow_id).await {
            Ok(events) => events,
            Err(StoreError::WorkflowNotFound(_)) if attempt < MAX_RETRY_ATTEMPTS - 1 => {
                // Workflow might not be visible yet due to race condition, retry
                debug!(
                    %workflow_id,
                    attempt = attempt + 1,
                    "Workflow not found when loading events, retrying"
                );
                tokio::time::sleep(std::time::Duration::from_millis(10 * (attempt as u64 + 1)))
                    .await;
                continue;
            }
            Err(e) => {
                warn!(%workflow_id, error = %e, "Failed to load events for workflow");
                return Err(e);
            }
        };
        let current_seq = events.len() as i32;

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
pub async fn record_activity_started<S: WorkflowEventStore>(
    store: &S,
    workflow_id: Uuid,
    activity_id: String,
    attempt: u32,
    worker_id: String,
) {
    let event = WorkflowEvent::ActivityStarted {
        activity_id,
        attempt,
        worker_id,
    };

    if let Err(e) = append_event(store, workflow_id, event).await {
        warn!(
            %workflow_id,
            error = %e,
            "Failed to record ActivityStarted event"
        );
    }
}

/// Record that an activity has completed successfully.
///
/// This should be called when a worker completes a task with a result.
pub async fn record_activity_completed<S: WorkflowEventStore>(
    store: &S,
    workflow_id: Uuid,
    activity_id: String,
    result: serde_json::Value,
) {
    let event = WorkflowEvent::ActivityCompleted {
        activity_id,
        result,
    };

    if let Err(e) = append_event(store, workflow_id, event).await {
        warn!(
            %workflow_id,
            error = %e,
            "Failed to record ActivityCompleted event"
        );
    }
}

/// Record that an activity has failed.
///
/// This should be called when a worker fails a task.
pub async fn record_activity_failed<S: WorkflowEventStore>(
    store: &S,
    workflow_id: Uuid,
    activity_id: String,
    error_message: String,
    will_retry: bool,
) {
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

    if let Err(e) = append_event(store, workflow_id, event).await {
        warn!(
            %workflow_id,
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
pub async fn record_workflow_failed<S: WorkflowEventStore>(
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

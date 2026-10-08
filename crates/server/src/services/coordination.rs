//! Thread turn listener for the coordination capability
//! (knowledge/runtime-resources/coordination.md).
//!
//! A thread finishes an assignment by calling `complete_assignment`. When its
//! turn ends without that call, the coordinator would otherwise never hear
//! back, so this listener watches thread turns and keeps the coordinator's
//! assignment honest:
//!
//! - a thread turn that completes or is cancelled while the assignment is still
//!   running flags it as needing attention (`awaiting_input`), which wakes the
//!   coordinator with the reason;
//! - a thread turn that fails fails the assignment;
//! - a thread turn that starts while the assignment waits on an answer puts it
//!   back to running.
//!
//! Decision: the work is spawned off the event path and is best-effort, like
//! run summaries: a missed transition costs a stale status, never a run.
//! Decision: the registry is bound after construction because listeners are
//! built before the event service the registry's waker needs.

use std::sync::{Arc, OnceLock};

use everruns_contracts::typed_id::SessionId;
use everruns_core::session_task::{
    SessionTaskFilter, SessionTaskRegistry, SessionTaskState, SessionTaskUpdate,
    TASK_KIND_ASSIGNMENT, TaskError, TaskInputRequest,
};

use crate::storage::StorageBackend;

const TURN_STARTED: &str = "turn.started";
const TURN_COMPLETED: &str = "turn.completed";
const TURN_FAILED: &str = "turn.failed";
const TURN_CANCELLED: &str = "turn.cancelled";

pub struct ThreadTurnListener {
    db: Arc<StorageBackend>,
    /// Bound once the event service exists (see the module decision).
    registry: OnceLock<Arc<dyn SessionTaskRegistry>>,
}

impl ThreadTurnListener {
    pub fn new(db: Arc<StorageBackend>) -> Self {
        Self {
            db,
            registry: OnceLock::new(),
        }
    }

    /// Bind a registry that emits task events and wakes the coordinator.
    pub fn bind_registry(
        &self,
        db: &Arc<StorageBackend>,
        event_service: &Arc<crate::services::EventService>,
        runner: &Arc<dyn everruns_core::host::TurnBackend>,
    ) {
        let waker = Arc::new(crate::storage::session_task_store::InjectedMessageWaker {
            db: db.clone(),
            event_service: (**event_service).clone(),
            runner: Some(runner.clone()),
        });
        let registry = crate::storage::DbSessionTaskRegistry::new(db.clone())
            .with_event_emitter(event_service.clone())
            .with_waker(waker);
        let _ = self.registry.set(Arc::new(registry));
    }
}

#[async_trait::async_trait]
impl everruns_core::event_listeners::EventListener for ThreadTurnListener {
    fn name(&self) -> &'static str {
        "coordination_thread_turns"
    }

    fn event_types(&self) -> Option<Vec<&'static str>> {
        Some(vec![
            TURN_STARTED,
            TURN_COMPLETED,
            TURN_FAILED,
            TURN_CANCELLED,
        ])
    }

    async fn on_event(&self, event: &everruns_core::Event) {
        let Some(registry) = self.registry.get().cloned() else {
            return;
        };
        let db = self.db.clone();
        let session_id = event.session_id;
        let event_type = event.event_type.clone();
        tokio::spawn(async move {
            if let Err(error) =
                settle_thread_turn(&db, registry.as_ref(), session_id, &event_type).await
            {
                tracing::debug!(%session_id, %error, "coordination: thread turn settle failed");
            }
        });
    }
}

/// Apply one thread turn event to the thread's open assignment, if any.
pub(crate) async fn settle_thread_turn(
    db: &StorageBackend,
    registry: &dyn SessionTaskRegistry,
    thread: SessionId,
    event_type: &str,
) -> anyhow::Result<()> {
    let Some(coordinator) = db
        .get_session_unscoped(thread)
        .await?
        .and_then(|row| row.parent_session_id)
    else {
        return Ok(());
    };
    let Some(assignment) = registry
        .list(
            coordinator,
            Some(&SessionTaskFilter {
                kind: Some(TASK_KIND_ASSIGNMENT.to_string()),
                state: None,
            }),
        )
        .await?
        .into_iter()
        .filter(|task| task.links.child_session_id == Some(thread) && !task.state.is_terminal())
        .max_by_key(|task| task.created_at)
    else {
        return Ok(());
    };

    let update = match (event_type, assignment.state) {
        (TURN_STARTED, SessionTaskState::AwaitingInput) => SessionTaskUpdate {
            state: Some(SessionTaskState::Running),
            ..Default::default()
        },
        (TURN_COMPLETED | TURN_CANCELLED, SessionTaskState::Running | SessionTaskState::Queued) => {
            let how = if event_type == TURN_CANCELLED {
                "was cancelled"
            } else {
                "ended its turn"
            };
            SessionTaskUpdate {
                input_request: Some(TaskInputRequest {
                    id: format!("stopped_{}", uuid::Uuid::now_v7().simple()),
                    prompt: format!(
                        "The thread {how} without completing the assignment. Read its last reply with get_thread, then message it or tell the person."
                    ),
                    expected: None,
                }),
                ..Default::default()
            }
        }
        (TURN_FAILED, _) => SessionTaskUpdate {
            state: Some(SessionTaskState::Failed),
            error: Some(TaskError {
                kind: "turn_failed".to_string(),
                message: "The thread's turn failed.".to_string(),
            }),
            ..Default::default()
        },
        _ => return Ok(()),
    };
    registry
        .update(
            coordinator,
            &assignment.id,
            SessionTaskUpdate {
                expected_attempt: Some(assignment.attempt),
                ..update
            },
        )
        .await?;
    Ok(())
}

#[cfg(test)]
#[path = "coordination_tests.rs"]
mod tests;

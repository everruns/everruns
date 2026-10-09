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
//! The settlement rules live with the capability
//! (`coordination::settle_thread_turn`) so the framework applies the same ones;
//! this listener maps server turn events onto them.
//!
//! Decision: the work is spawned off the event path and is best-effort, like
//! run summaries: a missed transition costs a stale status, never a run.
//! Decision: the registry is bound after construction because listeners are
//! built before the event service the registry's waker needs.

use std::sync::{Arc, OnceLock};

use everruns_capabilities::capabilities::coordination::{self, ThreadTurn};
use everruns_contracts::typed_id::SessionId;
use everruns_core::session_task::SessionTaskRegistry;

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
    let turn = match event_type {
        TURN_STARTED => ThreadTurn::Started,
        TURN_COMPLETED => ThreadTurn::Completed,
        TURN_CANCELLED => ThreadTurn::Cancelled,
        TURN_FAILED => ThreadTurn::Failed,
        _ => return Ok(()),
    };
    let Some(coordinator) = db
        .get_session_unscoped(thread)
        .await?
        .and_then(|row| row.parent_session_id)
    else {
        return Ok(());
    };
    coordination::settle_thread_turn(registry, coordinator, thread, turn).await?;
    Ok(())
}

#[cfg(test)]
mod tests;

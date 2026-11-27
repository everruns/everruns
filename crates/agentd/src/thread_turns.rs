//! A thread's turns settle its parent's assignment on the data plane, as
//! the control plane's thread turn listener does on v1
//! (`crates/server/src/listeners/coordination/mod.rs`): a turn that ends
//! without `finish` flags the assignment as needing attention, which wakes
//! the parent; a failed turn fails it; a turn that starts while it waits on
//! an answer puts it back to running. The rules are the capability's
//! (`threads::settle_thread_turn`), shared with every host.
//!
//! Decisions:
//! - Hooked where a session's events are appended (`hosting.rs`), on the
//!   node or executor that runs the turn, so it needs no feed or listener.
//!   The parent's assignment is reached through the gateway wherever the
//!   parent runs (`ShardTasks::with_parent`).
//! - Best-effort and off the append path, like v1's listener: a missed
//!   transition costs a stale status, never a turn.

use everruns_capabilities::capabilities::threads::{ThreadTurn, settle_thread_turn};
use everruns_contracts::typed_id::SessionId;
use everruns_core::events::Event;

use crate::hosting::Port;
use crate::tasks::ShardTasks;

/// How a turn event moves a thread's assignment, if it is a turn event.
pub(crate) fn thread_turn(event_type: &str) -> Option<ThreadTurn> {
    match event_type {
        everruns_core::TURN_STARTED => Some(ThreadTurn::Started),
        everruns_core::TURN_COMPLETED => Some(ThreadTurn::Completed),
        everruns_core::TURN_CANCELLED => Some(ThreadTurn::Cancelled),
        everruns_core::TURN_FAILED => Some(ThreadTurn::Failed),
        _ => None,
    }
}

/// Settle the assignment of `event`'s session, when it is a thread's turn
/// event.
pub(crate) fn observe(port: Port, event: &Event) {
    let Some(turn) = thread_turn(&event.event_type) else {
        return;
    };
    let thread = event.session_id;
    tokio::spawn(async move {
        let parent: Option<SessionId> = port
            .read(thread, |s| {
                s.platform
                    .origin
                    .as_ref()
                    .and_then(|origin| origin.parent_session_id.parse().ok())
            })
            .await
            .ok()
            .flatten();
        let Some(parent) = parent else {
            return;
        };
        let tasks = ShardTasks::new(port).with_parent(Some(parent));
        if let Err(error) = settle_thread_turn(&tasks, parent, thread, turn).await {
            tracing::debug!(%thread, %error, "thread turn settle failed");
        }
    });
}

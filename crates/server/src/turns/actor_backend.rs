//! The server's turn backend: sessions as actors (actor-based design, step 6c).
//!
//! Decisions:
//! - Starting a turn is marking the session's actor runnable, nothing more.
//!   A worker leases the actor, folds the session's log and runs the whole
//!   turn (crates/worker `actor_runtime`). The server never enqueues a step.
//! - The mark goes through the request pool, so a caller inside a command
//!   transaction (message intake) commits it with the input it persisted: a
//!   worker never sees one without the other, and a rolled-back message
//!   starts nothing.
//! - A stored message marks the inbox at that message's own sequence, so the
//!   worker knows which input a fresh turn starts from. Recorded tool results
//!   mark the inbox at the log's head: they continue the open turn, and every
//!   event the resolution wrote must be folded before the turn goes on.
//! - Cancel has one writer for its events. When a live holder runs the
//!   actor, the request lands on its row and the holder ends the turn and
//!   records it (`true`). Otherwise the actor is withdrawn and the caller
//!   records the cancel as before (`false`). See the repository header.
//! - A cancel that a holder records waits (bounded) until the holder ended
//!   the turn, so a caller that reads the session next sees the turn
//!   cancelled, as with the old runner. Inside a command transaction the
//!   wait runs once the transaction committed (the holder cannot see the
//!   request before), still before the command answers. Past the bound the
//!   answer is "cancel acknowledged": the request stays on the actor row and
//!   the holder, or the next one, records `turn.cancelled`.
//! - Workers hear about new work on the `task_available` channel: a database
//!   trigger notifies on commit under PostgreSQL LISTEN, and under NATS this
//!   backend publishes the same payload once the caller's transaction commits.
//!   The in-process DEV_MODE worker has no channel; this backend wakes it
//!   directly ([`ActorWakeups`]), also after the commit, so a lease never
//!   races the transaction that made the actor runnable.
//! - Every server caller drops the ticket. It still resolves sensibly: once
//!   the actor has nothing left to run, it maps the session's latest terminal
//!   turn event.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use everruns_contracts::error::{AgentLoopError, Result};
use everruns_contracts::typed_id::{SessionId, TurnId};
use everruns_core::host::{TurnBackend, TurnInput, TurnRequest, TurnResult, TurnTicket};
use everruns_core::turn::TurnStopReason;

use everruns_worker::actor_runtime::ActorWakeups;

use crate::live_updates::task_notifications::TaskBroadcaster;
use crate::storage::{ActorCancel, StorageBackend};

/// Wake-up payload for a newly leasable actor.
pub const SESSION_ACTOR_NOTIFICATION: &str = "session_actor";
/// Wake-up payload for a cancel on a leased actor.
pub const SESSION_ACTOR_CANCEL_NOTIFICATION: &str = "session_actor_cancel";

/// How often a ticket re-reads its actor while it waits for the turn to end.
const TICKET_POLL_INTERVAL: Duration = Duration::from_millis(250);

/// How long, by default, a cancel outside a transaction waits for the holder to end the
/// turn: two lease lives, so a holder that died is covered by its expiry.
const CANCEL_WAIT: Duration = Duration::from_secs(20);
const CANCEL_POLL_INTERVAL: Duration = Duration::from_millis(50);

const TERMINAL_TURN_EVENTS: [&str; 4] = [
    "turn.completed",
    "turn.failed",
    "turn.cancelled",
    "turn.sealed",
];

/// [`TurnBackend`] over the session actor index.
pub struct ActorTurnBackend {
    db: Arc<StorageBackend>,
    broadcaster: Option<Arc<TaskBroadcaster>>,
    /// The in-process worker's wake-ups (DEV_MODE).
    local: Option<ActorWakeups>,
    cancel_wait: Duration,
}

impl ActorTurnBackend {
    pub fn new(db: Arc<StorageBackend>, broadcaster: Option<Arc<TaskBroadcaster>>) -> Self {
        Self {
            db,
            broadcaster,
            local: None,
            cancel_wait: CANCEL_WAIT,
        }
    }

    /// Also wake an in-process worker through `wakeups`.
    pub fn with_local_wakeups(mut self, wakeups: ActorWakeups) -> Self {
        self.local = Some(wakeups);
        self
    }

    /// Bound how long a cancel waits for the holder (default [`CANCEL_WAIT`]).
    pub fn with_cancel_wait(mut self, wait: Duration) -> Self {
        self.cancel_wait = wait;
        self
    }

    /// Publish `payload` to workers once the current transaction commits.
    /// PostgreSQL LISTEN needs nothing: the actor table's trigger notifies.
    fn notify(&self, payload: &'static str) {
        if self.broadcaster.is_none() && self.local.is_none() {
            return;
        }
        let broadcaster = self.broadcaster.clone();
        let local = self.local.clone();
        crate::storage::transaction::spawn_after_commit(async move {
            if let Some(local) = local {
                match payload {
                    SESSION_ACTOR_CANCEL_NOTIFICATION => local.cancel_requested(),
                    _ => local.runnable(),
                }
            }
            if let Some(broadcaster) = broadcaster {
                broadcaster.notify_task_available(payload).await;
            }
        });
    }

    fn cancel_waiter(&self) -> CancelWaiter {
        CancelWaiter {
            db: self.db.clone(),
            wait: self.cancel_wait,
        }
    }

    fn ticket(&self, session_id: SessionId, turn_id: TurnId) -> TurnTicket {
        let db = self.db.clone();
        TurnTicket::new(session_id, turn_id, await_turn(db, session_id, turn_id))
    }
}

/// Waits for a holder to record a cancel; owned, so it can run after the
/// caller's transaction commits.
struct CancelWaiter {
    db: Arc<StorageBackend>,
    wait: Duration,
}

impl CancelWaiter {
    /// Wait until the holder ended the cancelled turn and released the actor
    /// (or its lease ran out), bounded by the backend's cancel wait. Past it the cancel
    /// stays pending on the row and the next holder records it.
    async fn await_cancel_recorded(&self, session_id: SessionId) {
        let deadline = tokio::time::Instant::now() + self.wait;
        while tokio::time::Instant::now() < deadline {
            match self.db.get_session_actor(session_id).await {
                Ok(Some(row))
                    if row.cancel_requested_at.is_some()
                        && row.lease_holder.is_some()
                        && row
                            .lease_expires_at
                            .is_some_and(|at| at > chrono::Utc::now()) => {}
                Ok(_) => return,
                Err(error) => {
                    tracing::warn!(session_id = %session_id, error = %error, "Failed to read the cancelled session's actor");
                    return;
                }
            }
            tokio::time::sleep(CANCEL_POLL_INTERVAL).await;
        }
        tracing::warn!(session_id = %session_id, "The session's holder did not record the cancel in time; it stays pending");
    }
}

fn store_error(context: &str, error: anyhow::Error) -> AgentLoopError {
    AgentLoopError::store(format!("{context}: {error:#}"))
}

#[async_trait]
impl TurnBackend for ActorTurnBackend {
    /// Mark the session's actor runnable for the stored input; see the
    /// module notes.
    async fn start_turn(&self, request: TurnRequest) -> Result<TurnTicket> {
        let TurnRequest {
            session_id,
            turn_id,
            input,
            steering,
            scope,
            ..
        } = request;
        // Mid-turn input reaches an actor only as persisted messages.
        steering.close();
        match input {
            TurnInput::StoredMessage { message_id } => {
                let scope = scope.ok_or_else(|| {
                    AgentLoopError::config(
                        "the actor turn backend needs TurnRequest::scope to start a turn \
                         from TurnInput::StoredMessage",
                    )
                })?;
                let input = self
                    .db
                    .find_input_message_event(session_id, message_id)
                    .await
                    .map_err(|e| store_error("Failed to read the input message", e))?
                    .ok_or_else(|| {
                        AgentLoopError::store(format!(
                            "input message {message_id} is not in session {session_id}'s log"
                        ))
                    })?;
                self.db
                    .mark_session_actor_runnable(
                        session_id,
                        scope.org_id,
                        i64::from(input.sequence),
                    )
                    .await
                    .map_err(|e| store_error("Failed to mark the session runnable", e))?;
            }
            TurnInput::RecordedToolResults { .. } => {
                self.db
                    .mark_session_actor_resumable(session_id)
                    .await
                    .map_err(|e| store_error("Failed to mark the session runnable", e))?;
            }
            other => return Err(unsupported_input(&other)),
        }
        self.notify(SESSION_ACTOR_NOTIFICATION);
        Ok(self.ticket(session_id, turn_id))
    }

    /// Ask the running turn to stop. `true` when a live holder runs it and
    /// records the cancel itself; `false` when the caller records it (the
    /// actor was only waiting to run, and is withdrawn, or nothing runs).
    async fn cancel(&self, session_id: SessionId) -> Result<bool> {
        match self
            .db
            .request_session_actor_cancel(session_id)
            .await
            .map_err(|e| store_error("Failed to cancel the session's turn", e))?
        {
            ActorCancel::Requested => {
                self.notify(SESSION_ACTOR_CANCEL_NOTIFICATION);
                // The trait's contract: once the cancel is answered, the turn
                // took its last step. Inside a command transaction the
                // request is not visible to the holder until commit, so the
                // wait runs after the commit, before the command's response
                // (`transaction::after_commit` effects are awaited by the
                // scope that commits). Outside one, it runs now.
                let waiter = self.cancel_waiter();
                crate::storage::transaction::after_commit(async move {
                    waiter.await_cancel_recorded(session_id).await;
                })
                .await;
                Ok(true)
            }
            ActorCancel::Withdrawn | ActorCancel::Idle => Ok(false),
        }
    }

    async fn is_running(&self, session_id: SessionId) -> bool {
        matches!(
            self.db.get_session_actor(session_id).await,
            Ok(Some(row)) if row.runnable_since.is_some() || row.lease_holder.is_some()
        )
    }

    async fn active_count(&self) -> usize {
        self.db
            .count_leased_session_actors()
            .await
            .ok()
            .and_then(|count| usize::try_from(count).ok())
            .unwrap_or_default()
    }
}

fn unsupported_input(input: &TurnInput) -> AgentLoopError {
    let name = match input {
        TurnInput::Message(_) => "TurnInput::Message",
        TurnInput::ResumeInterrupted => "TurnInput::ResumeInterrupted",
        TurnInput::ToolResults(_) => "TurnInput::ToolResults",
        _ => "this turn input",
    };
    AgentLoopError::config(format!(
        "the actor turn backend cannot run {name}; the server records input itself and \
         starts turns only from stored input (TurnInput::StoredMessage, \
         TurnInput::RecordedToolResults)"
    ))
}

/// Wait until the session's actor has nothing left to run, then report the
/// session's latest turn end.
async fn await_turn(
    db: Arc<StorageBackend>,
    session_id: SessionId,
    turn_id: TurnId,
) -> Result<TurnResult> {
    loop {
        let row = db
            .get_session_actor(session_id)
            .await
            .map_err(|e| store_error("Failed to read the session actor", e))?;
        let busy = row
            .as_ref()
            .is_some_and(|row| row.runnable_since.is_some() || row.lease_holder.is_some());
        if !busy {
            break;
        }
        tokio::time::sleep(TICKET_POLL_INTERVAL).await;
    }
    let types = TERMINAL_TURN_EVENTS.map(String::from);
    let latest = db
        .list_events(session_id, None, None, &types, &[], None, Some(1))
        .await
        .map_err(|e| store_error("Failed to read the turn's end", e))?
        .pop();
    let ended = |success: bool, stop_reason: TurnStopReason, error: Option<String>| TurnResult {
        response: String::new(),
        iterations: 0,
        tool_calls_count: 0,
        success,
        error,
        stop_reason,
        turn_id,
    };
    Ok(match latest.as_ref().map(|row| row.event_type.as_str()) {
        Some("turn.cancelled") => return Err(AgentLoopError::Cancelled),
        Some("turn.failed") => ended(false, TurnStopReason::Error, Some("turn failed".into())),
        Some("turn.sealed") => ended(false, TurnStopReason::Error, Some("turn sealed".into())),
        // Completed, or parked waiting for tool results (no terminal event).
        _ => ended(true, TurnStopReason::EndTurn, None),
    })
}

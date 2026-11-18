// Recording Sandbox events (`storage::backend::sandbox_events`).
//
// Decision: recording a cause never fails the action it explains. A failed
// insert is logged and dropped: a pause that happened matters more than the
// row saying why.

use everruns_contracts::typed_id::SessionId;

use crate::storage::StorageBackend;
pub use crate::storage::backend::sandbox_events::NewSandboxEvent;

pub mod actor {
    pub const IDLE_TIMER: &str = "idle_timer";
    pub const IDLE_SWEEP: &str = "idle_sweep";
    pub const RETENTION_SWEEP: &str = "retention_sweep";
    pub const PERSON: &str = "person";
}

pub mod action {
    pub const PAUSE: &str = "pause";
    pub const RESUME: &str = "resume";
    pub const DELETE: &str = "delete";
}

pub mod outcome {
    pub const DONE: &str = "done";
    pub const SKIPPED: &str = "skipped";
    pub const FAILED: &str = "failed";
}

pub mod reason {
    /// Unused for longer than its pinned `idle_after_seconds`.
    pub const IDLE_LIMIT: &str = "idle_limit";
    /// Its session claims a running turn, but the sandbox went unused for
    /// the stale-turn limit (a worker likely died mid-turn).
    pub const STALE_TURN: &str = "stale_turn";
    /// Its session is running a turn.
    pub const SESSION_ACTIVE: &str = "session_active";
    /// The pinned configuration turns idle pause off.
    pub const IDLE_PAUSE_OFF: &str = "idle_pause_off";
    /// Paused past the pinned `delete_after_seconds`.
    pub const RETENTION: &str = "retention";
    /// The pinned idle action is `delete`.
    pub const IDLE_ACTION_DELETE: &str = "idle_action_delete";
    /// The provider call failed.
    pub const PROVIDER_ERROR: &str = "provider_error";
    /// A person asked through the API, UI or CLI.
    pub const REQUESTED: &str = "requested";
}

/// Record `event` on the Session's primary Sandbox; never fails.
pub async fn record(db: &StorageBackend, session_id: SessionId, event: NewSandboxEvent) {
    if let Err(error) = db
        .record_sandbox_event_for_session(session_id.uuid(), &event)
        .await
    {
        tracing::warn!(%session_id, %error, reason = event.reason, "failed to record a sandbox event");
    }
}

/// A pause event (the sweep's and the timer's) with its outcome.
pub fn event(
    actor: &'static str,
    outcome: &'static str,
    reason: &'static str,
    message: impl Into<String>,
) -> NewSandboxEvent {
    NewSandboxEvent::new(actor, action::PAUSE, outcome, reason, message)
}

/// `done` with `message`, or `failed` with the provider's error.
pub fn pause_outcome(
    actor: &'static str,
    why: &'static str,
    message: String,
    result: &Result<(), everruns_core::ToolExecutionResult>,
) -> NewSandboxEvent {
    match result {
        Ok(()) => event(actor, outcome::DONE, why, message),
        Err(err) => event(
            actor,
            outcome::FAILED,
            reason::PROVIDER_ERROR,
            format!("Idle pause failed: {}", short_error(err)),
        ),
    }
}

/// A tool error's text, bounded for an event row.
pub fn short_error(err: &everruns_core::ToolExecutionResult) -> String {
    let text = format!("{err:?}");
    match text.char_indices().nth(500) {
        Some((at, _)) => format!("{}...", &text[..at]),
        None => text,
    }
}

//! Event writes for a session deleted under them (EVE-1235).

use everruns_contracts::error::AgentLoopError;
use everruns_contracts::typed_id::SessionId;

use super::EventService;
use crate::storage::CreateEventRow;

/// Foreign keys an event write fails on once its session is gone: the sequence
/// reservation (`event_sequences`) and the event row itself (`events`).
const SESSION_FOREIGN_KEYS: [&str; 2] =
    ["event_sequences_session_id_fkey", "events_session_id_fkey"];

/// Decision (EVE-1235): a write that lost its session to a concurrent delete is
/// not a storage fault. The worker still finishing a turn emits for a session
/// the user just deleted, and the insert fails the session FK (SQLSTATE 23503,
/// see `storage::repositories::session_delete`). Report it as
/// `SessionNotFound` so both worker transports can stop the turn quietly
/// instead of logging an internal error.
pub(super) fn session_gone(error: anyhow::Error, session_id: Option<SessionId>) -> anyhow::Error {
    let lost_session = crate::errors::violated_foreign_key_constraint(&error)
        .is_some_and(|name| SESSION_FOREIGN_KEYS.contains(&name.as_str()));
    match session_id {
        Some(session_id) if lost_session => AgentLoopError::session_not_found(session_id).into(),
        _ => error,
    }
}

/// The one session all of `rows` belong to, if they share one.
pub(super) fn single_session(rows: &[CreateEventRow]) -> Option<SessionId> {
    let first = rows.first()?.session_id;
    rows.iter()
        .all(|row| row.session_id == first)
        .then_some(first)
}

impl EventService {
    /// The session an emit failed for because it was deleted, if that is why
    /// it failed.
    pub fn deleted_session(error: &anyhow::Error) -> Option<SessionId> {
        error
            .chain()
            .find_map(|cause| match cause.downcast_ref::<AgentLoopError>() {
                Some(AgentLoopError::SessionNotFound(session_id)) => Some(*session_id),
                _ => None,
            })
    }

    /// An emit failure as a worker adapter reports it: a deleted session as
    /// `SessionNotFound`, anything else as an opaque store error, logged here.
    pub fn worker_emit_error(error: anyhow::Error) -> AgentLoopError {
        if let Some(session_id) = Self::deleted_session(&error) {
            tracing::debug!(%session_id, "event not stored: session was deleted");
            return AgentLoopError::session_not_found(session_id);
        }
        tracing::error!("Failed to emit event: {error}");
        AgentLoopError::store("Failed to emit event")
    }
}

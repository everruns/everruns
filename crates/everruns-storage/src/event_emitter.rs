//! Database-backed EventEmitter implementation
//!
//! This module implements the EventEmitter trait using the events table
//! as the storage backend. Events are stored with auto-incrementing sequence
//! numbers per session, enabling SSE streaming and event replay.

use async_trait::async_trait;
use everruns_core::{atoms::AtomEvent, traits::EventEmitter, AgentLoopError, Result};

use crate::models::CreateEventRow;
use crate::repositories::Database;

// ============================================================================
// DbEventEmitter - Stores events in the database
// ============================================================================

/// Database-backed event emitter
///
/// Stores atom lifecycle events in the events table.
/// Events are stored with auto-incrementing sequence numbers per session,
/// enabling SSE streaming and event replay.
#[derive(Clone)]
pub struct DbEventEmitter {
    db: Database,
}

impl DbEventEmitter {
    pub fn new(db: Database) -> Self {
        Self { db }
    }
}

#[async_trait]
impl EventEmitter for DbEventEmitter {
    async fn emit(&self, event: AtomEvent) -> Result<i32> {
        let session_id = event.session_id();
        let event_type = event.event_type().to_string();
        let data = event.to_json();

        let event_row = self
            .db
            .create_event(CreateEventRow {
                session_id,
                event_type,
                data,
            })
            .await
            .map_err(|e| AgentLoopError::store(e.to_string()))?;

        Ok(event_row.sequence)
    }
}

// ============================================================================
// Factory functions
// ============================================================================

/// Create a database-backed event emitter
pub fn create_db_event_emitter(db: Database) -> DbEventEmitter {
    DbEventEmitter::new(db)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_core::atoms::{AtomContext, InputStartedEvent};
    use uuid::Uuid;

    // Note: Integration tests would require a database connection.
    // Unit tests focus on the event conversion logic.

    #[test]
    fn test_atom_event_serialization() {
        let context = AtomContext::new(Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
        let event = AtomEvent::InputStarted(InputStartedEvent::new(context.clone()));

        let json = event.to_json();

        assert!(json.is_object());
        assert_eq!(json["type"], "input_started");
        assert!(json["context"].is_object());
    }

    #[test]
    fn test_atom_event_type() {
        let context = AtomContext::new(Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
        let event = AtomEvent::InputStarted(InputStartedEvent::new(context));

        assert_eq!(event.event_type(), "input.started");
    }

    #[test]
    fn test_atom_event_session_id() {
        let session_id = Uuid::now_v7();
        let context = AtomContext::new(session_id, Uuid::now_v7(), Uuid::now_v7());
        let event = AtomEvent::InputStarted(InputStartedEvent::new(context));

        assert_eq!(event.session_id(), session_id);
    }
}

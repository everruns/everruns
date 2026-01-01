//! Database-backed EventEmitter implementation
//!
//! This module implements the EventEmitter trait using the events table
//! as the storage backend. Events are stored with auto-incrementing sequence
//! numbers per session, enabling SSE streaming and event replay.

use async_trait::async_trait;
use everruns_core::{traits::EventEmitter, AgentLoopError, Event, Result};

use crate::models::CreateEventRow;
use crate::repositories::Database;

// ============================================================================
// DbEventEmitter - Stores events in the database
// ============================================================================

/// Database-backed event emitter
///
/// Stores events in the events table following the standard event protocol.
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
    async fn emit(&self, event: Event) -> Result<i32> {
        let session_id = event.session_id();
        let event_type = event.event_type.clone();

        // Serialize the full event to JSON for storage
        let data = serde_json::to_value(&event)
            .map_err(|e| AgentLoopError::store(format!("Failed to serialize event: {}", e)))?;

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
    use everruns_core::event::{EventContext, InputReceivedData, INPUT_RECEIVED};
    use everruns_core::message::Message;
    use everruns_core::Event;
    use uuid::Uuid;

    #[test]
    fn test_event_serialization() {
        let session_id = Uuid::now_v7();
        let event_context = EventContext::session(session_id);
        let event = Event::new(
            INPUT_RECEIVED,
            event_context,
            InputReceivedData::new(Message::user("test")),
        );

        let json = serde_json::to_value(&event).unwrap();

        assert!(json.is_object());
        assert_eq!(json["type"], "input.received");
        assert!(json["context"].is_object());
        assert_eq!(json["context"]["session_id"], session_id.to_string());
    }

    #[test]
    fn test_event_type() {
        let session_id = Uuid::now_v7();
        let event_context = EventContext::session(session_id);
        let event = Event::new(
            INPUT_RECEIVED,
            event_context,
            InputReceivedData::new(Message::user("test")),
        );

        assert_eq!(event.event_type, "input.received");
    }

    #[test]
    fn test_event_session_id() {
        let session_id = Uuid::now_v7();
        let event_context = EventContext::session(session_id);
        let event = Event::new(
            INPUT_RECEIVED,
            event_context,
            InputReceivedData::new(Message::user("test")),
        );

        assert_eq!(event.session_id(), session_id);
    }
}

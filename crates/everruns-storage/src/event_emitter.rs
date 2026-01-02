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
    use everruns_core::events::{EventContext, InputReceivedData, ToolCallCompletedData};
    use everruns_core::message::Message;
    use everruns_core::{ContentPart, Event};
    use uuid::Uuid;

    #[test]
    fn test_event_serialization() {
        let session_id = Uuid::now_v7();
        let event_context = EventContext::empty();
        let event = Event::new(
            session_id,
            event_context,
            InputReceivedData::new(Message::user("test")),
        );

        let json = serde_json::to_value(&event).unwrap();

        assert!(json.is_object());
        assert_eq!(json["type"], "input.received");
        assert_eq!(json["session_id"], session_id.to_string());
        assert!(json["context"].is_object());
    }

    #[test]
    fn test_event_type() {
        let session_id = Uuid::now_v7();
        let event_context = EventContext::empty();
        let event = Event::new(
            session_id,
            event_context,
            InputReceivedData::new(Message::user("test")),
        );

        assert_eq!(event.event_type, "input.received");
    }

    #[test]
    fn test_event_session_id() {
        let session_id = Uuid::now_v7();
        let event_context = EventContext::empty();
        let event = Event::new(
            session_id,
            event_context,
            InputReceivedData::new(Message::user("test")),
        );

        assert_eq!(event.session_id(), session_id);
    }

    #[test]
    fn test_tool_call_completed_event_serialization() {
        // This test verifies the exact JSON structure that the UI expects
        let session_id = Uuid::now_v7();
        let completed = ToolCallCompletedData::success(
            "call_abc123".to_string(),
            "get_weather".to_string(),
            vec![ContentPart::text("Sunny, 72°F")],
        );
        let event = Event::new(session_id, EventContext::empty(), completed);

        let json = serde_json::to_value(&event).unwrap();
        println!(
            "tool.call_completed event JSON:\n{}",
            serde_json::to_string_pretty(&json).unwrap()
        );

        // Verify top-level structure
        assert_eq!(json["type"], "tool.call_completed");
        assert_eq!(json["session_id"], session_id.to_string());

        // Verify data field contains the payload directly (untagged)
        let data = &json["data"];
        assert_eq!(data["tool_call_id"], "call_abc123");
        assert_eq!(data["tool_name"], "get_weather");
        assert_eq!(data["success"], true);
        assert_eq!(data["status"], "success");

        // Verify result is an array of ContentPart
        let result = &data["result"];
        assert!(result.is_array());
        assert_eq!(result[0]["type"], "text");
        assert_eq!(result[0]["text"], "Sunny, 72°F");
    }

    #[test]
    fn test_tool_call_completed_error_serialization() {
        let session_id = Uuid::now_v7();
        let completed = ToolCallCompletedData::failure(
            "call_xyz789".to_string(),
            "read_file".to_string(),
            "error".to_string(),
            "File not found".to_string(),
        );
        let event = Event::new(session_id, EventContext::empty(), completed);

        let json = serde_json::to_value(&event).unwrap();
        println!(
            "tool.call_completed error event JSON:\n{}",
            serde_json::to_string_pretty(&json).unwrap()
        );

        let data = &json["data"];
        assert_eq!(data["tool_call_id"], "call_xyz789");
        assert_eq!(data["success"], false);
        assert_eq!(data["error"], "File not found");
    }
}

// Event-based MessageStore implementation
//
// This module implements the core MessageStore trait using the events table
// as the sole source of truth for conversation messages.
//
// Messages are stored as typed events and reconstructed from the event data when loaded.

use async_trait::async_trait;
use chrono::Utc;
use everruns_core::{
    events::{
        EventContext, EventRequest, MessageAgentData, MessageUserData, ToolCallCompletedData,
    },
    traits::{InputMessage, MessageStore},
    AgentLoopError, ContentPart, Event, EventData, Message, MessageRole, Result,
};
use std::sync::Arc;
use uuid::Uuid;

use super::StorageBackend;
use crate::EventService;

// ============================================================================
// DbMessageStore - Stores messages as events
// ============================================================================

/// Event-based message store
///
/// Stores conversation messages as typed events in the events table.
/// Used by activities to load/store messages during workflow execution.
#[derive(Clone)]
pub struct DbMessageStore {
    db: Arc<StorageBackend>,
    event_service: EventService,
}

impl DbMessageStore {
    pub fn new(db: Arc<StorageBackend>) -> Self {
        let event_service = EventService::new(db.clone());
        Self { db, event_service }
    }
}

#[async_trait]
impl MessageStore for DbMessageStore {
    async fn add(&self, session_id: Uuid, input: InputMessage) -> Result<Message> {
        // Create the message
        let message = Message {
            id: Uuid::now_v7(),
            role: input.role,
            content: input.content,
            controls: input.controls,
            metadata: input.metadata,
            created_at: Utc::now(),
        };

        // Emit as typed event based on role
        let event_request = match message.role {
            MessageRole::User => EventRequest::new(
                session_id,
                EventContext::empty(),
                MessageUserData::new(message.clone()),
            ),
            MessageRole::Assistant => EventRequest::new(
                session_id,
                EventContext::empty(),
                MessageAgentData::new(message.clone()),
            ),
            // System and ToolResult messages are not stored as separate events
            MessageRole::System | MessageRole::ToolResult => {
                return Ok(message);
            }
        };

        self.event_service
            .emit(event_request)
            .await
            .map_err(|e| AgentLoopError::store(e.to_string()))?;

        Ok(message)
    }

    async fn get(&self, session_id: Uuid, message_id: Uuid) -> Result<Option<Message>> {
        let messages = self.load(session_id).await?;
        Ok(messages.into_iter().find(|m| m.id == message_id))
    }

    async fn store(&self, session_id: Uuid, message: Message) -> Result<()> {
        // Only store assistant messages (user messages go through add())
        // Tool results are stored as tool.call_completed events by ActAtom
        if message.role != MessageRole::Assistant {
            return Ok(());
        }

        let event_request = EventRequest::new(
            session_id,
            EventContext::empty(),
            MessageAgentData::new(message),
        );

        self.event_service
            .emit(event_request)
            .await
            .map_err(|e| AgentLoopError::store(e.to_string()))?;

        Ok(())
    }

    async fn load(&self, session_id: Uuid) -> Result<Vec<Message>> {
        let events = self
            .db
            .list_message_events(session_id)
            .await
            .map_err(|e| AgentLoopError::store(e.to_string()))?;

        let mut messages = Vec::with_capacity(events.len());

        for event_row in events {
            match event_to_message(&event_row.data, &event_row.event_type) {
                Ok(message) => messages.push(message),
                Err(e) => {
                    tracing::warn!("Failed to parse message from event {}: {}", event_row.id, e);
                }
            }
        }

        Ok(messages)
    }

    async fn count(&self, session_id: Uuid) -> Result<usize> {
        let events = self
            .db
            .list_message_events(session_id)
            .await
            .map_err(|e| AgentLoopError::store(e.to_string()))?;
        Ok(events.len())
    }
}

// ============================================================================
// Event Parsing
// ============================================================================

/// Convert stored event data to a Message
///
/// Handles two formats:
/// - Legacy format: full Event struct with id, type, data, etc.
/// - New format: EventData directly (MessageUserData, MessageAgentData, etc.)
fn event_to_message(
    data: &serde_json::Value,
    event_type: &str,
) -> std::result::Result<Message, String> {
    // Helper to extract message from EventData
    let extract_message = |event_data: EventData| -> std::result::Result<Message, String> {
        match event_data {
            EventData::MessageUser(d) => Ok(d.message),
            EventData::MessageAgent(d) => Ok(d.message),
            EventData::ToolCallCompleted(d) => Ok(tool_call_to_message(d)),
            _ => Err(format!("unexpected event type for message: {}", event_type)),
        }
    };

    // First try to parse as full Event (legacy format)
    // This has required fields like id, type, session_id, data
    if let Ok(event) = serde_json::from_value::<Event>(data.clone()) {
        return extract_message(event.data);
    }

    // Fallback: try to parse as EventData directly (new format)
    // Note: EventData has #[serde(untagged)] with Raw variant, so we use type hint
    match event_type {
        "message.user" => {
            let d: MessageUserData = serde_json::from_value(data.clone())
                .map_err(|e| format!("invalid message.user data: {}", e))?;
            Ok(d.message)
        }
        "message.agent" => {
            let d: MessageAgentData = serde_json::from_value(data.clone())
                .map_err(|e| format!("invalid message.agent data: {}", e))?;
            Ok(d.message)
        }
        "tool.call_completed" => {
            let d: ToolCallCompletedData = serde_json::from_value(data.clone())
                .map_err(|e| format!("invalid tool.call_completed data: {}", e))?;
            Ok(tool_call_to_message(d))
        }
        _ => Err(format!("unexpected event type for message: {}", event_type)),
    }
}

/// Convert ToolCallCompletedData to a ToolResult message
fn tool_call_to_message(data: ToolCallCompletedData) -> Message {
    // Extract result as JSON value
    let result: Option<serde_json::Value> = data.result.map(|parts| {
        // For simple text results, extract just the text
        if parts.len() == 1 {
            if let ContentPart::Text(t) = &parts[0] {
                return serde_json::Value::String(t.text.clone());
            }
        }
        serde_json::to_value(&parts).unwrap_or_default()
    });

    Message::tool_result(&data.tool_call_id, result, data.error)
}

// ============================================================================
// Factory
// ============================================================================

/// Create a database-backed message store
pub fn create_db_message_store(db: Arc<StorageBackend>) -> DbMessageStore {
    DbMessageStore::new(db)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use everruns_core::events::EventContext;
    use everruns_core::{ContentPart, Event, ToolCall, ToolCallCompletedData};
    use serde_json::json;

    use super::*;

    // ========================================================================
    // Test: Message constructors
    // ========================================================================

    #[test]
    fn test_user_message_content() {
        let message = Message::user("Hello, world!");
        assert_eq!(message.content.len(), 1);
        assert!(matches!(&message.content[0], ContentPart::Text(t) if t.text == "Hello, world!"));
    }

    #[test]
    fn test_assistant_message_content() {
        let message = Message::assistant("I can help you with that.");
        assert_eq!(message.content.len(), 1);
        assert!(
            matches!(&message.content[0], ContentPart::Text(t) if t.text == "I can help you with that.")
        );
    }

    #[test]
    fn test_assistant_with_tool_calls() {
        let tool_calls = vec![
            ToolCall {
                id: "call_1".to_string(),
                name: "get_weather".to_string(),
                arguments: json!({"city": "Tokyo"}),
            },
            ToolCall {
                id: "call_2".to_string(),
                name: "get_weather".to_string(),
                arguments: json!({"city": "London"}),
            },
        ];
        let message = Message::assistant_with_tools("Checking weather...", tool_calls);

        assert_eq!(message.content.len(), 3); // 1 text + 2 tool calls
        assert!(matches!(&message.content[0], ContentPart::Text(_)));
        assert!(matches!(&message.content[1], ContentPart::ToolCall(tc) if tc.id == "call_1"));
        assert!(matches!(&message.content[2], ContentPart::ToolCall(tc) if tc.id == "call_2"));
    }

    #[test]
    fn test_tool_result_success() {
        let message = Message::tool_result("call_123", Some(json!({"temperature": 72})), None);

        assert_eq!(message.role, MessageRole::ToolResult);
        assert_eq!(message.tool_call_id(), Some("call_123"));
    }

    #[test]
    fn test_tool_result_error() {
        let message = Message::tool_result("call_fail", None, Some("Division by zero".to_string()));

        if let ContentPart::ToolResult(tr) = &message.content[0] {
            assert!(tr.result.is_none());
            assert_eq!(tr.error, Some("Division by zero".to_string()));
        } else {
            panic!("Expected ToolResult content part");
        }
    }

    // ========================================================================
    // Test: Message helpers
    // ========================================================================

    #[test]
    fn test_message_text_helper() {
        let message = Message::user("Hello!");
        assert_eq!(message.text(), Some("Hello!"));
    }

    #[test]
    fn test_message_tool_calls_helper() {
        let tool_calls = vec![
            ToolCall {
                id: "call_1".to_string(),
                name: "func1".to_string(),
                arguments: json!({}),
            },
            ToolCall {
                id: "call_2".to_string(),
                name: "func2".to_string(),
                arguments: json!({}),
            },
        ];
        let message = Message::assistant_with_tools("", tool_calls);

        let extracted = message.tool_calls();
        assert_eq!(extracted.len(), 2);
        assert_eq!(extracted[0].id, "call_1");
        assert_eq!(extracted[1].id, "call_2");
    }

    #[test]
    fn test_message_has_tool_calls() {
        let without_tools = Message::assistant("Just text");
        assert!(!without_tools.has_tool_calls());

        let with_tools = Message::assistant_with_tools(
            "With tools",
            vec![ToolCall {
                id: "call_1".to_string(),
                name: "func".to_string(),
                arguments: json!({}),
            }],
        );
        assert!(with_tools.has_tool_calls());
    }

    // ========================================================================
    // Test: Event to Message parsing
    // ========================================================================

    #[test]
    fn test_parse_message_user_event() {
        let session_id = Uuid::now_v7();
        let message = Message::user("Hello from user!");
        let event = Event::new(
            session_id,
            EventContext::empty(),
            MessageUserData::new(message),
        );

        let stored = serde_json::to_value(&event).unwrap();
        let result = event_to_message(&stored, "message.user");

        assert!(result.is_ok());
        let parsed = result.unwrap();
        assert_eq!(parsed.role, MessageRole::User);
        assert_eq!(parsed.text(), Some("Hello from user!"));
    }

    #[test]
    fn test_parse_message_agent_event() {
        let session_id = Uuid::now_v7();
        let message = Message::assistant("Hello from agent!");
        let event = Event::new(
            session_id,
            EventContext::empty(),
            MessageAgentData::new(message),
        );

        let stored = serde_json::to_value(&event).unwrap();
        let result = event_to_message(&stored, "message.agent");

        assert!(result.is_ok());
        let parsed = result.unwrap();
        assert_eq!(parsed.role, MessageRole::Assistant);
        assert_eq!(parsed.text(), Some("Hello from agent!"));
    }

    #[test]
    fn test_parse_tool_call_completed_event() {
        let session_id = Uuid::now_v7();
        let completed = ToolCallCompletedData::success(
            "call_123".to_string(),
            "get_weather".to_string(),
            vec![ContentPart::text("Sunny, 72°F")],
        );
        let event = Event::new(session_id, EventContext::empty(), completed);

        let stored = serde_json::to_value(&event).unwrap();
        let result = event_to_message(&stored, "tool.call_completed");

        assert!(result.is_ok());
        let parsed = result.unwrap();
        assert_eq!(parsed.role, MessageRole::ToolResult);
        assert_eq!(parsed.tool_call_id(), Some("call_123"));
    }

    #[test]
    fn test_parse_tool_call_completed_error() {
        let session_id = Uuid::now_v7();
        let completed = ToolCallCompletedData::failure(
            "call_456".to_string(),
            "read_file".to_string(),
            "error".to_string(),
            "File not found".to_string(),
        );
        let event = Event::new(session_id, EventContext::empty(), completed);

        let stored = serde_json::to_value(&event).unwrap();
        let result = event_to_message(&stored, "tool.call_completed");

        assert!(result.is_ok());
        let parsed = result.unwrap();
        assert_eq!(parsed.role, MessageRole::ToolResult);

        if let ContentPart::ToolResult(tr) = &parsed.content[0] {
            assert_eq!(tr.error.as_deref(), Some("File not found"));
        } else {
            panic!("Expected ToolResult content part");
        }
    }

    #[test]
    fn test_parse_agent_message_with_tool_calls() {
        let session_id = Uuid::now_v7();
        let message = Message::assistant_with_tools(
            "Let me search for that",
            vec![ToolCall {
                id: "call_search".to_string(),
                name: "search".to_string(),
                arguments: json!({"query": "rust"}),
            }],
        );
        let event = Event::new(
            session_id,
            EventContext::empty(),
            MessageAgentData::new(message),
        );

        let stored = serde_json::to_value(&event).unwrap();
        let result = event_to_message(&stored, "message.agent");

        assert!(result.is_ok());
        let parsed = result.unwrap();
        assert!(parsed.has_tool_calls());
        assert_eq!(parsed.tool_calls().len(), 1);
        assert_eq!(parsed.tool_calls()[0].name, "search");
    }
}

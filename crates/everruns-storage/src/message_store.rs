// Event-based MessageStore implementation
//
// This module implements the core MessageStore trait using the events table
// as the sole source of truth for conversation messages.
//
// Messages are stored as events with type "message.*" and reconstructed
// from the event data when loaded.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use everruns_core::{
    traits::{InputMessage, MessageStore},
    AgentLoopError, ContentPart, Controls, Message, MessageRole, Result,
};
use std::collections::HashMap;
use uuid::Uuid;

use crate::models::CreateEventRow;
use crate::repositories::Database;

// ============================================================================
// DbMessageStore - Stores messages as events
// ============================================================================

/// Event-based message store
///
/// Stores conversation messages as events in the events table.
/// Used by activities to load/store messages during workflow execution.
#[derive(Clone)]
pub struct DbMessageStore {
    db: Database,
}

impl DbMessageStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }
}

#[async_trait]
impl MessageStore for DbMessageStore {
    async fn add(&self, session_id: Uuid, input: InputMessage) -> Result<Message> {
        // Generate a new message ID
        let message_id = Uuid::now_v7();
        let created_at = Utc::now();

        // Determine event type from message role
        let event_type = match input.role {
            MessageRole::User => "message.user",
            MessageRole::Assistant => "message.agent",
            MessageRole::ToolResult => "message.tool_result",
            MessageRole::System => "message.system",
        };

        let event_input = CreateEventRow {
            session_id,
            event_type: event_type.to_string(),
            data: serde_json::json!({
                "message_id": message_id,
                "role": input.role.to_string(),
                "content": input.content,
                "controls": input.controls,
                "metadata": input.metadata,
                "tags": input.tags,
            }),
        };

        self.db
            .create_event(event_input)
            .await
            .map_err(|e| AgentLoopError::store(e.to_string()))?;

        // Return the created message
        Ok(Message {
            id: message_id,
            role: input.role,
            content: input.content,
            controls: input.controls,
            metadata: input.metadata,
            created_at,
        })
    }

    async fn get(&self, session_id: Uuid, message_id: Uuid) -> Result<Option<Message>> {
        // Load all messages and find the one with the matching ID
        // This is not the most efficient approach, but it works with the current event-based storage
        // A more efficient approach would be to query by message_id directly
        let messages = self.load(session_id).await?;
        Ok(messages.into_iter().find(|m| m.id == message_id))
    }

    async fn store(&self, session_id: Uuid, message: Message) -> Result<()> {
        // Determine event type from message role
        // Note: user messages are handled by MessageService in the API layer
        // System messages are not emitted as events (per design decision)
        // Tool calls are embedded in assistant messages via ContentPart::ToolCall
        let event_type = match message.role {
            MessageRole::Assistant => Some("message.agent"),
            MessageRole::ToolResult => Some("message.tool_result"),
            MessageRole::User | MessageRole::System => None,
        };

        if let Some(event_type) = event_type {
            // Generate a new message ID
            let message_id = Uuid::now_v7();

            let event_input = CreateEventRow {
                session_id,
                event_type: event_type.to_string(),
                data: serde_json::json!({
                    "message_id": message_id,
                    "role": message.role.to_string(),
                    "content": message.content,
                    "controls": message.controls,
                    "metadata": message.metadata,
                    "tags": [],
                }),
            };

            self.db
                .create_event(event_input)
                .await
                .map_err(|e| AgentLoopError::store(e.to_string()))?;
        }

        Ok(())
    }

    async fn load(&self, session_id: Uuid) -> Result<Vec<Message>> {
        let events = self
            .db
            .list_message_events(session_id)
            .await
            .map_err(|e| AgentLoopError::store(e.to_string()))?;

        let mut messages = Vec::with_capacity(events.len());

        for event in events {
            match event_to_message(&event.data, event.created_at) {
                Ok(message) => messages.push(message),
                Err(e) => {
                    tracing::warn!("Failed to parse message from event {}: {}", event.id, e);
                    // Skip malformed events rather than failing the entire load
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

/// Convert event data to a Message
///
/// Handles multiple formats:
/// - Message events: { "message": { "id", "role", "content", ... } }
/// - Legacy message format: { "message_id", "role", "content", ... }
/// - Tool call completed: { "tool_call_id", "result", "error", ... }
fn event_to_message(
    data: &serde_json::Value,
    created_at: DateTime<Utc>,
) -> std::result::Result<Message, String> {
    // Try new format first (message wrapper)
    if let Some(message_obj) = data.get("message") {
        return parse_message_object(message_obj, created_at);
    }

    // Check if this is a tool.call_completed event (has tool_call_id at top level)
    if data.get("tool_call_id").is_some() {
        return parse_tool_call_completed(data, created_at);
    }

    // Fall back to legacy format (flat structure)
    parse_legacy_format(data, created_at)
}

/// Parse tool.call_completed event into a ToolResult message
fn parse_tool_call_completed(
    data: &serde_json::Value,
    _created_at: DateTime<Utc>,
) -> std::result::Result<Message, String> {
    let tool_call_id = data
        .get("tool_call_id")
        .and_then(|v| v.as_str())
        .ok_or("missing tool_call_id")?
        .to_string();

    // Extract result or error
    let result: Option<serde_json::Value> = data
        .get("result")
        .filter(|v| !v.is_null())
        .cloned()
        .map(|v| {
            // Result is Vec<ContentPart>, convert to JSON value
            // For simple text results, extract the text
            if let Some(arr) = v.as_array() {
                if arr.len() == 1 {
                    if let Some(text) = arr[0].get("text") {
                        return text.clone();
                    }
                }
            }
            v
        });

    let error: Option<String> = data
        .get("error")
        .filter(|v| !v.is_null())
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    Ok(Message::tool_result(&tool_call_id, result, error))
}

/// Parse message from new format with message wrapper
fn parse_message_object(
    message: &serde_json::Value,
    created_at: DateTime<Utc>,
) -> std::result::Result<Message, String> {
    // Extract id (can be string or UUID directly)
    let id = message
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or("missing message.id")?
        .parse::<Uuid>()
        .map_err(|e| format!("invalid message.id: {}", e))?;

    // Extract role
    let role_str = message
        .get("role")
        .and_then(|v| v.as_str())
        .ok_or("missing message.role")?;
    let role = MessageRole::from(role_str);

    // Extract content
    let content: Vec<ContentPart> = message
        .get("content")
        .cloned()
        .map(|v| serde_json::from_value(v).unwrap_or_default())
        .unwrap_or_default();

    // Extract optional controls
    let controls: Option<Controls> = message
        .get("controls")
        .filter(|v| !v.is_null())
        .cloned()
        .and_then(|v| serde_json::from_value(v).ok());

    // Extract optional metadata
    let metadata: Option<HashMap<String, serde_json::Value>> = message
        .get("metadata")
        .filter(|v| !v.is_null())
        .cloned()
        .and_then(|v| serde_json::from_value(v).ok());

    // Use created_at from message if present, otherwise from event
    let msg_created_at = message
        .get("created_at")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<DateTime<Utc>>().ok())
        .unwrap_or(created_at);

    Ok(Message {
        id,
        role,
        content,
        controls,
        metadata,
        created_at: msg_created_at,
    })
}

/// Parse message from legacy format (flat structure)
fn parse_legacy_format(
    data: &serde_json::Value,
    created_at: DateTime<Utc>,
) -> std::result::Result<Message, String> {
    // Extract message_id
    let id = data
        .get("message_id")
        .and_then(|v| v.as_str())
        .ok_or("missing message_id")?
        .parse::<Uuid>()
        .map_err(|e| format!("invalid message_id: {}", e))?;

    // Extract role
    let role_str = data
        .get("role")
        .and_then(|v| v.as_str())
        .ok_or("missing role")?;
    let role = MessageRole::from(role_str);

    // Extract content
    let content: Vec<ContentPart> = data
        .get("content")
        .cloned()
        .map(|v| serde_json::from_value(v).unwrap_or_default())
        .unwrap_or_default();

    // Extract optional controls
    let controls: Option<Controls> = data
        .get("controls")
        .filter(|v| !v.is_null())
        .cloned()
        .and_then(|v| serde_json::from_value(v).ok());

    // Extract optional metadata
    let metadata: Option<HashMap<String, serde_json::Value>> = data
        .get("metadata")
        .filter(|v| !v.is_null())
        .cloned()
        .and_then(|v| serde_json::from_value(v).ok());

    Ok(Message {
        id,
        role,
        content,
        controls,
        metadata,
        created_at,
    })
}

// ============================================================================
// Factory functions
// ============================================================================

/// Create an event-based message store
pub fn create_db_message_store(db: Database) -> DbMessageStore {
    DbMessageStore::new(db)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use everruns_core::{ContentPart, ToolCall};
    use serde_json::json;

    use super::*;

    // ========================================================================
    // Test: Message constructors create correct Vec<ContentPart>
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
    fn test_system_message_content() {
        let message = Message::system("You are a helpful assistant.");

        assert_eq!(message.content.len(), 1);
        assert!(
            matches!(&message.content[0], ContentPart::Text(t) if t.text == "You are a helpful assistant.")
        );
    }

    #[test]
    fn test_assistant_with_single_tool_call() {
        let tool_call = ToolCall {
            id: "call_abc123".to_string(),
            name: "get_weather".to_string(),
            arguments: json!({"city": "Tokyo"}),
        };
        let message = Message::assistant_with_tools("Let me check the weather.", vec![tool_call]);

        // Should have text part and tool call part
        assert_eq!(message.content.len(), 2);
        assert!(
            matches!(&message.content[0], ContentPart::Text(t) if t.text == "Let me check the weather.")
        );
        assert!(
            matches!(&message.content[1], ContentPart::ToolCall(tc) if tc.id == "call_abc123" && tc.name == "get_weather")
        );
    }

    #[test]
    fn test_assistant_with_multiple_tool_calls() {
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
            ToolCall {
                id: "call_3".to_string(),
                name: "get_weather".to_string(),
                arguments: json!({"city": "New York"}),
            },
        ];
        let message = Message::assistant_with_tools(
            "Let me check the weather for all three cities.",
            tool_calls,
        );

        // Should have 1 text + 3 tool calls = 4 parts
        assert_eq!(message.content.len(), 4);
        assert!(matches!(&message.content[0], ContentPart::Text(_)));
        assert!(matches!(&message.content[1], ContentPart::ToolCall(tc) if tc.id == "call_1"));
        assert!(matches!(&message.content[2], ContentPart::ToolCall(tc) if tc.id == "call_2"));
        assert!(matches!(&message.content[3], ContentPart::ToolCall(tc) if tc.id == "call_3"));
    }

    #[test]
    fn test_tool_result_success() {
        let message = Message::tool_result(
            "call_123",
            Some(json!({"temperature": 72, "unit": "F"})),
            None,
        );

        assert_eq!(message.content.len(), 1);
        if let ContentPart::ToolResult(tr) = &message.content[0] {
            assert_eq!(tr.result.as_ref().unwrap()["temperature"], 72);
            assert_eq!(tr.result.as_ref().unwrap()["unit"], "F");
            assert!(tr.error.is_none());
        } else {
            panic!("Expected ToolResult content part");
        }
    }

    #[test]
    fn test_tool_result_error() {
        let message = Message::tool_result("call_fail", None, Some("Division by zero".to_string()));

        assert_eq!(message.content.len(), 1);
        if let ContentPart::ToolResult(tr) = &message.content[0] {
            assert!(tr.result.is_none());
            assert_eq!(tr.error, Some("Division by zero".to_string()));
        } else {
            panic!("Expected ToolResult content part");
        }
    }

    // ========================================================================
    // Test: Message helper methods
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

    #[test]
    fn test_empty_tool_calls_vector() {
        let message = Message::assistant_with_tools("No tools needed.", vec![]);

        // Only text part, no tool calls
        assert_eq!(message.content.len(), 1);
        assert!(
            matches!(&message.content[0], ContentPart::Text(t) if t.text == "No tools needed.")
        );
        assert!(!message.has_tool_calls());
    }

    // ========================================================================
    // Test: Complex content preservation
    // ========================================================================

    #[test]
    fn test_tool_result_with_complex_nested_result() {
        let message = Message::tool_result(
            "call_search",
            Some(json!({
                "results": [
                    {"title": "Result 1", "score": 0.95},
                    {"title": "Result 2", "score": 0.87}
                ],
                "metadata": {
                    "query_time_ms": 42,
                    "total_count": 1000
                }
            })),
            None,
        );

        if let ContentPart::ToolResult(tr) = &message.content[0] {
            assert_eq!(
                tr.result.as_ref().unwrap()["results"][0]["title"],
                "Result 1"
            );
            assert_eq!(tr.result.as_ref().unwrap()["metadata"]["query_time_ms"], 42);
        } else {
            panic!("Expected ToolResult content part");
        }
    }

    // ========================================================================
    // Test: Event to Message conversion
    // ========================================================================

    #[test]
    fn test_event_to_message_basic() {
        let data = json!({
            "message_id": "01234567-89ab-cdef-0123-456789abcdef",
            "role": "assistant",
            "content": [{"type": "text", "text": "Hello!"}],
            "controls": null,
            "metadata": null,
            "tags": []
        });

        let result = event_to_message(&data, Utc::now());
        assert!(result.is_ok());

        let message = result.unwrap();
        assert_eq!(message.role, MessageRole::Assistant);
        assert_eq!(message.content.len(), 1);
    }

    #[test]
    fn test_event_to_message_with_controls() {
        // Use a valid UUID for model_id (now Controls expects UUID)
        let model_uuid = "11111111-1111-1111-1111-111111111111";
        let data = json!({
            "message_id": "01234567-89ab-cdef-0123-456789abcdef",
            "role": "user",
            "content": [{"type": "text", "text": "Test"}],
            "controls": {"model_id": model_uuid},
            "metadata": {"locale": "en-US"},
            "tags": []
        });

        let result = event_to_message(&data, Utc::now());
        assert!(result.is_ok());

        let message = result.unwrap();
        assert!(message.controls.is_some());
        assert_eq!(
            message.controls.as_ref().unwrap().model_id,
            Some(Uuid::parse_str(model_uuid).unwrap())
        );
        assert!(message.metadata.is_some());
    }

    #[test]
    fn test_event_to_message_new_format() {
        // New format: message wrapped in "message" key
        let data = json!({
            "message": {
                "id": "01234567-89ab-cdef-0123-456789abcdef",
                "role": "user",
                "content": [{"type": "text", "text": "Hello from new format!"}],
                "controls": null,
                "metadata": null,
                "created_at": "2024-01-01T12:00:00Z"
            },
            "tags": []
        });

        let result = event_to_message(&data, Utc::now());
        assert!(result.is_ok());

        let message = result.unwrap();
        assert_eq!(message.role, MessageRole::User);
        assert_eq!(message.content.len(), 1);
        assert_eq!(
            message.id,
            Uuid::parse_str("01234567-89ab-cdef-0123-456789abcdef").unwrap()
        );
    }

    #[test]
    fn test_event_to_message_tool_call_completed() {
        // tool.call_completed event format
        let data = json!({
            "tool_call_id": "call_123",
            "tool_name": "get_weather",
            "success": true,
            "status": "success",
            "result": [{"type": "text", "text": "Sunny, 72°F"}],
            "error": null
        });

        let result = event_to_message(&data, Utc::now());
        assert!(result.is_ok());

        let message = result.unwrap();
        assert_eq!(message.role, MessageRole::ToolResult);
        assert_eq!(message.tool_call_id(), Some("call_123"));
    }

    #[test]
    fn test_event_to_message_tool_call_completed_error() {
        // tool.call_completed event with error
        let data = json!({
            "tool_call_id": "call_456",
            "tool_name": "read_file",
            "success": false,
            "status": "error",
            "result": null,
            "error": "File not found"
        });

        let result = event_to_message(&data, Utc::now());
        assert!(result.is_ok());

        let message = result.unwrap();
        assert_eq!(message.role, MessageRole::ToolResult);
        assert_eq!(message.tool_call_id(), Some("call_456"));
        // Check that error is present
        if let ContentPart::ToolResult(tr) = &message.content[0] {
            assert_eq!(tr.error.as_deref(), Some("File not found"));
        } else {
            panic!("Expected ToolResult content part");
        }
    }
}

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
    traits::MessageStore, AgentLoopError, ContentPart, Controls, Message, MessageRole, Result,
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
    async fn store(&self, session_id: Uuid, message: Message) -> Result<()> {
        // Determine event type from message role
        // Note: user messages are handled by MessageService in the API layer
        // System messages are not emitted as events (per design decision)
        let event_type = match message.role {
            MessageRole::Assistant => Some("message.assistant"),
            MessageRole::ToolCall => Some("message.tool_call"),
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
fn event_to_message(
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
    fn test_tool_call_message_content() {
        let tool_call = ToolCall {
            id: "call_xyz".to_string(),
            name: "add".to_string(),
            arguments: json!({"a": 10, "b": 20}),
        };
        let message = Message::tool_call(&tool_call);

        assert_eq!(message.content.len(), 1);
        if let ContentPart::ToolCall(tc) = &message.content[0] {
            assert_eq!(tc.id, "call_xyz");
            assert_eq!(tc.name, "add");
            assert_eq!(tc.arguments["a"], 10);
            assert_eq!(tc.arguments["b"], 20);
        } else {
            panic!("Expected ToolCall content part");
        }
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
    fn test_tool_call_with_complex_arguments() {
        let tool_call = ToolCall {
            id: "call_complex".to_string(),
            name: "search".to_string(),
            arguments: json!({
                "query": "rust programming",
                "filters": {
                    "date_range": {"start": "2024-01-01", "end": "2024-12-31"},
                    "categories": ["tutorials", "documentation"],
                    "max_results": 10
                },
                "include_metadata": true
            }),
        };
        let message = Message::tool_call(&tool_call);

        if let ContentPart::ToolCall(tc) = &message.content[0] {
            assert_eq!(tc.arguments["query"], "rust programming");
            assert_eq!(tc.arguments["filters"]["categories"][0], "tutorials");
            assert_eq!(tc.arguments["filters"]["max_results"], 10);
        } else {
            panic!("Expected ToolCall content part");
        }
    }

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
        let data = json!({
            "message_id": "01234567-89ab-cdef-0123-456789abcdef",
            "role": "user",
            "content": [{"type": "text", "text": "Test"}],
            "controls": {"model_id": "openai/gpt-4o"},
            "metadata": {"locale": "en-US"},
            "tags": []
        });

        let result = event_to_message(&data, Utc::now());
        assert!(result.is_ok());

        let message = result.unwrap();
        assert!(message.controls.is_some());
        assert_eq!(
            message.controls.as_ref().unwrap().model_id,
            Some("openai/gpt-4o".to_string())
        );
        assert!(message.metadata.is_some());
    }
}

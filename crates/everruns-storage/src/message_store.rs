// Database-backed MessageStore implementation
//
// This module implements the core MessageStore trait for persisting
// conversation messages to the database during workflow execution.
//
// Note: Message.content is Vec<ContentPart> in both core and storage,
// so no conversion is needed - data passes through directly.
//
// Events: When messages are stored, corresponding events are emitted
// for SSE streaming. This allows the UI to render from events.

use async_trait::async_trait;
use everruns_core::{traits::MessageStore, AgentLoopError, Message, MessageRole, Result};
use uuid::Uuid;

use crate::models::{CreateEventRow, CreateMessageRow};
use crate::repositories::Database;

// ============================================================================
// DbMessageStore - Stores messages in database
// ============================================================================

/// Database-backed message store
///
/// Stores conversation messages in the messages table.
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
        let role = message.role.clone();
        let content = message.content.clone();

        let create_msg = CreateMessageRow {
            session_id,
            role: message.role.to_string(),
            content: message.content, // Direct pass-through - both are Vec<ContentPart>
            controls: message.controls,
            metadata: message.metadata,
            tags: vec![], // Core messages don't have tags currently
        };

        let stored_msg = self
            .db
            .create_message(create_msg)
            .await
            .map_err(|e| AgentLoopError::store(e.to_string()))?;

        // Emit event for SSE streaming
        // Note: user messages are handled by MessageService in the API layer
        let event_type = match role {
            MessageRole::Assistant => Some("message.assistant"),
            MessageRole::ToolCall => Some("message.tool_call"),
            MessageRole::ToolResult => Some("message.tool_result"),
            MessageRole::User | MessageRole::System => None, // User messages handled by API
        };

        if let Some(event_type) = event_type {
            let event_input = CreateEventRow {
                session_id,
                event_type: event_type.to_string(),
                data: serde_json::json!({
                    "message_id": stored_msg.id,
                    "role": role.to_string(),
                    "content": content,
                    "sequence": stored_msg.sequence,
                    "created_at": stored_msg.created_at,
                }),
            };
            if let Err(e) = self.db.create_event(event_input).await {
                tracing::warn!("Failed to emit {} event: {}", event_type, e);
            }
        }

        Ok(())
    }

    async fn load(&self, session_id: Uuid) -> Result<Vec<Message>> {
        let messages = self
            .db
            .list_messages(session_id)
            .await
            .map_err(|e| AgentLoopError::store(e.to_string()))?;

        // Message.tool_call_id is now derived from content - no need to set it separately
        let converted: Vec<Message> = messages
            .into_iter()
            .map(|msg| Message {
                id: msg.id,
                role: MessageRole::from(msg.role.as_str()),
                content: msg.content, // Direct pass-through - tool_call_id is in ToolResultContentPart
                controls: msg.controls.map(|j| j.0),
                metadata: msg.metadata.map(|j| j.0),
                created_at: msg.created_at,
            })
            .collect();

        Ok(converted)
    }

    async fn count(&self, session_id: Uuid) -> Result<usize> {
        let messages = self.load(session_id).await?;
        Ok(messages.len())
    }
}

// ============================================================================
// Factory functions
// ============================================================================

/// Create a database-backed message store
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
}

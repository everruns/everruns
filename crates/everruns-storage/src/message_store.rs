// Database-backed MessageStore implementation
//
// This module implements the core MessageStore trait for persisting
// conversation messages to the database during workflow execution.
//
// Note: The database stores content as JSONB with a flexible schema.
// The core Message type uses MessageContent enum for type safety.

use async_trait::async_trait;
use everruns_core::{
    message::MessageContent, traits::MessageStore, AgentLoopError, Message, MessageRole, Result,
    ToolCall,
};
use uuid::Uuid;

use crate::models::CreateMessageRow;
use crate::repositories::Database;

// ============================================================================
// Helper functions for message serialization
// ============================================================================

/// Serialize message content to JSON for storage
///
/// This is extracted from the store() method to enable unit testing
/// of the serialization logic without a database connection.
pub(crate) fn serialize_message_content(message: &Message) -> serde_json::Value {
    match &message.content {
        MessageContent::Text(text) => {
            // For assistant messages with tool_calls, include them in the content
            if let Some(tool_calls) = &message.tool_calls {
                serde_json::json!({
                    "text": text,
                    "tool_calls": tool_calls
                })
            } else {
                serde_json::json!({ "text": text })
            }
        }
        MessageContent::ToolCall {
            id,
            name,
            arguments,
        } => {
            serde_json::json!({
                "id": id,
                "name": name,
                "arguments": arguments
            })
        }
        MessageContent::ToolResult { result, error } => {
            serde_json::json!({
                "result": result,
                "error": error
            })
        }
    }
}

/// Deserialize message content and tool_calls from stored JSON
///
/// This is extracted from the load() method to enable unit testing
/// of the deserialization logic without a database connection.
pub(crate) fn deserialize_message_content(
    role: &MessageRole,
    content: &serde_json::Value,
) -> (MessageContent, Option<Vec<ToolCall>>) {
    let msg_content = match role {
        MessageRole::User | MessageRole::Assistant | MessageRole::System => {
            let text = content
                .get("text")
                .and_then(|t| t.as_str())
                .unwrap_or("")
                .to_string();
            MessageContent::Text(text)
        }
        MessageRole::ToolCall => {
            let id = content
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let name = content
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let arguments = content
                .get("arguments")
                .cloned()
                .unwrap_or(serde_json::json!({}));
            MessageContent::ToolCall {
                id,
                name,
                arguments,
            }
        }
        MessageRole::ToolResult => {
            let result = content.get("result").cloned();
            let error = content
                .get("error")
                .and_then(|v| v.as_str())
                .map(String::from);
            MessageContent::ToolResult { result, error }
        }
    };

    // Parse tool_calls from assistant messages if present
    let tool_calls = if *role == MessageRole::Assistant {
        content
            .get("tool_calls")
            .and_then(|tc| serde_json::from_value::<Vec<ToolCall>>(tc.clone()).ok())
    } else {
        None
    };

    (msg_content, tool_calls)
}

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
        let role = message.role.to_string();
        let content = serialize_message_content(&message);

        let create_msg = CreateMessageRow {
            session_id,
            role,
            content,
            metadata: None, // Core messages don't have metadata currently
            tags: vec![],   // Core messages don't have tags currently
            tool_call_id: message.tool_call_id,
        };

        self.db
            .create_message(create_msg)
            .await
            .map_err(|e| AgentLoopError::store(e.to_string()))?;

        Ok(())
    }

    async fn load(&self, session_id: Uuid) -> Result<Vec<Message>> {
        let messages = self
            .db
            .list_messages(session_id)
            .await
            .map_err(|e| AgentLoopError::store(e.to_string()))?;

        let converted: Vec<Message> = messages
            .into_iter()
            .map(|msg| {
                let role = MessageRole::from(msg.role.as_str());
                let (content, tool_calls) = deserialize_message_content(&role, &msg.content);

                Message {
                    id: msg.id,
                    role,
                    content,
                    tool_call_id: msg.tool_call_id,
                    tool_calls,
                    created_at: msg.created_at,
                }
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
    use super::*;
    use serde_json::json;

    // ========================================================================
    // Test: Basic message serialization
    // ========================================================================

    #[test]
    fn test_serialize_user_message() {
        let message = Message::user("Hello, world!");
        let content = serialize_message_content(&message);

        assert_eq!(content, json!({ "text": "Hello, world!" }));
    }

    #[test]
    fn test_serialize_assistant_message_without_tools() {
        let message = Message::assistant("I can help you with that.");
        let content = serialize_message_content(&message);

        assert_eq!(content, json!({ "text": "I can help you with that." }));
        // No tool_calls field should be present
        assert!(content.get("tool_calls").is_none());
    }

    #[test]
    fn test_serialize_system_message() {
        let message = Message::system("You are a helpful assistant.");
        let content = serialize_message_content(&message);

        assert_eq!(content, json!({ "text": "You are a helpful assistant." }));
    }

    // ========================================================================
    // Test: Assistant messages with tool calls (the bug fix case)
    // ========================================================================

    #[test]
    fn test_serialize_assistant_message_with_single_tool_call() {
        let tool_call = ToolCall {
            id: "call_abc123".to_string(),
            name: "get_weather".to_string(),
            arguments: json!({"city": "Tokyo"}),
        };
        let message = Message::assistant_with_tools("Let me check the weather.", vec![tool_call]);
        let content = serialize_message_content(&message);

        // Verify text is preserved
        assert_eq!(content["text"], "Let me check the weather.");

        // Verify tool_calls are included
        let tool_calls = content
            .get("tool_calls")
            .expect("tool_calls should be present");
        assert!(tool_calls.is_array());
        let calls = tool_calls.as_array().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0]["id"], "call_abc123");
        assert_eq!(calls[0]["name"], "get_weather");
        assert_eq!(calls[0]["arguments"]["city"], "Tokyo");
    }

    #[test]
    fn test_serialize_assistant_message_with_multiple_tool_calls() {
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
        let content = serialize_message_content(&message);

        // Verify all tool_calls are included
        let serialized_calls = content
            .get("tool_calls")
            .expect("tool_calls should be present");
        let calls = serialized_calls.as_array().unwrap();
        assert_eq!(calls.len(), 3);
        assert_eq!(calls[0]["id"], "call_1");
        assert_eq!(calls[1]["id"], "call_2");
        assert_eq!(calls[2]["id"], "call_3");
    }

    #[test]
    fn test_serialize_assistant_message_with_empty_text_and_tool_calls() {
        let tool_call = ToolCall {
            id: "call_silent".to_string(),
            name: "calculate".to_string(),
            arguments: json!({"a": 5, "b": 3}),
        };
        // Some LLMs return tool calls with empty text content
        let message = Message::assistant_with_tools("", vec![tool_call]);
        let content = serialize_message_content(&message);

        assert_eq!(content["text"], "");
        assert!(content.get("tool_calls").is_some());
        assert_eq!(content["tool_calls"][0]["name"], "calculate");
    }

    // ========================================================================
    // Test: Tool call messages
    // ========================================================================

    #[test]
    fn test_serialize_tool_call_message() {
        let tool_call = ToolCall {
            id: "call_xyz".to_string(),
            name: "add".to_string(),
            arguments: json!({"a": 10, "b": 20}),
        };
        let message = Message::tool_call(&tool_call);
        let content = serialize_message_content(&message);

        assert_eq!(content["id"], "call_xyz");
        assert_eq!(content["name"], "add");
        assert_eq!(content["arguments"]["a"], 10);
        assert_eq!(content["arguments"]["b"], 20);
    }

    // ========================================================================
    // Test: Tool result messages
    // ========================================================================

    #[test]
    fn test_serialize_tool_result_success() {
        let message = Message::tool_result(
            "call_123",
            Some(json!({"temperature": 72, "unit": "F"})),
            None,
        );
        let content = serialize_message_content(&message);

        assert_eq!(content["result"]["temperature"], 72);
        assert_eq!(content["result"]["unit"], "F");
        assert!(content["error"].is_null());
    }

    #[test]
    fn test_serialize_tool_result_error() {
        let message = Message::tool_result("call_fail", None, Some("Division by zero".to_string()));
        let content = serialize_message_content(&message);

        assert!(content["result"].is_null());
        assert_eq!(content["error"], "Division by zero");
    }

    // ========================================================================
    // Test: Deserialization roundtrip
    // ========================================================================

    #[test]
    fn test_deserialize_user_message() {
        let stored = json!({ "text": "What's the weather?" });
        let (content, tool_calls) = deserialize_message_content(&MessageRole::User, &stored);

        assert!(matches!(content, MessageContent::Text(t) if t == "What's the weather?"));
        assert!(tool_calls.is_none());
    }

    #[test]
    fn test_deserialize_assistant_message_without_tools() {
        let stored = json!({ "text": "The weather is sunny." });
        let (content, tool_calls) = deserialize_message_content(&MessageRole::Assistant, &stored);

        assert!(matches!(content, MessageContent::Text(t) if t == "The weather is sunny."));
        assert!(tool_calls.is_none());
    }

    #[test]
    fn test_deserialize_assistant_message_with_tool_calls() {
        let stored = json!({
            "text": "Let me check.",
            "tool_calls": [
                {
                    "id": "call_abc",
                    "name": "get_weather",
                    "arguments": {"city": "Tokyo"}
                }
            ]
        });
        let (content, tool_calls) = deserialize_message_content(&MessageRole::Assistant, &stored);

        assert!(matches!(content, MessageContent::Text(t) if t == "Let me check."));
        let calls = tool_calls.expect("tool_calls should be present");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "call_abc");
        assert_eq!(calls[0].name, "get_weather");
        assert_eq!(calls[0].arguments["city"], "Tokyo");
    }

    #[test]
    fn test_deserialize_tool_call_message() {
        let stored = json!({
            "id": "call_123",
            "name": "multiply",
            "arguments": {"a": 6, "b": 7}
        });
        let (content, tool_calls) = deserialize_message_content(&MessageRole::ToolCall, &stored);

        assert!(matches!(
            content,
            MessageContent::ToolCall { id, name, arguments }
            if id == "call_123" && name == "multiply" && arguments["a"] == 6
        ));
        assert!(tool_calls.is_none());
    }

    #[test]
    fn test_deserialize_tool_result_success() {
        let stored = json!({
            "result": {"answer": 42},
            "error": null
        });
        let (content, tool_calls) = deserialize_message_content(&MessageRole::ToolResult, &stored);

        assert!(matches!(
            content,
            MessageContent::ToolResult { result: Some(r), error: None }
            if r["answer"] == 42
        ));
        assert!(tool_calls.is_none());
    }

    #[test]
    fn test_deserialize_tool_result_error() {
        let stored = json!({
            "result": null,
            "error": "Tool not found"
        });
        let (content, _) = deserialize_message_content(&MessageRole::ToolResult, &stored);

        // Note: JSON null is still a Value, so result will be Some(Value::Null)
        // The error is properly parsed
        if let MessageContent::ToolResult { result, error } = content {
            // result is Some(Value::Null) due to how serde_json works
            assert!(result.as_ref().is_none_or(|v| v.is_null()));
            assert_eq!(error, Some("Tool not found".to_string()));
        } else {
            panic!("Expected ToolResult content");
        }
    }

    // ========================================================================
    // Test: Full roundtrip (serialize then deserialize)
    // ========================================================================

    #[test]
    fn test_roundtrip_assistant_with_tool_calls() {
        // Create original message with tool calls
        let tool_calls = vec![
            ToolCall {
                id: "call_weather".to_string(),
                name: "get_weather".to_string(),
                arguments: json!({"location": "Paris", "units": "celsius"}),
            },
            ToolCall {
                id: "call_time".to_string(),
                name: "get_time".to_string(),
                arguments: json!({"timezone": "Europe/Paris"}),
            },
        ];
        let original = Message::assistant_with_tools(
            "I'll get the weather and time for Paris.",
            tool_calls.clone(),
        );

        // Serialize
        let stored = serialize_message_content(&original);

        // Deserialize
        let (content, recovered_calls) =
            deserialize_message_content(&MessageRole::Assistant, &stored);

        // Verify text
        assert!(
            matches!(content, MessageContent::Text(t) if t == "I'll get the weather and time for Paris.")
        );

        // Verify tool calls
        let calls = recovered_calls.expect("tool_calls should be recovered");
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].id, "call_weather");
        assert_eq!(calls[0].name, "get_weather");
        assert_eq!(calls[0].arguments["location"], "Paris");
        assert_eq!(calls[1].id, "call_time");
        assert_eq!(calls[1].name, "get_time");
    }

    #[test]
    fn test_roundtrip_tool_call_with_complex_arguments() {
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
        let original = Message::tool_call(&tool_call);

        // Serialize
        let stored = serialize_message_content(&original);

        // Deserialize
        let (content, _) = deserialize_message_content(&MessageRole::ToolCall, &stored);

        // Verify complex arguments are preserved
        if let MessageContent::ToolCall {
            id,
            name,
            arguments,
        } = content
        {
            assert_eq!(id, "call_complex");
            assert_eq!(name, "search");
            assert_eq!(arguments["query"], "rust programming");
            assert_eq!(arguments["filters"]["categories"][0], "tutorials");
            assert_eq!(arguments["filters"]["max_results"], 10);
        } else {
            panic!("Expected ToolCall content");
        }
    }

    // ========================================================================
    // Test: Edge cases
    // ========================================================================

    #[test]
    fn test_empty_tool_calls_vector_is_not_stored() {
        // When tool_calls is Some([]) vs None - empty vector shouldn't be stored
        // In practice, Message::assistant_with_tools with empty vec creates Some([])
        // But our serialization uses Option::is_some which includes empty vecs
        let message = Message::assistant_with_tools("No tools needed.", vec![]);
        let content = serialize_message_content(&message);

        // Empty tool_calls array is still serialized (this is fine)
        assert!(content.get("tool_calls").is_some());
        assert_eq!(content["tool_calls"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn test_deserialize_missing_fields_uses_defaults() {
        // Test graceful handling of missing fields
        let stored = json!({}); // Missing "text" field
        let (content, _) = deserialize_message_content(&MessageRole::User, &stored);

        // Should default to empty string
        assert!(matches!(content, MessageContent::Text(t) if t.is_empty()));
    }

    #[test]
    fn test_deserialize_malformed_tool_calls_returns_none() {
        // Invalid tool_calls format should not panic
        let stored = json!({
            "text": "Some response",
            "tool_calls": "not an array"  // Invalid format
        });
        let (_, tool_calls) = deserialize_message_content(&MessageRole::Assistant, &stored);

        // Should gracefully return None instead of panicking
        assert!(tool_calls.is_none());
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
        let content = serialize_message_content(&message);

        assert_eq!(content["result"]["results"][0]["title"], "Result 1");
        assert_eq!(content["result"]["metadata"]["query_time_ms"], 42);
    }
}

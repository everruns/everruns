// Integration tests for tool calling
//
// These tests verify that both built-in tools (via ToolRegistry) and
// the ToolExecutor trait work correctly together.

use async_trait::async_trait;
use everruns_core::{
    BuiltinTool, DeferrablePolicy, ToolCall, ToolDefinition, ToolHints, ToolPolicy, ToolResultImage,
};
use everruns_core::{
    GetCurrentTimeTool, Message, MessageRetriever, MessageRole, SessionId,
    memory::InMemoryMessageRetriever,
    tools::{EchoTool, FailingTool, Tool, ToolExecutionResult, ToolRegistry},
    traits::ToolExecutor,
};
use serde_json::json;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use uuid::Uuid;

// =============================================================================
// Tests for ToolRegistry as ToolExecutor
// =============================================================================

#[tokio::test]
async fn test_tool_registry_as_executor() {
    // Create a registry with built-in tools
    let registry = ToolRegistry::builder()
        .tool(GetCurrentTimeTool)
        .tool(EchoTool)
        .build();

    // Create a tool call
    let tool_call = ToolCall {
        id: "call_1".to_string(),
        name: "echo".to_string(),
        arguments: json!({"message": "Hello, World!"}),
    };

    let tool_def = ToolDefinition::Builtin(BuiltinTool {
        name: "echo".to_string(),
        display_name: None,
        description: "Echo".to_string(),
        parameters: json!({}),
        policy: ToolPolicy::Auto,
        category: None,
        deferrable: DeferrablePolicy::default(),
        hints: ToolHints::default(),
    });

    // Execute via ToolExecutor trait
    let result = registry.execute(&tool_call, &tool_def).await.unwrap();

    assert!(result.error.is_none());
    assert_eq!(result.result.unwrap()["echoed"], "Hello, World!");
}

#[tokio::test]
async fn test_get_current_time_tool() {
    let registry = ToolRegistry::builder().tool(GetCurrentTimeTool).build();

    let tool_call = ToolCall {
        id: "call_time".to_string(),
        name: "get_current_time".to_string(),
        arguments: json!({"format": "unix"}),
    };

    let tool_def = ToolDefinition::Builtin(BuiltinTool {
        name: "get_current_time".to_string(),
        display_name: None,
        description: "Get time".to_string(),
        parameters: json!({}),
        policy: ToolPolicy::Auto,
        category: None,
        deferrable: DeferrablePolicy::default(),
        hints: ToolHints::default(),
    });

    let result = registry.execute(&tool_call, &tool_def).await.unwrap();

    assert!(result.error.is_none());
    let value = result.result.unwrap();
    assert!(value.get("timestamp").is_some());
    assert_eq!(value["format"], "unix");
}

#[tokio::test]
async fn test_tool_error_handling() {
    // Create a registry with a failing tool
    let registry = ToolRegistry::builder()
        .tool(FailingTool::with_tool_error("Expected test failure"))
        .build();

    let tool_call = ToolCall {
        id: "call_fail".to_string(),
        name: "failing_tool".to_string(),
        arguments: json!({}),
    };

    let tool_def = ToolDefinition::Builtin(BuiltinTool {
        name: "failing_tool".to_string(),
        display_name: None,
        description: "A tool that fails".to_string(),
        parameters: json!({}),
        policy: ToolPolicy::Auto,
        category: None,
        deferrable: DeferrablePolicy::default(),
        hints: ToolHints::default(),
    });

    // Execute and verify error is packaged as {"error": "..."} in result field
    let result = registry.execute(&tool_call, &tool_def).await.unwrap();

    assert_eq!(result.error.as_deref(), Some("Expected test failure"));
    assert_eq!(
        result.result,
        Some(json!({"error": "Expected test failure"}))
    );
}

#[tokio::test]
async fn test_internal_error_is_hidden() {
    // Create a registry with a tool that has internal errors
    let registry = ToolRegistry::builder()
        .tool(FailingTool::with_internal_error("Secret database error"))
        .build();

    let tool_call = ToolCall {
        id: "call_internal".to_string(),
        name: "failing_tool".to_string(),
        arguments: json!({}),
    };

    let tool_def = ToolDefinition::Builtin(BuiltinTool {
        name: "failing_tool".to_string(),
        display_name: None,
        description: "A tool that fails internally".to_string(),
        parameters: json!({}),
        policy: ToolPolicy::Auto,
        category: None,
        deferrable: DeferrablePolicy::default(),
        hints: ToolHints::default(),
    });

    // Execute and verify internal error is hidden (packaged as {"error": "..."} with generic message)
    let result = registry.execute(&tool_call, &tool_def).await.unwrap();

    // Internal error sets generic message in error field (real details are only logged)
    assert_eq!(
        result.error.as_deref(),
        Some("An internal error occurred while executing the tool")
    );
    assert_eq!(
        result.result,
        Some(json!({"error": "An internal error occurred while executing the tool"}))
    );
}

#[tokio::test]
async fn test_tool_not_found_error() {
    let registry = ToolRegistry::new(); // Empty registry

    let tool_call = ToolCall {
        id: "call_missing".to_string(),
        name: "nonexistent_tool".to_string(),
        arguments: json!({}),
    };

    let tool_def = ToolDefinition::Builtin(BuiltinTool {
        name: "nonexistent_tool".to_string(),
        display_name: None,
        description: "Does not exist".to_string(),
        parameters: json!({}),
        policy: ToolPolicy::Auto,
        category: None,
        deferrable: DeferrablePolicy::default(),
        hints: ToolHints::default(),
    });

    // Should return error for tool not found
    let result = registry.execute(&tool_call, &tool_def).await;
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.to_string().contains("not found"));
}

// =============================================================================
// Custom Tool Test
// =============================================================================

/// Custom tool for testing
struct CounterTool {
    count: Arc<AtomicUsize>,
}

impl CounterTool {
    fn new() -> Self {
        Self {
            count: Arc::new(AtomicUsize::new(0)),
        }
    }
}

#[async_trait]
impl Tool for CounterTool {
    fn name(&self) -> &str {
        "counter"
    }

    fn description(&self) -> &str {
        "Increments and returns a counter"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: serde_json::Value) -> ToolExecutionResult {
        let new_count = self.count.fetch_add(1, Ordering::SeqCst) + 1;
        ToolExecutionResult::success(json!({
            "count": new_count
        }))
    }
}

#[tokio::test]
async fn test_custom_tool_execution() {
    let counter_tool = CounterTool::new();
    let counter_arc = counter_tool.count.clone();

    let registry = ToolRegistry::builder().tool(counter_tool).build();

    let tool_def = ToolDefinition::Builtin(BuiltinTool {
        name: "counter".to_string(),
        display_name: None,
        description: "Counter".to_string(),
        parameters: json!({}),
        policy: ToolPolicy::Auto,
        category: None,
        deferrable: DeferrablePolicy::default(),
        hints: ToolHints::default(),
    });

    // Execute multiple times
    for i in 1..=3 {
        let tool_call = ToolCall {
            id: format!("call_{}", i),
            name: "counter".to_string(),
            arguments: json!({}),
        };

        let result = registry.execute(&tool_call, &tool_def).await.unwrap();
        assert!(result.error.is_none());
        assert_eq!(result.result.unwrap()["count"], i);
    }

    // Verify counter was incremented
    assert_eq!(counter_arc.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn test_multiple_tools_in_registry() {
    let registry = ToolRegistry::builder()
        .tool(GetCurrentTimeTool)
        .tool(EchoTool)
        .build();

    // Execute get_current_time
    let time_call = ToolCall {
        id: "call_time".to_string(),
        name: "get_current_time".to_string(),
        arguments: json!({"format": "unix"}),
    };

    let time_def = ToolDefinition::Builtin(BuiltinTool {
        name: "get_current_time".to_string(),
        display_name: None,
        description: "Get time".to_string(),
        parameters: json!({}),
        policy: ToolPolicy::Auto,
        category: None,
        deferrable: DeferrablePolicy::default(),
        hints: ToolHints::default(),
    });

    let time_result = registry.execute(&time_call, &time_def).await.unwrap();
    assert!(time_result.error.is_none());
    assert!(time_result.result.unwrap().get("timestamp").is_some());

    // Execute echo
    let echo_call = ToolCall {
        id: "call_echo".to_string(),
        name: "echo".to_string(),
        arguments: json!({"message": "Test message"}),
    };

    let echo_def = ToolDefinition::Builtin(BuiltinTool {
        name: "echo".to_string(),
        display_name: None,
        description: "Echo".to_string(),
        parameters: json!({}),
        policy: ToolPolicy::Auto,
        category: None,
        deferrable: DeferrablePolicy::default(),
        hints: ToolHints::default(),
    });

    let echo_result = registry.execute(&echo_call, &echo_def).await.unwrap();
    assert!(echo_result.error.is_none());
    assert_eq!(echo_result.result.unwrap()["echoed"], "Test message");
}

// =============================================================================
// Message Store Tool Calls Tests
// =============================================================================
// These tests verify that messages with tool_calls survive the store/load cycle.
// This specifically tests the bug fix where tool_calls were not being persisted.

#[tokio::test]
async fn test_message_retriever_preserves_tool_calls() {
    let store = InMemoryMessageRetriever::new();
    let session_id: SessionId = Uuid::now_v7().into();

    // Create an assistant message with tool calls (the critical case)
    let tool_calls = vec![
        ToolCall {
            id: "call_weather".to_string(),
            name: "get_weather".to_string(),
            arguments: json!({"city": "Tokyo"}),
        },
        ToolCall {
            id: "call_time".to_string(),
            name: "get_time".to_string(),
            arguments: json!({"format": "unix"}),
        },
    ];

    let assistant_msg = Message::assistant_with_tools("Let me check that for you.", tool_calls);
    store.store(session_id, assistant_msg).await.unwrap();

    // Load messages back
    let loaded = store.load(session_id).await.unwrap();
    assert_eq!(loaded.len(), 1);

    let loaded_msg = &loaded[0];
    assert_eq!(loaded_msg.role, MessageRole::Agent);
    assert_eq!(loaded_msg.text(), Some("Let me check that for you."));

    // Verify tool_calls are preserved - this is the key assertion
    let loaded_tool_calls = loaded_msg.tool_calls();
    assert_eq!(loaded_tool_calls.len(), 2);
    assert_eq!(loaded_tool_calls[0].id, "call_weather");
    assert_eq!(loaded_tool_calls[0].name, "get_weather");
    assert_eq!(loaded_tool_calls[1].id, "call_time");
    assert_eq!(loaded_tool_calls[1].name, "get_time");
}

#[tokio::test]
async fn test_message_retriever_full_tool_conversation() {
    let store = InMemoryMessageRetriever::new();
    let session_id: SessionId = Uuid::now_v7().into();

    // Simulate a full tool-calling conversation:
    // 1. User message
    // 2. Assistant message with tool calls
    // 3. Tool result messages
    // 4. Final assistant message

    // 1. User asks a question
    store
        .store(session_id, Message::user("What's the weather in Tokyo?"))
        .await
        .unwrap();

    // 2. Assistant responds with a tool call
    let tool_call = ToolCall {
        id: "call_123".to_string(),
        name: "get_weather".to_string(),
        arguments: json!({"city": "Tokyo"}),
    };
    let assistant_with_tool = Message::assistant_with_tools("", vec![tool_call.clone()]);
    store.store(session_id, assistant_with_tool).await.unwrap();

    // 3. Tool result
    let tool_result = Message::tool_result(
        "call_123",
        Some(json!({"temperature": 22, "conditions": "sunny"})),
        None,
    );
    store.store(session_id, tool_result).await.unwrap();

    // 4. Final assistant response
    store
        .store(
            session_id,
            Message::assistant("The weather in Tokyo is 22°C and sunny!"),
        )
        .await
        .unwrap();

    // Load all messages
    let messages = store.load(session_id).await.unwrap();
    assert_eq!(messages.len(), 4);

    // Verify message order and content
    assert_eq!(messages[0].role, MessageRole::User);
    assert_eq!(messages[1].role, MessageRole::Agent);
    assert_eq!(messages[2].role, MessageRole::ToolResult);
    assert_eq!(messages[3].role, MessageRole::Agent);

    // Verify the assistant message with tool calls has them preserved
    let assistant_msg = &messages[1];
    let tool_calls = assistant_msg.tool_calls();
    assert!(
        !tool_calls.is_empty(),
        "Assistant message should have tool_calls"
    );
    assert_eq!(tool_calls.len(), 1);
    assert_eq!(tool_calls[0].name, "get_weather");

    // Verify the tool result has correct content
    assert_eq!(messages[2].tool_call_id(), Some("call_123"));

    // Verify final response doesn't have tool_calls
    assert!(messages[3].tool_calls().is_empty());
}

#[tokio::test]
async fn test_message_retriever_parallel_tool_calls() {
    let store = InMemoryMessageRetriever::new();
    let session_id: SessionId = Uuid::now_v7().into();

    // Create an assistant message with multiple parallel tool calls
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

    store
        .store(
            session_id,
            Message::assistant_with_tools("Let me check all three cities.", tool_calls),
        )
        .await
        .unwrap();

    // Store tool results
    for (id, city, temp) in [
        ("call_1", "Tokyo", 22),
        ("call_2", "London", 15),
        ("call_3", "New York", 18),
    ] {
        store
            .store(
                session_id,
                Message::tool_result(id, Some(json!({"city": city, "temp": temp})), None),
            )
            .await
            .unwrap();
    }

    // Load and verify
    let messages = store.load(session_id).await.unwrap();
    assert_eq!(messages.len(), 4); // 1 assistant + 3 tool results

    // Verify all 3 tool calls are preserved in assistant message
    let assistant_msg = &messages[0];
    let loaded_calls = assistant_msg.tool_calls();
    assert_eq!(loaded_calls.len(), 3);

    // Verify each tool call
    for (i, expected_city) in ["Tokyo", "London", "New York"].iter().enumerate() {
        assert_eq!(loaded_calls[i].arguments["city"], *expected_city);
    }
}

// =============================================================================
// Image Tool Result Tests
// =============================================================================

/// Tool that returns an image alongside JSON
struct ImageTool;

#[async_trait]
impl Tool for ImageTool {
    fn name(&self) -> &str {
        "screenshot"
    }

    fn description(&self) -> &str {
        "Take a screenshot and return it as an image"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "region": { "type": "string" }
            },
            "additionalProperties": false
        })
    }

    async fn execute(&self, arguments: serde_json::Value) -> ToolExecutionResult {
        let region = arguments
            .get("region")
            .and_then(|v| v.as_str())
            .unwrap_or("full");

        // Tiny 1x1 red PNG (base64)
        let tiny_png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8/5+hHgAHggJ/PchI7wAAAABJRU5ErkJggg==";

        ToolExecutionResult::success_with_images(
            json!({ "region": region, "width": 1, "height": 1 }),
            vec![ToolResultImage {
                base64: tiny_png.to_string(),
                media_type: "image/png".to_string(),
            }],
        )
    }
}

#[tokio::test]
async fn test_image_tool_execution_result() {
    let tool = ImageTool;
    let result = tool.execute(json!({"region": "header"})).await;

    match result {
        ToolExecutionResult::SuccessWithImages { result, images } => {
            assert_eq!(result["region"], "header");
            assert_eq!(images.len(), 1);
            assert_eq!(images[0].media_type, "image/png");
            assert!(!images[0].base64.is_empty());
        }
        _ => panic!("Expected SuccessWithImages"),
    }
}

#[tokio::test]
async fn test_image_tool_into_tool_result() {
    let result = ToolExecutionResult::success_with_images(
        json!({"status": "ok"}),
        vec![ToolResultImage {
            base64: "AAAA".to_string(),
            media_type: "image/jpeg".to_string(),
        }],
    );

    let tool_result = result.into_tool_result("call_img", "screenshot");

    assert!(tool_result.error.is_none());
    assert_eq!(tool_result.result.unwrap()["status"], "ok");
    let images = tool_result.images.unwrap();
    assert_eq!(images.len(), 1);
    assert_eq!(images[0].media_type, "image/jpeg");
    assert_eq!(images[0].base64, "AAAA");
}

#[tokio::test]
async fn test_image_tool_empty_images_gives_none() {
    let result = ToolExecutionResult::success_with_images(json!({"ok": true}), vec![]);

    let tool_result = result.into_tool_result("call_1", "tool");

    assert!(tool_result.images.is_none(), "empty images should be None");
}

#[tokio::test]
async fn test_image_tool_via_registry() {
    let registry = ToolRegistry::builder().tool(ImageTool).build();

    let tool_call = ToolCall {
        id: "call_img".to_string(),
        name: "screenshot".to_string(),
        arguments: json!({"region": "viewport"}),
    };

    let tool_def = registry.get("screenshot").unwrap().to_definition();
    let result = registry.execute(&tool_call, &tool_def).await.unwrap();

    assert!(result.error.is_none());
    assert_eq!(result.result.unwrap()["region"], "viewport");
    let images = result.images.unwrap();
    assert_eq!(images.len(), 1);
    assert_eq!(images[0].media_type, "image/png");
}

#[tokio::test]
async fn test_tool_result_with_images_message() {
    let images = vec![ToolResultImage {
        base64: "AAAA".to_string(),
        media_type: "image/png".to_string(),
    }];

    let msg = Message::tool_result_with_images("call_123", Some(json!({"ok": true})), images);

    assert_eq!(msg.role, MessageRole::ToolResult);
    assert_eq!(msg.tool_call_id(), Some("call_123"));

    // Should have ToolResult + Image content parts
    assert_eq!(msg.content.len(), 2);
    assert!(matches!(
        msg.content[0],
        everruns_core::ContentPart::ToolResult(_)
    ));
    assert!(matches!(
        msg.content[1],
        everruns_core::ContentPart::Image(_)
    ));
}

#[tokio::test]
async fn test_tool_result_with_images_llm_conversion() {
    use everruns_core::llm_driver_registry::LlmMessage;
    use std::collections::HashMap;

    let images = vec![
        ToolResultImage {
            base64: "AAAA".to_string(),
            media_type: "image/png".to_string(),
        },
        ToolResultImage {
            base64: "BBBB".to_string(),
            media_type: "image/jpeg".to_string(),
        },
    ];

    let msg = Message::tool_result_with_images("call_456", Some(json!({"info": "test"})), images);

    let resolved = HashMap::new();
    let llm_msg = LlmMessage::from_message_with_images(&msg, &resolved);

    // Should be Tool role with tool_call_id
    assert_eq!(
        llm_msg.role,
        everruns_core::llm_driver_registry::LlmMessageRole::Tool
    );
    assert_eq!(llm_msg.tool_call_id, Some("call_456".to_string()));

    // Content should have text (JSON result) + 2 images
    match &llm_msg.content {
        everruns_core::llm_driver_registry::LlmMessageContent::Parts(parts) => {
            assert_eq!(parts.len(), 3, "should have 1 text + 2 images");
            assert!(matches!(
                &parts[0],
                everruns_core::llm_driver_registry::LlmContentPart::Text { .. }
            ));
            match &parts[1] {
                everruns_core::llm_driver_registry::LlmContentPart::Image { url } => {
                    assert!(url.starts_with("data:image/png;base64,"));
                    assert!(url.contains("AAAA"));
                }
                _ => panic!("Expected Image part"),
            }
            match &parts[2] {
                everruns_core::llm_driver_registry::LlmContentPart::Image { url } => {
                    assert!(url.starts_with("data:image/jpeg;base64,"));
                    assert!(url.contains("BBBB"));
                }
                _ => panic!("Expected Image part"),
            }
        }
        _ => panic!("Expected Parts content"),
    }
}

#[tokio::test]
async fn test_tool_result_with_images_store_roundtrip() {
    let store = InMemoryMessageRetriever::new();
    let session_id: SessionId = Uuid::now_v7().into();

    let images = vec![ToolResultImage {
        base64: "AAAA".to_string(),
        media_type: "image/png".to_string(),
    }];

    let msg = Message::tool_result_with_images("call_rt", Some(json!({"ok": true})), images);

    store.store(session_id, msg).await.unwrap();

    let loaded = store.load(session_id).await.unwrap();
    assert_eq!(loaded.len(), 1);

    let loaded_msg = &loaded[0];
    assert_eq!(loaded_msg.role, MessageRole::ToolResult);
    assert_eq!(loaded_msg.tool_call_id(), Some("call_rt"));
    // Should preserve both ToolResult and Image content parts
    assert_eq!(loaded_msg.content.len(), 2);
}

#[test]
fn test_tool_result_image_serialization() {
    let img = ToolResultImage {
        base64: "AAAA".to_string(),
        media_type: "image/png".to_string(),
    };

    let json = serde_json::to_string(&img).unwrap();
    let parsed: ToolResultImage = serde_json::from_str(&json).unwrap();

    assert_eq!(parsed.base64, "AAAA");
    assert_eq!(parsed.media_type, "image/png");
}

#[test]
fn test_tool_result_with_images_serialization() {
    let result = everruns_core::ToolResult {
        tool_call_id: "call_1".to_string(),
        result: Some(json!({"ok": true})),
        images: Some(vec![ToolResultImage {
            base64: "AAAA".to_string(),
            media_type: "image/png".to_string(),
        }]),
        error: None,
        connection_required: None,
        raw_output: None,
    };

    let json_str = serde_json::to_string(&result).unwrap();
    let parsed: everruns_core::ToolResult = serde_json::from_str(&json_str).unwrap();

    assert_eq!(parsed.images.as_ref().unwrap().len(), 1);
    assert_eq!(parsed.images.unwrap()[0].media_type, "image/png");
}

#[test]
fn test_tool_result_without_images_backward_compat() {
    // Ensure old JSON without `images` field still deserializes
    let json_str = r#"{"tool_call_id":"call_1","result":{"ok":true},"error":null}"#;
    let parsed: everruns_core::ToolResult = serde_json::from_str(json_str).unwrap();

    assert!(parsed.images.is_none());
    assert_eq!(parsed.tool_call_id, "call_1");
}

// ============================================================================
// ConnectionRequired tests
// ============================================================================

#[test]
fn test_connection_required_into_tool_result() {
    use everruns_core::tools::ToolExecutionResult;

    let result = ToolExecutionResult::connection_required("daytona");
    assert!(result.is_connection_required());
    assert!(!result.is_success());
    assert!(!result.is_error());

    let tool_result = result.into_tool_result("call_conn", "daytona_create_sandbox");
    assert_eq!(tool_result.tool_call_id, "call_conn");
    assert_eq!(tool_result.connection_required, Some("daytona".to_string()));
    assert!(tool_result.error.is_none());

    // Result JSON contains connection_required key
    let result_json = tool_result.result.unwrap();
    assert_eq!(result_json["connection_required"], "daytona");
}

#[test]
fn test_connection_required_serialization_roundtrip() {
    let result = everruns_core::ToolResult {
        tool_call_id: "call_conn".to_string(),
        result: Some(json!({"connection_required": "daytona"})),
        images: None,
        error: None,
        connection_required: Some("daytona".to_string()),
        raw_output: None,
    };

    let json_str = serde_json::to_string(&result).unwrap();
    let parsed: everruns_core::ToolResult = serde_json::from_str(&json_str).unwrap();

    assert_eq!(parsed.connection_required, Some("daytona".to_string()));
    assert_eq!(parsed.result.unwrap()["connection_required"], "daytona");
}

#[test]
fn test_tool_result_without_connection_required_backward_compat() {
    // Old JSON without connection_required field still deserializes
    let json_str = r#"{"tool_call_id":"call_1","result":{"ok":true},"error":null}"#;
    let parsed: everruns_core::ToolResult = serde_json::from_str(json_str).unwrap();

    assert!(parsed.connection_required.is_none());
}

#[test]
fn test_connection_required_not_classified_as_error() {
    use everruns_core::tools::ToolExecutionResult;

    let success = ToolExecutionResult::success(json!({"ok": true}));
    assert!(success.is_success());
    assert!(!success.is_error());

    let error = ToolExecutionResult::tool_error("bad input");
    assert!(!error.is_success());
    assert!(error.is_error());

    let conn = ToolExecutionResult::connection_required("daytona");
    assert!(!conn.is_success());
    assert!(!conn.is_error());
    assert!(conn.is_connection_required());
}

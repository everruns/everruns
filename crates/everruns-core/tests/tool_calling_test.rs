// Integration tests for tool calling in the agent loop
//
// These tests verify that both built-in tools (via ToolRegistry) and
// the ToolExecutor trait work correctly together.

use async_trait::async_trait;
use everruns_core::{
    memory::{InMemoryEventEmitter, InMemoryMessageStore, MockLlmProvider, MockLlmResponse},
    tools::{EchoTool, FailingTool, Tool, ToolExecutionResult, ToolRegistry},
    traits::ToolExecutor,
    AgentConfig, AgentLoop, GetCurrentTimeTool, LoopEvent, Message, MessageRole,
};
use everruns_core::{BuiltinTool, ToolCall, ToolDefinition, ToolPolicy};
use serde_json::json;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
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
        description: "Echo".to_string(),
        parameters: json!({}),
        policy: ToolPolicy::Auto,
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
        description: "Get time".to_string(),
        parameters: json!({}),
        policy: ToolPolicy::Auto,
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
        description: "A tool that fails".to_string(),
        parameters: json!({}),
        policy: ToolPolicy::Auto,
    });

    // Execute and verify error is packaged as {"error": "..."} in result field
    let result = registry.execute(&tool_call, &tool_def).await.unwrap();

    assert!(result.error.is_none());
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
        description: "A tool that fails internally".to_string(),
        parameters: json!({}),
        policy: ToolPolicy::Auto,
    });

    // Execute and verify internal error is hidden (packaged as {"error": "..."} with generic message)
    let result = registry.execute(&tool_call, &tool_def).await.unwrap();

    assert!(result.error.is_none());
    // Internal error message should be replaced with generic message in result field
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
        description: "Does not exist".to_string(),
        parameters: json!({}),
        policy: ToolPolicy::Auto,
    });

    // Should return error for tool not found
    let result = registry.execute(&tool_call, &tool_def).await;
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.to_string().contains("not found"));
}

// =============================================================================
// Tests for AgentLoop with Tool Execution
// =============================================================================

#[tokio::test]
async fn test_agent_loop_with_tool_execution() {
    // Create a tool call that will be returned by the mock LLM
    let tool_call = ToolCall {
        id: "call_test".to_string(),
        name: "get_current_time".to_string(),
        arguments: json!({"format": "iso8601"}),
    };

    // Create mock LLM provider
    let llm_provider = MockLlmProvider::new();
    llm_provider
        .set_responses(vec![
            // First call: return tool call
            MockLlmResponse::with_tools(String::new(), vec![tool_call]),
            // Second call: final response
            MockLlmResponse::text("The current time is provided above."),
        ])
        .await;

    // Create agent config with the tool definition
    let config = AgentConfig::new("You are a helpful assistant", "gpt-test")
        .with_max_iterations(5)
        .with_tools(vec![ToolDefinition::Builtin(BuiltinTool {
            name: "get_current_time".to_string(),
            description: "Get the current time".to_string(),
            parameters: json!({}),
            policy: ToolPolicy::Auto,
        })]);

    // Create a registry with built-in tools
    let registry = ToolRegistry::builder().tool(GetCurrentTimeTool).build();

    // Create in-memory backends
    let event_emitter = InMemoryEventEmitter::new();
    let message_store = InMemoryMessageStore::new();

    // We'll create the actual agent loop below with Arc-wrapped components
    let _ = AgentLoop::new(config, event_emitter, message_store, llm_provider, registry);

    // Seed session with user message
    let session_id = Uuid::now_v7();

    // Create a standalone store and seed it
    let message_store = InMemoryMessageStore::new();
    message_store
        .seed(session_id, vec![Message::user("What time is it?")])
        .await;

    // Create a fresh LLM provider
    let llm_provider = MockLlmProvider::new();
    let tool_call = ToolCall {
        id: "call_test".to_string(),
        name: "get_current_time".to_string(),
        arguments: json!({"format": "iso8601"}),
    };
    llm_provider
        .set_responses(vec![
            MockLlmResponse::with_tools(String::new(), vec![tool_call]),
            MockLlmResponse::text("The current time is provided above."),
        ])
        .await;

    let registry = ToolRegistry::builder().tool(GetCurrentTimeTool).build();
    let event_emitter = InMemoryEventEmitter::new();

    let config = AgentConfig::new("You are a helpful assistant", "gpt-test")
        .with_max_iterations(5)
        .with_tools(vec![ToolDefinition::Builtin(BuiltinTool {
            name: "get_current_time".to_string(),
            description: "Get the current time".to_string(),
            parameters: json!({}),
            policy: ToolPolicy::Auto,
        })]);

    // Use Arc to share message store
    let message_store_arc = Arc::new(message_store);
    let event_emitter_arc = Arc::new(event_emitter);
    let llm_provider_arc = Arc::new(llm_provider);
    let registry_arc = Arc::new(registry);

    let agent_loop = AgentLoop::with_arcs(
        config,
        event_emitter_arc.clone(),
        message_store_arc.clone(),
        llm_provider_arc,
        registry_arc,
    );

    // Run the loop
    let result = agent_loop.run(session_id).await.unwrap();

    // Verify the loop completed with 2 iterations (tool call + final response)
    assert_eq!(result.iterations, 2);

    // Verify tool was executed (messages should include tool result)
    let tool_result_msg = result
        .messages
        .iter()
        .find(|m| m.role == MessageRole::ToolResult);
    assert!(
        tool_result_msg.is_some(),
        "Expected tool result message in conversation"
    );

    // Verify events were emitted for tool execution
    let events = event_emitter_arc.events().await;
    let tool_started = events
        .iter()
        .any(|e| matches!(e, LoopEvent::ToolExecutionStarted { .. }));
    let tool_completed = events
        .iter()
        .any(|e| matches!(e, LoopEvent::ToolExecutionCompleted { .. }));
    assert!(tool_started, "Expected ToolExecutionStarted event");
    assert!(tool_completed, "Expected ToolExecutionCompleted event");
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
        description: "Counter".to_string(),
        parameters: json!({}),
        policy: ToolPolicy::Auto,
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
        description: "Get time".to_string(),
        parameters: json!({}),
        policy: ToolPolicy::Auto,
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
        description: "Echo".to_string(),
        parameters: json!({}),
        policy: ToolPolicy::Auto,
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
async fn test_message_store_preserves_tool_calls() {
    use everruns_core::traits::MessageStore;

    let store = InMemoryMessageStore::new();
    let session_id = Uuid::now_v7();

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
    assert_eq!(loaded_msg.role, MessageRole::Assistant);
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
async fn test_message_store_full_tool_conversation() {
    use everruns_core::traits::MessageStore;

    let store = InMemoryMessageStore::new();
    let session_id = Uuid::now_v7();

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
    assert_eq!(messages[1].role, MessageRole::Assistant);
    assert_eq!(messages[2].role, MessageRole::ToolResult);
    assert_eq!(messages[3].role, MessageRole::Assistant);

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
async fn test_message_store_parallel_tool_calls() {
    use everruns_core::traits::MessageStore;

    let store = InMemoryMessageStore::new();
    let session_id = Uuid::now_v7();

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

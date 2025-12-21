// Integration tests for tool calling in the agent loop
//
// These tests verify that both built-in tools (via ToolRegistry) and
// the ToolExecutor trait work correctly together.

use async_trait::async_trait;
use everruns_core::{
    memory::{InMemoryEventEmitter, InMemoryMessageStore, MockLlmProvider, MockLlmResponse},
    tools::{EchoTool, FailingTool, GetCurrentTime, Tool, ToolExecutionResult, ToolRegistry},
    traits::ToolExecutor,
    AgentConfig, AgentLoop, LoopEvent, Message, MessageRole,
};
use everruns_core::{BuiltinTool, BuiltinToolKind, ToolCall, ToolDefinition, ToolPolicy};
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
        .tool(GetCurrentTime)
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
        kind: BuiltinToolKind::HttpGet,
        policy: ToolPolicy::Auto,
    });

    // Execute via ToolExecutor trait
    let result = registry.execute(&tool_call, &tool_def).await.unwrap();

    assert!(result.error.is_none());
    assert_eq!(result.result.unwrap()["echoed"], "Hello, World!");
}

#[tokio::test]
async fn test_get_current_time_tool() {
    let registry = ToolRegistry::builder().tool(GetCurrentTime).build();

    let tool_call = ToolCall {
        id: "call_time".to_string(),
        name: "get_current_time".to_string(),
        arguments: json!({"format": "unix"}),
    };

    let tool_def = ToolDefinition::Builtin(BuiltinTool {
        name: "get_current_time".to_string(),
        description: "Get time".to_string(),
        parameters: json!({}),
        kind: BuiltinToolKind::HttpGet,
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
        kind: BuiltinToolKind::HttpGet,
        policy: ToolPolicy::Auto,
    });

    // Execute and verify error is returned
    let result = registry.execute(&tool_call, &tool_def).await.unwrap();

    assert!(result.result.is_none());
    assert_eq!(result.error, Some("Expected test failure".to_string()));
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
        kind: BuiltinToolKind::HttpGet,
        policy: ToolPolicy::Auto,
    });

    // Execute and verify internal error is hidden
    let result = registry.execute(&tool_call, &tool_def).await.unwrap();

    assert!(result.result.is_none());
    // Internal error message should be replaced with generic message
    assert_eq!(
        result.error,
        Some("An internal error occurred while executing the tool".to_string())
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
        kind: BuiltinToolKind::HttpGet,
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
            kind: BuiltinToolKind::HttpGet,
            policy: ToolPolicy::Auto,
        })]);

    // Create a registry with built-in tools
    let registry = ToolRegistry::builder().tool(GetCurrentTime).build();

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

    let registry = ToolRegistry::builder().tool(GetCurrentTime).build();
    let event_emitter = InMemoryEventEmitter::new();

    let config = AgentConfig::new("You are a helpful assistant", "gpt-test")
        .with_max_iterations(5)
        .with_tools(vec![ToolDefinition::Builtin(BuiltinTool {
            name: "get_current_time".to_string(),
            description: "Get the current time".to_string(),
            parameters: json!({}),
            kind: BuiltinToolKind::HttpGet,
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
        kind: BuiltinToolKind::HttpGet,
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
        .tool(GetCurrentTime)
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
        kind: BuiltinToolKind::HttpGet,
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
        kind: BuiltinToolKind::HttpGet,
        policy: ToolPolicy::Auto,
    });

    let echo_result = registry.execute(&echo_call, &echo_def).await.unwrap();
    assert!(echo_result.error.is_none());
    assert_eq!(echo_result.result.unwrap()["echoed"], "Test message");
}

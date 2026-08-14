// Integration tests for tool calling
//
// These tests verify that both built-in tools (via ToolRegistry) and
// the ToolExecutor trait work correctly together.

use async_trait::async_trait;
use everruns_core::{
    Message, MessageRole,
    tool_execution::ToolExecutor,
    tools::{Tool, ToolExecutionResult, ToolRegistry},
};
use everruns_provider::ToolResultImage;
use everruns_provider::tool_types::{
    BuiltinTool, DeferrablePolicy, ToolCall, ToolDefinition, ToolHints, ToolPolicy,
};
use serde_json::json;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

struct EchoTool;

#[async_trait]
impl Tool for EchoTool {
    fn name(&self) -> &str {
        "echo"
    }

    fn description(&self) -> &str {
        "Echo the provided message"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({"type": "object"})
    }

    async fn execute(&self, arguments: serde_json::Value) -> ToolExecutionResult {
        let message = arguments
            .get("message")
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        ToolExecutionResult::success(json!({"echoed": message, "length": message.len()}))
    }
}

struct FailingTool {
    message: String,
    internal: bool,
}

impl FailingTool {
    fn with_tool_error(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            internal: false,
        }
    }

    fn with_internal_error(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            internal: true,
        }
    }
}

#[async_trait]
impl Tool for FailingTool {
    fn name(&self) -> &str {
        "failing_tool"
    }

    fn description(&self) -> &str {
        "Always fails"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({"type": "object"})
    }

    async fn execute(&self, _arguments: serde_json::Value) -> ToolExecutionResult {
        if self.internal {
            ToolExecutionResult::internal_error_msg(&self.message)
        } else {
            ToolExecutionResult::tool_error(&self.message)
        }
    }
}

// =============================================================================
// Tests for ToolRegistry as ToolExecutor
// =============================================================================

#[tokio::test]
async fn test_tool_registry_as_executor() {
    // Create a registry with built-in tools
    let registry = ToolRegistry::builder().tool(EchoTool).build();

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
        full_parameters: None,
    });

    // Execute via ToolExecutor trait
    let result = registry.execute(&tool_call, &tool_def).await.unwrap();

    assert!(result.error.is_none());
    assert_eq!(result.result.unwrap()["echoed"], "Hello, World!");
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
        full_parameters: None,
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
        full_parameters: None,
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
        full_parameters: None,
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
        full_parameters: None,
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
async fn test_registered_tool_executes() {
    let registry = ToolRegistry::builder().tool(EchoTool).build();

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
        full_parameters: None,
    });

    let echo_result = registry.execute(&echo_call, &echo_def).await.unwrap();
    assert!(echo_result.error.is_none());
    assert_eq!(echo_result.result.unwrap()["echoed"], "Test message");
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
    let llm_msg =
        everruns_core::llm_conversions::llm_message_from_message_with_images(&msg, &resolved);

    // Should be Tool role with tool_call_id
    assert_eq!(
        llm_msg.role,
        everruns_provider::driver_registry::LlmMessageRole::Tool
    );
    assert_eq!(llm_msg.tool_call_id, Some("call_456".to_string()));

    // Content should have text (JSON result) + 2 images
    match &llm_msg.content {
        everruns_provider::driver_registry::LlmMessageContent::Parts(parts) => {
            assert_eq!(parts.len(), 3, "should have 1 text + 2 images");
            assert!(matches!(
                &parts[0],
                everruns_provider::driver_registry::LlmContentPart::Text { .. }
            ));
            match &parts[1] {
                everruns_provider::driver_registry::LlmContentPart::Image { url } => {
                    assert!(url.starts_with("data:image/png;base64,"));
                    assert!(url.contains("AAAA"));
                }
                _ => panic!("Expected Image part"),
            }
            match &parts[2] {
                everruns_provider::driver_registry::LlmContentPart::Image { url } => {
                    assert!(url.starts_with("data:image/jpeg;base64,"));
                    assert!(url.contains("BBBB"));
                }
                _ => panic!("Expected Image part"),
            }
        }
        _ => panic!("Expected Parts content"),
    }
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
    let result = everruns_provider::tool_types::ToolResult {
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
    let parsed: everruns_provider::tool_types::ToolResult =
        serde_json::from_str(&json_str).unwrap();

    assert_eq!(parsed.images.as_ref().unwrap().len(), 1);
    assert_eq!(parsed.images.unwrap()[0].media_type, "image/png");
}

#[test]
fn test_tool_result_without_images_backward_compat() {
    // Ensure old JSON without `images` field still deserializes
    let json_str = r#"{"tool_call_id":"call_1","result":{"ok":true},"error":null}"#;
    let parsed: everruns_provider::tool_types::ToolResult = serde_json::from_str(json_str).unwrap();

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
    let result = everruns_provider::tool_types::ToolResult {
        tool_call_id: "call_conn".to_string(),
        result: Some(json!({"connection_required": "daytona"})),
        images: None,
        error: None,
        connection_required: Some("daytona".to_string()),
        raw_output: None,
    };

    let json_str = serde_json::to_string(&result).unwrap();
    let parsed: everruns_provider::tool_types::ToolResult =
        serde_json::from_str(&json_str).unwrap();

    assert_eq!(parsed.connection_required, Some("daytona".to_string()));
    assert_eq!(parsed.result.unwrap()["connection_required"], "daytona");
}

#[test]
fn test_tool_result_without_connection_required_backward_compat() {
    // Old JSON without connection_required field still deserializes
    let json_str = r#"{"tool_call_id":"call_1","result":{"ok":true},"error":null}"#;
    let parsed: everruns_provider::tool_types::ToolResult = serde_json::from_str(json_str).unwrap();

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

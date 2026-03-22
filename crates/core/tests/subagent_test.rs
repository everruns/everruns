// Integration tests for SubagentCapability
//
// Tests the subagent tools (spawn_subagent, get_subagents, message_subagent)
// via the Capability trait since the individual tool structs are not publicly exported.

use everruns_core::capabilities::{Capability, CapabilityStatus, SubagentCapability};
use everruns_core::tools::{Tool, ToolExecutionResult};
use everruns_core::traits::ToolContext;
use everruns_core::typed_id::SessionId;
use serde_json::json;
use uuid::Uuid;

// =============================================================================
// Helpers
// =============================================================================

/// Get tools from SubagentCapability, returning them in a predictable order.
fn subagent_tools() -> Vec<Box<dyn Tool>> {
    SubagentCapability.tools()
}

/// Find a tool by name from the capability's tool list.
fn find_tool(name: &str) -> Box<dyn Tool> {
    subagent_tools()
        .into_iter()
        .find(|t| t.name() == name)
        .unwrap_or_else(|| panic!("Tool '{name}' not found in SubagentCapability"))
}

/// Create a minimal ToolContext with no stores (triggers context-required errors).
fn empty_context() -> ToolContext {
    ToolContext::new(SessionId::from(Uuid::now_v7()))
}

// =============================================================================
// 1. Capability registration
// =============================================================================

#[test]
fn test_subagent_capability_registration() {
    let cap = SubagentCapability;

    assert_eq!(cap.id(), "subagents");
    assert_eq!(cap.name(), "Subagents");
    assert_eq!(cap.status(), CapabilityStatus::Available);
    assert_eq!(cap.tools().len(), 3);
    assert!(cap.system_prompt_addition().is_some());
    assert_eq!(cap.features(), vec!["subagents"]);

    // Verify system prompt mentions delegation and blueprints
    let prompt = cap.system_prompt_addition().unwrap();
    assert!(prompt.contains("spawn_subagent"));
    assert!(prompt.contains("blueprint"));
}

// =============================================================================
// 2. spawn_subagent — missing name
// =============================================================================

#[tokio::test]
async fn test_spawn_subagent_missing_name() {
    let tool = find_tool("spawn_subagent");

    // Call execute (no context) — should return ToolError about requiring context
    let result = tool.execute(json!({"task": "do something"})).await;
    assert!(
        matches!(result, ToolExecutionResult::ToolError(_)),
        "Expected ToolError when calling execute without context, got: {result:?}"
    );

    // Call execute_with_context with missing name — should return ToolError about missing param
    let ctx = empty_context();
    let result = tool
        .execute_with_context(json!({"task": "do something"}), &ctx)
        .await;
    match &result {
        ToolExecutionResult::ToolError(msg) => {
            assert!(
                msg.contains("name") || msg.contains("parameter") || msg.contains("platform_store"),
                "Error should mention 'name' or 'parameter', got: {msg}"
            );
        }
        _ => panic!("Expected ToolError for missing name, got: {result:?}"),
    }
}

// =============================================================================
// 3. spawn_subagent — missing task
// =============================================================================

#[tokio::test]
async fn test_spawn_subagent_missing_task() {
    let tool = find_tool("spawn_subagent");
    let ctx = empty_context();

    let result = tool
        .execute_with_context(json!({"name": "Test Runner"}), &ctx)
        .await;
    match &result {
        ToolExecutionResult::ToolError(msg) => {
            assert!(
                msg.contains("task") || msg.contains("parameter") || msg.contains("platform_store"),
                "Error should mention 'task' or 'parameter', got: {msg}"
            );
        }
        _ => panic!("Expected ToolError for missing task, got: {result:?}"),
    }
}

// =============================================================================
// 4. get_subagents — no context (execute without context)
// =============================================================================

#[tokio::test]
async fn test_get_subagents_no_context() {
    let tool = find_tool("get_subagents");
    let result = tool.execute(json!({})).await;
    assert!(
        matches!(result, ToolExecutionResult::ToolError(_)),
        "Expected ToolError when calling get_subagents without context, got: {result:?}"
    );
}

// =============================================================================
// 5. message_subagent — missing params
// =============================================================================

#[tokio::test]
async fn test_message_subagent_missing_params() {
    let tool = find_tool("message_subagent");

    // Without context
    let result = tool
        .execute(json!({"name_or_id": "Test", "message": "hello"}))
        .await;
    assert!(
        matches!(result, ToolExecutionResult::ToolError(_)),
        "Expected ToolError without context"
    );

    // With context but missing name_or_id
    let ctx = empty_context();
    let result = tool
        .execute_with_context(json!({"message": "hello"}), &ctx)
        .await;
    match &result {
        ToolExecutionResult::ToolError(msg) => {
            assert!(
                msg.contains("name_or_id")
                    || msg.contains("parameter")
                    || msg.contains("platform_store"),
                "Error should reference missing param, got: {msg}"
            );
        }
        _ => panic!("Expected ToolError for missing name_or_id, got: {result:?}"),
    }

    // With context but missing message
    let result = tool
        .execute_with_context(json!({"name_or_id": "Test"}), &ctx)
        .await;
    match &result {
        ToolExecutionResult::ToolError(msg) => {
            assert!(
                msg.contains("message")
                    || msg.contains("parameter")
                    || msg.contains("platform_store"),
                "Error should reference missing param, got: {msg}"
            );
        }
        _ => panic!("Expected ToolError for missing message, got: {result:?}"),
    }
}

// =============================================================================
// 6. spawn_subagent — schema validation
// =============================================================================

#[test]
fn test_spawn_subagent_schema_validation() {
    let tool = find_tool("spawn_subagent");
    let schema = tool.parameters_schema();

    // Check required fields
    let required = schema["required"]
        .as_array()
        .expect("required should be array");
    assert!(required.contains(&json!("name")));
    assert!(required.contains(&json!("task")));
    assert_eq!(required.len(), 2);

    // Check property types
    let props = &schema["properties"];
    assert_eq!(props["name"]["type"], "string");
    assert_eq!(props["task"]["type"], "string");

    // Check additionalProperties
    assert_eq!(schema["additionalProperties"], json!(false));
}

// =============================================================================
// 7. get_subagents — schema validation
// =============================================================================

#[test]
fn test_get_subagents_schema_validation() {
    let tool = find_tool("get_subagents");
    let schema = tool.parameters_schema();

    // No required fields
    assert!(
        schema.get("required").is_none()
            || schema["required"].as_array().is_none_or(|a| a.is_empty()),
        "get_subagents should have no required fields"
    );

    // Check optional properties
    let props = &schema["properties"];
    assert_eq!(props["name_or_id"]["type"], "string");
    assert_eq!(props["status_filter"]["type"], "string");

    // status_filter should have enum values
    let status_enum = props["status_filter"]["enum"]
        .as_array()
        .expect("status_filter should have enum");
    assert!(status_enum.contains(&json!("all")));
    assert!(status_enum.contains(&json!("running")));
    assert!(status_enum.contains(&json!("completed")));
    assert!(status_enum.contains(&json!("failed")));
}

// =============================================================================
// 8. message_subagent — schema validation
// =============================================================================

#[test]
fn test_message_subagent_schema_validation() {
    let tool = find_tool("message_subagent");
    let schema = tool.parameters_schema();

    // Check required fields
    let required = schema["required"]
        .as_array()
        .expect("required should be array");
    assert!(required.contains(&json!("name_or_id")));
    assert!(required.contains(&json!("message")));
    assert_eq!(required.len(), 2);

    // Check property types
    let props = &schema["properties"];
    assert_eq!(props["name_or_id"]["type"], "string");
    assert_eq!(props["message"]["type"], "string");
    assert_eq!(props["cancel"]["type"], "boolean");

    // cancel should have a default value
    assert_eq!(props["cancel"]["default"], json!(false));

    // cancel should NOT be in required
    assert!(!required.contains(&json!("cancel")));
}

// =============================================================================
// 9. Tool display names
// =============================================================================

#[test]
fn test_tool_display_names() {
    let tools = subagent_tools();

    let display_names: Vec<Option<&str>> = tools.iter().map(|t| t.display_name()).collect();

    assert_eq!(display_names[0], Some("Spawn Subagent"));
    assert_eq!(display_names[1], Some("Get Subagents"));
    assert_eq!(display_names[2], Some("Message Subagent"));
}

// =============================================================================
// 10. All tools require context
// =============================================================================

#[test]
fn test_tool_requires_context() {
    let tools = subagent_tools();

    for tool in &tools {
        assert!(
            tool.requires_context(),
            "Tool '{}' should require context",
            tool.name()
        );
    }
}

// =============================================================================
// Additional: tool names are correct and in expected order
// =============================================================================

#[test]
fn test_tool_names_and_order() {
    let tools = subagent_tools();
    let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();

    assert_eq!(
        names,
        vec!["spawn_subagent", "get_subagents", "message_subagent"]
    );
}

// =============================================================================
// Additional: execute_with_context on empty context returns platform_store error
// =============================================================================

#[tokio::test]
async fn test_spawn_subagent_no_platform_store() {
    let tool = find_tool("spawn_subagent");
    let ctx = empty_context();

    let result = tool
        .execute_with_context(json!({"name": "Runner", "task": "run tests"}), &ctx)
        .await;

    match &result {
        ToolExecutionResult::ToolError(msg) => {
            assert!(
                msg.contains("platform_store") || msg.contains("context"),
                "Should mention platform_store requirement, got: {msg}"
            );
        }
        _ => panic!("Expected ToolError for missing platform_store, got: {result:?}"),
    }
}

#[tokio::test]
async fn test_get_subagents_no_platform_store() {
    let tool = find_tool("get_subagents");
    let ctx = empty_context();

    let result = tool.execute_with_context(json!({}), &ctx).await;

    match &result {
        ToolExecutionResult::ToolError(msg) => {
            assert!(
                msg.contains("platform_store") || msg.contains("context"),
                "Should mention platform_store requirement, got: {msg}"
            );
        }
        _ => panic!("Expected ToolError for missing platform_store, got: {result:?}"),
    }
}

#[tokio::test]
async fn test_message_subagent_no_platform_store() {
    let tool = find_tool("message_subagent");
    let ctx = empty_context();

    let result = tool
        .execute_with_context(json!({"name_or_id": "Runner", "message": "hello"}), &ctx)
        .await;

    match &result {
        ToolExecutionResult::ToolError(msg) => {
            assert!(
                msg.contains("platform_store") || msg.contains("context"),
                "Should mention platform_store requirement, got: {msg}"
            );
        }
        _ => panic!("Expected ToolError for missing platform_store, got: {result:?}"),
    }
}

// =============================================================================
// Additional: capability metadata
// =============================================================================

#[test]
fn test_subagent_capability_metadata() {
    let cap = SubagentCapability;

    assert_eq!(cap.icon(), Some("git-branch"));
    assert_eq!(cap.category(), Some("Orchestration"));
    assert_eq!(
        cap.description(),
        "Spawn and manage subagents for parallel task execution in isolated context windows."
    );
}

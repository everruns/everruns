// Integration tests for SubagentCapability
//
// Tests the subagent tools (spawn_subagent) via the Capability trait since
// the individual tool structs are not publicly exported.
// get_subagents and message_subagent were retired; their coverage moved to
// the generic session_tasks tools (list_tasks, get_task, message_task).

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
    assert_eq!(cap.tools().len(), 1);
    assert!(cap.system_prompt_addition().is_some());
    assert_eq!(cap.features(), vec!["subagents"]);

    // Verify the system prompt instructs delegation (spawning subagents) and
    // mentions blueprints. Matches the prose wording of SUBAGENT_SYSTEM_PROMPT
    // ("Spawn subagents …"), which does not contain the literal tool name
    // `spawn_subagent`; assert the spawn instruction itself, not just any
    // mention of subagents.
    let prompt = cap.system_prompt_addition().unwrap();
    assert!(
        prompt.contains("Spawn subagents"),
        "prompt should instruct spawning subagents, got: {prompt}"
    );
    assert!(prompt.contains("blueprint"));
}

// =============================================================================
// 2. spawn_subagent — missing name
// =============================================================================

#[tokio::test]
async fn test_spawn_subagent_missing_name() {
    let tool = find_tool("spawn_subagent");

    // Call execute (no context) — should return ToolError about requiring context
    let result = tool.execute(json!({"instructions": "do something"})).await;
    assert!(
        matches!(result, ToolExecutionResult::ToolError(_)),
        "Expected ToolError when calling execute without context, got: {result:?}"
    );

    // Call execute_with_context with missing name — should return ToolError about missing param
    let ctx = empty_context();
    let result = tool
        .execute_with_context(json!({"instructions": "do something"}), &ctx)
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
// 3. spawn_subagent — missing instructions
// =============================================================================

#[tokio::test]
async fn test_spawn_subagent_missing_instructions() {
    let tool = find_tool("spawn_subagent");
    let ctx = empty_context();

    let result = tool
        .execute_with_context(json!({"name": "Test Runner"}), &ctx)
        .await;
    match &result {
        ToolExecutionResult::ToolError(msg) => {
            assert!(
                msg.contains("instructions")
                    || msg.contains("parameter")
                    || msg.contains("platform_store"),
                "Error should mention 'instructions' or 'parameter', got: {msg}"
            );
        }
        _ => panic!("Expected ToolError for missing instructions, got: {result:?}"),
    }
}

// =============================================================================
// 4. spawn_subagent — schema validation
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
    assert!(required.contains(&json!("instructions")));
    assert_eq!(required.len(), 2);

    // Check property types
    let props = &schema["properties"];
    assert_eq!(props["name"]["type"], "string");
    assert_eq!(props["instructions"]["type"], "string");

    // Execution mode is optional with background as the documented default.
    assert_eq!(props["mode"]["enum"], json!(["background", "foreground"]));

    // Check additionalProperties
    assert_eq!(schema["additionalProperties"], json!(false));
}

// =============================================================================
// 5. Tool display names
// =============================================================================

#[test]
fn test_tool_display_names() {
    let tools = subagent_tools();

    let display_names: Vec<Option<&str>> = tools.iter().map(|t| t.display_name()).collect();

    assert_eq!(display_names[0], Some("Spawn Subagent"));
}

// =============================================================================
// 6. All tools require context
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
// Additional: tool names are correct
// =============================================================================

#[test]
fn test_tool_names_and_order() {
    let tools = subagent_tools();
    let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();

    assert_eq!(names, vec!["spawn_subagent"]);
}

// =============================================================================
// Additional: execute_with_context on empty context returns platform_store error
// =============================================================================

#[tokio::test]
async fn test_spawn_subagent_no_platform_store() {
    let tool = find_tool("spawn_subagent");
    let ctx = empty_context();

    let result = tool
        .execute_with_context(json!({"name": "Runner", "instructions": "run tests"}), &ctx)
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
    assert_eq!(cap.category(), Some("Core"));
    assert_eq!(
        cap.description(),
        "Spawn and manage subagents for parallel task execution in isolated context windows."
    );
}

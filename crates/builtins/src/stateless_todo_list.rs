//! StatelessTodoList Capability - task list management for tracking work progress
//!
//! # Design Decision: Stateless Implementation
//!
//! This capability is intentionally **stateless** - it does not persist todos to a database.
//! State is maintained through conversation history (message storage).
//!
//! ## Why Stateless?
//!
//! This follows the same pattern as Claude Code's TodoWrite tool:
//! - Each `write_todos` call receives and returns the **complete** todo list
//! - The LLM remembers todos by reading previous tool calls from conversation history
//! - No separate storage layer needed - simpler implementation
//!
//! ## Alternative Approaches (from research)
//!
//! **LangChain DeepAgents TodoListMiddleware**:
//! - Uses dedicated `todos` state channel (not message history)
//! - Thread-scoped lifecycle with subagent isolation
//! - Known issue: context tokens grow quickly (proposed `auto_clean_context` flag)
//! - Reference: <https://deepwiki.com/langchain-ai/deepagents/2.4-state-management>
//!
//! **OpenAI Codex CLI update_plan**:
//! - Maintains plan history across resumed runs
//! - Supports "compacting conversation state" for longer sessions
//! - Reference: <https://github.com/openai/codex>
//!
//! ## Trade-offs
//!
//! | Approach | Pros | Cons |
//! |----------|------|------|
//! | Stateless (current) | Simple, no DB changes | Context grows with messages |
//! | State channel | Efficient context | Complex middleware needed |
//! | DB persistence | Survives context loss | Requires schema changes |
//!
//! ## Future Improvements
//!
//! Consider adding context compaction (prune old write_todos calls) if context
//! growth becomes an issue in long-running sessions.

use everruns_core::capabilities::{Capability, CapabilityLocalization, CapabilityStatus};
use everruns_core::tools::Tool;

pub const STATELESS_TODO_LIST_CAPABILITY_ID: &str = "stateless_todo_list";

/// Stateless Todo List capability - enables agents to create and manage task lists
/// for tracking work progress. State is maintained in conversation history.
pub struct StatelessTodoListCapability;

impl Capability for StatelessTodoListCapability {
    fn id(&self) -> &str {
        STATELESS_TODO_LIST_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "Task Management"
    }

    fn description(&self) -> &str {
        "Enables agents to create and manage structured task lists for tracking multi-step work progress. State is maintained in conversation history."
    }

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![CapabilityLocalization::text(
            "uk",
            "Керування завданнями",
            "Дає агентам змогу створювати структуровані списки завдань і керувати ними для відстеження прогресу багатоетапної роботи. Стан зберігається в історії розмови.",
        )]
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn icon(&self) -> Option<&str> {
        Some("list-checks")
    }

    fn category(&self) -> Option<&str> {
        Some("Core")
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some(SYSTEM_PROMPT)
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![Box::new(WriteTodosTool)]
    }
}

/// System prompt addition for StatelessTodoList capability.
///
/// Kept intentionally short — the tool schema describes fields and the
/// status enum. This prompt covers only behaviors the model cannot infer
/// from the schema.
const SYSTEM_PROMPT: &str = r#"## Task Management (`write_todos`)

Use for work spanning 3+ distinct steps. Skip for greetings, single-step
edits, or read-only checks.

Each `write_todos` call replaces the full list. Keep exactly one task
`in_progress`. Mark `completed` only when the step is fully done (tests
pass, no unresolved errors)."#;

// ============================================================================
// Tool: write_todos
// ============================================================================

/// Tool for creating and updating a task list
// EVE-884: the tool itself is part of the kernel default tool set, so it
// lives in core; this capability advertises and documents it.
pub use everruns_core::default_tools::WriteTodosTool;

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_core::tools::{Tool, ToolExecutionResult};

    // Metadata/tool-list constants covered by builtin_capabilities_satisfy_registry_invariants.

    #[test]
    fn test_capability_has_system_prompt() {
        let capability = StatelessTodoListCapability;

        let system_prompt = capability.system_prompt_addition().unwrap();
        assert!(system_prompt.contains("Task Management"));
        assert!(system_prompt.contains("write_todos"));
        assert!(system_prompt.contains("in_progress"));
        assert!(system_prompt.contains("completed"));
    }

    #[tokio::test]
    async fn test_write_todos_tool_valid_input() {
        let tool = WriteTodosTool;
        let result = tool
            .execute(serde_json::json!({
                "todos": [
                    {"content": "Run tests", "activeForm": "Running tests", "status": "completed"},
                    {"content": "Fix bug", "activeForm": "Fixing bug", "status": "in_progress"},
                    {"content": "Deploy", "activeForm": "Deploying", "status": "pending"}
                ]
            }))
            .await;

        if let ToolExecutionResult::Success(value) = result {
            assert!(value.get("success").unwrap().as_bool().unwrap());
            assert_eq!(value.get("total_tasks").unwrap().as_u64().unwrap(), 3);
            assert_eq!(value.get("pending").unwrap().as_u64().unwrap(), 1);
            assert_eq!(value.get("in_progress").unwrap().as_u64().unwrap(), 1);
            assert_eq!(value.get("completed").unwrap().as_u64().unwrap(), 1);
            assert!(value.get("warning").is_none());
        } else {
            panic!("Expected success");
        }
    }

    #[tokio::test]
    async fn test_write_todos_tool_warning_no_in_progress() {
        let tool = WriteTodosTool;
        let result = tool
            .execute(serde_json::json!({
                "todos": [
                    {"content": "Task 1", "activeForm": "Doing task 1", "status": "pending"},
                    {"content": "Task 2", "activeForm": "Doing task 2", "status": "pending"}
                ]
            }))
            .await;

        if let ToolExecutionResult::Success(value) = result {
            assert!(value.get("warning").is_some());
            assert!(
                value
                    .get("warning")
                    .unwrap()
                    .as_str()
                    .unwrap()
                    .contains("No task is marked as 'in_progress'")
            );
        } else {
            panic!("Expected success");
        }
    }

    #[tokio::test]
    async fn test_write_todos_tool_warning_multiple_in_progress() {
        let tool = WriteTodosTool;
        let result = tool
            .execute(serde_json::json!({
                "todos": [
                    {"content": "Task 1", "activeForm": "Doing task 1", "status": "in_progress"},
                    {"content": "Task 2", "activeForm": "Doing task 2", "status": "in_progress"}
                ]
            }))
            .await;

        if let ToolExecutionResult::Success(value) = result {
            assert!(value.get("warning").is_some());
            assert!(
                value
                    .get("warning")
                    .unwrap()
                    .as_str()
                    .unwrap()
                    .contains("Multiple tasks")
            );
        } else {
            panic!("Expected success");
        }
    }

    #[tokio::test]
    async fn test_write_todos_tool_empty_list() {
        let tool = WriteTodosTool;
        let result = tool
            .execute(serde_json::json!({
                "todos": []
            }))
            .await;

        if let ToolExecutionResult::Success(value) = result {
            assert!(value.get("success").unwrap().as_bool().unwrap());
            assert_eq!(value.get("total_tasks").unwrap().as_u64().unwrap(), 0);
        } else {
            panic!("Expected success");
        }
    }

    #[tokio::test]
    async fn test_write_todos_tool_missing_content() {
        let tool = WriteTodosTool;
        let result = tool
            .execute(serde_json::json!({
                "todos": [
                    {"activeForm": "Doing task", "status": "pending"}
                ]
            }))
            .await;

        if let ToolExecutionResult::ToolError(msg) = result {
            assert!(msg.contains("content"));
        } else {
            panic!("Expected tool error");
        }
    }

    #[tokio::test]
    async fn test_write_todos_tool_invalid_status() {
        let tool = WriteTodosTool;
        let result = tool
            .execute(serde_json::json!({
                "todos": [
                    {"content": "Task", "activeForm": "Doing task", "status": "invalid"}
                ]
            }))
            .await;

        if let ToolExecutionResult::ToolError(msg) = result {
            assert!(msg.contains("invalid status"));
        } else {
            panic!("Expected tool error");
        }
    }

    #[tokio::test]
    async fn test_write_todos_tool_all_completed_no_warning() {
        let tool = WriteTodosTool;
        let result = tool
            .execute(serde_json::json!({
                "todos": [
                    {"content": "Task 1", "activeForm": "Doing task 1", "status": "completed"},
                    {"content": "Task 2", "activeForm": "Doing task 2", "status": "completed"}
                ]
            }))
            .await;

        if let ToolExecutionResult::Success(value) = result {
            // No warning when all tasks are completed (end of workflow)
            assert!(value.get("warning").is_none());
        } else {
            panic!("Expected success");
        }
    }
}

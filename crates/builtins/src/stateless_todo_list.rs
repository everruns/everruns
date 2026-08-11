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
use everruns_core::tool_types::ToolHints;
use everruns_core::tools::{Tool, ToolExecutionResult};
use async_trait::async_trait;
use serde_json::Value;

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
pub struct WriteTodosTool;

#[async_trait]
impl Tool for WriteTodosTool {
    fn narrate(
        &self,
        _tool_call: &everruns_core::tool_types::ToolCall,
        phase: everruns_core::tool_narration::ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: everruns_core::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        Some(everruns_core::tool_narration::narrate_write_todos(phase, locale))
    }

    fn name(&self) -> &str {
        "write_todos"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Write Todos")
    }

    fn description(&self) -> &str {
        "Create or update a task list for tracking multi-step work. Each task must have 'content' (imperative form like 'Run tests'), 'activeForm' (present continuous like 'Running tests'), and 'status' (pending/in_progress/completed). Exactly one task should be 'in_progress' at a time."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "todos": {
                    "type": "array",
                    "description": "Complete list of tasks (replaces any existing tasks)",
                    "items": {
                        "type": "object",
                        "properties": {
                            "content": {
                                "type": "string",
                                "minLength": 1,
                                "description": "Imperative form of the task (e.g., 'Run tests', 'Fix the bug', 'Build the project')"
                            },
                            "activeForm": {
                                "type": "string",
                                "minLength": 1,
                                "description": "Present continuous form shown during execution (e.g., 'Running tests', 'Fixing the bug', 'Building the project')"
                            },
                            "status": {
                                "type": "string",
                                "enum": ["pending", "in_progress", "completed"],
                                "description": "Current status of the task"
                            }
                        },
                        "required": ["content", "activeForm", "status"]
                    }
                }
            },
            "required": ["todos"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        // Each call replaces the entire shared todo list; serialize concurrent
        // writes within a batch so one does not clobber another.
        ToolHints::default()
            .with_idempotent(true)
            .with_concurrency_class("session_todos")
    }

    async fn execute(&self, arguments: Value) -> ToolExecutionResult {
        // Parse the todos array
        let todos = match arguments.get("todos") {
            Some(Value::Array(arr)) => arr,
            Some(_) => {
                return ToolExecutionResult::tool_error("Invalid 'todos' field: expected an array");
            }
            None => {
                return ToolExecutionResult::tool_error("Missing required field: 'todos'");
            }
        };

        // Validate each todo item
        let mut pending_count = 0;
        let mut in_progress_count = 0;
        let mut completed_count = 0;
        let mut validated_todos = Vec::new();

        for (idx, todo) in todos.iter().enumerate() {
            // Validate content
            let content = match todo.get("content").and_then(|v| v.as_str()) {
                Some(s) if !s.is_empty() => s,
                _ => {
                    return ToolExecutionResult::tool_error(format!(
                        "Task {} is missing or has empty 'content' field",
                        idx + 1
                    ));
                }
            };

            // Validate activeForm
            let active_form = match todo.get("activeForm").and_then(|v| v.as_str()) {
                Some(s) if !s.is_empty() => s,
                _ => {
                    return ToolExecutionResult::tool_error(format!(
                        "Task {} is missing or has empty 'activeForm' field",
                        idx + 1
                    ));
                }
            };

            // Validate status
            let status = match todo.get("status").and_then(|v| v.as_str()) {
                Some("pending") => {
                    pending_count += 1;
                    "pending"
                }
                Some("in_progress") => {
                    in_progress_count += 1;
                    "in_progress"
                }
                Some("completed") => {
                    completed_count += 1;
                    "completed"
                }
                Some(other) => {
                    return ToolExecutionResult::tool_error(format!(
                        "Task {} has invalid status '{}'. Must be 'pending', 'in_progress', or 'completed'",
                        idx + 1,
                        other
                    ));
                }
                None => {
                    return ToolExecutionResult::tool_error(format!(
                        "Task {} is missing 'status' field",
                        idx + 1
                    ));
                }
            };

            validated_todos.push(serde_json::json!({
                "content": content,
                "activeForm": active_form,
                "status": status
            }));
        }

        // Warn if no task is in progress (but don't fail - this can happen at the end of a workflow)
        let warning = if in_progress_count == 0 && pending_count > 0 {
            Some("No task is marked as 'in_progress'. Consider marking one task as in_progress.")
        } else if in_progress_count > 1 {
            Some(
                "Multiple tasks are marked as 'in_progress'. Best practice is to have exactly one in_progress task at a time.",
            )
        } else {
            None
        };

        let total = validated_todos.len();

        let mut result = serde_json::json!({
            "success": true,
            "total_tasks": total,
            "pending": pending_count,
            "in_progress": in_progress_count,
            "completed": completed_count,
            "todos": validated_todos
        });

        if let Some(warn_msg) = warning {
            result["warning"] = serde_json::json!(warn_msg);
        }

        ToolExecutionResult::success(result)
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use everruns_core::capabilities::*;

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

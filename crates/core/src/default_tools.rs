//! Tools in the kernel's default executor registry.
//!
//! [`ToolRegistry::with_defaults`](crate::tools::ToolRegistry::with_defaults)
//! and the scheduled-probe registry hand these to the executor regardless of
//! which capabilities a session selected, so they stay in core while the
//! capabilities that advertise them to the model live in `everruns-builtins`
//! (EVE-884).

use crate::tool_types::ToolHints;
use crate::tools::{Tool, ToolExecutionResult};
use async_trait::async_trait;
use serde_json::Value;

/// Returns the current date and time in a requested timezone/format.
pub struct GetCurrentTimeTool;

#[async_trait]
impl Tool for GetCurrentTimeTool {
    fn narrate(
        &self,
        _tool_call: &crate::tool_types::ToolCall,
        phase: crate::tool_narration::ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: crate::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        Some(crate::tool_narration::narrate_current_time(phase, locale))
    }

    fn name(&self) -> &str {
        "get_current_time"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Get Current Time")
    }

    fn description(&self) -> &str {
        "Get the current date and time. Can return time in different formats and timezones."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "timezone": {
                    "type": "string",
                    "description": "Timezone to return the time in (e.g., 'UTC', 'America/New_York', 'Europe/London'). Defaults to UTC."
                },
                "format": {
                    "type": "string",
                    "enum": ["iso8601", "unix", "human"],
                    "description": "Output format: 'iso8601' for ISO 8601 format, 'unix' for Unix timestamp, 'human' for human-readable format. Defaults to 'iso8601'."
                }
            },
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_readonly(true)
            .with_idempotent(true)
    }

    async fn execute(&self, arguments: Value) -> ToolExecutionResult {
        let format = arguments
            .get("format")
            .and_then(|v| v.as_str())
            .unwrap_or("iso8601");

        let _timezone = arguments
            .get("timezone")
            .and_then(|v| v.as_str())
            .unwrap_or("UTC");

        // Note: For simplicity, we're using UTC. Full timezone support would require
        // the chrono-tz crate which adds significant dependencies.
        let now = chrono::Utc::now();

        let result = match format {
            "unix" => serde_json::json!({
                "timestamp": now.timestamp(),
                "format": "unix",
                "timezone": "UTC"
            }),
            "human" => serde_json::json!({
                "datetime": now.format("%A, %B %d, %Y at %H:%M:%S UTC").to_string(),
                "format": "human",
                "timezone": "UTC"
            }),
            _ => serde_json::json!({
                "datetime": now.to_rfc3339(),
                "format": "iso8601",
                "timezone": "UTC"
            }),
        };

        ToolExecutionResult::success(result)
    }
}

/// Replaces the session's todo list with the model-supplied set.
pub struct WriteTodosTool;

#[async_trait]
impl Tool for WriteTodosTool {
    fn narrate(
        &self,
        _tool_call: &crate::tool_types::ToolCall,
        phase: crate::tool_narration::ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: crate::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        Some(crate::tool_narration::narrate_write_todos(phase, locale))
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

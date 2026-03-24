//! Tool implementations for PI sandbox coding agent.

use async_trait::async_trait;
use everruns_core::ToolHints;
use everruns_core::tools::{Tool, ToolExecutionResult};
use everruns_core::traits::ToolContext;
use serde_json::{Value, json};

use crate::PI_DEFAULT_MODEL;
use crate::state::{
    PiAgentStatus, get_agent_state, list_agent_states, new_agent_id, parse_timeout_seconds,
    release_agent_lease, required_str, save_agent_state, touch_agent_lease,
};

// =============================================================================
// Tool: start_pi
// =============================================================================

pub struct StartPiTool;

#[async_trait]
impl Tool for StartPiTool {
    fn name(&self) -> &str {
        "start_pi"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Start PI")
    }

    fn description(&self) -> &str {
        "Start a PI coding agent in an isolated sandbox. PI will create a sandbox, checkout the repository, and begin working on the task autonomously. Returns a pi_agent_id for tracking."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "task": {
                    "type": "string",
                    "description": "Task description — what PI should do (e.g. 'Fix the auth bug in login flow')."
                },
                "repo": {
                    "type": "string",
                    "description": "Repository to checkout (e.g. 'owner/repo'). Optional if working in an existing sandbox."
                },
                "branch": {
                    "type": "string",
                    "description": "Branch to checkout. Defaults to the repo's default branch."
                },
                "model": {
                    "type": "string",
                    "description": "LLM model for PI to use (e.g. 'claude-sonnet-4-20250514', 'gpt-4o'). Defaults to claude-sonnet-4-20250514."
                },
                "sandbox_id": {
                    "type": "string",
                    "description": "Existing sandbox ID to reuse. If omitted, a new sandbox is created."
                },
                "timeout_seconds": {
                    "type": "integer",
                    "minimum": 60,
                    "description": "Maximum time for the PI agent to run, in seconds. Default: 3600 (1 hour)."
                }
            },
            "required": ["task"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_open_world(true)
            .with_requires_secrets(true)
            .with_long_running(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "start_pi requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let task = match required_str(&arguments, "task") {
            Ok(v) => v.to_string(),
            Err(e) => return e,
        };
        let timeout_seconds = match parse_timeout_seconds(&arguments) {
            Ok(v) => v,
            Err(e) => return e,
        };

        let model = arguments
            .get("model")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or(PI_DEFAULT_MODEL)
            .to_string();

        let repo = arguments
            .get("repo")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());

        let branch = arguments
            .get("branch")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());

        let sandbox_id = arguments
            .get("sandbox_id")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("pending")
            .to_string();

        let agent_id = new_agent_id();

        let state = crate::state::PiAgentState {
            agent_id: agent_id.clone(),
            sandbox_id: sandbox_id.clone(),
            repo: repo.clone(),
            branch: branch.clone(),
            model: model.clone(),
            status: PiAgentStatus::Starting,
            task: task.clone(),
            started_at: chrono::Utc::now().to_rfc3339(),
            timeout_seconds,
        };

        if let Err(err) = save_agent_state(context, &state).await {
            return err;
        }
        if let Err(err) = touch_agent_lease(
            context,
            &state,
            Some(format!("PI: {}", truncate_task(&task, 60))),
        )
        .await
        {
            return err;
        }

        // TODO: Phase 2 — actually provision sandbox via E2B/Daytona capability,
        // clone repo, install PI CLI, and start the agent process.
        // For now, we record the intent and return the agent handle.

        ToolExecutionResult::success(json!({
            "pi_agent_id": agent_id,
            "status": "starting",
            "model": model,
            "repo": repo,
            "branch": branch,
            "sandbox_id": sandbox_id,
            "task": task,
            "timeout_seconds": timeout_seconds,
        }))
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// =============================================================================
// Tool: check_pi
// =============================================================================

pub struct CheckPiTool;

#[async_trait]
impl Tool for CheckPiTool {
    fn name(&self) -> &str {
        "check_pi"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Check PI")
    }

    fn description(&self) -> &str {
        "Check the status of a PI agent by ID, or list all PI agents in this session."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pi_agent_id": {
                    "type": "string",
                    "description": "PI agent ID to check. Omit to list all PI agents."
                }
            },
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_readonly(true)
            .with_idempotent(true)
            .with_requires_secrets(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "check_pi requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let pi_agent_id = arguments
            .get("pi_agent_id")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty());

        if let Some(agent_id) = pi_agent_id {
            // Single agent detail
            let state = match get_agent_state(context, agent_id).await {
                Ok(s) => s,
                Err(e) => return e,
            };

            ToolExecutionResult::success(json!({
                "pi_agent_id": state.agent_id,
                "sandbox_id": state.sandbox_id,
                "repo": state.repo,
                "branch": state.branch,
                "model": state.model,
                "status": state.status.to_string(),
                "task": state.task,
                "started_at": state.started_at,
                "timeout_seconds": state.timeout_seconds,
            }))
        } else {
            // List all agents
            match list_agent_states(context).await {
                Ok(states) => ToolExecutionResult::success(json!({
                    "agents": states.iter().map(|s| json!({
                        "pi_agent_id": s.agent_id,
                        "model": s.model,
                        "status": s.status.to_string(),
                        "task": truncate_task(&s.task, 100),
                        "started_at": s.started_at,
                        "repo": s.repo,
                    })).collect::<Vec<_>>(),
                    "count": states.len(),
                })),
                Err(e) => e,
            }
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// =============================================================================
// Tool: message_pi
// =============================================================================

pub struct MessagePiTool;

#[async_trait]
impl Tool for MessagePiTool {
    fn name(&self) -> &str {
        "message_pi"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Message PI")
    }

    fn description(&self) -> &str {
        "Send a follow-up message to a running PI agent. Use to steer its work, ask for status updates, or cancel it."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pi_agent_id": {
                    "type": "string",
                    "description": "PI agent ID to message."
                },
                "message": {
                    "type": "string",
                    "description": "Message to send to the PI agent."
                },
                "cancel": {
                    "type": "boolean",
                    "description": "If true, cancel the PI agent after delivering the message.",
                    "default": false
                }
            },
            "required": ["pi_agent_id", "message"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_open_world(true)
            .with_long_running(true)
            .with_requires_secrets(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "message_pi requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let agent_id = match required_str(&arguments, "pi_agent_id") {
            Ok(v) => v.to_string(),
            Err(e) => return e,
        };
        let message = match required_str(&arguments, "message") {
            Ok(v) => v.to_string(),
            Err(e) => return e,
        };
        let cancel = arguments
            .get("cancel")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let mut state = match get_agent_state(context, &agent_id).await {
            Ok(s) => s,
            Err(e) => return e,
        };

        if cancel {
            state.status = PiAgentStatus::Cancelled;
            if let Err(e) = save_agent_state(context, &state).await {
                return e;
            }
            if let Err(e) = release_agent_lease(context, &agent_id).await {
                return e;
            }

            return ToolExecutionResult::success(json!({
                "pi_agent_id": agent_id,
                "delivered": true,
                "cancel_requested": true,
                "status": "cancelled",
                "message": message,
            }));
        }

        // TODO: Phase 2 — forward message to the running PI process in the sandbox.
        // For now, acknowledge delivery.

        if let Err(e) = touch_agent_lease(context, &state, None).await {
            return e;
        }

        ToolExecutionResult::success(json!({
            "pi_agent_id": agent_id,
            "delivered": true,
            "status": state.status.to_string(),
            "message": message,
        }))
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// =============================================================================
// Helpers
// =============================================================================

fn truncate_task(task: &str, max_len: usize) -> String {
    let char_count = task.chars().count();
    if char_count <= max_len {
        task.to_string()
    } else if max_len <= 3 {
        "...".to_string()
    } else {
        let prefix: String = task.chars().take(max_len - 3).collect();
        format!("{prefix}...")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_pi_tool_schema_has_required_fields() {
        let schema = StartPiTool.parameters_schema();
        let required = schema["required"].as_array().unwrap();
        assert!(required.contains(&json!("task")));
    }

    #[test]
    fn start_pi_tool_schema_has_model_field() {
        let schema = StartPiTool.parameters_schema();
        let props = schema["properties"].as_object().unwrap();
        assert!(props.contains_key("model"));
        assert!(props.contains_key("repo"));
        assert!(props.contains_key("branch"));
        assert!(props.contains_key("sandbox_id"));
    }

    #[tokio::test]
    async fn no_context_tools_return_context_error() {
        let result = StartPiTool.execute(json!({})).await;
        assert!(matches!(result, ToolExecutionResult::ToolError(_)));

        let result = CheckPiTool.execute(json!({})).await;
        assert!(matches!(result, ToolExecutionResult::ToolError(_)));

        let result = MessagePiTool.execute(json!({})).await;
        assert!(matches!(result, ToolExecutionResult::ToolError(_)));
    }

    #[test]
    fn truncate_task_short() {
        assert_eq!(truncate_task("hello", 10), "hello");
    }

    #[test]
    fn truncate_task_long() {
        assert_eq!(truncate_task("hello world this is long", 10), "hello w...");
    }
}

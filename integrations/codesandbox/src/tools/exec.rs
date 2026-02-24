//! Execution tools: run commands and check status.

use crate::client::CodeSandboxClient;
use crate::state::*;
use crate::types::*;

use async_trait::async_trait;
use everruns_core::tools::{Tool, ToolExecutionResult};
use everruns_core::traits::ToolContext;
use serde_json::{Value, json};
use tracing::{debug, warn};

// ----------------------------------------------------------------------------
// CsbExecTool
// ----------------------------------------------------------------------------

pub struct CsbExecTool;

#[async_trait]
impl Tool for CsbExecTool {
    fn name(&self) -> &str {
        "csb_exec"
    }

    fn description(&self) -> &str {
        "Execute a shell command in a CodeSandbox VM. Set wait=true (default) to get output, \
         or wait=false to get an exec_id for polling with csb_exec_status."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "sandbox_id": {
                    "type": "string",
                    "description": "Sandbox ID to execute in"
                },
                "command": {
                    "type": "string",
                    "description": "Shell command to execute (e.g., 'python app.py')"
                },
                "wait": {
                    "type": "boolean",
                    "description": "Wait for completion and return output (default: true). Set false for long-running commands."
                }
            },
            "required": ["sandbox_id", "command"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "csb_exec requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let sandbox_id = match required_str(&arguments, "sandbox_id") {
            Ok(s) => s,
            Err(e) => return e,
        };
        let command = match required_str(&arguments, "command") {
            Ok(s) => s,
            Err(e) => return e,
        };
        let wait = arguments
            .get("wait")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        let api_key = match get_api_key(context).await {
            Ok(k) => k,
            Err(e) => return e,
        };
        let state = match get_sandbox_state(context, sandbox_id).await {
            Ok(s) => s,
            Err(e) => return e,
        };

        let client = CodeSandboxClient::new(api_key);

        // Exec API requires command and args split: {"command": "bash", "args": ["-c", "..."]}
        debug!("Executing in sandbox {sandbox_id}: {command}");
        let exec_info = match client
            .exec_create(&state, "bash", vec!["-c".to_string(), command.to_string()])
            .await
        {
            Ok(info) => info,
            Err(e) => return ToolExecutionResult::tool_error(e),
        };

        if !wait {
            return ToolExecutionResult::success(json!({
                "exec_id": exec_info.id,
                "status": exec_info.status,
                "message": "Command started. Use csb_exec_status to check progress."
            }));
        }

        // Poll until completion
        let start = std::time::Instant::now();
        loop {
            if start.elapsed() > EXEC_POLL_MAX_WAIT {
                return ToolExecutionResult::success(json!({
                    "exec_id": exec_info.id,
                    "status": "timeout",
                    "message": "Command still running after timeout. Use csb_exec_status to check."
                }));
            }

            tokio::time::sleep(EXEC_POLL_INTERVAL).await;

            match client.exec_get(&state, &exec_info.id).await {
                Ok(status) => {
                    let is_done = status.status == "finished"
                        || status.status == "EXITED"
                        || status.status == "exited"
                        || status.status == "error"
                        || status.exit_code.is_some();

                    if is_done {
                        // Get output
                        let output = client
                            .exec_get_output(&state, &exec_info.id)
                            .await
                            .unwrap_or_default();

                        return ToolExecutionResult::success(json!({
                            "exec_id": exec_info.id,
                            "status": status.status,
                            "exit_code": status.exit_code,
                            "output": output
                        }));
                    }
                }
                Err(e) => {
                    warn!("Error polling exec status: {e}");
                }
            }
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ----------------------------------------------------------------------------
// CsbExecStatusTool
// ----------------------------------------------------------------------------

pub struct CsbExecStatusTool;

#[async_trait]
impl Tool for CsbExecStatusTool {
    fn name(&self) -> &str {
        "csb_exec_status"
    }

    fn description(&self) -> &str {
        "Check execution status and get output for a command started with csb_exec (wait=false)."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "sandbox_id": {
                    "type": "string",
                    "description": "Sandbox ID"
                },
                "exec_id": {
                    "type": "string",
                    "description": "Execution ID returned by csb_exec"
                }
            },
            "required": ["sandbox_id", "exec_id"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "csb_exec_status requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let sandbox_id = match required_str(&arguments, "sandbox_id") {
            Ok(s) => s,
            Err(e) => return e,
        };
        let exec_id = match required_str(&arguments, "exec_id") {
            Ok(s) => s,
            Err(e) => return e,
        };

        let api_key = match get_api_key(context).await {
            Ok(k) => k,
            Err(e) => return e,
        };
        let state = match get_sandbox_state(context, sandbox_id).await {
            Ok(s) => s,
            Err(e) => return e,
        };

        let client = CodeSandboxClient::new(api_key);

        let exec_info = match client.exec_get(&state, exec_id).await {
            Ok(info) => info,
            Err(e) => return ToolExecutionResult::tool_error(e),
        };

        let output = client
            .exec_get_output(&state, exec_id)
            .await
            .unwrap_or_default();

        ToolExecutionResult::success(json!({
            "exec_id": exec_info.id,
            "status": exec_info.status,
            "exit_code": exec_info.exit_code,
            "output": output
        }))
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ----------------------------------------------------------------------------
// Shared helper
// ----------------------------------------------------------------------------

/// Poll exec completion and return output. Used by git clone tool.
pub async fn poll_exec_completion(
    client: &CodeSandboxClient,
    state: &SandboxState,
    exec_id: &str,
) -> Result<String, ToolExecutionResult> {
    let start = std::time::Instant::now();
    loop {
        if start.elapsed() > EXEC_POLL_MAX_WAIT {
            return Ok("(timed out waiting for completion)".to_string());
        }

        match client.exec_get(state, exec_id).await {
            Ok(info) if info.status == "finished" => {
                return match client.exec_get_output(state, exec_id).await {
                    Ok(output) => Ok(output),
                    Err(_) => Ok(String::new()),
                };
            }
            Ok(_) => {
                tokio::time::sleep(EXEC_POLL_INTERVAL).await;
            }
            Err(e) => {
                return Err(ToolExecutionResult::tool_error(format!(
                    "Failed to check exec status: {e}"
                )));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_core::tools::Tool;

    #[test]
    fn test_exec_schema_has_required_fields() {
        let tool = CsbExecTool;
        let schema = tool.parameters_schema();
        let required = schema["required"].as_array().unwrap();
        let required_strs: Vec<&str> = required.iter().map(|v| v.as_str().unwrap()).collect();
        assert!(required_strs.contains(&"sandbox_id"));
        assert!(required_strs.contains(&"command"));
    }

    #[test]
    fn test_exec_schema_has_wait_field() {
        let tool = CsbExecTool;
        let schema = tool.parameters_schema();
        assert!(schema["properties"]["wait"]["type"].as_str() == Some("boolean"));
    }

    #[test]
    fn test_exec_status_schema_has_required_fields() {
        let tool = CsbExecStatusTool;
        let schema = tool.parameters_schema();
        let required = schema["required"].as_array().unwrap();
        let required_strs: Vec<&str> = required.iter().map(|v| v.as_str().unwrap()).collect();
        assert!(required_strs.contains(&"sandbox_id"));
        assert!(required_strs.contains(&"exec_id"));
    }

    #[tokio::test]
    async fn test_exec_without_context() {
        let tool = CsbExecTool;
        let result = tool
            .execute(json!({"sandbox_id": "test", "command": "ls"}))
            .await;
        match result {
            ToolExecutionResult::ToolError(msg) => assert!(msg.contains("requires context")),
            _ => panic!("Expected tool error"),
        }
    }

    #[tokio::test]
    async fn test_exec_status_without_context() {
        let tool = CsbExecStatusTool;
        let result = tool
            .execute(json!({"sandbox_id": "test", "exec_id": "e1"}))
            .await;
        match result {
            ToolExecutionResult::ToolError(msg) => assert!(msg.contains("requires context")),
            _ => panic!("Expected tool error"),
        }
    }

    #[test]
    fn test_all_exec_tools_require_context() {
        assert!(CsbExecTool.requires_context());
        assert!(CsbExecStatusTool.requires_context());
    }
}

//! Tool implementations for Deno sandbox operations.
//!
//! Decision: keep the surface small for the first version: create, exec, read,
//! write, list, delete.

use async_trait::async_trait;
use everruns_core::ToolHints;
use everruns_core::tools::{Tool, ToolExecutionResult};
use everruns_core::traits::ToolContext;
use serde_json::{Value, json};
use tracing::debug;

use crate::client::{CreateSandboxRequest, DenoClient};
use crate::state::{
    SandboxState, delete_sandbox_state, get_credentials, get_sandbox_state, list_sandbox_states,
    release_sandbox_lease, required_str, save_sandbox_state, touch_sandbox_lease,
};
use crate::{DENO_DEFAULT_MEMORY_MB, DENO_MAX_MEMORY_MB, DENO_SANDBOX_TIMEOUT};

fn parse_memory_mb(arguments: &Value) -> Result<Option<u64>, String> {
    let Some(value) = arguments.get("memory_mb") else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let memory_mb = value
        .as_u64()
        .ok_or_else(|| "Invalid 'memory_mb': must be a positive integer".to_string())?;
    if memory_mb == 0 || memory_mb > DENO_MAX_MEMORY_MB {
        return Err(format!(
            "Invalid 'memory_mb': must be between 1 and {DENO_MAX_MEMORY_MB}"
        ));
    }
    Ok(Some(memory_mb))
}

fn parse_timeout_seconds(timeout: &str) -> Result<u64, String> {
    if timeout == "session" {
        return Err("Deno sandboxes cannot use timeout='session' because Everruns closes the creator websocket after each tool. Use a concrete duration like '20m'.".to_string());
    }
    if let Some(minutes) = timeout.strip_suffix('m') {
        let minutes = minutes
            .parse::<u64>()
            .map_err(|_| "Invalid timeout: expected e.g. '20m' or '600s'".to_string())?;
        return minutes
            .checked_mul(60)
            .ok_or_else(|| "Timeout too large".to_string());
    }
    if let Some(seconds) = timeout.strip_suffix('s') {
        return seconds
            .parse::<u64>()
            .map_err(|_| "Invalid timeout: expected e.g. '20m' or '600s'".to_string());
    }
    Err("Invalid timeout: expected e.g. '20m' or '600s'".to_string())
}

pub struct DenoCreateSandboxTool;

#[async_trait]
impl Tool for DenoCreateSandboxTool {
    fn name(&self) -> &str {
        "deno_create_sandbox"
    }

    fn description(&self) -> &str {
        "Create a new Deno sandbox. Returns the sandbox id and workspace path."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "title": { "type": "string", "description": "Sandbox label shown in Deno" },
                "region": { "type": "string", "description": "Region (for example 'ord' or 'ams')" },
                "timeout": { "type": "string", "description": "Sandbox lifetime, e.g. '20m' or '600s'", "default": DENO_SANDBOX_TIMEOUT },
                "memory_mb": { "type": "integer", "description": "Sandbox memory in MiB (1-16384)", "minimum": 1, "maximum": DENO_MAX_MEMORY_MB, "default": DENO_DEFAULT_MEMORY_MB },
                "allow_net": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional outbound network allowlist hosts/IPs"
                }
            },
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
            "deno_create_sandbox requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let credentials = match get_credentials(context).await {
            Ok(credentials) => credentials,
            Err(error) => return error,
        };

        let timeout = arguments
            .get("timeout")
            .and_then(Value::as_str)
            .unwrap_or(DENO_SANDBOX_TIMEOUT);
        let timeout_seconds = match parse_timeout_seconds(timeout) {
            Ok(timeout) => timeout,
            Err(error) => return ToolExecutionResult::tool_error(error),
        };
        let memory_mb = match parse_memory_mb(&arguments) {
            Ok(memory_mb) => memory_mb,
            Err(error) => return ToolExecutionResult::tool_error(error),
        };
        let region = arguments
            .get("region")
            .and_then(Value::as_str)
            .map(str::to_string);
        let title = arguments
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("Everruns Deno Sandbox");
        let allow_net = arguments
            .get("allow_net")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        let mut labels = serde_json::Map::new();
        labels.insert("everruns".to_string(), json!("true"));
        labels.insert(
            "everruns.session_id".to_string(),
            json!(context.session_id.to_string()),
        );
        labels.insert("everruns.title".to_string(), json!(title));
        if let Some(session_store) = &context.session_store
            && let Ok(Some(session)) = session_store.get_session(context.session_id).await
        {
            labels.insert(
                "everruns.harness_id".to_string(),
                json!(session.harness_id.to_string()),
            );
            labels.insert(
                "everruns.org_id".to_string(),
                json!(session.organization_id.to_string()),
            );
            if let Some(agent_id) = &session.agent_id {
                labels.insert("everruns.agent_id".to_string(), json!(agent_id.to_string()));
            }
        }

        let client = DenoClient::new(credentials.token, credentials.org);
        let created = match client
            .create_sandbox(CreateSandboxRequest {
                region,
                timeout_seconds: Some(timeout_seconds),
                memory_mb,
                labels,
                allow_net,
            })
            .await
        {
            Ok(created) => created,
            Err(error) => return ToolExecutionResult::tool_error(error),
        };

        let state = SandboxState {
            sandbox_id: created.sandbox_id.clone(),
            region: created.region.clone(),
            org: client.org().map(str::to_string),
            workspace_path: created.workspace_path.clone(),
            started_at: chrono::Utc::now().to_rfc3339(),
        };
        if let Err(error) = save_sandbox_state(context, &state).await {
            return error;
        }
        if let Err(error) = touch_sandbox_lease(context, &state, Some(title.to_string())).await {
            return error;
        }

        ToolExecutionResult::success(json!({
            "sandbox_id": created.sandbox_id,
            "region": created.region,
            "workspace_path": created.workspace_path,
            "status": "running",
            "timeout": timeout,
        }))
    }

    fn requires_context(&self) -> bool {
        true
    }
}

pub struct DenoExecTool;

#[async_trait]
impl Tool for DenoExecTool {
    fn name(&self) -> &str {
        "deno_exec"
    }

    fn description(&self) -> &str {
        "Execute a shell command in a Deno sandbox. Returns stdout, stderr, and exit code."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "sandbox_id": { "type": "string", "description": "Sandbox ID" },
                "command": { "type": "string", "description": "Shell command to run" },
                "cwd": { "type": "string", "description": "Optional working directory" }
            },
            "required": ["sandbox_id", "command"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_open_world(true)
            .with_requires_secrets(true)
            .with_long_running(true)
            .with_persist_output(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "deno_exec requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let sandbox_id = match required_str(&arguments, "sandbox_id") {
            Ok(value) => value,
            Err(error) => return error,
        };
        let command = match required_str(&arguments, "command") {
            Ok(value) => value,
            Err(error) => return error,
        };
        let cwd = arguments.get("cwd").and_then(Value::as_str);

        let credentials = match get_credentials(context).await {
            Ok(credentials) => credentials,
            Err(error) => return error,
        };
        let state = match get_sandbox_state(context, sandbox_id).await {
            Ok(state) => state,
            Err(error) => return error,
        };

        let client = DenoClient::new(credentials.token, credentials.org);
        debug!(sandbox_id, command, "Executing command in Deno sandbox");
        let exec = match client.exec(sandbox_id, &state.region, command, cwd).await {
            Ok(exec) => exec,
            Err(error) => return ToolExecutionResult::tool_error(error),
        };

        if let Err(error) = touch_sandbox_lease(context, &state, None).await {
            return error;
        }

        {
            use everruns_core::tool_output_sanitizer::{
                EXEC_OUTPUT_BUDGET, clean_exec_output, middle_truncate,
            };
            let clean_stdout = clean_exec_output(&exec.stdout);
            let clean_stderr = clean_exec_output(&exec.stderr);
            let stdout = middle_truncate(&clean_stdout, EXEC_OUTPUT_BUDGET);
            let stderr = middle_truncate(&clean_stderr, 4096);
            let mut raw = clean_stdout;
            if !clean_stderr.is_empty() {
                raw.push_str("\n--- stderr ---\n");
                raw.push_str(&clean_stderr);
            }
            ToolExecutionResult::success_with_raw_output(
                json!({
                    "exit_code": exec.exit_code,
                    "stdout": stdout,
                    "stderr": stderr,
                }),
                raw,
            )
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

pub struct DenoReadFileTool;

#[async_trait]
impl Tool for DenoReadFileTool {
    fn name(&self) -> &str {
        "deno_read_file"
    }

    fn description(&self) -> &str {
        "Read a text file from a Deno sandbox filesystem."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "sandbox_id": { "type": "string", "description": "Sandbox ID" },
                "path": { "type": "string", "description": "Path to read" }
            },
            "required": ["sandbox_id", "path"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_readonly(true)
            .with_open_world(true)
            .with_requires_secrets(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "deno_read_file requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let sandbox_id = match required_str(&arguments, "sandbox_id") {
            Ok(value) => value,
            Err(error) => return error,
        };
        let path = match required_str(&arguments, "path") {
            Ok(value) => value,
            Err(error) => return error,
        };

        let credentials = match get_credentials(context).await {
            Ok(credentials) => credentials,
            Err(error) => return error,
        };
        let state = match get_sandbox_state(context, sandbox_id).await {
            Ok(state) => state,
            Err(error) => return error,
        };

        let client = DenoClient::new(credentials.token, credentials.org);
        let content = match client.read_text_file(sandbox_id, &state.region, path).await {
            Ok(content) => content,
            Err(error) => return ToolExecutionResult::tool_error(error),
        };
        if let Err(error) = touch_sandbox_lease(context, &state, None).await {
            return error;
        }

        ToolExecutionResult::success(json!({
            "path": path,
            "content": content,
        }))
    }

    fn requires_context(&self) -> bool {
        true
    }
}

pub struct DenoWriteFileTool;

#[async_trait]
impl Tool for DenoWriteFileTool {
    fn name(&self) -> &str {
        "deno_write_file"
    }

    fn description(&self) -> &str {
        "Write a text file into a Deno sandbox filesystem."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "sandbox_id": { "type": "string", "description": "Sandbox ID" },
                "path": { "type": "string", "description": "Path to write" },
                "content": { "type": "string", "description": "File content" }
            },
            "required": ["sandbox_id", "path", "content"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_open_world(true)
            .with_requires_secrets(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "deno_write_file requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let sandbox_id = match required_str(&arguments, "sandbox_id") {
            Ok(value) => value,
            Err(error) => return error,
        };
        let path = match required_str(&arguments, "path") {
            Ok(value) => value,
            Err(error) => return error,
        };
        let content = match required_str(&arguments, "content") {
            Ok(value) => value,
            Err(error) => return error,
        };

        let credentials = match get_credentials(context).await {
            Ok(credentials) => credentials,
            Err(error) => return error,
        };
        let state = match get_sandbox_state(context, sandbox_id).await {
            Ok(state) => state,
            Err(error) => return error,
        };

        let client = DenoClient::new(credentials.token, credentials.org);
        match client
            .write_text_file(sandbox_id, &state.region, path, content)
            .await
        {
            Ok(()) => {}
            Err(error) => return ToolExecutionResult::tool_error(error),
        }
        if let Err(error) = touch_sandbox_lease(context, &state, None).await {
            return error;
        }

        ToolExecutionResult::success(json!({
            "path": path,
            "success": true,
        }))
    }

    fn requires_context(&self) -> bool {
        true
    }
}

pub struct DenoListSandboxesTool;

#[async_trait]
impl Tool for DenoListSandboxesTool {
    fn name(&self) -> &str {
        "deno_list_sandboxes"
    }

    fn description(&self) -> &str {
        "List all Deno sandboxes created in this session."
    }

    fn parameters_schema(&self) -> Value {
        json!({ "type": "object", "properties": {}, "additionalProperties": false })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_readonly(true)
            .with_idempotent(true)
            .with_open_world(true)
            .with_requires_secrets(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "deno_list_sandboxes requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        _arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let sandboxes = match list_sandbox_states(context).await {
            Ok(sandboxes) => sandboxes,
            Err(error) => return error,
        };

        ToolExecutionResult::success(json!({
            "sandboxes": sandboxes.iter().map(|state| {
                json!({
                    "sandbox_id": state.sandbox_id,
                    "region": state.region,
                    "workspace_path": state.workspace_path,
                    "started_at": state.started_at,
                })
            }).collect::<Vec<_>>(),
            "count": sandboxes.len(),
        }))
    }

    fn requires_context(&self) -> bool {
        true
    }
}

pub struct DenoManageSandboxTool;

#[async_trait]
impl Tool for DenoManageSandboxTool {
    fn name(&self) -> &str {
        "deno_manage_sandbox"
    }

    fn description(&self) -> &str {
        "Delete a Deno sandbox."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "sandbox_id": { "type": "string", "description": "Sandbox ID" },
                "action": { "type": "string", "enum": ["delete"], "description": "Lifecycle action" }
            },
            "required": ["sandbox_id", "action"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_open_world(true)
            .with_requires_secrets(true)
            .with_destructive(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "deno_manage_sandbox requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let sandbox_id = match required_str(&arguments, "sandbox_id") {
            Ok(value) => value,
            Err(error) => return error,
        };
        let action = match required_str(&arguments, "action") {
            Ok(value) => value,
            Err(error) => return error,
        };
        if action != "delete" {
            return ToolExecutionResult::tool_error(
                "Unsupported action. Deno sandboxes currently support only action='delete'.",
            );
        }

        let credentials = match get_credentials(context).await {
            Ok(credentials) => credentials,
            Err(error) => return error,
        };
        let state = match get_sandbox_state(context, sandbox_id).await {
            Ok(state) => state,
            Err(error) => return error,
        };

        let client = DenoClient::new(credentials.token, credentials.org);
        if let Err(error) = client.delete_sandbox(sandbox_id, &state.region).await {
            return ToolExecutionResult::tool_error(error);
        }
        if let Err(error) = delete_sandbox_state(context, sandbox_id).await {
            return error;
        }
        if let Err(error) = release_sandbox_lease(context, sandbox_id).await {
            return error;
        }

        ToolExecutionResult::success(json!({
            "sandbox_id": sandbox_id,
            "action": action,
            "success": true,
        }))
    }

    fn requires_context(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_timeout_rejects_session() {
        let err = parse_timeout_seconds("session").unwrap_err();
        assert!(err.contains("session"));
    }

    #[test]
    fn parse_memory_validates_bounds() {
        assert!(parse_memory_mb(&json!({"memory_mb": 0})).is_err());
        assert!(
            parse_memory_mb(&json!({"memory_mb": 1280}))
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn create_tool_mentions_fixed_timeout() {
        let tool = DenoCreateSandboxTool;
        let schema = tool.parameters_schema();
        assert_eq!(
            schema["properties"]["timeout"]["default"],
            DENO_SANDBOX_TIMEOUT
        );
        assert_eq!(crate::DENO_WORKSPACE_PATH, "/home/sandbox");
    }
}

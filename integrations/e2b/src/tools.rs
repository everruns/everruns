//! Tool implementations for E2B sandboxes.

use async_trait::async_trait;
use everruns_core::ToolHints;
use everruns_core::tools::{Tool, ToolExecutionResult};
use everruns_core::traits::ToolContext;
use serde_json::{Value, json};
use tracing::warn;

use crate::client::E2BClient;
use crate::state::{
    E2BSandboxDetail, build_state, delete_sandbox_state, get_api_key, get_sandbox_state,
    list_sandbox_states, release_sandbox_lease, required_str, save_sandbox_state,
    touch_sandbox_lease,
};
use crate::{E2B_DEFAULT_TEMPLATE, E2B_DEFAULT_TIMEOUT_SECS, E2B_DEFAULT_WORKSPACE_PATH};

fn parse_timeout_seconds(arguments: &Value) -> Result<u64, ToolExecutionResult> {
    match arguments.get("timeout_seconds") {
        None => Ok(E2B_DEFAULT_TIMEOUT_SECS),
        Some(value) => value
            .as_u64()
            .filter(|timeout| *timeout > 0)
            .ok_or_else(|| {
                ToolExecutionResult::tool_error(
                    "Invalid 'timeout_seconds': must be a positive integer",
                )
            }),
    }
}

fn detail_from_create(create: crate::state::E2BSandboxCreateResponse) -> E2BSandboxDetail {
    E2BSandboxDetail {
        client_id: create.client_id,
        cpu_count: 0,
        disk_size_mb: 0,
        end_at: String::new(),
        envd_version: create.envd_version,
        memory_mb: 0,
        sandbox_id: create.sandbox_id,
        started_at: chrono::Utc::now().to_rfc3339(),
        state: "running".to_string(),
        template_id: create.template_id,
        alias: create.alias,
        domain: create.domain,
        envd_access_token: create.envd_access_token,
        metadata: json!({}),
    }
}

pub struct E2BCreateSandboxTool;

#[async_trait]
impl Tool for E2BCreateSandboxTool {
    fn name(&self) -> &str {
        "e2b_create_sandbox"
    }

    fn description(&self) -> &str {
        "Create a new E2B cloud sandbox. Optionally upload files from session storage."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "title": {"type": "string", "description": "Human-readable sandbox name"},
                "template": {"type": "string", "description": "E2B template ID (defaults to base)"},
                "timeout_seconds": {"type": "integer", "description": "Sandbox TTL in seconds", "minimum": 1, "default": E2B_DEFAULT_TIMEOUT_SECS},
                "env_vars": {
                    "type": "object",
                    "description": "Environment variables injected into the sandbox",
                    "additionalProperties": {"type": "string"}
                },
                "upload_files": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "session_path": {"type": "string"},
                            "sandbox_path": {"type": "string"}
                        },
                        "required": ["session_path", "sandbox_path"],
                        "additionalProperties": false
                    }
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
            "e2b_create_sandbox requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let api_key = match get_api_key(context).await {
            Ok(key) => key,
            Err(err) => return err,
        };
        let timeout_seconds = match parse_timeout_seconds(&arguments) {
            Ok(timeout) => timeout,
            Err(err) => return err,
        };
        let template = arguments
            .get("template")
            .and_then(|v| v.as_str())
            .unwrap_or(E2B_DEFAULT_TEMPLATE);
        let title = arguments
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("Everruns E2B Sandbox");

        let mut metadata = serde_json::Map::new();
        metadata.insert("everruns".to_string(), json!("true"));
        metadata.insert("everruns.title".to_string(), json!(title));
        metadata.insert(
            "everruns.session_id".to_string(),
            json!(context.session_id.to_string()),
        );
        if let Some(session_store) = &context.session_store
            && let Ok(Some(session)) = session_store.get_session(context.session_id).await
        {
            metadata.insert(
                "everruns.harness_id".to_string(),
                json!(session.harness_id.to_string()),
            );
            metadata.insert(
                "everruns.org_id".to_string(),
                json!(session.organization_id),
            );
            if let Some(agent_id) = session.agent_id {
                metadata.insert("everruns.agent_id".to_string(), json!(agent_id.to_string()));
            }
        }

        let env_vars = arguments
            .get("env_vars")
            .cloned()
            .unwrap_or_else(|| json!({}));

        let client = E2BClient::new(api_key);
        let create = match client
            .create_sandbox(template, timeout_seconds, json!(metadata), env_vars)
            .await
        {
            Ok(create) => create,
            Err(err) => return ToolExecutionResult::tool_error(err),
        };

        let detail = detail_from_create(create);
        let state = build_state(&detail, timeout_seconds);
        if let Err(err) = save_sandbox_state(context, &state).await {
            return err;
        }
        if let Err(err) = touch_sandbox_lease(context, &state, Some(title.to_string())).await {
            return err;
        }

        let mut uploaded_count = 0;
        if let Some(file_specs) = arguments.get("upload_files").and_then(|v| v.as_array()) {
            let Some(file_store) = context.file_store.as_ref() else {
                return ToolExecutionResult::tool_error("File store not available for file upload");
            };
            for (index, file_spec) in file_specs.iter().enumerate() {
                let Some(session_path) = file_spec.get("session_path").and_then(|v| v.as_str())
                else {
                    return ToolExecutionResult::tool_error(format!(
                        "Invalid upload_files[{index}]: missing required 'session_path'"
                    ));
                };
                let Some(sandbox_path) = file_spec.get("sandbox_path").and_then(|v| v.as_str())
                else {
                    return ToolExecutionResult::tool_error(format!(
                        "Invalid upload_files[{index}]: missing required 'sandbox_path'"
                    ));
                };
                match file_store.read_file(context.session_id, session_path).await {
                    Ok(Some(file)) => {
                        if let Some(content) = file.content {
                            if let Err(err) =
                                client.write_file(&state, sandbox_path, &content).await
                            {
                                warn!("Failed to upload {session_path} to E2B sandbox: {err}");
                            } else {
                                uploaded_count += 1;
                            }
                        }
                    }
                    Ok(None) => warn!("Session file not found during E2B upload: {session_path}"),
                    Err(err) => warn!("Failed to read {session_path} for E2B upload: {err}"),
                }
            }
        }

        ToolExecutionResult::success(json!({
            "sandbox_id": state.sandbox_id,
            "status": "running",
            "template": template,
            "workspace_path": state.workspace_path,
            "timeout_seconds": timeout_seconds,
            "files_uploaded": uploaded_count,
        }))
    }

    fn requires_context(&self) -> bool {
        true
    }
}

pub struct E2BExecTool;

#[async_trait]
impl Tool for E2BExecTool {
    fn name(&self) -> &str {
        "e2b_exec"
    }

    fn description(&self) -> &str {
        "Execute a shell command in an E2B sandbox. Returns output synchronously."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "sandbox_id": {"type": "string"},
                "command": {"type": "string"},
                "cwd": {"type": "string", "description": "Working directory inside the sandbox"},
                "timeout_ms": {"type": "integer", "minimum": 1, "description": "Command timeout in milliseconds"}
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
            "e2b_exec requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let api_key = match get_api_key(context).await {
            Ok(key) => key,
            Err(err) => return err,
        };
        let sandbox_id = match required_str(&arguments, "sandbox_id") {
            Ok(value) => value,
            Err(err) => return err,
        };
        let command = match required_str(&arguments, "command") {
            Ok(value) => value,
            Err(err) => return err,
        };
        let cwd = arguments.get("cwd").and_then(|v| v.as_str());
        let timeout_ms = arguments.get("timeout_ms").and_then(|v| v.as_u64());
        let state = match get_sandbox_state(context, sandbox_id).await {
            Ok(state) => state,
            Err(err) => return err,
        };

        let client = E2BClient::new(api_key);
        let result = match client.exec(&state, command, cwd, timeout_ms).await {
            Ok(result) => result,
            Err(err) => return ToolExecutionResult::tool_error(err),
        };

        if let Err(err) = client
            .set_timeout(&state.sandbox_id, state.timeout_seconds)
            .await
        {
            warn!("Failed to refresh E2B sandbox timeout after exec: {err}");
        }
        if let Err(err) = touch_sandbox_lease(context, &state, None).await {
            return err;
        }

        {
            use everruns_core::tool_output_sanitizer::{
                EXEC_OUTPUT_BUDGET, clean_exec_output, priority_aware_truncate,
            };
            let clean_stdout = clean_exec_output(&result.stdout);
            let clean_stderr = clean_exec_output(&result.stderr);
            let stdout = priority_aware_truncate(&clean_stdout, EXEC_OUTPUT_BUDGET);
            let stderr = priority_aware_truncate(&clean_stderr, 4096);
            let mut raw = clean_stdout;
            if !clean_stderr.is_empty() {
                raw.push_str("\n--- stderr ---\n");
                raw.push_str(&clean_stderr);
            }
            ToolExecutionResult::success_with_raw_output(
                json!({
                    "sandbox_id": sandbox_id,
                    "cwd": cwd.unwrap_or(E2B_DEFAULT_WORKSPACE_PATH),
                    "stdout": stdout,
                    "stderr": stderr,
                    "exit_code": result.exit_code,
                    "error": result.error,
                }),
                raw,
            )
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

pub struct E2BReadFileTool;

#[async_trait]
impl Tool for E2BReadFileTool {
    fn name(&self) -> &str {
        "e2b_read_file"
    }

    fn description(&self) -> &str {
        "Read a file from an E2B sandbox."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "sandbox_id": {"type": "string"},
                "path": {"type": "string"}
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
            "e2b_read_file requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let api_key = match get_api_key(context).await {
            Ok(key) => key,
            Err(err) => return err,
        };
        let sandbox_id = match required_str(&arguments, "sandbox_id") {
            Ok(value) => value,
            Err(err) => return err,
        };
        let path = match required_str(&arguments, "path") {
            Ok(value) => value,
            Err(err) => return err,
        };
        let state = match get_sandbox_state(context, sandbox_id).await {
            Ok(state) => state,
            Err(err) => return err,
        };
        let client = E2BClient::new(api_key);
        match client.read_file(&state, path).await {
            Ok(content) => ToolExecutionResult::success(json!({
                "sandbox_id": sandbox_id,
                "path": path,
                "content": content,
            })),
            Err(err) => ToolExecutionResult::tool_error(err),
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

pub struct E2BWriteFileTool;

#[async_trait]
impl Tool for E2BWriteFileTool {
    fn name(&self) -> &str {
        "e2b_write_file"
    }

    fn description(&self) -> &str {
        "Write a file into an E2B sandbox. Creates parent directories when supported by the sandbox runtime."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "sandbox_id": {"type": "string"},
                "path": {"type": "string"},
                "content": {"type": "string"}
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
            "e2b_write_file requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let api_key = match get_api_key(context).await {
            Ok(key) => key,
            Err(err) => return err,
        };
        let sandbox_id = match required_str(&arguments, "sandbox_id") {
            Ok(value) => value,
            Err(err) => return err,
        };
        let path = match required_str(&arguments, "path") {
            Ok(value) => value,
            Err(err) => return err,
        };
        let content = match required_str(&arguments, "content") {
            Ok(value) => value,
            Err(err) => return err,
        };
        let state = match get_sandbox_state(context, sandbox_id).await {
            Ok(state) => state,
            Err(err) => return err,
        };
        let client = E2BClient::new(api_key);
        match client.write_file(&state, path, content).await {
            Ok(()) => ToolExecutionResult::success(json!({
                "sandbox_id": sandbox_id,
                "path": path,
                "success": true,
            })),
            Err(err) => ToolExecutionResult::tool_error(err),
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

pub struct E2BListSandboxesTool;

#[async_trait]
impl Tool for E2BListSandboxesTool {
    fn name(&self) -> &str {
        "e2b_list_sandboxes"
    }

    fn description(&self) -> &str {
        "List E2B sandboxes created in the current session."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        })
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
            "e2b_list_sandboxes requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        _arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        match list_sandbox_states(context).await {
            Ok(states) => ToolExecutionResult::success(json!({
                "sandboxes": states.iter().map(|state| json!({
                    "sandbox_id": state.sandbox_id,
                    "sandbox_domain": state.sandbox_domain,
                    "workspace_path": state.workspace_path,
                    "started_at": state.started_at,
                    "timeout_seconds": state.timeout_seconds,
                })).collect::<Vec<_>>(),
                "count": states.len(),
            })),
            Err(err) => err,
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

pub struct E2BManageSandboxTool;

#[async_trait]
impl Tool for E2BManageSandboxTool {
    fn name(&self) -> &str {
        "e2b_manage_sandbox"
    }

    fn description(&self) -> &str {
        "Manage an E2B sandbox lifecycle. Supported actions: pause, resume, delete."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "sandbox_id": {"type": "string"},
                "action": {"type": "string", "enum": ["pause", "resume", "delete"]},
                "timeout_seconds": {"type": "integer", "minimum": 1, "description": "Timeout to apply when resuming a sandbox"}
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
            "e2b_manage_sandbox requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let api_key = match get_api_key(context).await {
            Ok(key) => key,
            Err(err) => return err,
        };
        let sandbox_id = match required_str(&arguments, "sandbox_id") {
            Ok(value) => value,
            Err(err) => return err,
        };
        let action = match required_str(&arguments, "action") {
            Ok(value) => value,
            Err(err) => return err,
        };
        let state = match get_sandbox_state(context, sandbox_id).await {
            Ok(state) => state,
            Err(err) => return err,
        };
        let client = E2BClient::new(api_key);

        match action {
            "pause" => {
                if let Err(err) = client.pause_sandbox(sandbox_id).await {
                    return ToolExecutionResult::tool_error(err);
                }
                if let Err(err) = touch_sandbox_lease(context, &state, None).await {
                    return err;
                }
                ToolExecutionResult::success(json!({
                    "sandbox_id": sandbox_id,
                    "action": action,
                    "success": true,
                }))
            }
            "resume" => {
                let timeout_seconds = arguments
                    .get("timeout_seconds")
                    .and_then(|value| value.as_u64())
                    .unwrap_or(state.timeout_seconds);
                let resumed = match client.resume_sandbox(sandbox_id, timeout_seconds).await {
                    Ok(resumed) => resumed,
                    Err(err) => return ToolExecutionResult::tool_error(err),
                };
                let detail = detail_from_create(resumed);
                let refreshed = build_state(&detail, timeout_seconds);
                if let Err(err) = save_sandbox_state(context, &refreshed).await {
                    return err;
                }
                if let Err(err) = touch_sandbox_lease(context, &refreshed, None).await {
                    return err;
                }
                ToolExecutionResult::success(json!({
                    "sandbox_id": sandbox_id,
                    "action": action,
                    "success": true,
                    "timeout_seconds": timeout_seconds,
                }))
            }
            "delete" => {
                if let Err(err) = client.delete_sandbox(sandbox_id).await {
                    return ToolExecutionResult::tool_error(err);
                }
                if let Err(err) = delete_sandbox_state(context, sandbox_id).await {
                    return err;
                }
                if let Err(err) = release_sandbox_lease(context, sandbox_id).await {
                    return err;
                }
                ToolExecutionResult::success(json!({
                    "sandbox_id": sandbox_id,
                    "action": action,
                    "success": true,
                }))
            }
            _ => ToolExecutionResult::tool_error(
                "Invalid action. Expected one of: pause, resume, delete.",
            ),
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_sandbox_tool_schema_requires_expected_fields() {
        let schema = E2BCreateSandboxTool.parameters_schema();
        let upload_files = schema
            .get("properties")
            .and_then(|value| value.get("upload_files"))
            .and_then(|value| value.get("items"))
            .and_then(|value| value.get("required"))
            .and_then(Value::as_array)
            .expect("upload_files required array");

        let required: Vec<_> = upload_files.iter().filter_map(Value::as_str).collect();
        assert_eq!(required, vec!["session_path", "sandbox_path"]);
    }

    #[tokio::test]
    async fn no_context_tools_return_context_error() {
        let result = E2BExecTool.execute(json!({})).await;
        match result {
            ToolExecutionResult::ToolError(message) => {
                assert!(message.contains("requires context"));
            }
            other => panic!("expected ToolError, got {other:?}"),
        }
    }
}

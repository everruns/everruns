//! Session Sandbox capability.
//!
//! One managed sandbox per session. The concrete provider is chosen by config
//! (`provider: "daytona"` initially), while the tool surface stays stable.

use super::{Capability, CapabilityStatus};
pub use crate::session_sandbox::SESSION_SANDBOX_CAPABILITY_ID;
use crate::session_sandbox::{
    SessionSandboxConfig, create_session_sandbox_provider, delete_session_sandbox,
    ensure_session_sandbox_running, load_session_sandbox_state, pause_session_sandbox,
    session_sandbox_tool_hints,
};
use crate::tools::{Tool, ToolExecutionResult};
use crate::traits::ToolContext;
use async_trait::async_trait;
use serde_json::{Value, json};

pub struct SessionSandboxCapability;

impl Capability for SessionSandboxCapability {
    fn id(&self) -> &str {
        SESSION_SANDBOX_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "Session Sandbox"
    }

    fn description(&self) -> &str {
        "One managed sandbox owned by the current session. Supports exec and file operations with provider-managed lifecycle."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn icon(&self) -> Option<&str> {
        Some("terminal")
    }

    fn category(&self) -> Option<&str> {
        Some("Execution")
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some(
            "Use the managed sandbox tools for code execution and filesystem access. \
             You do not need to create or pick a sandbox: this session owns a single managed sandbox. \
             Use `sandbox_exec` for shell commands, `sandbox_read_file`/`sandbox_write_file` for file I/O, \
             `sandbox_status` to inspect lifecycle state, and `sandbox_manage` for pause/resume/delete.",
        )
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        self.tools_with_config(&json!({}))
    }

    fn tools_with_config(&self, config: &Value) -> Vec<Box<dyn Tool>> {
        vec![
            Box::new(SandboxExecTool::new(config.clone())),
            Box::new(SandboxReadFileTool::new(config.clone())),
            Box::new(SandboxWriteFileTool::new(config.clone())),
            Box::new(SandboxStatusTool::new(config.clone())),
            Box::new(SandboxManageTool::new(config.clone())),
        ]
    }

    fn dependencies(&self) -> Vec<&'static str> {
        vec!["session_storage"]
    }

    fn features(&self) -> Vec<&'static str> {
        vec!["managed_sandbox"]
    }
}

fn parse_config(config: &Value) -> Result<SessionSandboxConfig, ToolExecutionResult> {
    let config: SessionSandboxConfig = serde_json::from_value(config.clone()).map_err(|e| {
        ToolExecutionResult::tool_error(format!("Invalid session_sandbox capability config: {e}"))
    })?;

    if config.provider.trim().is_empty() {
        return Err(ToolExecutionResult::tool_error(
            "session_sandbox capability requires a non-empty provider",
        ));
    }
    if config.idle_pause_after_seconds == 0 {
        return Err(ToolExecutionResult::tool_error(
            "session_sandbox idle_pause_after_seconds must be >= 1",
        ));
    }

    Ok(config)
}

fn provider_for_config(
    config: &SessionSandboxConfig,
) -> Result<Box<dyn crate::SessionSandboxProvider>, ToolExecutionResult> {
    create_session_sandbox_provider(&config.provider).ok_or_else(|| {
        ToolExecutionResult::tool_error(format!(
            "Session sandbox provider '{}' is not registered",
            config.provider
        ))
    })
}

#[derive(Clone)]
pub struct SandboxExecTool {
    config: Value,
}

impl SandboxExecTool {
    pub fn new(config: Value) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for SandboxExecTool {
    fn name(&self) -> &str {
        "sandbox_exec"
    }

    fn description(&self) -> &str {
        "Execute a shell command inside the session-managed sandbox."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": { "type": "string", "description": "Shell command to execute" },
                "cwd": { "type": "string", "description": "Optional working directory inside the sandbox" },
                "timeout_ms": { "type": "integer", "minimum": 1, "description": "Optional execution timeout in milliseconds" }
            },
            "required": ["command"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> crate::ToolHints {
        session_sandbox_tool_hints()
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "sandbox_exec requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let config = match parse_config(&self.config) {
            Ok(config) => config,
            Err(err) => return err,
        };
        let Some(command) = arguments.get("command").and_then(|v| v.as_str()) else {
            return ToolExecutionResult::tool_error("Missing required parameter: command");
        };
        let timeout_ms = match arguments.get("timeout_ms") {
            None => None,
            Some(value) => match value.as_u64() {
                Some(timeout_ms) if timeout_ms > 0 => Some(timeout_ms),
                _ => {
                    return ToolExecutionResult::tool_error(
                        "timeout_ms must be a positive integer",
                    );
                }
            },
        };
        let provider = match provider_for_config(&config) {
            Ok(provider) => provider,
            Err(err) => return err,
        };
        let state = match ensure_session_sandbox_running(context, &config).await {
            Ok(state) => state,
            Err(err) => return err,
        };

        match provider
            .exec(
                context,
                &config,
                &state.instance,
                &crate::SessionSandboxExecRequest {
                    command: command.to_string(),
                    cwd: arguments
                        .get("cwd")
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string),
                    timeout_ms,
                    output_mode: "concise".to_string(),
                },
            )
            .await
        {
            Ok(response) => ToolExecutionResult::success(json!({
                "exit_code": response.exit_code,
                "output": response.output,
                "raw_output": response.raw_output,
                "hint": response.hint,
            })),
            Err(err) => err,
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

#[derive(Clone)]
pub struct SandboxReadFileTool {
    config: Value,
}

impl SandboxReadFileTool {
    pub fn new(config: Value) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for SandboxReadFileTool {
    fn name(&self) -> &str {
        "sandbox_read_file"
    }

    fn description(&self) -> &str {
        "Read a file from the session-managed sandbox filesystem."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Path to read inside the sandbox" }
            },
            "required": ["path"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> crate::ToolHints {
        session_sandbox_tool_hints().with_readonly(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "sandbox_read_file requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let config = match parse_config(&self.config) {
            Ok(config) => config,
            Err(err) => return err,
        };
        let provider = match provider_for_config(&config) {
            Ok(provider) => provider,
            Err(err) => return err,
        };
        let state = match ensure_session_sandbox_running(context, &config).await {
            Ok(state) => state,
            Err(err) => return err,
        };
        let Some(path) = arguments.get("path").and_then(|v| v.as_str()) else {
            return ToolExecutionResult::tool_error("Missing required parameter: path");
        };

        match provider
            .read_file(context, &config, &state.instance, path)
            .await
        {
            Ok(response) => ToolExecutionResult::success(json!({
                "path": response.path,
                "content": response.content,
                "encoding": response.encoding,
            })),
            Err(err) => err,
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

#[derive(Clone)]
pub struct SandboxWriteFileTool {
    config: Value,
}

impl SandboxWriteFileTool {
    pub fn new(config: Value) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for SandboxWriteFileTool {
    fn name(&self) -> &str {
        "sandbox_write_file"
    }

    fn description(&self) -> &str {
        "Write a file into the session-managed sandbox filesystem."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Destination path inside the sandbox" },
                "content": { "type": "string", "description": "File content to write" }
            },
            "required": ["path", "content"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> crate::ToolHints {
        session_sandbox_tool_hints()
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "sandbox_write_file requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let config = match parse_config(&self.config) {
            Ok(config) => config,
            Err(err) => return err,
        };
        let provider = match provider_for_config(&config) {
            Ok(provider) => provider,
            Err(err) => return err,
        };
        let state = match ensure_session_sandbox_running(context, &config).await {
            Ok(state) => state,
            Err(err) => return err,
        };
        let Some(path) = arguments.get("path").and_then(|v| v.as_str()) else {
            return ToolExecutionResult::tool_error("Missing required parameter: path");
        };
        let Some(content) = arguments.get("content").and_then(|v| v.as_str()) else {
            return ToolExecutionResult::tool_error("Missing required parameter: content");
        };

        match provider
            .write_file(context, &config, &state.instance, path, content)
            .await
        {
            Ok(response) => ToolExecutionResult::success(json!({
                "path": response.path,
                "bytes_written": response.bytes_written,
            })),
            Err(err) => err,
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

#[derive(Clone)]
pub struct SandboxStatusTool {
    config: Value,
}

impl SandboxStatusTool {
    pub fn new(config: Value) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for SandboxStatusTool {
    fn name(&self) -> &str {
        "sandbox_status"
    }

    fn description(&self) -> &str {
        "Inspect the current state of the session-managed sandbox."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        })
    }

    fn hints(&self) -> crate::ToolHints {
        session_sandbox_tool_hints()
            .with_readonly(true)
            .with_idempotent(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "sandbox_status requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        _arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let config = match parse_config(&self.config) {
            Ok(config) => config,
            Err(err) => return err,
        };
        let Some(state) = (match load_session_sandbox_state(context).await {
            Ok(state) => state,
            Err(err) => return err,
        }) else {
            return ToolExecutionResult::success(json!({
                "exists": false,
                "provider": config.provider,
            }));
        };
        let provider = match provider_for_config(&config) {
            Ok(provider) => provider,
            Err(err) => return err,
        };

        match provider.status(context, &config, &state).await {
            Ok(response) => ToolExecutionResult::success(json!({
                "exists": true,
                "provider": response.provider,
                "session_status": response.session_status,
                "external_id": response.external_id,
                "display_name": response.display_name,
                "workspace_path": response.workspace_path,
                "metadata": response.metadata,
            })),
            Err(err) => err,
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

#[derive(Clone)]
pub struct SandboxManageTool {
    config: Value,
}

impl SandboxManageTool {
    pub fn new(config: Value) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for SandboxManageTool {
    fn name(&self) -> &str {
        "sandbox_manage"
    }

    fn description(&self) -> &str {
        "Pause, resume, or delete the session-managed sandbox."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["pause", "resume", "delete"],
                    "description": "Lifecycle action to apply"
                }
            },
            "required": ["action"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> crate::ToolHints {
        session_sandbox_tool_hints().with_destructive(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "sandbox_manage requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let config = match parse_config(&self.config) {
            Ok(config) => config,
            Err(err) => return err,
        };
        let Some(action) = arguments.get("action").and_then(|v| v.as_str()) else {
            return ToolExecutionResult::tool_error("Missing required parameter: action");
        };

        match action {
            "pause" => match pause_session_sandbox(context, &config).await {
                Ok(Some(state)) => ToolExecutionResult::success(json!({
                    "action": action,
                    "provider": state.provider,
                    "session_status": state.status,
                    "external_id": state.instance.external_id,
                })),
                Ok(None) => ToolExecutionResult::success(json!({
                    "action": action,
                    "exists": false,
                })),
                Err(err) => err,
            },
            "resume" => match ensure_session_sandbox_running(context, &config).await {
                Ok(state) => ToolExecutionResult::success(json!({
                    "action": action,
                    "provider": state.provider,
                    "session_status": state.status,
                    "external_id": state.instance.external_id,
                })),
                Err(err) => err,
            },
            "delete" => match delete_session_sandbox(context, &config).await {
                Ok(deleted) => ToolExecutionResult::success(json!({
                    "action": action,
                    "deleted": deleted,
                })),
                Err(err) => err,
            },
            _ => ToolExecutionResult::tool_error(
                "Invalid action: must be one of pause, resume, delete",
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
    use crate::capabilities::{Capability, CapabilityRegistry};
    use crate::deployment::DeploymentGrade;
    use crate::traits::ToolContext;

    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn lock_env() -> std::sync::MutexGuard<'static, ()> {
        ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn session_sandbox_capability_metadata() {
        let cap = SessionSandboxCapability;
        assert_eq!(cap.id(), SESSION_SANDBOX_CAPABILITY_ID);
        assert_eq!(cap.name(), "Session Sandbox");
        assert_eq!(cap.status(), CapabilityStatus::Available);
        assert_eq!(cap.dependencies(), vec!["session_storage"]);
    }

    #[test]
    fn session_sandbox_tools_with_config() {
        let cap = SessionSandboxCapability;
        let tools = cap.tools_with_config(&json!({"provider": "daytona"}));
        let names: Vec<&str> = tools.iter().map(|tool| tool.name()).collect();
        assert_eq!(names.len(), 5);
        assert!(names.contains(&"sandbox_exec"));
        assert!(names.contains(&"sandbox_read_file"));
        assert!(names.contains(&"sandbox_write_file"));
        assert!(names.contains(&"sandbox_status"));
        assert!(names.contains(&"sandbox_manage"));
    }

    #[test]
    fn session_sandbox_registry_is_flag_gated() {
        let _lock = lock_env();
        unsafe { std::env::remove_var("FEATURE_SESSION_SANDBOX") };
        let registry = CapabilityRegistry::with_builtins_for_grade(DeploymentGrade::Dev);
        assert!(!registry.has("session_sandbox"));

        unsafe { std::env::set_var("FEATURE_SESSION_SANDBOX", "true") };
        let registry = CapabilityRegistry::with_builtins_for_grade(DeploymentGrade::Dev);
        assert!(registry.has("session_sandbox"));
        unsafe { std::env::remove_var("FEATURE_SESSION_SANDBOX") };
    }

    #[tokio::test]
    async fn sandbox_exec_rejects_zero_timeout() {
        let tool = SandboxExecTool::new(json!({ "provider": "missing-provider" }));
        let context = ToolContext::new(crate::typed_id::SessionId::new());

        let result = tool
            .execute_with_context(
                json!({
                    "command": "echo hi",
                    "timeout_ms": 0,
                }),
                &context,
            )
            .await;

        match result {
            ToolExecutionResult::ToolError(message) => {
                assert!(message.contains("timeout_ms must be a positive integer"));
            }
            other => panic!("expected ToolError, got {other:?}"),
        }
    }
}

//! Session Sandbox capability.
//!
//! One managed sandbox per session. The concrete provider is chosen by config
//! (`provider: "daytona"` initially), while the tool surface stays stable.

pub use crate::session_sandbox::SESSION_SANDBOX_CAPABILITY_ID;
use crate::session_sandbox::{
    DEFAULT_SESSION_SANDBOX_IDLE_TIMEOUT_SECS, SessionSandboxConfig, checkpoint_session_sandbox,
    create_session_sandbox_provider, delete_session_sandbox, ensure_session_sandbox_running,
    load_session_sandbox_state, pause_session_sandbox, session_sandbox_tool_hints,
};
use async_trait::async_trait;
use everruns_core::capabilities::{Capability, CapabilityLocalization, CapabilityStatus};
use everruns_core::tool_context::ToolContext;
use everruns_core::tool_output_sanitizer::{
    READ_FILE_DEFAULT_LIMIT, build_text_read_file_result, parse_read_file_window_args,
};
use everruns_core::tools::{Tool, ToolExecutionResult};
use everruns_core::truncation_info::TruncationInfo;
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
        Some("Sandboxes")
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some(
            "This session owns one managed sandbox. Use sandbox tools for commands and sandbox file I/O; inspect lifecycle state before lifecycle-sensitive work and pause/resume/delete only when requested or cleaning up.",
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

    /// Exposes only the lifecycle knobs users tune: `provider`, `auto_start`,
    /// and `idle_pause_after_seconds`. `provider_config` (provider-specific
    /// payload) and `init` (bootstrap commands) are advanced settings kept out
    /// of the schema; `validate_config` still accepts them through the typed
    /// `SessionSandboxConfig` parse.
    fn config_schema(&self) -> Option<Value> {
        Some(json!({
            "type": "object",
            "properties": {
                "provider": {
                    "type": "string",
                    "title": "Provider",
                    "description": "Sandbox provider id (e.g. daytona)."
                },
                "auto_start": {
                    "type": "boolean",
                    "title": "Auto-start",
                    "description": "Start the sandbox proactively when the session is created.",
                    "default": true
                },
                "idle_pause_after_seconds": {
                    "type": "integer",
                    "title": "Idle pause timeout (seconds)",
                    "description": "Pause the sandbox after this many seconds of session inactivity.",
                    "minimum": 1,
                    "default": DEFAULT_SESSION_SANDBOX_IDLE_TIMEOUT_SECS
                }
            }
        }))
    }

    fn validate_config(&self, config: &Value) -> Result<(), String> {
        // The capability may be attached before it is configured, so null and
        // empty-object configs are valid; tools reject unconfigured use at
        // execution time via `parse_config`.
        if config.is_null() {
            return Ok(());
        }
        let Some(object) = config.as_object() else {
            return Err("session_sandbox config must be an object".to_string());
        };
        if object.is_empty() {
            return Ok(());
        }
        let typed: SessionSandboxConfig = serde_json::from_value(config.clone())
            .map_err(|e| format!("invalid session_sandbox config: {e}"))?;
        // Same checks `parse_config` applies at tool-execution time.
        if typed.provider.trim().is_empty() {
            return Err("session_sandbox requires a non-empty provider".to_string());
        }
        if typed.idle_pause_after_seconds == 0 {
            return Err("idle_pause_after_seconds must be >= 1".to_string());
        }
        Ok(())
    }

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![
            CapabilityLocalization {
                locale: "en",
                name: None,
                description: None,
                config_description: Some(
                    "Controls the sandbox provider, auto-start behavior, and how long the \
                     sandbox may sit idle before pausing.",
                ),
                config_overlay: None,
            },
            CapabilityLocalization {
                locale: "uk",
                name: Some("Пісочниця сесії"),
                description: Some(
                    "Одна керована пісочниця, що належить поточній сесії. Підтримує \
                     виконання команд і файлові операції з життєвим циклом, яким керує \
                     провайдер.",
                ),
                config_description: Some(
                    "Визначає провайдера пісочниці, автозапуск і час простою до призупинення.",
                ),
                config_overlay: Some(json!({
                    "properties": {
                        "provider": {
                            "title": "Провайдер",
                            "description": "Ідентифікатор провайдера пісочниці (наприклад, daytona)."
                        },
                        "auto_start": {
                            "title": "Автозапуск",
                            "description": "Запускати пісочницю одразу після створення сесії."
                        },
                        "idle_pause_after_seconds": {
                            "title": "Призупинення після простою (секунди)",
                            "description": "Призупиняти пісочницю після зазначеної кількості секунд неактивності сесії."
                        }
                    }
                })),
            },
        ]
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
) -> Result<Box<dyn crate::session_sandbox::SessionSandboxProvider>, ToolExecutionResult> {
    create_session_sandbox_provider(&config.provider).ok_or_else(|| {
        ToolExecutionResult::tool_error(format!(
            "Session sandbox provider '{}' is not registered",
            config.provider
        ))
    })
}

fn build_sandbox_exec_result(
    response: crate::session_sandbox::SessionSandboxExecResponse,
    cwd: Option<&str>,
) -> ToolExecutionResult {
    let mut result = json!({
        "stdout": response.stdout,
        "stderr": response.stderr,
        "exit_code": response.exit_code,
        "success": response.success,
        "truncated": response.truncated,
        "total_lines": response.total_lines,
        "hint": response.hint,
    });
    if let Some(cwd) = cwd {
        result["cwd"] = json!(cwd);
    }

    if let Some(raw_output) = response.raw_output {
        ToolExecutionResult::success_with_raw_output(result, raw_output)
    } else {
        ToolExecutionResult::success(result)
    }
}

fn build_sandbox_read_file_result(
    response: crate::session_sandbox::SessionSandboxReadFileResponse,
    offset: usize,
    limit: usize,
) -> ToolExecutionResult {
    if response.encoding != "text" && response.encoding != "utf-8" {
        let bytes_returned = response.content.len();
        let mut result = json!({
            "path": response.path,
            "content": response.content,
            "encoding": response.encoding,
            "size_bytes": bytes_returned,
        });
        TruncationInfo::not_truncated(bytes_returned).attach(&mut result);
        return ToolExecutionResult::success(result);
    }

    ToolExecutionResult::success(build_text_read_file_result(
        "sandbox_read_file",
        &response.path,
        &response.content,
        &response.encoding,
        offset,
        limit,
    ))
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
    fn narrate(
        &self,
        tool_call: &everruns_core::tool_types::ToolCall,
        phase: everruns_core::tool_narration::ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: everruns_core::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        let fallback = self.display_name().unwrap_or("Sandbox");
        Some(everruns_core::tool_narration::narrate_shell_exec(
            &tool_call.arguments,
            fallback,
            phase,
            locale,
        ))
    }

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
                "timeout_ms": { "type": "integer", "minimum": 1, "description": "Optional execution timeout in milliseconds" },
                "output": everruns_core::tool_output_sanitizer::output_verbosity_schema()
            },
            "required": ["command"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> everruns_core::ToolHints {
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
        let mut state = match ensure_session_sandbox_running(context, &config).await {
            Ok(state) => state,
            Err(err) => return err,
        };

        match provider
            .exec(
                context,
                &config,
                &state.instance,
                &crate::session_sandbox::SessionSandboxExecRequest {
                    command: command.to_string(),
                    cwd: arguments
                        .get("cwd")
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string),
                    timeout_ms,
                    // EVE-489: persistence-first default — `auto` returns
                    // compact summaries on success, diagnostics on failure.
                    output_mode: arguments
                        .get("output")
                        .and_then(|v| v.as_str())
                        .unwrap_or("auto")
                        .to_string(),
                },
            )
            .await
        {
            Ok(response) => {
                if let Err(err) =
                    checkpoint_session_sandbox(context, provider.as_ref(), &config, &mut state)
                        .await
                {
                    return err;
                }
                build_sandbox_exec_result(response, arguments.get("cwd").and_then(|v| v.as_str()))
            }
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
    fn narrate(
        &self,
        tool_call: &everruns_core::tool_types::ToolCall,
        phase: everruns_core::tool_narration::ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: everruns_core::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        Some(everruns_core::tool_narration::narrate_read_file(
            &tool_call.arguments,
            phase,
            locale,
        ))
    }

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
                "path": { "type": "string", "description": "Path to read inside the sandbox" },
                "offset": {
                    "type": "integer",
                    "minimum": 0,
                    "default": 0,
                    "description": "Zero-based line offset to start reading from"
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "default": READ_FILE_DEFAULT_LIMIT,
                    "description": "Maximum number of lines to return"
                }
            },
            "required": ["path"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> everruns_core::ToolHints {
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
        let (offset, limit) = match parse_read_file_window_args(&arguments) {
            Ok(window) => window,
            Err(err) => return ToolExecutionResult::tool_error(err),
        };

        match provider
            .read_file(context, &config, &state.instance, path)
            .await
        {
            Ok(response) => build_sandbox_read_file_result(response, offset, limit),
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
    fn narrate(
        &self,
        tool_call: &everruns_core::tool_types::ToolCall,
        phase: everruns_core::tool_narration::ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: everruns_core::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        Some(everruns_core::tool_narration::narrate_write_file(
            &tool_call.arguments,
            phase,
            locale,
        ))
    }

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

    fn hints(&self) -> everruns_core::ToolHints {
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
        let mut state = match ensure_session_sandbox_running(context, &config).await {
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
            Ok(response) => {
                if let Err(err) =
                    checkpoint_session_sandbox(context, provider.as_ref(), &config, &mut state)
                        .await
                {
                    return err;
                }
                ToolExecutionResult::success(json!({
                    "path": response.path,
                    "bytes_written": response.bytes_written,
                }))
            }
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
    fn narrate(
        &self,
        _tool_call: &everruns_core::tool_types::ToolCall,
        phase: everruns_core::tool_narration::ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: everruns_core::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        Some(everruns_core::tool_narration::narrate_sandbox_status(
            phase, locale,
        ))
    }

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

    fn hints(&self) -> everruns_core::ToolHints {
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
    fn narrate(
        &self,
        tool_call: &everruns_core::tool_types::ToolCall,
        phase: everruns_core::tool_narration::ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: everruns_core::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        Some(everruns_core::tool_narration::narrate_sandbox_manage(
            &tool_call.arguments,
            phase,
            locale,
        ))
    }

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

    fn hints(&self) -> everruns_core::ToolHints {
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
    use everruns_core::capabilities::Capability;
    use everruns_core::deployment::DeploymentGrade;
    use everruns_core::tool_context::ToolContext;

    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn lock_env() -> std::sync::MutexGuard<'static, ()> {
        ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    // Metadata/dependency constants covered by builtin_capabilities_satisfy_registry_invariants.

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
    fn session_sandbox_tools_share_concurrency_class() {
        let cap = SessionSandboxCapability;
        let tools = cap.tools_with_config(&json!({"provider": "daytona"}));

        for tool in tools {
            let definition = tool.to_definition();
            assert_eq!(
                definition.concurrency_class(),
                Some("session_sandbox"),
                "{} should serialize against other session sandbox tools",
                tool.name()
            );
        }
    }

    #[test]
    fn session_sandbox_config_schema_and_validation() {
        let cap = SessionSandboxCapability;

        let schema = cap.config_schema().expect("config schema");
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["provider"].is_object());
        assert!(schema["properties"]["auto_start"].is_object());
        assert!(schema["properties"]["idle_pause_after_seconds"].is_object());
        // Advanced settings stay out of the schema.
        assert!(schema["properties"].get("provider_config").is_none());
        assert!(schema["properties"].get("init").is_none());

        // Unconfigured attachment is valid.
        assert!(cap.validate_config(&serde_json::Value::Null).is_ok());
        assert!(cap.validate_config(&json!({})).is_ok());

        // Valid config, including tolerated advanced fields.
        assert!(
            cap.validate_config(&json!({
                "provider": "daytona",
                "auto_start": false,
                "idle_pause_after_seconds": 60,
                "provider_config": { "snapshot": "base" },
                "init": { "commands": ["echo ok"] }
            }))
            .is_ok()
        );

        // Invalid configs are rejected with the same checks parse_config applies.
        assert!(cap.validate_config(&json!({ "provider": "  " })).is_err());
        let err = cap
            .validate_config(&json!({
                "provider": "daytona",
                "idle_pause_after_seconds": 0
            }))
            .unwrap_err();
        assert!(err.contains("idle_pause_after_seconds"));
    }

    #[test]
    fn session_sandbox_localizations_resolve_uk() {
        let cap = SessionSandboxCapability;
        assert_eq!(cap.localized_name(Some("uk-UA")), "Пісочниця сесії");
        assert!(cap.describe_schema(None).is_some());
    }

    #[test]
    fn session_sandbox_registry_is_flag_gated() {
        // The gate moved with the capability (EVE-886): product composition
        // decides, the kernel preset no longer carries it either way.
        let _lock = lock_env();
        unsafe { std::env::remove_var("FEATURE_SESSION_SANDBOX") };
        let registry =
            crate::capabilities::hosted_capability_registry_for_grade(DeploymentGrade::Dev);
        assert!(!registry.has("session_sandbox"));

        unsafe { std::env::set_var("FEATURE_SESSION_SANDBOX", "true") };
        let registry =
            crate::capabilities::hosted_capability_registry_for_grade(DeploymentGrade::Dev);
        assert!(registry.has("session_sandbox"));
        unsafe { std::env::remove_var("FEATURE_SESSION_SANDBOX") };
    }

    #[tokio::test]
    async fn sandbox_exec_rejects_zero_timeout() {
        let tool = SandboxExecTool::new(json!({ "provider": "missing-provider" }));
        let context = ToolContext::new(everruns_core::typed_id::SessionId::new());

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

    #[test]
    fn sandbox_exec_result_preserves_absent_raw_output() {
        let result = build_sandbox_exec_result(
            crate::session_sandbox::SessionSandboxExecResponse {
                exit_code: 0,
                stdout: "ok".to_string(),
                stderr: String::new(),
                success: true,
                truncated: false,
                total_lines: 1,
                raw_output: None,
                hint: None,
            },
            Some("/workspace"),
        )
        .into_tool_result("call_1", "sandbox_exec");

        assert_eq!(result.raw_output, None);
        assert_eq!(result.result.unwrap()["cwd"], "/workspace");
    }

    #[test]
    fn sandbox_exec_result_keeps_raw_output_sidecar_when_present() {
        let result = build_sandbox_exec_result(
            crate::session_sandbox::SessionSandboxExecResponse {
                exit_code: 17,
                stdout: "trimmed".to_string(),
                stderr: "warn".to_string(),
                success: false,
                truncated: true,
                total_lines: 42,
                raw_output: Some("full output".to_string()),
                hint: Some("non-zero".to_string()),
            },
            None,
        )
        .into_tool_result("call_1", "sandbox_exec");

        assert_eq!(result.raw_output.as_deref(), Some("full output"));
        let payload = result.result.unwrap();
        assert_eq!(payload["exit_code"], 17);
        assert_eq!(payload["truncated"], true);
        assert_eq!(payload["hint"], "non-zero");
    }

    #[test]
    fn sandbox_read_file_result_applies_line_window() {
        let result = build_sandbox_read_file_result(
            crate::session_sandbox::SessionSandboxReadFileResponse {
                path: "/workspace/src/lib.rs".to_string(),
                content: "alpha\nbeta\ngamma\ndelta".to_string(),
                encoding: "text".to_string(),
            },
            1,
            2,
        )
        .into_tool_result("call_1", "sandbox_read_file");

        let payload = result.result.unwrap();
        assert_eq!(payload["path"], "/workspace/src/lib.rs");
        assert_eq!(payload["content"], "2|beta\n3|gamma");
        assert_eq!(payload["total_lines"], 4);
        assert_eq!(payload["lines_shown"]["start"], 2);
        assert_eq!(payload["lines_shown"]["end"], 3);
        assert_eq!(payload["truncated"], true);
        assert_eq!(payload["truncation"]["next_offset"], 3);
        assert!(
            payload["truncation"]["resume_hint"]
                .as_str()
                .unwrap()
                .contains("sandbox_read_file")
        );
    }

    #[test]
    fn sandbox_read_file_result_marks_untruncated_window() {
        let result = build_sandbox_read_file_result(
            crate::session_sandbox::SessionSandboxReadFileResponse {
                path: "/workspace/src/lib.rs".to_string(),
                content: "alpha\nbeta".to_string(),
                encoding: "text".to_string(),
            },
            0,
            10,
        )
        .into_tool_result("call_1", "sandbox_read_file");

        let payload = result.result.unwrap();
        assert_eq!(payload["content"], "1|alpha\n2|beta");
        assert_eq!(payload["truncated"], false);
        assert_eq!(payload["truncation"]["truncated"], false);
    }
}

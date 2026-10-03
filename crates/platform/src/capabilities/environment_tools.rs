//! Stable model-facing tools for managed execution Environments.

use async_trait::async_trait;
use everruns_core::tool_context::ToolContext;
use everruns_core::tool_output_sanitizer::{
    READ_FILE_DEFAULT_LIMIT, build_text_read_file_result, parse_read_file_window_args,
};
use everruns_core::tools::{Tool, ToolExecutionResult};
use everruns_core::truncation_info::TruncationInfo;
use serde_json::{Value, json};

use super::session_sandbox::{parse_config, provider_for_config};
use crate::session_sandbox::{
    SessionSandboxConfig, SessionSandboxExecRequest, SessionSandboxExecResponse,
    SessionSandboxInstance, SessionSandboxReadFileResponse, checkpoint_session_sandbox,
    ensure_session_sandbox_running, session_sandbox_tool_hints,
};

const DEFAULT_SEARCH_LIMIT: usize = 200;
const MAX_SEARCH_LIMIT: usize = 1_000;
const SEARCH_TIMEOUT_MS: u64 = 30_000;

fn workspace_root(instance: &SessionSandboxInstance) -> &str {
    instance.workspace_path.as_deref().unwrap_or("/workspace")
}

fn resolve_workspace_path(
    instance: &SessionSandboxInstance,
    input: &str,
) -> Result<String, String> {
    let relative = input
        .strip_prefix("/workspace/")
        .or_else(|| (input == "/workspace").then_some(""))
        .unwrap_or_else(|| input.trim_start_matches('/'));
    if relative.split('/').any(|part| part == "..") {
        return Err("Path must stay inside /workspace".to_string());
    }
    let root = workspace_root(instance).trim_end_matches('/');
    Ok(if relative.is_empty() {
        root.to_string()
    } else {
        format!("{root}/{relative}")
    })
}

fn shell_escape(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn is_workspace_start_boundary(value: Option<char>) -> bool {
    value.is_none_or(|character| {
        !character.is_ascii_alphanumeric() && !matches!(character, '_' | '-' | '.' | '/')
    })
}

fn is_workspace_end_boundary(value: Option<char>) -> bool {
    value.is_none_or(|character| {
        !character.is_ascii_alphanumeric() && !matches!(character, '_' | '-' | '.')
    })
}

/// Translate the model-facing mount into the provider's workspace path.
///
/// Providers are allowed to place the worktree anywhere, but `bash` is part of
/// the same `/workspace` namespace as the file tools. Keep the translation at
/// this boundary so neither prompts nor provider implementations need to know
/// one another's paths.
fn translate_workspace_command(command: &str, provider_root: &str) -> String {
    if provider_root == "/workspace" {
        return command.to_string();
    }

    let mut translated = String::with_capacity(command.len());
    let mut cursor = 0;
    while let Some(offset) = command[cursor..].find("/workspace") {
        let start = cursor + offset;
        let end = start + "/workspace".len();
        let before = command[..start].chars().next_back();
        let after = command[end..].chars().next();
        if is_workspace_start_boundary(before) && is_workspace_end_boundary(after) {
            translated.push_str(&command[cursor..start]);
            translated.push_str(provider_root.trim_end_matches('/'));
            cursor = end;
        } else {
            translated.push_str(&command[cursor..end]);
            cursor = end;
        }
    }
    translated.push_str(&command[cursor..]);
    translated
}

fn canonicalize_workspace_output(output: &str, provider_root: &str) -> String {
    if provider_root == "/workspace" {
        output.to_string()
    } else {
        output.replace(provider_root.trim_end_matches('/'), "/workspace")
    }
}

fn parse_limit(arguments: &Value) -> Result<usize, ToolExecutionResult> {
    match arguments.get("limit") {
        None => Ok(DEFAULT_SEARCH_LIMIT),
        Some(value) => match value.as_u64() {
            Some(limit) if limit > 0 => Ok((limit as usize).min(MAX_SEARCH_LIMIT)),
            _ => Err(ToolExecutionResult::tool_error(
                "limit must be a positive integer",
            )),
        },
    }
}

fn parse_timeout(arguments: &Value) -> Result<Option<u64>, ToolExecutionResult> {
    match arguments.get("timeout_ms") {
        None => Ok(None),
        Some(value) => match value.as_u64() {
            Some(timeout) if timeout > 0 => Ok(Some(timeout)),
            _ => Err(ToolExecutionResult::tool_error(
                "timeout_ms must be a positive integer",
            )),
        },
    }
}

async fn exec(
    config: &SessionSandboxConfig,
    context: &ToolContext,
    command: String,
    cwd: Option<String>,
    timeout_ms: Option<u64>,
    checkpoint: bool,
) -> Result<SessionSandboxExecResponse, ToolExecutionResult> {
    let provider = provider_for_config(config)?;
    let mut state = ensure_session_sandbox_running(context, config).await?;
    let provider_root = workspace_root(&state.instance).to_string();
    let mut response = provider
        .exec(
            context,
            config,
            &state.instance,
            &SessionSandboxExecRequest {
                command: translate_workspace_command(&command, &provider_root),
                cwd: Some(match cwd {
                    Some(path) => resolve_workspace_path(&state.instance, &path)
                        .map_err(ToolExecutionResult::tool_error)?,
                    None => workspace_root(&state.instance).to_string(),
                }),
                timeout_ms,
                output_mode: "auto".to_string(),
            },
        )
        .await?;
    response.stdout = canonicalize_workspace_output(&response.stdout, &provider_root);
    response.stderr = canonicalize_workspace_output(&response.stderr, &provider_root);
    response.raw_output = response
        .raw_output
        .map(|raw| canonicalize_workspace_output(&raw, &provider_root));
    if checkpoint {
        checkpoint_session_sandbox(context, provider.as_ref(), config, &mut state).await?;
    }
    Ok(response)
}

fn exec_result(response: SessionSandboxExecResponse, cwd: Option<&str>) -> ToolExecutionResult {
    let mut value = json!({
        "stdout": response.stdout,
        "stderr": response.stderr,
        "exit_code": response.exit_code,
        "success": response.success,
        "truncated": response.truncated,
        "total_lines": response.total_lines,
        "hint": response.hint,
    });
    value["cwd"] = json!(cwd.unwrap_or("/workspace"));
    match response.raw_output {
        Some(raw) => ToolExecutionResult::success_with_raw_output(value, raw),
        None => ToolExecutionResult::success(value),
    }
}

fn read_result(
    mut response: SessionSandboxReadFileResponse,
    display_path: &str,
    offset: usize,
    limit: usize,
) -> ToolExecutionResult {
    response.path = display_path.to_string();
    if response.encoding != "text" && response.encoding != "utf-8" {
        let bytes_returned = response.content.len();
        let mut value = json!({
            "path": response.path,
            "content": response.content,
            "encoding": response.encoding,
            "size_bytes": bytes_returned,
        });
        TruncationInfo::not_truncated(bytes_returned).attach(&mut value);
        return ToolExecutionResult::success(value);
    }
    ToolExecutionResult::success(build_text_read_file_result(
        "read_file",
        &response.path,
        &response.content,
        &response.encoding,
        offset,
        limit,
    ))
}

#[derive(Clone)]
pub struct EnvironmentBashTool(Value);

impl EnvironmentBashTool {
    pub fn new(config: Value) -> Self {
        Self(config)
    }
}

#[async_trait]
impl Tool for EnvironmentBashTool {
    fn narrate(
        &self,
        tool_call: &everruns_contracts::tool_types::ToolCall,
        phase: everruns_core::tool_narration::ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: everruns_core::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        Some(everruns_core::tool_narration::narrate_shell_exec(
            &tool_call.arguments,
            "shell command",
            phase,
            locale,
        ))
    }

    fn name(&self) -> &str {
        "bash"
    }

    fn description(&self) -> &str {
        "Run a shell command in the active Environment. Commands start in /workspace; files written there are visible to the generic file tools and checkpointed before the call succeeds."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": {"type": "string"},
                "cwd": {"type": "string", "default": "/workspace"},
                "timeout_ms": {"type": "integer", "minimum": 1}
            },
            "required": ["command"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> everruns_contracts::tool_types::ToolHints {
        session_sandbox_tool_hints()
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("bash requires session context")
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let config = match parse_config(&self.0) {
            Ok(value) => value,
            Err(error) => return error,
        };
        let Some(command) = arguments.get("command").and_then(Value::as_str) else {
            return ToolExecutionResult::tool_error("Missing required parameter: command");
        };
        let timeout_ms = match parse_timeout(&arguments) {
            Ok(value) => value,
            Err(error) => return error,
        };
        let cwd = arguments.get("cwd").and_then(Value::as_str);
        match exec(
            &config,
            context,
            command.to_string(),
            cwd.map(ToString::to_string),
            timeout_ms,
            true,
        )
        .await
        {
            Ok(response) => exec_result(response, cwd),
            Err(error) => error,
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

#[derive(Clone)]
pub struct EnvironmentReadFileTool(Value);

impl EnvironmentReadFileTool {
    pub fn new(config: Value) -> Self {
        Self(config)
    }
}

#[async_trait]
impl Tool for EnvironmentReadFileTool {
    fn narrate(
        &self,
        tool_call: &everruns_contracts::tool_types::ToolCall,
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
        "read_file"
    }

    fn description(&self) -> &str {
        "Read a file from /workspace in the active Environment."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string"},
                "offset": {"type": "integer", "minimum": 0, "default": 0},
                "limit": {"type": "integer", "minimum": 1, "default": READ_FILE_DEFAULT_LIMIT}
            },
            "required": ["path"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> everruns_contracts::tool_types::ToolHints {
        session_sandbox_tool_hints()
            .with_readonly(true)
            .with_idempotent(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("read_file requires session context")
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let config = match parse_config(&self.0) {
            Ok(value) => value,
            Err(error) => return error,
        };
        let Some(path) = arguments.get("path").and_then(Value::as_str) else {
            return ToolExecutionResult::tool_error("Missing required parameter: path");
        };
        let (offset, limit) = match parse_read_file_window_args(&arguments) {
            Ok(value) => value,
            Err(error) => return ToolExecutionResult::tool_error(error),
        };
        let provider = match provider_for_config(&config) {
            Ok(value) => value,
            Err(error) => return error,
        };
        let state = match ensure_session_sandbox_running(context, &config).await {
            Ok(value) => value,
            Err(error) => return error,
        };
        let resolved = match resolve_workspace_path(&state.instance, path) {
            Ok(value) => value,
            Err(error) => return ToolExecutionResult::tool_error(error),
        };
        match provider
            .read_file(context, &config, &state.instance, &resolved)
            .await
        {
            Ok(response) => read_result(response, path, offset, limit),
            Err(error) => error,
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

#[derive(Clone)]
pub struct EnvironmentWriteFileTool(Value);

impl EnvironmentWriteFileTool {
    pub fn new(config: Value) -> Self {
        Self(config)
    }
}

#[async_trait]
impl Tool for EnvironmentWriteFileTool {
    fn narrate(
        &self,
        tool_call: &everruns_contracts::tool_types::ToolCall,
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
        "write_file"
    }

    fn description(&self) -> &str {
        "Write UTF-8 text to a file under /workspace in the active Environment. Parent directories are created and the workspace is checkpointed before success."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string"},
                "content": {"type": "string"}
            },
            "required": ["path", "content"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> everruns_contracts::tool_types::ToolHints {
        session_sandbox_tool_hints().with_destructive(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("write_file requires session context")
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let config = match parse_config(&self.0) {
            Ok(value) => value,
            Err(error) => return error,
        };
        let Some(path) = arguments.get("path").and_then(Value::as_str) else {
            return ToolExecutionResult::tool_error("Missing required parameter: path");
        };
        let Some(content) = arguments.get("content").and_then(Value::as_str) else {
            return ToolExecutionResult::tool_error("Missing required parameter: content");
        };
        write_file(&config, context, path, content.as_bytes()).await
    }

    fn requires_context(&self) -> bool {
        true
    }
}

async fn write_file(
    config: &SessionSandboxConfig,
    context: &ToolContext,
    display_path: &str,
    content: &[u8],
) -> ToolExecutionResult {
    let provider = match provider_for_config(config) {
        Ok(value) => value,
        Err(error) => return error,
    };
    let mut state = match ensure_session_sandbox_running(context, config).await {
        Ok(value) => value,
        Err(error) => return error,
    };
    let resolved = match resolve_workspace_path(&state.instance, display_path) {
        Ok(value) => value,
        Err(error) => return ToolExecutionResult::tool_error(error),
    };
    match provider
        .write_file(context, config, &state.instance, &resolved, content)
        .await
    {
        Ok(response) => {
            if let Err(error) =
                checkpoint_session_sandbox(context, provider.as_ref(), config, &mut state).await
            {
                return error;
            }
            ToolExecutionResult::success(json!({
                "path": display_path,
                "bytes_written": response.bytes_written,
            }))
        }
        Err(error) => error,
    }
}

#[derive(Clone)]
pub struct EnvironmentEditFileTool(Value);

impl EnvironmentEditFileTool {
    pub fn new(config: Value) -> Self {
        Self(config)
    }
}

#[async_trait]
impl Tool for EnvironmentEditFileTool {
    fn narrate(
        &self,
        tool_call: &everruns_contracts::tool_types::ToolCall,
        phase: everruns_core::tool_narration::ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: everruns_core::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        Some(everruns_core::tool_narration::narrate_edit_file(
            &tool_call.arguments,
            phase,
            locale,
        ))
    }

    fn name(&self) -> &str {
        "edit_file"
    }

    fn description(&self) -> &str {
        "Replace exact text in a UTF-8 file under /workspace. By default old_text must match exactly once."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string"},
                "old_text": {"type": "string"},
                "new_text": {"type": "string"},
                "replace_all": {"type": "boolean", "default": false}
            },
            "required": ["path", "old_text", "new_text"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> everruns_contracts::tool_types::ToolHints {
        session_sandbox_tool_hints().with_destructive(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("edit_file requires session context")
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let config = match parse_config(&self.0) {
            Ok(value) => value,
            Err(error) => return error,
        };
        let Some(path) = arguments.get("path").and_then(Value::as_str) else {
            return ToolExecutionResult::tool_error("Missing required parameter: path");
        };
        let Some(old_text) = arguments.get("old_text").and_then(Value::as_str) else {
            return ToolExecutionResult::tool_error("Missing required parameter: old_text");
        };
        let Some(new_text) = arguments.get("new_text").and_then(Value::as_str) else {
            return ToolExecutionResult::tool_error("Missing required parameter: new_text");
        };
        if old_text.is_empty() {
            return ToolExecutionResult::tool_error("old_text cannot be empty");
        }
        let provider = match provider_for_config(&config) {
            Ok(value) => value,
            Err(error) => return error,
        };
        let state = match ensure_session_sandbox_running(context, &config).await {
            Ok(value) => value,
            Err(error) => return error,
        };
        let resolved = match resolve_workspace_path(&state.instance, path) {
            Ok(value) => value,
            Err(error) => return ToolExecutionResult::tool_error(error),
        };
        let read = match provider
            .read_file(context, &config, &state.instance, &resolved)
            .await
        {
            Ok(value) => value,
            Err(error) => return error,
        };
        if read.encoding != "text" && read.encoding != "utf-8" {
            return ToolExecutionResult::tool_error("edit_file only supports UTF-8 text files");
        }
        let count = read.content.matches(old_text).count();
        let replace_all = arguments
            .get("replace_all")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if count == 0 {
            return ToolExecutionResult::tool_error("old_text was not found");
        }
        if count > 1 && !replace_all {
            return ToolExecutionResult::tool_error(
                "old_text matched more than once; provide more context or set replace_all",
            );
        }
        let edited = if replace_all {
            read.content.replace(old_text, new_text)
        } else {
            read.content.replacen(old_text, new_text, 1)
        };
        match write_file(&config, context, path, edited.as_bytes()).await {
            ToolExecutionResult::Success(_) => ToolExecutionResult::success(json!({
                "path": path,
                "replacements": if replace_all { count } else { 1 },
                "bytes_written": edited.len(),
            })),
            error => error,
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

#[derive(Clone)]
pub struct EnvironmentGlobTool(Value);

impl EnvironmentGlobTool {
    pub fn new(config: Value) -> Self {
        Self(config)
    }
}

#[async_trait]
impl Tool for EnvironmentGlobTool {
    fn narrate(
        &self,
        tool_call: &everruns_contracts::tool_types::ToolCall,
        phase: everruns_core::tool_narration::ToolNarrationPhase,
        locale: Option<&str>,
        ctx: everruns_core::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        Some(everruns_core::tool_narration::narrate_list_directory(
            &tool_call.arguments,
            phase,
            locale,
            ctx,
        ))
    }

    fn name(&self) -> &str {
        "glob"
    }

    fn description(&self) -> &str {
        "List files under /workspace matching a shell glob."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {"type": "string"},
                "limit": {"type": "integer", "minimum": 1, "maximum": MAX_SEARCH_LIMIT, "default": DEFAULT_SEARCH_LIMIT}
            },
            "required": ["pattern"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> everruns_contracts::tool_types::ToolHints {
        session_sandbox_tool_hints()
            .with_readonly(true)
            .with_idempotent(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("glob requires session context")
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let config = match parse_config(&self.0) {
            Ok(value) => value,
            Err(error) => return error,
        };
        let Some(pattern) = arguments.get("pattern").and_then(Value::as_str) else {
            return ToolExecutionResult::tool_error("Missing required parameter: pattern");
        };
        let limit = match parse_limit(&arguments) {
            Ok(value) => value,
            Err(error) => return error,
        };
        let normalized = pattern
            .strip_prefix("/workspace/")
            .unwrap_or(pattern.trim_start_matches('/'));
        if normalized.split('/').any(|part| part == "..") {
            return ToolExecutionResult::tool_error("Pattern must stay inside /workspace");
        }
        let predicate = if normalized.contains('/') {
            format!("-path {}", shell_escape(&format!("./{normalized}")))
        } else {
            format!("-name {}", shell_escape(normalized))
        };
        let fetch_limit = limit + 1;
        let command = format!("find . -type f {predicate} -print | sort | head -n {fetch_limit}");
        match exec(
            &config,
            context,
            command,
            None,
            Some(SEARCH_TIMEOUT_MS),
            false,
        )
        .await
        {
            Ok(response) if response.exit_code == 0 => {
                let mut paths = response
                    .stdout
                    .lines()
                    .map(|line| format!("/workspace/{}", line.trim_start_matches("./")))
                    .collect::<Vec<_>>();
                let truncated = paths.len() > limit;
                paths.truncate(limit);
                ToolExecutionResult::success(json!({"paths": paths, "truncated": truncated}))
            }
            Ok(response) => exec_result(response, None),
            Err(error) => error,
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

#[derive(Clone)]
pub struct EnvironmentGrepTool(Value);

impl EnvironmentGrepTool {
    pub fn new(config: Value) -> Self {
        Self(config)
    }
}

#[async_trait]
impl Tool for EnvironmentGrepTool {
    fn narrate(
        &self,
        tool_call: &everruns_contracts::tool_types::ToolCall,
        phase: everruns_core::tool_narration::ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: everruns_core::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        Some(everruns_core::tool_narration::narrate_grep_files(
            &tool_call.arguments,
            phase,
            locale,
        ))
    }

    fn name(&self) -> &str {
        "grep"
    }

    fn description(&self) -> &str {
        "Search UTF-8 files under /workspace with an extended regular expression."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {"type": "string"},
                "glob": {"type": "string"},
                "limit": {"type": "integer", "minimum": 1, "maximum": MAX_SEARCH_LIMIT, "default": DEFAULT_SEARCH_LIMIT}
            },
            "required": ["pattern"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> everruns_contracts::tool_types::ToolHints {
        session_sandbox_tool_hints()
            .with_readonly(true)
            .with_idempotent(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("grep requires session context")
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let config = match parse_config(&self.0) {
            Ok(value) => value,
            Err(error) => return error,
        };
        let Some(pattern) = arguments.get("pattern").and_then(Value::as_str) else {
            return ToolExecutionResult::tool_error("Missing required parameter: pattern");
        };
        let limit = match parse_limit(&arguments) {
            Ok(value) => value,
            Err(error) => return error,
        };
        let include = arguments
            .get("glob")
            .and_then(Value::as_str)
            .map(|value| format!(" --include={}", shell_escape(value)))
            .unwrap_or_default();
        let fetch_limit = limit + 1;
        let command = format!(
            "grep -RInE{include} -- {} . | head -n {fetch_limit}",
            shell_escape(pattern)
        );
        match exec(
            &config,
            context,
            command,
            None,
            Some(SEARCH_TIMEOUT_MS),
            false,
        )
        .await
        {
            Ok(response) if matches!(response.exit_code, 0 | 1) => {
                let mut matches = response
                    .stdout
                    .lines()
                    .map(|line| line.replacen("./", "/workspace/", 1))
                    .collect::<Vec<_>>();
                let truncated = matches.len() > limit;
                matches.truncate(limit);
                ToolExecutionResult::success(json!({"matches": matches, "truncated": truncated}))
            }
            Ok(response) => exec_result(response, None),
            Err(error) => error,
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use everruns_core::tools::Tool;
    use everruns_host::InMemorySessionStorageStore;

    use super::*;

    #[test]
    fn maps_model_workspace_paths_to_the_provider_workspace() {
        let instance = SessionSandboxInstance {
            workspace_path: Some("/home/daytona/workspace".to_string()),
            ..Default::default()
        };
        assert_eq!(
            resolve_workspace_path(&instance, "/workspace/src/lib.rs").unwrap(),
            "/home/daytona/workspace/src/lib.rs"
        );
        assert!(resolve_workspace_path(&instance, "/workspace/../secret").is_err());
    }

    #[test]
    fn maps_shell_workspace_paths_without_rewriting_larger_tokens() {
        let command = "pwd; cat /workspace/a; cat '/workspace/b'; P=/workspace/c; echo /workspace-old https://example.test/workspace";
        assert_eq!(
            translate_workspace_command(command, "/home/daytona/workspace"),
            "pwd; cat /home/daytona/workspace/a; cat '/home/daytona/workspace/b'; P=/home/daytona/workspace/c; echo /workspace-old https://example.test/workspace"
        );
    }

    #[test]
    fn canonicalizes_provider_workspace_in_command_output() {
        assert_eq!(
            canonicalize_workspace_output(
                "cwd=/home/daytona/workspace\n/home/daytona/workspace/src/main.rs\n",
                "/home/daytona/workspace",
            ),
            "cwd=/workspace\n/workspace/src/main.rs\n"
        );
    }

    #[tokio::test]
    async fn bash_rejects_zero_timeout_before_provider_resolution() {
        let tool = EnvironmentBashTool::new(json!({"provider": "missing-provider"}));
        let context = ToolContext::new(everruns_contracts::typed_id::SessionId::new());

        let result = tool
            .execute_with_context(json!({"command": "echo hi", "timeout_ms": 0}), &context)
            .await;

        let ToolExecutionResult::ToolError(message) = result else {
            panic!("expected tool error, got {result:?}");
        };
        assert!(message.contains("timeout_ms must be a positive integer"));
    }

    #[tokio::test]
    async fn file_and_shell_tools_share_one_provider_workspace() {
        let context = ToolContext::with_storage_store(
            everruns_contracts::typed_id::SessionId::new(),
            Arc::new(InMemorySessionStorageStore::new()),
        );
        let config = json!({"provider": "core-test-session-sandbox"});

        let write = EnvironmentWriteFileTool::new(config.clone())
            .execute_with_context(
                json!({"path": "/workspace/proof.txt", "content": "same workspace"}),
                &context,
            )
            .await;
        assert!(
            matches!(write, ToolExecutionResult::Success(_)),
            "{write:?}"
        );

        let read = EnvironmentReadFileTool::new(config.clone())
            .execute_with_context(json!({"path": "/workspace/proof.txt"}), &context)
            .await;
        let ToolExecutionResult::Success(read) = read else {
            panic!("read_file failed: {read:?}");
        };
        assert_eq!(read["content"], "1|same workspace");

        let shell = EnvironmentBashTool::new(config)
            .execute_with_context(json!({"command": "cat /workspace/proof.txt"}), &context)
            .await;
        let ToolExecutionResult::Success(shell) = shell else {
            panic!("bash failed: {shell:?}");
        };
        assert_eq!(shell["stdout"], "same workspace");
    }
}

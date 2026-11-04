//! Agent tools for boxd machines (the `boxd` capability).
//!
//! These are for machines an agent manages itself, beside or instead of the
//! session's primary Sandbox. Machines are created the same way the managed
//! provider creates them (isolated, region-routed, egress from the session
//! access list), but carry no `everruns.managed` label: the lease, not the
//! orphan scan, cleans them up.

use std::time::Duration;

use async_trait::async_trait;
use everruns_contracts::runtime::exec_tool_result::ExecToolResultPayload;
use everruns_contracts::runtime::tool_context::ToolContext;
use everruns_contracts::runtime::tool_narration::{
    ToolNarrationContext, ToolNarrationPhase, narrate_labeled_action, narrate_shell_exec,
};
use everruns_contracts::runtime::tool_output_sanitizer::{
    READ_FILE_DEFAULT_LIMIT, build_text_read_file_result, output_verbosity_schema,
    parse_read_file_window_args,
};
use everruns_contracts::runtime::tools::{Tool, ToolExecutionResult};
use everruns_contracts::session_sandbox::SessionSandboxContext;
use everruns_contracts::tool_types::{ToolCall, ToolHints};
use serde_json::{Value, json};

use super::client::{BoxdClient, CreateMachineParams, is_not_found};
use super::state::{
    BoxdSandboxState, client_for, delete_sandbox_state, get_api_key, get_sandbox_state,
    list_sandbox_states, release_sandbox_lease, required_str, save_sandbox_state,
    touch_sandbox_lease,
};
use super::{
    BOXD_DEFAULT_EXEC_TIMEOUT_SECS, BOXD_MAX_EXEC_TIMEOUT_SECS, BOXD_MAX_READ_FILE_BYTES,
    BOXD_REGIONS, BOXD_SIZES, BOXD_WORKSPACE_PATH, MachineRef, command_in, exit_code_hint,
    machine_name,
};

/// Largest file `boxd_write_file` accepts.
const MAX_WRITE_BYTES: usize = 10 * 1024 * 1024;

/// Turn a `Result<T, ToolExecutionResult>` into `T` or return the error result.
macro_rules! try_tool {
    ($expr:expr) => {
        match $expr {
            Ok(value) => value,
            Err(err) => return err,
        }
    };
}

fn context_required(name: &str) -> ToolExecutionResult {
    ToolExecutionResult::tool_error(format!(
        "{name} requires context. This tool must be executed with session context."
    ))
}

fn optional_choice<'a>(
    args: &'a Value,
    name: &str,
    choices: &[&str],
) -> Result<Option<&'a str>, ToolExecutionResult> {
    match args.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) if choices.contains(&value.as_str()) => Ok(Some(value)),
        Some(_) => Err(ToolExecutionResult::tool_error(format!(
            "Invalid '{name}': must be one of {}",
            choices.join(", ")
        ))),
    }
}

fn resolve_path(state: &BoxdSandboxState, path: &str) -> String {
    if path.starts_with('/') {
        return path.to_string();
    }
    let joined = format!("{}/{}", state.workspace_path.trim_end_matches('/'), path);
    joined.trim_end_matches("/.").to_string()
}

async fn client_and_state(
    context: &ToolContext,
    arguments: &Value,
) -> Result<(BoxdClient, BoxdSandboxState), ToolExecutionResult> {
    let sandbox_id = required_str(arguments, "sandbox_id")?;
    let api_key = get_api_key(context).await?;
    let state = get_sandbox_state(context, sandbox_id).await?;
    Ok((client_for(api_key, context)?, state))
}

/// A machine that is not running cannot run commands; say what to do.
fn explain(state: &BoxdSandboxState, error: String) -> ToolExecutionResult {
    if is_not_found(&error) {
        ToolExecutionResult::tool_error(format!(
            "The boxd machine {} no longer exists. Create a new one with boxd_create_sandbox.",
            state.sandbox_id
        ))
    } else {
        ToolExecutionResult::tool_error(error)
    }
}

fn labeled(
    call: &ToolCall,
    phase: ToolNarrationPhase,
    locale: Option<&str>,
    english: (&str, &str, &str),
    ukrainian: (&str, &str, &str),
    keys: &[&str],
) -> Option<String> {
    Some(narrate_labeled_action(
        &call.arguments,
        phase,
        locale,
        english,
        ukrainian,
        keys,
    ))
}

// ============================================================================
// boxd_create_sandbox
// ============================================================================

/// Create a boxd machine.
pub struct BoxdCreateSandboxTool;

#[async_trait]
impl Tool for BoxdCreateSandboxTool {
    fn narrate(
        &self,
        call: &ToolCall,
        phase: ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: ToolNarrationContext<'_>,
    ) -> Option<String> {
        labeled(
            call,
            phase,
            locale,
            (
                "Creating boxd machine",
                "Created boxd machine",
                "Could not create boxd machine",
            ),
            (
                "Створюю машину boxd",
                "Створив машину boxd",
                "Не вдалося створити машину boxd",
            ),
            &["title", "region"],
        )
    }

    fn name(&self) -> &str {
        "boxd_create_sandbox"
    }

    fn description(&self) -> &str {
        "Create a boxd machine: a full Linux VM (Ubuntu 24.04, Docker preinstalled) isolated from the user's other machines. Returns a sandbox_id for the other boxd tools."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "title": {"type": "string", "description": "Human-readable label"},
                "region": {"type": "string", "enum": BOXD_REGIONS, "description": "fra (Frankfurt) or nyc (New York). Default: the boxd org's default region, else fra"},
                "size": {"type": "string", "enum": BOXD_SIZES, "description": "vCPU x GiB RAM. Default: the org default (2x8)"},
                "snapshot": {"type": "string", "description": "Name of a boxd snapshot to start from instead of the default image"}
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
        context_required(self.name())
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let region = try_tool!(optional_choice(&arguments, "region", BOXD_REGIONS));
        let size = try_tool!(optional_choice(&arguments, "size", BOXD_SIZES));
        let snapshot = arguments
            .get("snapshot")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty());
        if snapshot.is_some() && size.is_some() {
            return ToolExecutionResult::tool_error(
                "'snapshot' and 'size' cannot be combined: a snapshot keeps its own size",
            );
        }
        let title = arguments
            .get("title")
            .and_then(Value::as_str)
            .filter(|t| !t.trim().is_empty())
            .map(str::to_string);
        let rules = try_tool!(super::egress::tool_rules(context));
        let egress = try_tool!(super::egress::entries_for(&rules));

        let api_key = try_tool!(get_api_key(context).await);
        let client = try_tool!(client_for(api_key, context));
        let region = match client.resolve_region(region).await {
            Ok(region) => region,
            Err(err) => return ToolExecutionResult::tool_error(err),
        };
        let labels = SessionSandboxContext::resource_labels(context)
            .await
            .into_iter()
            .filter_map(|(key, value)| value.as_str().map(|v| (key, v.to_string())))
            .collect();
        let params = CreateMachineParams {
            name: machine_name(),
            size: size.map(str::to_string),
            snapshot: snapshot.map(str::to_string),
            labels,
            egress,
            delete_after_secs: None,
        };
        let machine = match client.create_machine(&region.id, &params).await {
            Ok(machine) => machine,
            Err(err) => return ToolExecutionResult::tool_error(err),
        };
        let machine_ref = MachineRef {
            region: region.id.clone(),
            id: machine.id.clone(),
        };
        let state = BoxdSandboxState {
            sandbox_id: machine_ref.to_string(),
            region: region.id.clone(),
            machine_id: machine.id.clone(),
            name: machine.name.clone(),
            workspace_path: BOXD_WORKSPACE_PATH.to_string(),
            started_at: chrono::Utc::now().to_rfc3339(),
            title,
            url: machine.url.clone(),
        };
        // Lease first: a machine nothing can clean up must not outlive this call.
        if let Err(err) = touch_sandbox_lease(context, &state).await {
            let _ = client.destroy(&region.id, &machine.id).await;
            return err;
        }
        if let Err(err) = save_sandbox_state(context, &state).await {
            let _ = client.destroy(&region.id, &machine.id).await;
            let _ = release_sandbox_lease(context, &state.sandbox_id).await;
            return err;
        }
        if let Err(err) = client
            .mkdir(&region.id, &machine.id, BOXD_WORKSPACE_PATH)
            .await
        {
            tracing::warn!(machine = %machine_ref, "Failed to create boxd workspace: {err}");
        }
        ToolExecutionResult::success(json!({
            "sandbox_id": state.sandbox_id,
            "status": machine.status,
            "region": region.id,
            "name": machine.name,
            "url": machine.url,
            "workspace_path": state.workspace_path,
        }))
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// boxd_exec
// ============================================================================

/// Run a shell command on a machine.
pub struct BoxdExecTool;

#[async_trait]
impl Tool for BoxdExecTool {
    fn narrate(
        &self,
        call: &ToolCall,
        phase: ToolNarrationPhase,
        locale: Option<&str>,
        ctx: ToolNarrationContext<'_>,
    ) -> Option<String> {
        let fallback = self.display_name().unwrap_or("boxd");
        Some(narrate_shell_exec(
            &call.arguments,
            fallback,
            phase,
            locale,
            ctx,
        ))
    }

    fn name(&self) -> &str {
        "boxd_exec"
    }

    fn description(&self) -> &str {
        "Run a shell command on a boxd machine and return stdout, stderr and the exit code. A hibernated machine is woken first."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "sandbox_id": {"type": "string"},
                "command": {"type": "string", "description": "Shell command, run with sh -c"},
                "cwd": {"type": "string", "description": format!("Working directory (default {BOXD_WORKSPACE_PATH})")},
                "timeout_seconds": {
                    "type": "integer", "minimum": 1, "maximum": BOXD_MAX_EXEC_TIMEOUT_SECS,
                    "default": BOXD_DEFAULT_EXEC_TIMEOUT_SECS
                },
                "output": output_verbosity_schema()
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
        context_required(self.name())
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let command = try_tool!(required_str(&arguments, "command"));
        let timeout_secs = match arguments.get("timeout_seconds") {
            None | Some(Value::Null) => BOXD_DEFAULT_EXEC_TIMEOUT_SECS,
            Some(value) => match value
                .as_u64()
                .filter(|v| (1..=u64::from(BOXD_MAX_EXEC_TIMEOUT_SECS)).contains(v))
            {
                Some(v) => v as u32,
                None => {
                    return ToolExecutionResult::tool_error(format!(
                        "Invalid 'timeout_seconds': must be between 1 and {BOXD_MAX_EXEC_TIMEOUT_SECS}"
                    ));
                }
            },
        };
        let output_mode = arguments
            .get("output")
            .and_then(Value::as_str)
            .unwrap_or("auto");
        let (client, state) = try_tool!(client_and_state(context, &arguments).await);
        let cwd = arguments
            .get("cwd")
            .and_then(Value::as_str)
            .map(|cwd| resolve_path(&state, cwd))
            .unwrap_or_else(|| state.workspace_path.clone());
        let output = match client
            .exec(
                &state.region,
                &state.machine_id,
                &command_in(&cwd, command),
                None,
                Duration::from_secs(u64::from(timeout_secs)),
            )
            .await
        {
            Ok(output) => output,
            Err(err) => return explain(&state, err),
        };
        try_tool!(touch_sandbox_lease(context, &state).await);

        let payload = ExecToolResultPayload::new(
            &output.stdout,
            &output.stderr,
            output.exit_code,
            output_mode,
        );
        let mut result = json!({
            "sandbox_id": state.sandbox_id,
            "cwd": cwd,
            "stdout": payload.stdout,
            "stderr": payload.stderr,
            "exit_code": payload.exit_code,
            "success": payload.success,
            "truncated": payload.truncated || output.truncated,
            "total_lines": payload.total_lines,
        });
        if let Some(hint) = exit_code_hint(payload.exit_code) {
            result["hint"] = json!(hint);
        }
        ToolExecutionResult::success_with_raw_output(result, payload.raw_output)
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// boxd_read_file / boxd_write_file
// ============================================================================

/// Read a text file from a machine.
pub struct BoxdReadFileTool;

#[async_trait]
impl Tool for BoxdReadFileTool {
    fn narrate(
        &self,
        call: &ToolCall,
        phase: ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: ToolNarrationContext<'_>,
    ) -> Option<String> {
        labeled(
            call,
            phase,
            locale,
            (
                "Reading boxd file",
                "Read boxd file",
                "Could not read boxd file",
            ),
            (
                "Читаю файл boxd",
                "Прочитав файл boxd",
                "Не вдалося прочитати файл boxd",
            ),
            &["path"],
        )
    }

    fn name(&self) -> &str {
        "boxd_read_file"
    }

    fn description(&self) -> &str {
        "Read a text file from a boxd machine (NOT the session /workspace). Relative paths resolve against the machine workspace."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "sandbox_id": {"type": "string"},
                "path": {"type": "string"},
                "offset": {"type": "integer", "minimum": 0, "default": 0, "description": "Zero-based line offset"},
                "limit": {"type": "integer", "minimum": 1, "default": READ_FILE_DEFAULT_LIMIT, "description": "Maximum lines to return"}
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
        context_required(self.name())
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let path = try_tool!(required_str(&arguments, "path"));
        let (offset, limit) = match parse_read_file_window_args(&arguments) {
            Ok(window) => window,
            Err(err) => return ToolExecutionResult::tool_error(err),
        };
        let (client, state) = try_tool!(client_and_state(context, &arguments).await);
        let full_path = resolve_path(&state, path);
        let bytes = match client
            .read_file(
                &state.region,
                &state.machine_id,
                &full_path,
                BOXD_MAX_READ_FILE_BYTES,
            )
            .await
        {
            Ok(bytes) => bytes,
            Err(err) => return explain(&state, format!("Failed to read {full_path}: {err}")),
        };
        try_tool!(touch_sandbox_lease(context, &state).await);
        let text = String::from_utf8_lossy(&bytes);
        let mut result =
            build_text_read_file_result("boxd_read_file", &full_path, &text, "text", offset, limit);
        result["sandbox_id"] = json!(state.sandbox_id);
        ToolExecutionResult::success(result)
    }

    fn requires_context(&self) -> bool {
        true
    }
}

/// Write a text file onto a machine.
pub struct BoxdWriteFileTool;

#[async_trait]
impl Tool for BoxdWriteFileTool {
    fn narrate(
        &self,
        call: &ToolCall,
        phase: ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: ToolNarrationContext<'_>,
    ) -> Option<String> {
        labeled(
            call,
            phase,
            locale,
            (
                "Writing boxd file",
                "Wrote boxd file",
                "Could not write boxd file",
            ),
            (
                "Записую файл boxd",
                "Записав файл boxd",
                "Не вдалося записати файл boxd",
            ),
            &["path"],
        )
    }

    fn name(&self) -> &str {
        "boxd_write_file"
    }

    fn description(&self) -> &str {
        "Write a text file onto a boxd machine, creating parent directories. Relative paths resolve against the machine workspace."
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
        context_required(self.name())
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let path = try_tool!(required_str(&arguments, "path"));
        let content = try_tool!(required_str(&arguments, "content"));
        if content.len() > MAX_WRITE_BYTES {
            return ToolExecutionResult::tool_error(format!(
                "Content is {} bytes; boxd_write_file accepts at most {MAX_WRITE_BYTES}",
                content.len()
            ));
        }
        let (client, state) = try_tool!(client_and_state(context, &arguments).await);
        let full_path = resolve_path(&state, path);
        if let Err(err) = client
            .write_file(
                &state.region,
                &state.machine_id,
                &full_path,
                content.as_bytes(),
            )
            .await
        {
            return explain(&state, format!("Failed to write {full_path}: {err}"));
        }
        try_tool!(touch_sandbox_lease(context, &state).await);
        ToolExecutionResult::success(json!({
            "sandbox_id": state.sandbox_id,
            "path": full_path,
            "bytes_written": content.len(),
            "success": true,
        }))
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// boxd_list_sandboxes
// ============================================================================

/// List the session's machines.
pub struct BoxdListSandboxesTool;

#[async_trait]
impl Tool for BoxdListSandboxesTool {
    fn narrate(
        &self,
        call: &ToolCall,
        phase: ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: ToolNarrationContext<'_>,
    ) -> Option<String> {
        labeled(
            call,
            phase,
            locale,
            (
                "Listing boxd machines",
                "Listed boxd machines",
                "Could not list boxd machines",
            ),
            (
                "Перелічую машини boxd",
                "Перелічив машини boxd",
                "Не вдалося перелічити машини boxd",
            ),
            &[],
        )
    }

    fn name(&self) -> &str {
        "boxd_list_sandboxes"
    }

    fn description(&self) -> &str {
        "List boxd machines created in this session."
    }

    fn parameters_schema(&self) -> Value {
        json!({"type": "object", "properties": {}, "additionalProperties": false})
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_readonly(true)
            .with_idempotent(true)
            .with_requires_secrets(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        context_required(self.name())
    }

    async fn execute_with_context(
        &self,
        _arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let states = try_tool!(list_sandbox_states(context).await);
        ToolExecutionResult::success(json!({
            "sandboxes": states.iter().map(|s| json!({
                "sandbox_id": s.sandbox_id,
                "title": s.title,
                "region": s.region,
                "name": s.name,
                "url": s.url,
                "workspace_path": s.workspace_path,
                "started_at": s.started_at,
            })).collect::<Vec<_>>(),
            "count": states.len(),
        }))
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// boxd_manage_sandbox
// ============================================================================

const MANAGE_ACTIONS: &[&str] = &["status", "hibernate", "wake", "delete"];

/// Check on, hibernate, wake or delete a machine.
pub struct BoxdManageSandboxTool;

#[async_trait]
impl Tool for BoxdManageSandboxTool {
    fn narrate(
        &self,
        call: &ToolCall,
        phase: ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: ToolNarrationContext<'_>,
    ) -> Option<String> {
        let (english, ukrainian) = match call.arguments.get("action").and_then(Value::as_str) {
            Some("hibernate") => (
                (
                    "Hibernating boxd machine",
                    "Hibernated boxd machine",
                    "Could not hibernate boxd machine",
                ),
                (
                    "Присипляю машину boxd",
                    "Приспав машину boxd",
                    "Не вдалося приспати машину boxd",
                ),
            ),
            Some("wake") => (
                (
                    "Waking boxd machine",
                    "Woke boxd machine",
                    "Could not wake boxd machine",
                ),
                (
                    "Пробуджую машину boxd",
                    "Пробудив машину boxd",
                    "Не вдалося пробудити машину boxd",
                ),
            ),
            Some("delete") => (
                (
                    "Deleting boxd machine",
                    "Deleted boxd machine",
                    "Could not delete boxd machine",
                ),
                (
                    "Видаляю машину boxd",
                    "Видалив машину boxd",
                    "Не вдалося видалити машину boxd",
                ),
            ),
            _ => (
                (
                    "Checking boxd machine",
                    "Checked boxd machine",
                    "Could not check boxd machine",
                ),
                (
                    "Перевіряю машину boxd",
                    "Перевірив машину boxd",
                    "Не вдалося перевірити машину boxd",
                ),
            ),
        };
        labeled(call, phase, locale, english, ukrainian, &["sandbox_id"])
    }

    fn name(&self) -> &str {
        "boxd_manage_sandbox"
    }

    fn description(&self) -> &str {
        "Check a boxd machine's status, hibernate it (memory and running processes kept, disk-only cost), wake it, or delete it. Delete machines when done."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "sandbox_id": {"type": "string"},
                "action": {"type": "string", "enum": MANAGE_ACTIONS}
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
        context_required(self.name())
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let action = try_tool!(required_str(&arguments, "action"));
        if !MANAGE_ACTIONS.contains(&action) {
            return ToolExecutionResult::tool_error(format!(
                "Invalid action. Expected one of: {}.",
                MANAGE_ACTIONS.join(", ")
            ));
        }
        let (client, state) = try_tool!(client_and_state(context, &arguments).await);
        let (region, id) = (state.region.as_str(), state.machine_id.as_str());
        let outcome = match action {
            "status" => client.get_machine(region, id).await.map(|m| m.status),
            "hibernate" => client
                .hibernate(region, id)
                .await
                .map(|()| "hibernated".to_string()),
            "wake" => {
                // A machine boxd stopped (rather than hibernated) needs a
                // start; waking it is refused.
                let woken = match client.get_machine(region, id).await {
                    Ok(machine) if machine.status == "stopped" => client.start(region, id).await,
                    Ok(machine) if machine.status == "running" => Ok(()),
                    Ok(_) => client.wake(region, id).await,
                    Err(err) => Err(err),
                };
                match woken {
                    Ok(()) => client
                        .wait_until_running(region, id)
                        .await
                        .map(|m| m.status),
                    Err(err) => Err(err),
                }
            }
            _ => {
                match client.destroy(region, id).await {
                    Ok(()) => {}
                    Err(err) if is_not_found(&err) => {}
                    Err(err) => return ToolExecutionResult::tool_error(err),
                }
                try_tool!(delete_sandbox_state(context, &state.sandbox_id).await);
                try_tool!(release_sandbox_lease(context, &state.sandbox_id).await);
                return ToolExecutionResult::success(json!({
                    "sandbox_id": state.sandbox_id,
                    "action": "delete",
                    "success": true,
                }));
            }
        };
        let status = match outcome {
            Ok(status) => status,
            Err(err) => return explain(&state, err),
        };
        try_tool!(touch_sandbox_lease(context, &state).await);
        ToolExecutionResult::success(json!({
            "sandbox_id": state.sandbox_id,
            "action": action,
            "status": status,
            "region": state.region,
            "url": state.url,
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
    fn choices_are_checked() {
        assert_eq!(
            optional_choice(&json!({"region": "nyc"}), "region", BOXD_REGIONS).unwrap(),
            Some("nyc")
        );
        assert_eq!(
            optional_choice(&json!({}), "region", BOXD_REGIONS).unwrap(),
            None
        );
        assert!(optional_choice(&json!({"region": "lon"}), "region", BOXD_REGIONS).is_err());
    }

    #[test]
    fn tool_paths_resolve_against_the_machine_workspace() {
        let state = BoxdSandboxState {
            sandbox_id: "fra/vm_1".into(),
            region: "fra".into(),
            machine_id: "vm_1".into(),
            name: "er-1".into(),
            workspace_path: BOXD_WORKSPACE_PATH.into(),
            started_at: String::new(),
            title: None,
            url: None,
        };
        assert_eq!(resolve_path(&state, "src"), "/home/boxd/workspace/src");
        assert_eq!(resolve_path(&state, "/tmp/x"), "/tmp/x");
    }
}

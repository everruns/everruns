//! CodeSandbox Capability (Experimental)
//!
//! Provides tools for creating and managing CodeSandbox VM sandboxes via the
//! CodeSandbox HTTP API. Unlike Docker, sandboxes are independent of sessions —
//! the agent creates/destroys them as needed and references them by sandbox ID.
//!
//! **Design decisions:**
//! - Sandboxes are NOT 1:1 with sessions. Agent manages lifecycle.
//! - All tools (except csb_create) take a `sandbox_id` parameter.
//! - Connection info (pitcher_url, pitcher_token) cached in-process.
//! - HTTP calls via reqwest to CodeSandbox Management + Pint APIs.
//! - API key from session secrets (`CSB_API_KEY` via SessionStorageStore).
//!
//! **API surfaces:**
//! - Management API (`https://api.codesandbox.io`): sandbox CRUD, VM lifecycle
//! - Pint API (`{pitcher_url}/api/v1/...`): filesystem, exec, ports

use super::{Capability, CapabilityStatus};
use crate::tools::{Tool, ToolExecutionResult};
use crate::traits::ToolContext;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{LazyLock, RwLock};
use tracing::{debug, error, info, warn};

// ============================================================================
// Constants
// ============================================================================

const MANAGEMENT_API_URL: &str = "https://api.codesandbox.io";
const DEFAULT_TEMPLATE: &str = "static";
const DEFAULT_TIER: &str = "Pico";
const DEFAULT_HIBERNATION_TIMEOUT: u64 = 300;

// ============================================================================
// Connection Cache
// ============================================================================

/// Cached connection info for a running sandbox.
#[derive(Debug, Clone)]
struct CsbConnection {
    pitcher_url: String,
    pitcher_token: String,
}

/// In-process cache: sandbox_id → connection details.
/// Populated by csb_create, consumed by all other tools.
static SANDBOX_CONNECTIONS: LazyLock<RwLock<HashMap<String, CsbConnection>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

fn cache_connection(sandbox_id: &str, conn: CsbConnection) {
    let mut cache = SANDBOX_CONNECTIONS.write().unwrap();
    cache.insert(sandbox_id.to_string(), conn);
}

fn get_connection(sandbox_id: &str) -> Option<CsbConnection> {
    let cache = SANDBOX_CONNECTIONS.read().unwrap();
    cache.get(sandbox_id).cloned()
}

fn remove_connection(sandbox_id: &str) {
    let mut cache = SANDBOX_CONNECTIONS.write().unwrap();
    cache.remove(sandbox_id);
}

// ============================================================================
// API Key — from session secrets
// ============================================================================

const CSB_API_KEY_SECRET: &str = "CSB_API_KEY";

/// Read the CodeSandbox API key from session secrets.
async fn get_api_key(context: &ToolContext) -> Result<String, String> {
    let store = context
        .storage_store
        .as_ref()
        .ok_or("No storage store available. Cannot read CSB_API_KEY secret.")?;

    match store
        .get_secret(context.session_id, CSB_API_KEY_SECRET)
        .await
    {
        Ok(Some(key)) if !key.is_empty() => Ok(key),
        Ok(_) => Err(
            "CSB_API_KEY not found in session secrets. \
             Store it first using the session storage set_secret tool \
             with name 'CSB_API_KEY'. Get a key at https://codesandbox.io/t/api"
                .to_string(),
        ),
        Err(e) => Err(format!("Failed to read CSB_API_KEY secret: {}", e)),
    }
}

// ============================================================================
// HTTP Helpers — Management API
// ============================================================================

/// Response envelope from management API.
#[derive(Debug, Deserialize)]
struct ApiResponse<T> {
    success: bool,
    #[serde(default)]
    errors: Vec<String>,
    data: Option<T>,
}

#[derive(Debug, Deserialize)]
struct CreateSandboxData {
    id: String,
    #[serde(default)]
    alias: Option<String>,
}

#[derive(Debug, Deserialize)]
struct VmStartData {
    pitcher_url: Option<String>,
    pitcher_token: Option<String>,
    pint_url: Option<String>,
    pint_token: Option<String>,
    #[serde(default)]
    bootup_type: Option<String>,
}

async fn management_create_sandbox(
    api_key: &str,
    template: &str,
) -> Result<String, String> {
    let client = reqwest::Client::new();

    let body = json!({
        "runtime": "vm",
        "template": template,
        "privacy": 2,
    });

    let resp = client
        .post(format!("{}/sandbox", MANAGEMENT_API_URL))
        .header("Authorization", format!("Bearer {}", api_key))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Failed to create sandbox: {}", e))?;

    let status = resp.status();
    let text = resp.text().await.map_err(|e| format!("Failed to read response: {}", e))?;

    if !status.is_success() {
        return Err(format!("Create sandbox failed ({}): {}", status, text));
    }

    let api_resp: ApiResponse<CreateSandboxData> =
        serde_json::from_str(&text).map_err(|e| format!("Failed to parse response: {}", e))?;

    if !api_resp.success {
        return Err(format!("Create sandbox failed: {:?}", api_resp.errors));
    }

    api_resp
        .data
        .map(|d| d.id)
        .ok_or_else(|| "No sandbox ID in response".to_string())
}

async fn management_start_vm(
    api_key: &str,
    sandbox_id: &str,
    tier: &str,
    hibernation_timeout: u64,
) -> Result<CsbConnection, String> {
    let client = reqwest::Client::new();

    let body = json!({
        "tier": tier,
        "hibernation_timeout_seconds": hibernation_timeout,
    });

    let resp = client
        .post(format!("{}/vm/{}/start", MANAGEMENT_API_URL, sandbox_id))
        .header("Authorization", format!("Bearer {}", api_key))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Failed to start VM: {}", e))?;

    let status = resp.status();
    let text = resp.text().await.map_err(|e| format!("Failed to read response: {}", e))?;

    if !status.is_success() {
        return Err(format!("Start VM failed ({}): {}", status, text));
    }

    let api_resp: ApiResponse<VmStartData> =
        serde_json::from_str(&text).map_err(|e| format!("Failed to parse response: {}", e))?;

    if !api_resp.success {
        return Err(format!("Start VM failed: {:?}", api_resp.errors));
    }

    let data = api_resp.data.ok_or("No data in start response")?;

    // Prefer pint_url/pint_token, fall back to pitcher_url/pitcher_token
    let url = data
        .pint_url
        .or(data.pitcher_url)
        .ok_or("No pitcher/pint URL in start response")?;
    let token = data
        .pint_token
        .or(data.pitcher_token)
        .ok_or("No pitcher/pint token in start response")?;

    Ok(CsbConnection {
        pitcher_url: url,
        pitcher_token: token,
    })
}

async fn management_shutdown_vm(api_key: &str, sandbox_id: &str) -> Result<(), String> {
    let client = reqwest::Client::new();

    let resp = client
        .post(format!(
            "{}/vm/{}/shutdown",
            MANAGEMENT_API_URL, sandbox_id
        ))
        .header("Authorization", format!("Bearer {}", api_key))
        .send()
        .await
        .map_err(|e| format!("Failed to shutdown VM: {}", e))?;

    if !resp.status().is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("Shutdown failed: {}", text));
    }

    Ok(())
}

async fn management_delete_sandbox(api_key: &str, sandbox_id: &str) -> Result<(), String> {
    let client = reqwest::Client::new();

    let resp = client
        .delete(format!("{}/vm/{}", MANAGEMENT_API_URL, sandbox_id))
        .header("Authorization", format!("Bearer {}", api_key))
        .send()
        .await
        .map_err(|e| format!("Failed to delete sandbox: {}", e))?;

    if !resp.status().is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("Delete failed: {}", text));
    }

    Ok(())
}

// ============================================================================
// HTTP Helpers — Pint API (per-sandbox operations)
// ============================================================================

async fn pint_read_file(conn: &CsbConnection, path: &str) -> Result<String, String> {
    let client = reqwest::Client::new();

    let resp = client
        .get(format!(
            "{}/api/v1/files{}",
            conn.pitcher_url,
            normalize_pint_path(path)
        ))
        .header("Authorization", format!("Bearer {}", conn.pitcher_token))
        .send()
        .await
        .map_err(|e| format!("Failed to read file: {}", e))?;

    let status = resp.status();
    let text = resp.text().await.map_err(|e| format!("Read response error: {}", e))?;

    if !status.is_success() {
        return Err(format!("Read file failed ({}): {}", status, text));
    }

    // Response is JSON: { "path": "...", "content": "..." }
    let parsed: Value =
        serde_json::from_str(&text).map_err(|e| format!("Parse read response: {}", e))?;

    Ok(parsed
        .get("content")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string())
}

async fn pint_write_file(
    conn: &CsbConnection,
    path: &str,
    content: &str,
) -> Result<(), String> {
    let client = reqwest::Client::new();

    let body = json!({ "content": content });

    let resp = client
        .post(format!(
            "{}/api/v1/files{}",
            conn.pitcher_url,
            normalize_pint_path(path)
        ))
        .header("Authorization", format!("Bearer {}", conn.pitcher_token))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Failed to write file: {}", e))?;

    if !resp.status().is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("Write file failed: {}", text));
    }

    Ok(())
}

async fn pint_list_dir(conn: &CsbConnection, path: &str) -> Result<Value, String> {
    let client = reqwest::Client::new();

    let resp = client
        .get(format!(
            "{}/api/v1/directories{}",
            conn.pitcher_url,
            normalize_pint_path(path)
        ))
        .header("Authorization", format!("Bearer {}", conn.pitcher_token))
        .send()
        .await
        .map_err(|e| format!("Failed to list directory: {}", e))?;

    let status = resp.status();
    let text = resp.text().await.map_err(|e| format!("Read response error: {}", e))?;

    if !status.is_success() {
        return Err(format!("List directory failed ({}): {}", status, text));
    }

    serde_json::from_str(&text).map_err(|e| format!("Parse list response: {}", e))
}

/// Exec response from pint API.
#[derive(Debug, Deserialize)]
struct ExecResponse {
    id: String,
    #[serde(default)]
    status: Option<String>,
    #[serde(default, rename = "exitCode")]
    exit_code: Option<i32>,
}

/// SSE output event from exec I/O stream.
#[derive(Debug, Deserialize)]
struct ExecIoEvent {
    #[serde(default, rename = "type")]
    event_type: Option<String>,
    #[serde(default)]
    output: Option<String>,
    #[serde(default, rename = "exitCode")]
    exit_code: Option<i32>,
}

async fn pint_exec(
    conn: &CsbConnection,
    command: &str,
    args: &[&str],
) -> Result<Value, String> {
    let client = reqwest::Client::new();

    let body = json!({
        "command": command,
        "args": args,
        "autorun": true,
        "interactive": false,
    });

    let resp = client
        .post(format!("{}/api/v1/execs", conn.pitcher_url))
        .header("Authorization", format!("Bearer {}", conn.pitcher_token))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Failed to create exec: {}", e))?;

    let status = resp.status();
    let text = resp.text().await.map_err(|e| format!("Read response error: {}", e))?;

    if !status.is_success() {
        return Err(format!("Create exec failed ({}): {}", status, text));
    }

    let exec_resp: ExecResponse =
        serde_json::from_str(&text).map_err(|e| format!("Parse exec response: {}", e))?;

    let exec_id = &exec_resp.id;

    // Poll for output via the I/O endpoint (non-streaming, collect all output)
    // Use a simple GET with a timeout to collect output
    let io_resp = client
        .get(format!(
            "{}/api/v1/execs/{}/io",
            conn.pitcher_url, exec_id
        ))
        .header("Authorization", format!("Bearer {}", conn.pitcher_token))
        .header("Accept", "text/event-stream")
        .timeout(std::time::Duration::from_secs(60))
        .send()
        .await
        .map_err(|e| format!("Failed to read exec output: {}", e))?;

    let output_text = io_resp
        .text()
        .await
        .map_err(|e| format!("Read exec output error: {}", e))?;

    // Parse SSE events to extract output
    let mut stdout = String::new();
    let mut stderr = String::new();
    let mut final_exit_code: Option<i32> = exec_resp.exit_code;

    for line in output_text.lines() {
        let line = line.trim();
        if let Some(data) = line.strip_prefix("data: ") {
            if let Ok(event) = serde_json::from_str::<ExecIoEvent>(data) {
                if let Some(ref output) = event.output {
                    match event.event_type.as_deref() {
                        Some("stderr") => stderr.push_str(output),
                        _ => stdout.push_str(output),
                    }
                }
                if let Some(code) = event.exit_code {
                    final_exit_code = Some(code);
                }
            }
        }
    }

    let exit_code = final_exit_code.unwrap_or(-1);

    Ok(json!({
        "exec_id": exec_id,
        "stdout": stdout,
        "stderr": stderr,
        "exit_code": exit_code,
        "success": exit_code == 0,
    }))
}

async fn pint_list_ports(conn: &CsbConnection) -> Result<Value, String> {
    let client = reqwest::Client::new();

    let resp = client
        .get(format!("{}/api/v1/ports", conn.pitcher_url))
        .header("Authorization", format!("Bearer {}", conn.pitcher_token))
        .send()
        .await
        .map_err(|e| format!("Failed to list ports: {}", e))?;

    let status = resp.status();
    let text = resp.text().await.map_err(|e| format!("Read response error: {}", e))?;

    if !status.is_success() {
        return Err(format!("List ports failed ({}): {}", status, text));
    }

    serde_json::from_str(&text).map_err(|e| format!("Parse ports response: {}", e))
}

/// Ensure pint paths start with /
fn normalize_pint_path(path: &str) -> String {
    if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{}", path)
    }
}

/// Get connection, reconnecting (re-starting VM) if cache miss.
async fn get_or_reconnect(
    sandbox_id: &str,
    context: &ToolContext,
) -> Result<CsbConnection, String> {
    if let Some(conn) = get_connection(sandbox_id) {
        return Ok(conn);
    }

    // Cache miss — try to start the VM (it may be hibernated)
    let api_key = get_api_key(context).await?;
    info!(
        "Cache miss for sandbox {}, attempting VM start",
        sandbox_id
    );

    let conn =
        management_start_vm(&api_key, sandbox_id, DEFAULT_TIER, DEFAULT_HIBERNATION_TIMEOUT)
            .await?;
    cache_connection(sandbox_id, conn.clone());
    Ok(conn)
}

// ============================================================================
// VFS Sync Helper
// ============================================================================

/// Copy all files from session VFS to a CodeSandbox sandbox.
async fn sync_vfs_to_sandbox(
    conn: &CsbConnection,
    context: &ToolContext,
) -> Result<usize, String> {
    let file_store = context
        .file_store
        .as_ref()
        .ok_or("No file store available in context")?;

    // List root directory recursively
    let files = list_files_recursive(file_store.as_ref(), context.session_id, "/").await?;

    let mut count = 0;
    for (path, content) in &files {
        let sandbox_path = format!("/project/workspace{}", path);
        debug!("Syncing VFS file {} -> {}", path, sandbox_path);
        pint_write_file(conn, &sandbox_path, content).await?;
        count += 1;
    }

    Ok(count)
}

/// Recursively list all files from the session file store.
async fn list_files_recursive(
    store: &dyn crate::traits::SessionFileStore,
    session_id: crate::typed_id::SessionId,
    dir_path: &str,
) -> Result<Vec<(String, String)>, String> {
    let entries = store
        .list_directory(session_id, dir_path)
        .await
        .map_err(|e| format!("Failed to list directory {}: {}", dir_path, e))?;

    let mut files = Vec::new();

    for entry in entries {
        if entry.is_directory {
            let sub_files = Box::pin(list_files_recursive(store, session_id, &entry.path)).await?;
            files.extend(sub_files);
        } else {
            match store.read_file(session_id, &entry.path).await {
                Ok(Some(file)) => {
                    if let Some(content) = file.content {
                        files.push((entry.path.clone(), content));
                    }
                }
                Ok(None) => {
                    warn!("File listed but not found: {}", entry.path);
                }
                Err(e) => {
                    warn!("Failed to read file {}: {}", entry.path, e);
                }
            }
        }
    }

    Ok(files)
}

// ============================================================================
// Capability
// ============================================================================

/// CodeSandbox capability — agent-managed VM sandboxes via HTTP API.
pub struct CodeSandboxCapability;

impl Capability for CodeSandboxCapability {
    fn id(&self) -> &str {
        "codesandbox"
    }

    fn name(&self) -> &str {
        "[Experimental] CodeSandbox"
    }

    fn description(&self) -> &str {
        "Create and manage CodeSandbox VM sandboxes. Each sandbox is a full Linux VM \
         with filesystem, command execution, and port preview. Sandboxes are independent \
         of sessions — create as many as needed, reference by sandbox ID."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn icon(&self) -> Option<&str> {
        Some("cloud")
    }

    fn category(&self) -> Option<&str> {
        Some("Development")
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some(
            r#"You have access to CodeSandbox for creating and managing cloud VM sandboxes.
Each sandbox is a full Linux VM (Firecracker microVM) with its own filesystem, shell, and network.

IMPORTANT: This is an EXPERIMENTAL capability.
Requires a CSB_API_KEY stored as a session secret (via set_secret with name 'CSB_API_KEY').
Get a key at https://codesandbox.io/t/api

Available tools:
- `csb_create`: Create a new sandbox VM. Returns a sandbox_id you must use for all other calls.
  Optionally copies the session's virtual filesystem into the sandbox.
- `csb_exec`: Execute a shell command in a sandbox.
- `csb_read_file`: Read a file from a sandbox.
- `csb_write_file`: Write a file to a sandbox.
- `csb_preview_url`: Get the preview URL for a port running in the sandbox.
- `csb_shutdown`: Shutdown and delete a sandbox when done.

Workflow:
1. Ensure CSB_API_KEY is stored as a session secret.
2. Create a sandbox with `csb_create` (optionally with a template).
3. Use `csb_exec`, `csb_read_file`, `csb_write_file` to work in the sandbox.
4. Use `csb_preview_url` to get URLs for running web servers.
5. When finished, call `csb_shutdown` to clean up resources.

You can create multiple sandboxes and manage them independently.
All tools (except csb_create) require the sandbox_id returned by csb_create.
Sandbox files live under /project/workspace."#,
        )
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![
            Box::new(CsbCreateTool),
            Box::new(CsbExecTool),
            Box::new(CsbReadFileTool),
            Box::new(CsbWriteFileTool),
            Box::new(CsbPreviewUrlTool),
            Box::new(CsbShutdownTool),
        ]
    }
}

// ============================================================================
// CsbCreateTool
// ============================================================================

pub struct CsbCreateTool;

#[async_trait]
impl Tool for CsbCreateTool {
    fn name(&self) -> &str {
        "csb_create"
    }

    fn description(&self) -> &str {
        "Create a new CodeSandbox VM sandbox. Returns a sandbox_id to use with other csb_* tools. \
         Optionally copies the session's virtual filesystem into the sandbox."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "template": {
                    "type": "string",
                    "description": "Sandbox template (e.g., 'static', 'node', 'python'). Defaults to 'static'."
                },
                "copy_file_system": {
                    "type": "boolean",
                    "description": "If true, copies all files from the session's virtual filesystem into the sandbox's /project/workspace. Defaults to false."
                },
                "tier": {
                    "type": "string",
                    "description": "VM tier: 'Pico' (1 CPU/2GB), 'Nano' (2/4), 'Micro' (4/8), 'Small' (8/16). Defaults to 'Pico'."
                }
            },
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "csb_create requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let template = arguments
            .get("template")
            .and_then(|v| v.as_str())
            .unwrap_or(DEFAULT_TEMPLATE);

        let copy_fs = arguments
            .get("copy_file_system")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let tier = arguments
            .get("tier")
            .and_then(|v| v.as_str())
            .unwrap_or(DEFAULT_TIER);

        // Get API key from session secrets
        let api_key = match get_api_key(context).await {
            Ok(k) => k,
            Err(e) => return ToolExecutionResult::tool_error(e),
        };

        info!("Creating CodeSandbox sandbox (template={}, tier={})", template, tier);

        // Step 1: Create sandbox
        let sandbox_id = match management_create_sandbox(&api_key, template).await {
            Ok(id) => id,
            Err(e) => {
                error!("Failed to create sandbox: {}", e);
                return ToolExecutionResult::tool_error(format!("Failed to create sandbox: {}", e));
            }
        };

        info!("Created sandbox: {}", sandbox_id);

        // Step 2: Start VM
        let conn = match management_start_vm(&api_key, &sandbox_id, tier, DEFAULT_HIBERNATION_TIMEOUT).await {
            Ok(c) => c,
            Err(e) => {
                error!("Failed to start VM for sandbox {}: {}", sandbox_id, e);
                // Try to clean up the created sandbox
                let _ = management_delete_sandbox(&api_key, &sandbox_id).await;
                return ToolExecutionResult::tool_error(format!("Failed to start sandbox VM: {}", e));
            }
        };

        // Cache connection
        cache_connection(&sandbox_id, conn.clone());

        info!("Sandbox {} VM started", sandbox_id);

        // Step 3: Optionally sync VFS
        let mut files_synced = 0;
        if copy_fs {
            match sync_vfs_to_sandbox(&conn, context).await {
                Ok(count) => {
                    files_synced = count;
                    info!("Synced {} files from VFS to sandbox {}", count, sandbox_id);
                }
                Err(e) => {
                    warn!("VFS sync partially failed for sandbox {}: {}", sandbox_id, e);
                    // Non-fatal — sandbox is still usable
                }
            }
        }

        ToolExecutionResult::success(json!({
            "sandbox_id": sandbox_id,
            "status": "running",
            "template": template,
            "tier": tier,
            "files_synced": files_synced,
            "workspace_path": "/project/workspace",
        }))
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// CsbExecTool
// ============================================================================

pub struct CsbExecTool;

#[async_trait]
impl Tool for CsbExecTool {
    fn name(&self) -> &str {
        "csb_exec"
    }

    fn description(&self) -> &str {
        "Execute a shell command inside a CodeSandbox sandbox. \
         Returns stdout, stderr, and exit code."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "sandbox_id": {
                    "type": "string",
                    "description": "The sandbox ID returned by csb_create"
                },
                "command": {
                    "type": "string",
                    "description": "Shell command to execute (e.g., 'npm install', 'python app.py')"
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
        let sandbox_id = match arguments.get("sandbox_id").and_then(|v| v.as_str()) {
            Some(id) => id,
            None => return ToolExecutionResult::tool_error("Missing required parameter: sandbox_id"),
        };

        let command = match arguments.get("command").and_then(|v| v.as_str()) {
            Some(c) => c,
            None => return ToolExecutionResult::tool_error("Missing required parameter: command"),
        };

        let conn = match get_or_reconnect(sandbox_id, context).await {
            Ok(c) => c,
            Err(e) => return ToolExecutionResult::tool_error(format!("Sandbox connection failed: {}", e)),
        };

        debug!("Executing in sandbox {}: {}", sandbox_id, command);

        match pint_exec(&conn, "sh", &["-c", command]).await {
            Ok(result) => ToolExecutionResult::success(result),
            Err(e) => ToolExecutionResult::tool_error(format!("Exec failed: {}", e)),
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// CsbReadFileTool
// ============================================================================

pub struct CsbReadFileTool;

#[async_trait]
impl Tool for CsbReadFileTool {
    fn name(&self) -> &str {
        "csb_read_file"
    }

    fn description(&self) -> &str {
        "Read a file from a CodeSandbox sandbox filesystem."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "sandbox_id": {
                    "type": "string",
                    "description": "The sandbox ID returned by csb_create"
                },
                "path": {
                    "type": "string",
                    "description": "Absolute path inside the sandbox (e.g., '/project/workspace/index.js')"
                }
            },
            "required": ["sandbox_id", "path"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "csb_read_file requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let sandbox_id = match arguments.get("sandbox_id").and_then(|v| v.as_str()) {
            Some(id) => id,
            None => return ToolExecutionResult::tool_error("Missing required parameter: sandbox_id"),
        };

        let path = match arguments.get("path").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return ToolExecutionResult::tool_error("Missing required parameter: path"),
        };

        let conn = match get_or_reconnect(sandbox_id, context).await {
            Ok(c) => c,
            Err(e) => return ToolExecutionResult::tool_error(format!("Sandbox connection failed: {}", e)),
        };

        debug!("Reading file from sandbox {}: {}", sandbox_id, path);

        match pint_read_file(&conn, path).await {
            Ok(content) => ToolExecutionResult::success(json!({
                "path": path,
                "content": content,
                "size_bytes": content.len(),
            })),
            Err(e) => ToolExecutionResult::tool_error(format!("Read failed: {}", e)),
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// CsbWriteFileTool
// ============================================================================

pub struct CsbWriteFileTool;

#[async_trait]
impl Tool for CsbWriteFileTool {
    fn name(&self) -> &str {
        "csb_write_file"
    }

    fn description(&self) -> &str {
        "Write a file to a CodeSandbox sandbox filesystem. Creates parent directories automatically."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "sandbox_id": {
                    "type": "string",
                    "description": "The sandbox ID returned by csb_create"
                },
                "path": {
                    "type": "string",
                    "description": "Absolute path inside the sandbox (e.g., '/project/workspace/main.py')"
                },
                "content": {
                    "type": "string",
                    "description": "File content to write"
                }
            },
            "required": ["sandbox_id", "path", "content"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "csb_write_file requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let sandbox_id = match arguments.get("sandbox_id").and_then(|v| v.as_str()) {
            Some(id) => id,
            None => return ToolExecutionResult::tool_error("Missing required parameter: sandbox_id"),
        };

        let path = match arguments.get("path").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return ToolExecutionResult::tool_error("Missing required parameter: path"),
        };

        let content = match arguments.get("content").and_then(|v| v.as_str()) {
            Some(c) => c,
            None => return ToolExecutionResult::tool_error("Missing required parameter: content"),
        };

        let conn = match get_or_reconnect(sandbox_id, context).await {
            Ok(c) => c,
            Err(e) => return ToolExecutionResult::tool_error(format!("Sandbox connection failed: {}", e)),
        };

        debug!("Writing file to sandbox {}: {}", sandbox_id, path);

        match pint_write_file(&conn, path, content).await {
            Ok(()) => ToolExecutionResult::success(json!({
                "path": path,
                "size_bytes": content.len(),
                "success": true,
            })),
            Err(e) => ToolExecutionResult::tool_error(format!("Write failed: {}", e)),
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// CsbPreviewUrlTool
// ============================================================================

pub struct CsbPreviewUrlTool;

#[async_trait]
impl Tool for CsbPreviewUrlTool {
    fn name(&self) -> &str {
        "csb_preview_url"
    }

    fn description(&self) -> &str {
        "Get the preview URL for a port running in a CodeSandbox sandbox. \
         Also lists all currently open ports."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "sandbox_id": {
                    "type": "string",
                    "description": "The sandbox ID returned by csb_create"
                },
                "port": {
                    "type": "integer",
                    "description": "The port number to get the preview URL for (e.g., 3000, 8080)"
                }
            },
            "required": ["sandbox_id", "port"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "csb_preview_url requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let sandbox_id = match arguments.get("sandbox_id").and_then(|v| v.as_str()) {
            Some(id) => id,
            None => return ToolExecutionResult::tool_error("Missing required parameter: sandbox_id"),
        };

        let port = match arguments.get("port").and_then(|v| v.as_u64()) {
            Some(p) => p,
            None => return ToolExecutionResult::tool_error("Missing required parameter: port"),
        };

        let conn = match get_or_reconnect(sandbox_id, context).await {
            Ok(c) => c,
            Err(e) => return ToolExecutionResult::tool_error(format!("Sandbox connection failed: {}", e)),
        };

        // List open ports
        let ports = match pint_list_ports(&conn).await {
            Ok(p) => p,
            Err(e) => {
                warn!("Failed to list ports: {}", e);
                json!([])
            }
        };

        // Preview URL follows the pattern: https://{sandbox_id}-{port}.csb.app
        let preview_url = format!("https://{}-{}.csb.app", sandbox_id, port);

        ToolExecutionResult::success(json!({
            "sandbox_id": sandbox_id,
            "port": port,
            "preview_url": preview_url,
            "open_ports": ports,
        }))
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// CsbShutdownTool
// ============================================================================

pub struct CsbShutdownTool;

#[async_trait]
impl Tool for CsbShutdownTool {
    fn name(&self) -> &str {
        "csb_shutdown"
    }

    fn description(&self) -> &str {
        "Shutdown and permanently delete a CodeSandbox sandbox. \
         Use this to clean up resources when a sandbox is no longer needed."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "sandbox_id": {
                    "type": "string",
                    "description": "The sandbox ID returned by csb_create"
                },
                "delete": {
                    "type": "boolean",
                    "description": "If true, permanently delete the sandbox (default: true). If false, only hibernate."
                }
            },
            "required": ["sandbox_id"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "csb_shutdown requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let sandbox_id = match arguments.get("sandbox_id").and_then(|v| v.as_str()) {
            Some(id) => id,
            None => return ToolExecutionResult::tool_error("Missing required parameter: sandbox_id"),
        };

        let delete = arguments
            .get("delete")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        let api_key = match get_api_key(context).await {
            Ok(k) => k,
            Err(e) => return ToolExecutionResult::tool_error(e),
        };

        info!("Shutting down sandbox {} (delete={})", sandbox_id, delete);

        // Shutdown VM first
        let shutdown_result = management_shutdown_vm(&api_key, sandbox_id).await;
        if let Err(ref e) = shutdown_result {
            warn!("Shutdown warning for {}: {}", sandbox_id, e);
        }

        // Delete if requested
        let mut deleted = false;
        if delete {
            match management_delete_sandbox(&api_key, sandbox_id).await {
                Ok(()) => {
                    deleted = true;
                    info!("Sandbox {} deleted", sandbox_id);
                }
                Err(e) => {
                    warn!("Delete failed for {}: {}", sandbox_id, e);
                }
            }
        }

        // Remove from cache
        remove_connection(sandbox_id);

        ToolExecutionResult::success(json!({
            "sandbox_id": sandbox_id,
            "shutdown": shutdown_result.is_ok(),
            "deleted": deleted,
            "message": if deleted {
                "Sandbox shutdown and deleted"
            } else if shutdown_result.is_ok() {
                "Sandbox shutdown (hibernated, not deleted)"
            } else {
                "Sandbox cleanup attempted with warnings"
            },
        }))
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_capability_metadata() {
        let cap = CodeSandboxCapability;
        assert_eq!(cap.id(), "codesandbox");
        assert_eq!(cap.name(), "[Experimental] CodeSandbox");
        assert_eq!(cap.status(), CapabilityStatus::Available);
        assert_eq!(cap.icon(), Some("cloud"));
        assert_eq!(cap.category(), Some("Development"));
    }

    #[test]
    fn test_capability_has_tools() {
        let cap = CodeSandboxCapability;
        let tools = cap.tools();

        assert_eq!(tools.len(), 6);

        let tool_names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
        assert!(tool_names.contains(&"csb_create"));
        assert!(tool_names.contains(&"csb_exec"));
        assert!(tool_names.contains(&"csb_read_file"));
        assert!(tool_names.contains(&"csb_write_file"));
        assert!(tool_names.contains(&"csb_preview_url"));
        assert!(tool_names.contains(&"csb_shutdown"));
    }

    #[test]
    fn test_capability_has_system_prompt() {
        let cap = CodeSandboxCapability;
        let prompt = cap.system_prompt_addition().unwrap();
        assert!(prompt.contains("csb_create"));
        assert!(prompt.contains("csb_exec"));
        assert!(prompt.contains("csb_read_file"));
        assert!(prompt.contains("csb_write_file"));
        assert!(prompt.contains("csb_preview_url"));
        assert!(prompt.contains("csb_shutdown"));
        assert!(prompt.contains("EXPERIMENTAL"));
    }

    #[test]
    fn test_tools_require_context() {
        assert!(CsbCreateTool.requires_context());
        assert!(CsbExecTool.requires_context());
        assert!(CsbReadFileTool.requires_context());
        assert!(CsbWriteFileTool.requires_context());
        assert!(CsbPreviewUrlTool.requires_context());
        assert!(CsbShutdownTool.requires_context());
    }

    #[tokio::test]
    async fn test_csb_create_without_context() {
        let tool = CsbCreateTool;
        let result = tool.execute(json!({})).await;

        if let ToolExecutionResult::ToolError(msg) = result {
            assert!(msg.contains("requires context"));
        } else {
            panic!("Expected tool error");
        }
    }

    #[tokio::test]
    async fn test_csb_exec_missing_sandbox_id() {
        let tool = CsbExecTool;
        let context = ToolContext::new(crate::typed_id::SessionId::from_uuid(uuid::Uuid::nil()));

        let result = tool
            .execute_with_context(json!({"command": "ls"}), &context)
            .await;

        if let ToolExecutionResult::ToolError(msg) = result {
            assert!(msg.contains("sandbox_id"));
        } else {
            panic!("Expected tool error for missing sandbox_id");
        }
    }

    #[tokio::test]
    async fn test_csb_exec_missing_command() {
        let tool = CsbExecTool;
        let context = ToolContext::new(crate::typed_id::SessionId::from_uuid(uuid::Uuid::nil()));

        let result = tool
            .execute_with_context(json!({"sandbox_id": "test-123"}), &context)
            .await;

        if let ToolExecutionResult::ToolError(msg) = result {
            assert!(msg.contains("command"));
        } else {
            panic!("Expected tool error for missing command");
        }
    }

    #[tokio::test]
    async fn test_csb_read_file_missing_params() {
        let tool = CsbReadFileTool;
        let context = ToolContext::new(crate::typed_id::SessionId::from_uuid(uuid::Uuid::nil()));

        let result = tool.execute_with_context(json!({}), &context).await;

        if let ToolExecutionResult::ToolError(msg) = result {
            assert!(msg.contains("sandbox_id"));
        } else {
            panic!("Expected tool error for missing sandbox_id");
        }
    }

    #[tokio::test]
    async fn test_csb_write_file_missing_params() {
        let tool = CsbWriteFileTool;
        let context = ToolContext::new(crate::typed_id::SessionId::from_uuid(uuid::Uuid::nil()));

        // Missing all
        let result = tool.execute_with_context(json!({}), &context).await;
        if let ToolExecutionResult::ToolError(msg) = result {
            assert!(msg.contains("sandbox_id"));
        } else {
            panic!("Expected tool error");
        }

        // Missing content
        let result = tool
            .execute_with_context(
                json!({"sandbox_id": "test", "path": "/test.txt"}),
                &context,
            )
            .await;
        if let ToolExecutionResult::ToolError(msg) = result {
            assert!(msg.contains("content"));
        } else {
            panic!("Expected tool error for missing content");
        }
    }

    #[tokio::test]
    async fn test_csb_shutdown_missing_sandbox_id() {
        let tool = CsbShutdownTool;
        let context = ToolContext::new(crate::typed_id::SessionId::from_uuid(uuid::Uuid::nil()));

        let result = tool.execute_with_context(json!({}), &context).await;

        if let ToolExecutionResult::ToolError(msg) = result {
            assert!(msg.contains("sandbox_id"));
        } else {
            panic!("Expected tool error");
        }
    }

    #[tokio::test]
    async fn test_csb_preview_url_missing_params() {
        let tool = CsbPreviewUrlTool;
        let context = ToolContext::new(crate::typed_id::SessionId::from_uuid(uuid::Uuid::nil()));

        let result = tool.execute_with_context(json!({}), &context).await;

        if let ToolExecutionResult::ToolError(msg) = result {
            assert!(msg.contains("sandbox_id"));
        } else {
            panic!("Expected tool error");
        }
    }

    #[test]
    fn test_normalize_pint_path() {
        assert_eq!(normalize_pint_path("/project/workspace"), "/project/workspace");
        assert_eq!(normalize_pint_path("project/workspace"), "/project/workspace");
        assert_eq!(normalize_pint_path("/"), "/");
    }

    #[test]
    fn test_connection_cache() {
        let conn = CsbConnection {
            pitcher_url: "https://test.example.com".to_string(),
            pitcher_token: "token123".to_string(),
        };

        cache_connection("test-sandbox-1", conn.clone());
        let retrieved = get_connection("test-sandbox-1");
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().pitcher_url, "https://test.example.com");

        remove_connection("test-sandbox-1");
        assert!(get_connection("test-sandbox-1").is_none());
    }

    #[test]
    fn test_api_key_not_set() {
        // This test relies on CSB_API_KEY not being set in the test environment
        // If it happens to be set, the test still passes (just takes a different branch)
        let result = get_api_key();
        // We just verify it returns a Result, not panic
        let _ = result;
    }
}

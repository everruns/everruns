//! Daytona Integration (Experimental)
//!
//! Cloud-based sandboxed code execution via Daytona REST API.
//! Supports multiple sandboxes per session, each identified by sandbox_id.
//!
//! Decision: External integration crate, auto-registered via inventory plugin system
//! Decision: Use secrets store for all state (API key + per-sandbox connection info)
//! Decision: Two-tier API: Management API for lifecycle, Toolbox API for in-sandbox ops
//! Decision: Sync exec via toolbox process/execute endpoint
//! Decision: session_storage dependency for API key and state persistence

use everruns_core::capabilities::{Capability, CapabilityStatus, IntegrationPlugin};
use everruns_core::tools::{Tool, ToolExecutionResult};
use everruns_core::traits::ToolContext;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::time::Duration;
use tracing::{debug, error, warn};

// ============================================================================
// Integration Plugin Registration
// ============================================================================

inventory::submit! {
    IntegrationPlugin {
        experimental_only: true,
        factory: || Box::new(DaytonaCapability),
    }
}

// ============================================================================
// Constants
// ============================================================================

const DAYTONA_API_BASE: &str = "https://app.daytona.io/api";
const DAYTONA_TOOLBOX_BASE: &str = "https://proxy.app.daytona.io/toolbox";
const DAYTONA_API_KEY_SECRET: &str = "DAYTONA_API_KEY";
const DAYTONA_SANDBOX_SECRET_PREFIX: &str = "daytona_sandbox:";
const EXEC_TIMEOUT_MS: u64 = 120_000;
const SANDBOX_READY_POLL_INTERVAL: Duration = Duration::from_secs(2);
const SANDBOX_READY_MAX_WAIT: Duration = Duration::from_secs(60);
/// Auto-stop after 5 minutes of inactivity (safety net)
const AUTO_STOP_INTERVAL_MINUTES: u64 = 5;

// ============================================================================
// API Response Types
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SandboxInfo {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecResult {
    #[serde(default)]
    pub result: String,
    #[serde(default)]
    pub exit_code: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    pub session_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionExecResult {
    #[serde(default)]
    pub cmd_id: Option<String>,
    #[serde(default)]
    pub output: Option<String>,
    #[serde(default)]
    pub exit_code: Option<i32>,
}

// ============================================================================
// Persisted Sandbox State (stored in session secrets as JSON)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxState {
    pub sandbox_id: String,
    pub workspace_path: String,
    pub started_at: String,
}

// ============================================================================
// DaytonaClient - HTTP client for Daytona APIs
// ============================================================================

pub struct DaytonaClient {
    http: reqwest::Client,
    api_key: String,
    api_base: String,
    toolbox_base: String,
}

impl DaytonaClient {
    pub fn new(api_key: String) -> Self {
        Self::with_base_urls(
            api_key,
            DAYTONA_API_BASE.to_string(),
            DAYTONA_TOOLBOX_BASE.to_string(),
        )
    }

    pub fn with_base_urls(api_key: String, api_base: String, toolbox_base: String) -> Self {
        Self {
            http: reqwest::Client::new(),
            api_key,
            api_base,
            toolbox_base,
        }
    }

    // --- Generic request helpers ---

    async fn management_request(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value, String> {
        let url = format!("{}{}", self.api_base, path);
        let mut req = self
            .http
            .request(method, &url)
            .bearer_auth(&self.api_key)
            .header("Content-Type", "application/json");

        if let Some(b) = body {
            req = req.json(&b);
        }

        let resp = req
            .send()
            .await
            .map_err(|e| format!("Failed to connect to Daytona API: {e}"))?;

        let status = resp.status();
        let body_text = resp
            .text()
            .await
            .map_err(|e| format!("Failed to read response: {e}"))?;

        if !status.is_success() {
            return Err(format!("Daytona API error ({status}): {body_text}"));
        }

        if body_text.is_empty() {
            return Ok(json!({}));
        }

        serde_json::from_str(&body_text).map_err(|e| format!("Invalid JSON from Daytona: {e}"))
    }

    /// Make a request to the Toolbox API (in-sandbox HTTP REST API).
    async fn toolbox_request(
        &self,
        method: reqwest::Method,
        sandbox_id: &str,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value, String> {
        let url = format!("{}/{sandbox_id}{path}", self.toolbox_base);
        let mut req = self
            .http
            .request(method, &url)
            .bearer_auth(&self.api_key)
            .header("Content-Type", "application/json");

        if let Some(b) = body {
            req = req.json(&b);
        }

        let resp = req
            .send()
            .await
            .map_err(|e| format!("Failed to connect to sandbox: {e}"))?;

        let status = resp.status();
        let body_text = resp
            .text()
            .await
            .map_err(|e| format!("Failed to read sandbox response: {e}"))?;

        if !status.is_success() {
            return Err(format!("Sandbox API error ({status}): {body_text}"));
        }

        if body_text.is_empty() {
            return Ok(json!({}));
        }

        serde_json::from_str(&body_text).map_err(|e| format!("Invalid JSON from sandbox: {e}"))
    }

    /// Raw toolbox GET that returns bytes (for file download).
    async fn toolbox_download(&self, sandbox_id: &str, path: &str) -> Result<Vec<u8>, String> {
        let url = format!("{}/{sandbox_id}{path}", self.toolbox_base);
        let resp = self
            .http
            .get(&url)
            .bearer_auth(&self.api_key)
            .send()
            .await
            .map_err(|e| format!("Failed to connect to sandbox: {e}"))?;

        let status = resp.status();
        if !status.is_success() {
            let body_text = resp
                .text()
                .await
                .map_err(|e| format!("Failed to read response: {e}"))?;
            return Err(format!("Sandbox API error ({status}): {body_text}"));
        }

        resp.bytes()
            .await
            .map(|b| b.to_vec())
            .map_err(|e| format!("Failed to read file bytes: {e}"))
    }

    /// Multipart file upload to the toolbox API.
    async fn toolbox_upload(
        &self,
        sandbox_id: &str,
        remote_path: &str,
        content: &[u8],
        filename: &str,
    ) -> Result<(), String> {
        let url = format!(
            "{}/{sandbox_id}/files/upload?path={}",
            self.toolbox_base,
            urlencoding::encode(remote_path)
        );

        let part = reqwest::multipart::Part::bytes(content.to_vec())
            .file_name(filename.to_string())
            .mime_str("application/octet-stream")
            .map_err(|e| format!("Failed to create multipart part: {e}"))?;

        let form = reqwest::multipart::Form::new().part("file", part);

        let resp = self
            .http
            .post(&url)
            .bearer_auth(&self.api_key)
            .multipart(form)
            .send()
            .await
            .map_err(|e| format!("Failed to upload file: {e}"))?;

        let status = resp.status();
        if !status.is_success() {
            let body_text = resp
                .text()
                .await
                .map_err(|e| format!("Failed to read response: {e}"))?;
            return Err(format!("File upload error ({status}): {body_text}"));
        }

        Ok(())
    }

    // --- Management API ---

    pub async fn create_sandbox(&self, body: Value) -> Result<SandboxInfo, String> {
        let resp = self
            .management_request(reqwest::Method::POST, "/sandbox", Some(body))
            .await?;
        serde_json::from_value(resp).map_err(|e| format!("Failed to parse sandbox info: {e}"))
    }

    pub async fn get_sandbox(&self, sandbox_id: &str) -> Result<SandboxInfo, String> {
        let resp = self
            .management_request(
                reqwest::Method::GET,
                &format!("/sandbox/{sandbox_id}"),
                None,
            )
            .await?;
        serde_json::from_value(resp).map_err(|e| format!("Failed to parse sandbox info: {e}"))
    }

    pub async fn start_sandbox(&self, sandbox_id: &str) -> Result<(), String> {
        self.management_request(
            reqwest::Method::POST,
            &format!("/sandbox/{sandbox_id}/start"),
            None,
        )
        .await?;
        Ok(())
    }

    pub async fn stop_sandbox(&self, sandbox_id: &str) -> Result<(), String> {
        self.management_request(
            reqwest::Method::POST,
            &format!("/sandbox/{sandbox_id}/stop"),
            None,
        )
        .await?;
        Ok(())
    }

    pub async fn delete_sandbox(&self, sandbox_id: &str) -> Result<(), String> {
        self.management_request(
            reqwest::Method::DELETE,
            &format!("/sandbox/{sandbox_id}"),
            None,
        )
        .await?;
        Ok(())
    }

    pub async fn set_autostop(
        &self,
        sandbox_id: &str,
        interval_minutes: u64,
    ) -> Result<(), String> {
        self.management_request(
            reqwest::Method::POST,
            &format!("/sandbox/{sandbox_id}/autostop/{interval_minutes}"),
            None,
        )
        .await?;
        Ok(())
    }

    /// Wait for sandbox to reach "started" state after creation.
    pub async fn wait_for_ready(&self, sandbox_id: &str) -> Result<(), String> {
        let start = std::time::Instant::now();
        let mut interval = SANDBOX_READY_POLL_INTERVAL;

        while start.elapsed() < SANDBOX_READY_MAX_WAIT {
            match self.get_sandbox(sandbox_id).await {
                Ok(info) => {
                    if info.state == "started" {
                        debug!("Sandbox ready (state: started) after {:?}", start.elapsed());
                        return Ok(());
                    }
                    if info.state == "error" || info.state == "build_failed" {
                        return Err(format!("Sandbox entered error state: {}", info.state));
                    }
                    debug!("Sandbox not ready yet (state: {}), retrying...", info.state);
                }
                Err(e) => {
                    debug!("Failed to poll sandbox state: {e}, retrying...");
                }
            }

            tokio::time::sleep(interval).await;
            interval = std::cmp::min(interval * 2, Duration::from_secs(4));
        }

        Err(format!(
            "Sandbox did not become ready within {}s",
            SANDBOX_READY_MAX_WAIT.as_secs()
        ))
    }

    // --- Toolbox API: Process ---

    pub async fn exec(
        &self,
        sandbox_id: &str,
        command: &str,
        cwd: Option<&str>,
        timeout_ms: Option<u64>,
    ) -> Result<ExecResult, String> {
        let mut body = json!({ "command": command });
        if let Some(c) = cwd {
            body["cwd"] = json!(c);
        }
        if let Some(t) = timeout_ms {
            body["timeout"] = json!(t);
        }
        let resp = self
            .toolbox_request(
                reqwest::Method::POST,
                sandbox_id,
                "/process/execute",
                Some(body),
            )
            .await?;
        serde_json::from_value(resp).map_err(|e| format!("Failed to parse exec result: {e}"))
    }

    // --- Toolbox API: Files ---

    pub async fn file_download(&self, sandbox_id: &str, path: &str) -> Result<Vec<u8>, String> {
        let encoded = urlencoding::encode(path);
        self.toolbox_download(sandbox_id, &format!("/files/download?path={encoded}"))
            .await
    }

    pub async fn file_upload(
        &self,
        sandbox_id: &str,
        path: &str,
        content: &[u8],
    ) -> Result<(), String> {
        let filename = path.rsplit('/').next().unwrap_or("file");
        self.toolbox_upload(sandbox_id, path, content, filename)
            .await
    }

    pub async fn file_list(&self, sandbox_id: &str, path: &str) -> Result<Vec<Value>, String> {
        let encoded = urlencoding::encode(path);
        let resp = self
            .toolbox_request(
                reqwest::Method::GET,
                sandbox_id,
                &format!("/files/?path={encoded}"),
                None,
            )
            .await?;

        // Response is an array of file entries
        match resp.as_array() {
            Some(arr) => Ok(arr.clone()),
            None => Ok(vec![resp]),
        }
    }

    pub async fn file_delete(&self, sandbox_id: &str, path: &str) -> Result<(), String> {
        let encoded = urlencoding::encode(path);
        self.toolbox_request(
            reqwest::Method::DELETE,
            sandbox_id,
            &format!("/files?path={encoded}"),
            None,
        )
        .await?;
        Ok(())
    }

    pub async fn create_folder(
        &self,
        sandbox_id: &str,
        path: &str,
        mode: &str,
    ) -> Result<(), String> {
        let encoded_path = urlencoding::encode(path);
        let encoded_mode = urlencoding::encode(mode);
        self.toolbox_request(
            reqwest::Method::POST,
            sandbox_id,
            &format!("/files/folder?path={encoded_path}&mode={encoded_mode}"),
            None,
        )
        .await?;
        Ok(())
    }
}

// ============================================================================
// URL encoding helper (inline, avoids extra dependency)
// ============================================================================

mod urlencoding {
    /// Percent-encode a string for use in query parameters.
    pub fn encode(input: &str) -> String {
        let mut result = String::with_capacity(input.len() * 2);
        for byte in input.bytes() {
            match byte {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                    result.push(byte as char);
                }
                _ => {
                    result.push('%');
                    result.push_str(&format!("{byte:02X}"));
                }
            }
        }
        result
    }
}

// ============================================================================
// Helper Functions for State Management
// ============================================================================

async fn get_api_key(context: &ToolContext) -> Result<String, ToolExecutionResult> {
    let storage = context
        .storage_store
        .as_ref()
        .ok_or_else(|| ToolExecutionResult::tool_error("Storage not available in this context"))?;

    storage
        .get_secret(context.session_id, DAYTONA_API_KEY_SECRET)
        .await
        .map_err(|e| {
            error!("Failed to read DAYTONA_API_KEY secret: {e}");
            ToolExecutionResult::internal_error_msg(format!("Failed to read API key: {e}"))
        })?
        .ok_or_else(|| {
            ToolExecutionResult::tool_error(
                "DAYTONA_API_KEY not set. Use `secret_store set DAYTONA_API_KEY <your-key>` first. \
                 Get your key at https://app.daytona.io/ under API Keys.",
            )
        })
}

async fn get_sandbox_state(
    context: &ToolContext,
    sandbox_id: &str,
) -> Result<SandboxState, ToolExecutionResult> {
    let storage = context
        .storage_store
        .as_ref()
        .ok_or_else(|| ToolExecutionResult::tool_error("Storage not available in this context"))?;

    let secret_name = format!("{DAYTONA_SANDBOX_SECRET_PREFIX}{sandbox_id}");
    let json_str = storage
        .get_secret(context.session_id, &secret_name)
        .await
        .map_err(|e| {
            error!("Failed to read sandbox state: {e}");
            ToolExecutionResult::internal_error_msg(format!("Failed to read sandbox state: {e}"))
        })?
        .ok_or_else(|| {
            ToolExecutionResult::tool_error(format!(
                "Sandbox '{sandbox_id}' not found. Create one first with daytona_create_sandbox."
            ))
        })?;

    serde_json::from_str(&json_str).map_err(|e| {
        error!("Corrupt sandbox state for {sandbox_id}: {e}");
        ToolExecutionResult::internal_error_msg(format!("Corrupt sandbox state: {e}"))
    })
}

async fn save_sandbox_state(
    context: &ToolContext,
    state: &SandboxState,
) -> Result<(), ToolExecutionResult> {
    let storage = context
        .storage_store
        .as_ref()
        .ok_or_else(|| ToolExecutionResult::tool_error("Storage not available in this context"))?;

    let secret_name = format!("{DAYTONA_SANDBOX_SECRET_PREFIX}{}", state.sandbox_id);
    let json_str = serde_json::to_string(state).map_err(|e| {
        ToolExecutionResult::internal_error_msg(format!("Failed to serialize sandbox state: {e}"))
    })?;

    storage
        .set_secret(context.session_id, &secret_name, &json_str)
        .await
        .map_err(|e| {
            error!("Failed to save sandbox state: {e}");
            ToolExecutionResult::internal_error_msg(format!("Failed to save sandbox state: {e}"))
        })
}

async fn delete_sandbox_state(
    context: &ToolContext,
    sandbox_id: &str,
) -> Result<(), ToolExecutionResult> {
    let storage = context
        .storage_store
        .as_ref()
        .ok_or_else(|| ToolExecutionResult::tool_error("Storage not available in this context"))?;

    let secret_name = format!("{DAYTONA_SANDBOX_SECRET_PREFIX}{sandbox_id}");
    storage
        .delete_secret(context.session_id, &secret_name)
        .await
        .map_err(|e| {
            error!("Failed to delete sandbox state: {e}");
            ToolExecutionResult::internal_error_msg(format!("Failed to delete sandbox state: {e}"))
        })?;
    Ok(())
}

async fn list_sandbox_states(
    context: &ToolContext,
) -> Result<Vec<SandboxState>, ToolExecutionResult> {
    let storage = context
        .storage_store
        .as_ref()
        .ok_or_else(|| ToolExecutionResult::tool_error("Storage not available in this context"))?;

    let secrets = storage
        .list_secrets(context.session_id)
        .await
        .map_err(|e| {
            error!("Failed to list secrets: {e}");
            ToolExecutionResult::internal_error_msg(format!("Failed to list secrets: {e}"))
        })?;

    let mut states = Vec::new();
    for secret_info in secrets {
        if let Some(sandbox_id) = secret_info.name.strip_prefix(DAYTONA_SANDBOX_SECRET_PREFIX) {
            match get_sandbox_state(context, sandbox_id).await {
                Ok(state) => states.push(state),
                Err(_) => {
                    warn!("Skipping corrupt sandbox state: {}", sandbox_id);
                }
            }
        }
    }

    Ok(states)
}

/// Extract a required string parameter from tool arguments.
fn required_str<'a>(args: &'a Value, name: &str) -> Result<&'a str, ToolExecutionResult> {
    args.get(name).and_then(|v| v.as_str()).ok_or_else(|| {
        ToolExecutionResult::tool_error(format!("Missing required parameter: {name}"))
    })
}

// ============================================================================
// DaytonaCapability
// ============================================================================

pub struct DaytonaCapability;

impl Capability for DaytonaCapability {
    fn id(&self) -> &str {
        "daytona"
    }

    fn name(&self) -> &str {
        "[Experimental] Daytona"
    }

    fn description(&self) -> &str {
        "Run code in cloud-based sandboxes powered by Daytona. \
         Create multiple isolated Linux environments per session, execute commands, \
         manage files, and download results. EXPERIMENTAL: This capability may change."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn icon(&self) -> Option<&str> {
        Some("cloud")
    }

    fn category(&self) -> Option<&str> {
        Some("Execution")
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some(
            r#"## Daytona (Experimental)

Cloud-based sandboxes via Daytona. Each sandbox is an isolated environment with full Linux and network access.

Prerequisite: DAYTONA_API_KEY must be set in session secrets before using any sandbox tool.

Tools:
- `daytona_create_sandbox` - Create and start a new sandbox
- `daytona_exec` - Run a shell command (synchronous, returns output)
- `daytona_read_file` / `daytona_write_file` - Read/write files in sandbox
- `daytona_download_workspace` - Download sandbox workspace to session storage
- `daytona_list_sandboxes` - List session sandboxes
- `daytona_manage_sandbox` - Stop or delete a sandbox

All tools except `daytona_create_sandbox` and `daytona_list_sandboxes` require a `sandbox_id`.
Sandboxes auto-stop after 5 minutes of inactivity.
Always DELETE sandboxes when done (stop leaves them on the dashboard)."#,
        )
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![
            Box::new(DaytonaCreateSandboxTool),
            Box::new(DaytonaExecTool),
            Box::new(DaytonaReadFileTool),
            Box::new(DaytonaWriteFileTool),
            Box::new(DaytonaDownloadWorkspaceTool),
            Box::new(DaytonaListSandboxesTool),
            Box::new(DaytonaManageSandboxTool),
        ]
    }

    fn dependencies(&self) -> Vec<&'static str> {
        vec!["session_storage"]
    }
}

// ============================================================================
// DaytonaCreateSandboxTool
// ============================================================================

pub struct DaytonaCreateSandboxTool;

#[async_trait]
impl Tool for DaytonaCreateSandboxTool {
    fn name(&self) -> &str {
        "daytona_create_sandbox"
    }

    fn description(&self) -> &str {
        "Create a new Daytona cloud sandbox. Optionally upload files from session storage."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "title": {
                    "type": "string",
                    "description": "Sandbox name (optional)"
                },
                "image": {
                    "type": "string",
                    "description": "Container image (optional, uses Daytona default if omitted)"
                },
                "upload_files": {
                    "type": "array",
                    "description": "Files to upload from session storage (optional)",
                    "items": {
                        "type": "object",
                        "properties": {
                            "session_path": { "type": "string", "description": "Source path in session storage" },
                            "sandbox_path": { "type": "string", "description": "Destination path in sandbox" }
                        },
                        "required": ["session_path", "sandbox_path"]
                    }
                }
            },
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "daytona_create_sandbox requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let api_key = match get_api_key(context).await {
            Ok(k) => k,
            Err(e) => return e,
        };

        let client = DaytonaClient::new(api_key);

        // Build create request
        let title = arguments
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("Everruns Sandbox");
        let mut create_body = json!({
            "name": title,
            "autoStopInterval": AUTO_STOP_INTERVAL_MINUTES,
        });
        if let Some(image) = arguments.get("image").and_then(|v| v.as_str()) {
            create_body["image"] = json!(image);
        }

        // Create sandbox
        debug!("Creating Daytona sandbox: {title}");
        let sandbox_info = match client.create_sandbox(create_body).await {
            Ok(info) => info,
            Err(e) => return ToolExecutionResult::tool_error(e),
        };

        let sandbox_id = &sandbox_info.id;

        // Wait for sandbox to reach "started" state
        debug!("Waiting for sandbox to start: {sandbox_id}");
        if let Err(e) = client.wait_for_ready(sandbox_id).await {
            warn!("Sandbox readiness check failed: {e}");
            // Continue anyway — the sandbox was created, agent can retry later
        }

        let workspace_path = "/home/daytona".to_string();

        // Save state
        let state = SandboxState {
            sandbox_id: sandbox_id.clone(),
            workspace_path: workspace_path.clone(),
            started_at: chrono::Utc::now().to_rfc3339(),
        };
        if let Err(e) = save_sandbox_state(context, &state).await {
            return e;
        }

        // Optionally upload files
        let mut uploaded_count = 0;
        if let Some(files) = arguments.get("upload_files").and_then(|v| v.as_array()) {
            let file_store = match context.file_store.as_ref() {
                Some(fs) => fs,
                None => {
                    return ToolExecutionResult::tool_error(
                        "File store not available for file upload",
                    );
                }
            };

            for file_spec in files {
                let session_path = match file_spec.get("session_path").and_then(|v| v.as_str()) {
                    Some(p) => p,
                    None => continue,
                };
                let sandbox_path = match file_spec.get("sandbox_path").and_then(|v| v.as_str()) {
                    Some(p) => p,
                    None => continue,
                };

                // Read from session storage
                match file_store.read_file(context.session_id, session_path).await {
                    Ok(Some(file)) => {
                        let content = file.content.unwrap_or_default();
                        if let Err(e) = client
                            .file_upload(sandbox_id, sandbox_path, content.as_bytes())
                            .await
                        {
                            warn!("Failed to upload {session_path} to sandbox: {e}");
                        } else {
                            uploaded_count += 1;
                        }
                    }
                    Ok(None) => {
                        warn!("File not found in session storage: {session_path}");
                    }
                    Err(e) => {
                        warn!("Failed to read {session_path}: {e}");
                    }
                }
            }
        }

        ToolExecutionResult::success(json!({
            "sandbox_id": sandbox_id,
            "status": "running",
            "workspace_path": workspace_path,
            "files_uploaded": uploaded_count
        }))
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// DaytonaExecTool
// ============================================================================

pub struct DaytonaExecTool;

#[async_trait]
impl Tool for DaytonaExecTool {
    fn name(&self) -> &str {
        "daytona_exec"
    }

    fn description(&self) -> &str {
        "Execute a shell command in a Daytona sandbox. Returns output synchronously."
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
                "cwd": {
                    "type": "string",
                    "description": "Working directory (optional, defaults to sandbox workspace)"
                },
                "timeout": {
                    "type": "integer",
                    "description": "Timeout in milliseconds (optional, default: 120000)"
                }
            },
            "required": ["sandbox_id", "command"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "daytona_exec requires context. This tool must be executed with session context.",
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
        let cwd = arguments.get("cwd").and_then(|v| v.as_str());
        let timeout = arguments
            .get("timeout")
            .and_then(|v| v.as_u64())
            .unwrap_or(EXEC_TIMEOUT_MS);

        let api_key = match get_api_key(context).await {
            Ok(k) => k,
            Err(e) => return e,
        };
        // Verify sandbox exists in state
        if let Err(e) = get_sandbox_state(context, sandbox_id).await {
            return e;
        }

        let client = DaytonaClient::new(api_key);

        debug!("Executing in sandbox {sandbox_id}: {command}");
        match client.exec(sandbox_id, command, cwd, Some(timeout)).await {
            Ok(result) => ToolExecutionResult::success(json!({
                "exit_code": result.exit_code,
                "output": result.result
            })),
            Err(e) => ToolExecutionResult::tool_error(e),
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// DaytonaReadFileTool
// ============================================================================

pub struct DaytonaReadFileTool;

#[async_trait]
impl Tool for DaytonaReadFileTool {
    fn name(&self) -> &str {
        "daytona_read_file"
    }

    fn description(&self) -> &str {
        "Read a file from a Daytona sandbox filesystem."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "sandbox_id": {
                    "type": "string",
                    "description": "Sandbox ID"
                },
                "path": {
                    "type": "string",
                    "description": "Path to file in sandbox (e.g., '/home/daytona/main.py')"
                }
            },
            "required": ["sandbox_id", "path"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "daytona_read_file requires context. This tool must be executed with session context.",
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
        let path = match required_str(&arguments, "path") {
            Ok(s) => s,
            Err(e) => return e,
        };

        let api_key = match get_api_key(context).await {
            Ok(k) => k,
            Err(e) => return e,
        };
        if let Err(e) = get_sandbox_state(context, sandbox_id).await {
            return e;
        }

        let client = DaytonaClient::new(api_key);

        match client.file_download(sandbox_id, path).await {
            Ok(bytes) => {
                let content = String::from_utf8_lossy(&bytes).to_string();
                ToolExecutionResult::success(json!({
                    "path": path,
                    "content": content
                }))
            }
            Err(e) => ToolExecutionResult::tool_error(e),
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// DaytonaWriteFileTool
// ============================================================================

pub struct DaytonaWriteFileTool;

#[async_trait]
impl Tool for DaytonaWriteFileTool {
    fn name(&self) -> &str {
        "daytona_write_file"
    }

    fn description(&self) -> &str {
        "Write content to a file in a Daytona sandbox filesystem."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "sandbox_id": {
                    "type": "string",
                    "description": "Sandbox ID"
                },
                "path": {
                    "type": "string",
                    "description": "Path for file in sandbox (e.g., '/home/daytona/main.py')"
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
            "daytona_write_file requires context. This tool must be executed with session context.",
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
        let path = match required_str(&arguments, "path") {
            Ok(s) => s,
            Err(e) => return e,
        };
        let content = match required_str(&arguments, "content") {
            Ok(s) => s,
            Err(e) => return e,
        };

        let api_key = match get_api_key(context).await {
            Ok(k) => k,
            Err(e) => return e,
        };
        if let Err(e) = get_sandbox_state(context, sandbox_id).await {
            return e;
        }

        let client = DaytonaClient::new(api_key);

        match client
            .file_upload(sandbox_id, path, content.as_bytes())
            .await
        {
            Ok(()) => ToolExecutionResult::success(json!({
                "path": path,
                "success": true
            })),
            Err(e) => ToolExecutionResult::tool_error(e),
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// DaytonaDownloadWorkspaceTool
// ============================================================================

pub struct DaytonaDownloadWorkspaceTool;

#[async_trait]
impl Tool for DaytonaDownloadWorkspaceTool {
    fn name(&self) -> &str {
        "daytona_download_workspace"
    }

    fn description(&self) -> &str {
        "Download the entire sandbox workspace to session file storage."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "sandbox_id": {
                    "type": "string",
                    "description": "Sandbox ID"
                },
                "sandbox_path": {
                    "type": "string",
                    "description": "Root path in sandbox to download (default: workspace path)"
                },
                "session_path": {
                    "type": "string",
                    "description": "Destination path in session storage (default: /workspace)"
                }
            },
            "required": ["sandbox_id"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "daytona_download_workspace requires context. This tool must be executed with session context.",
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

        let api_key = match get_api_key(context).await {
            Ok(k) => k,
            Err(e) => return e,
        };
        let state = match get_sandbox_state(context, sandbox_id).await {
            Ok(s) => s,
            Err(e) => return e,
        };

        let file_store = match context.file_store.as_ref() {
            Some(fs) => fs,
            None => {
                return ToolExecutionResult::tool_error(
                    "File store not available for workspace download",
                );
            }
        };

        let sandbox_root = arguments
            .get("sandbox_path")
            .and_then(|v| v.as_str())
            .unwrap_or(&state.workspace_path);
        let session_root = arguments
            .get("session_path")
            .and_then(|v| v.as_str())
            .unwrap_or("/workspace");

        let client = DaytonaClient::new(api_key);

        // Recursively list and download files
        let mut downloaded = 0u64;
        let mut skipped = 0u64;
        let mut errors = Vec::new();

        let mut dirs_to_visit = vec![sandbox_root.to_string()];

        while let Some(dir_path) = dirs_to_visit.pop() {
            let entries = match client.file_list(sandbox_id, &dir_path).await {
                Ok(e) => e,
                Err(e) => {
                    errors.push(format!("Failed to list {dir_path}: {e}"));
                    continue;
                }
            };

            for entry in entries {
                let name = entry
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                let is_dir = entry
                    .get("isDir")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);

                let full_path = if dir_path.ends_with('/') {
                    format!("{dir_path}{name}")
                } else {
                    format!("{dir_path}/{name}")
                };

                if is_dir {
                    dirs_to_visit.push(full_path);
                } else {
                    // Download file
                    match client.file_download(sandbox_id, &full_path).await {
                        Ok(bytes) => {
                            let content = String::from_utf8_lossy(&bytes).to_string();
                            let relative =
                                full_path.strip_prefix(sandbox_root).unwrap_or(&full_path);
                            let session_dest = format!(
                                "{}{}",
                                session_root.trim_end_matches('/'),
                                if relative.starts_with('/') {
                                    relative.to_string()
                                } else {
                                    format!("/{relative}")
                                }
                            );

                            match file_store
                                .write_file(context.session_id, &session_dest, &content, "utf-8")
                                .await
                            {
                                Ok(_) => downloaded += 1,
                                Err(e) => {
                                    errors.push(format!("Failed to write {session_dest}: {e}"));
                                    skipped += 1;
                                }
                            }
                        }
                        Err(e) => {
                            debug!("Skipping {full_path}: {e}");
                            skipped += 1;
                        }
                    }
                }
            }
        }

        let mut result = json!({
            "files_downloaded": downloaded,
            "files_skipped": skipped
        });
        if !errors.is_empty() {
            result["errors"] = json!(errors);
        }

        ToolExecutionResult::success(result)
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// DaytonaListSandboxesTool
// ============================================================================

pub struct DaytonaListSandboxesTool;

#[async_trait]
impl Tool for DaytonaListSandboxesTool {
    fn name(&self) -> &str {
        "daytona_list_sandboxes"
    }

    fn description(&self) -> &str {
        "List all Daytona sandboxes created in this session."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "daytona_list_sandboxes requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        _arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let states = match list_sandbox_states(context).await {
            Ok(s) => s,
            Err(e) => return e,
        };

        let sandboxes: Vec<Value> = states
            .iter()
            .map(|s| {
                json!({
                    "sandbox_id": s.sandbox_id,
                    "started_at": s.started_at,
                    "workspace_path": s.workspace_path
                })
            })
            .collect();

        ToolExecutionResult::success(json!({
            "sandboxes": sandboxes,
            "count": sandboxes.len()
        }))
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// DaytonaManageSandboxTool
// ============================================================================

pub struct DaytonaManageSandboxTool;

#[async_trait]
impl Tool for DaytonaManageSandboxTool {
    fn name(&self) -> &str {
        "daytona_manage_sandbox"
    }

    fn description(&self) -> &str {
        "Manage sandbox lifecycle: stop or delete a Daytona sandbox."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "sandbox_id": {
                    "type": "string",
                    "description": "Sandbox ID"
                },
                "action": {
                    "type": "string",
                    "enum": ["stop", "delete"],
                    "description": "Action to perform: stop (halt sandbox), delete (permanently remove)"
                }
            },
            "required": ["sandbox_id", "action"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "daytona_manage_sandbox requires context. This tool must be executed with session context.",
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
        let action = match required_str(&arguments, "action") {
            Ok(s) => s,
            Err(e) => return e,
        };

        let api_key = match get_api_key(context).await {
            Ok(k) => k,
            Err(e) => return e,
        };
        // Verify sandbox exists in state
        if let Err(e) = get_sandbox_state(context, sandbox_id).await {
            return e;
        }

        let client = DaytonaClient::new(api_key);

        let result = match action {
            "delete" => {
                let r = client.delete_sandbox(sandbox_id).await;
                if r.is_ok() {
                    let _ = delete_sandbox_state(context, sandbox_id).await;
                }
                r
            }
            "stop" => client.stop_sandbox(sandbox_id).await,
            _ => {
                return ToolExecutionResult::tool_error(format!(
                    "Invalid action: '{action}'. Must be one of: stop, delete"
                ));
            }
        };

        match result {
            Ok(()) => ToolExecutionResult::success(json!({
                "sandbox_id": sandbox_id,
                "action": action,
                "success": true
            })),
            Err(e) => ToolExecutionResult::tool_error(e),
        }
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

    // --- Capability metadata tests ---

    #[test]
    fn test_capability_metadata() {
        let cap = DaytonaCapability;
        assert_eq!(cap.id(), "daytona");
        assert_eq!(cap.name(), "[Experimental] Daytona");
        assert_eq!(cap.status(), CapabilityStatus::Available);
        assert_eq!(cap.icon(), Some("cloud"));
        assert_eq!(cap.category(), Some("Execution"));
    }

    #[test]
    fn test_capability_has_all_tools() {
        let cap = DaytonaCapability;
        let tools = cap.tools();
        assert_eq!(tools.len(), 7);

        let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
        assert!(names.contains(&"daytona_create_sandbox"));
        assert!(names.contains(&"daytona_exec"));
        assert!(names.contains(&"daytona_read_file"));
        assert!(names.contains(&"daytona_write_file"));
        assert!(names.contains(&"daytona_download_workspace"));
        assert!(names.contains(&"daytona_list_sandboxes"));
        assert!(names.contains(&"daytona_manage_sandbox"));
    }

    #[test]
    fn test_capability_has_system_prompt() {
        let cap = DaytonaCapability;
        let prompt = cap.system_prompt_addition().unwrap();
        assert!(prompt.contains("daytona_create_sandbox"));
        assert!(prompt.contains("DAYTONA_API_KEY"));
        assert!(prompt.contains("Experimental"));
        assert!(prompt.contains("daytona_exec"));
        assert!(prompt.contains("daytona_download_workspace"));
        assert!(prompt.contains("Prerequisite"));
    }

    #[test]
    fn test_all_tools_require_context() {
        let cap = DaytonaCapability;
        for tool in cap.tools() {
            assert!(
                tool.requires_context(),
                "Tool {} should require context",
                tool.name()
            );
        }
    }

    #[test]
    fn test_capability_dependencies() {
        let cap = DaytonaCapability;
        assert_eq!(cap.dependencies(), vec!["session_storage"]);
    }

    // --- State serialization tests ---

    #[test]
    fn test_sandbox_state_roundtrip() {
        let state = SandboxState {
            sandbox_id: "sb_123".to_string(),
            workspace_path: "/home/daytona".to_string(),
            started_at: "2026-02-16T10:00:00Z".to_string(),
        };
        let json = serde_json::to_string(&state).unwrap();
        let deserialized: SandboxState = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.sandbox_id, "sb_123");
        assert_eq!(deserialized.workspace_path, "/home/daytona");
    }

    #[test]
    fn test_sandbox_state_with_special_chars() {
        let state = SandboxState {
            sandbox_id: "sb-test_123".to_string(),
            workspace_path: "/home/daytona/my project".to_string(),
            started_at: "2026-02-16T10:00:00+05:30".to_string(),
        };
        let json = serde_json::to_string(&state).unwrap();
        let deserialized: SandboxState = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.workspace_path, "/home/daytona/my project");
    }

    // --- URL encoding tests ---

    #[test]
    fn test_encode_simple() {
        assert_eq!(urlencoding::encode("hello"), "hello");
    }

    #[test]
    fn test_encode_path_with_slashes() {
        assert_eq!(
            urlencoding::encode("/home/daytona/main.py"),
            "%2Fhome%2Fdaytona%2Fmain.py"
        );
    }

    #[test]
    fn test_encode_path_with_spaces() {
        let encoded = urlencoding::encode("/my project/file name.txt");
        assert!(encoded.contains("%20"));
    }

    // --- Error path tests ---

    #[tokio::test]
    async fn test_daytona_exec_without_context() {
        let tool = DaytonaExecTool;
        let result = tool
            .execute(json!({"sandbox_id": "test", "command": "ls"}))
            .await;
        match result {
            ToolExecutionResult::ToolError(msg) => {
                assert!(msg.contains("requires context"));
            }
            _ => panic!("Expected tool error"),
        }
    }

    #[tokio::test]
    async fn test_daytona_create_sandbox_without_context() {
        let tool = DaytonaCreateSandboxTool;
        let result = tool.execute(json!({})).await;
        match result {
            ToolExecutionResult::ToolError(msg) => {
                assert!(msg.contains("requires context"));
            }
            _ => panic!("Expected tool error"),
        }
    }

    #[tokio::test]
    async fn test_daytona_read_file_without_context() {
        let tool = DaytonaReadFileTool;
        let result = tool
            .execute(json!({"sandbox_id": "test", "path": "/test.txt"}))
            .await;
        match result {
            ToolExecutionResult::ToolError(msg) => {
                assert!(msg.contains("requires context"));
            }
            _ => panic!("Expected tool error"),
        }
    }

    #[tokio::test]
    async fn test_daytona_write_file_without_context() {
        let tool = DaytonaWriteFileTool;
        let result = tool
            .execute(json!({"sandbox_id": "test", "path": "/test.txt", "content": "hello"}))
            .await;
        match result {
            ToolExecutionResult::ToolError(msg) => {
                assert!(msg.contains("requires context"));
            }
            _ => panic!("Expected tool error"),
        }
    }

    #[tokio::test]
    async fn test_daytona_download_workspace_without_context() {
        let tool = DaytonaDownloadWorkspaceTool;
        let result = tool.execute(json!({"sandbox_id": "test"})).await;
        match result {
            ToolExecutionResult::ToolError(msg) => {
                assert!(msg.contains("requires context"));
            }
            _ => panic!("Expected tool error"),
        }
    }

    #[tokio::test]
    async fn test_daytona_list_sandboxes_without_context() {
        let tool = DaytonaListSandboxesTool;
        let result = tool.execute(json!({})).await;
        match result {
            ToolExecutionResult::ToolError(msg) => {
                assert!(msg.contains("requires context"));
            }
            _ => panic!("Expected tool error"),
        }
    }

    #[tokio::test]
    async fn test_daytona_manage_sandbox_without_context() {
        let tool = DaytonaManageSandboxTool;
        let result = tool
            .execute(json!({"sandbox_id": "test", "action": "stop"}))
            .await;
        match result {
            ToolExecutionResult::ToolError(msg) => {
                assert!(msg.contains("requires context"));
            }
            _ => panic!("Expected tool error"),
        }
    }

    // --- Parameter schema tests ---

    #[test]
    fn test_daytona_exec_schema_has_required_fields() {
        let tool = DaytonaExecTool;
        let schema = tool.parameters_schema();
        let required = schema["required"].as_array().unwrap();
        let required_strs: Vec<&str> = required.iter().map(|v| v.as_str().unwrap()).collect();
        assert!(required_strs.contains(&"sandbox_id"));
        assert!(required_strs.contains(&"command"));
    }

    #[test]
    fn test_daytona_create_sandbox_schema_no_required() {
        let tool = DaytonaCreateSandboxTool;
        let schema = tool.parameters_schema();
        assert!(schema.get("required").is_none());
    }

    #[test]
    fn test_daytona_manage_sandbox_schema_has_enum() {
        let tool = DaytonaManageSandboxTool;
        let schema = tool.parameters_schema();
        let action_enum = &schema["properties"]["action"]["enum"];
        let values: Vec<&str> = action_enum
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert!(values.contains(&"stop"));
        assert!(values.contains(&"delete"));
    }

    // --- wiremock integration tests ---

    mod client_tests {
        use super::*;
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        #[tokio::test]
        async fn test_client_create_sandbox() {
            let mock_server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/sandbox"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "id": "sb_test123",
                    "name": "Test Sandbox",
                    "state": "started"
                })))
                .mount(&mock_server)
                .await;

            let client = DaytonaClient::with_base_urls(
                "test_key".to_string(),
                mock_server.uri(),
                mock_server.uri(),
            );
            let result = client.create_sandbox(json!({"name": "Test"})).await;
            assert!(result.is_ok());
            let info = result.unwrap();
            assert_eq!(info.id, "sb_test123");
            assert_eq!(info.name, Some("Test Sandbox".to_string()));
        }

        #[tokio::test]
        async fn test_client_get_sandbox() {
            let mock_server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path("/sandbox/sb_test"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "id": "sb_test",
                    "name": "Test",
                    "state": "started"
                })))
                .mount(&mock_server)
                .await;

            let client = DaytonaClient::with_base_urls(
                "test_key".to_string(),
                mock_server.uri(),
                mock_server.uri(),
            );
            let result = client.get_sandbox("sb_test").await;
            assert!(result.is_ok());
            let info = result.unwrap();
            assert_eq!(info.state, "started");
        }

        #[tokio::test]
        async fn test_client_stop_sandbox() {
            let mock_server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/sandbox/sb_test/stop"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
                .mount(&mock_server)
                .await;

            let client = DaytonaClient::with_base_urls(
                "test_key".to_string(),
                mock_server.uri(),
                mock_server.uri(),
            );
            let result = client.stop_sandbox("sb_test").await;
            assert!(result.is_ok());
        }

        #[tokio::test]
        async fn test_client_delete_sandbox() {
            let mock_server = MockServer::start().await;
            Mock::given(method("DELETE"))
                .and(path("/sandbox/sb_test"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
                .mount(&mock_server)
                .await;

            let client = DaytonaClient::with_base_urls(
                "test_key".to_string(),
                mock_server.uri(),
                mock_server.uri(),
            );
            let result = client.delete_sandbox("sb_test").await;
            assert!(result.is_ok());
        }

        #[tokio::test]
        async fn test_client_exec() {
            let mock_server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/sb_test/process/execute"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "result": "hello world\n",
                    "exitCode": 0
                })))
                .mount(&mock_server)
                .await;

            let client = DaytonaClient::with_base_urls(
                "test_key".to_string(),
                mock_server.uri(),
                mock_server.uri(),
            );
            let result = client.exec("sb_test", "echo hello world", None, None).await;
            assert!(result.is_ok());
            let exec_result = result.unwrap();
            assert_eq!(exec_result.exit_code, 0);
            assert_eq!(exec_result.result, "hello world\n");
        }

        #[tokio::test]
        async fn test_client_file_list() {
            let mock_server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path("/sb_test/files/"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!([
                    {"name": "main.py", "isDir": false, "size": 42},
                    {"name": "src", "isDir": true, "size": 0}
                ])))
                .mount(&mock_server)
                .await;

            let client = DaytonaClient::with_base_urls(
                "test_key".to_string(),
                mock_server.uri(),
                mock_server.uri(),
            );
            let result = client.file_list("sb_test", "/home/daytona").await;
            assert!(result.is_ok());
            let entries = result.unwrap();
            assert_eq!(entries.len(), 2);
        }

        // --- Error response tests ---

        #[tokio::test]
        async fn test_client_404_error() {
            let mock_server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path("/sandbox/nonexistent"))
                .respond_with(
                    ResponseTemplate::new(404).set_body_string("{\"error\":\"Sandbox not found\"}"),
                )
                .mount(&mock_server)
                .await;

            let client = DaytonaClient::with_base_urls(
                "test_key".to_string(),
                mock_server.uri(),
                mock_server.uri(),
            );
            let result = client.get_sandbox("nonexistent").await;
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert!(err.contains("404"));
        }

        #[tokio::test]
        async fn test_client_401_error() {
            let mock_server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/sandbox"))
                .respond_with(
                    ResponseTemplate::new(401).set_body_string("{\"error\":\"Unauthorized\"}"),
                )
                .mount(&mock_server)
                .await;

            let client = DaytonaClient::with_base_urls(
                "bad_key".to_string(),
                mock_server.uri(),
                mock_server.uri(),
            );
            let result = client.create_sandbox(json!({"name": "Test"})).await;
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert!(err.contains("401"));
        }

        #[tokio::test]
        async fn test_client_500_error() {
            let mock_server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/sandbox/sb_test/start"))
                .respond_with(ResponseTemplate::new(500).set_body_string("Internal Server Error"))
                .mount(&mock_server)
                .await;

            let client = DaytonaClient::with_base_urls(
                "test_key".to_string(),
                mock_server.uri(),
                mock_server.uri(),
            );
            let result = client.start_sandbox("sb_test").await;
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert!(err.contains("500"));
        }

        #[tokio::test]
        async fn test_client_malformed_response() {
            let mock_server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/sandbox"))
                .respond_with(ResponseTemplate::new(200).set_body_string("not valid json"))
                .mount(&mock_server)
                .await;

            let client = DaytonaClient::with_base_urls(
                "test_key".to_string(),
                mock_server.uri(),
                mock_server.uri(),
            );
            let result = client.create_sandbox(json!({"name": "Test"})).await;
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert!(err.contains("Invalid JSON"));
        }
    }
}

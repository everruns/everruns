//! CodeSandbox Capability (Experimental)
//!
//! Cloud-based sandboxed code execution via CodeSandbox REST API.
//! Supports multiple sandboxes per session, each identified by sandbox_id.
//!
//! Decision: Use secrets store for all state (API key + per-sandbox pitcher tokens)
//! Decision: Two-tier API: Management API for lifecycle, Pint API for in-sandbox ops
//! Decision: Sync+async exec modes via `wait` parameter
//! Decision: session_storage dependency for API key and state persistence
//!
//! Tools provided:
//! - `csb_create_sandbox`: Create a new sandbox VM
//! - `csb_exec`: Execute a command in a sandbox
//! - `csb_exec_status`: Check execution status and get output
//! - `csb_read_file`: Read a file from sandbox
//! - `csb_write_file`: Write a file to sandbox
//! - `csb_download_workspace`: Download entire workspace to session storage
//! - `csb_list_sandboxes`: List session sandboxes
//! - `csb_manage_sandbox`: Shutdown/hibernate/delete sandbox

use super::{Capability, CapabilityStatus};
use crate::tools::{Tool, ToolExecutionResult};
use crate::traits::ToolContext;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::time::Duration;
use tracing::{debug, error, warn};

// ============================================================================
// Constants
// ============================================================================

const CSB_API_BASE: &str = "https://api.codesandbox.io";
const CSB_API_KEY_SECRET: &str = "CSB_API_KEY";
const CSB_SANDBOX_SECRET_PREFIX: &str = "csb_sandbox:";
const EXEC_POLL_INTERVAL: Duration = Duration::from_millis(500);
const EXEC_POLL_MAX_WAIT: Duration = Duration::from_secs(120);
const SSE_READ_TIMEOUT: Duration = Duration::from_secs(5);
const PINT_READY_POLL_INTERVAL: Duration = Duration::from_secs(2);
const PINT_READY_MAX_WAIT: Duration = Duration::from_secs(30);
/// Auto-hibernate after 5 minutes of inactivity (safety net)
const HIBERNATE_TIMEOUT_SECS: u64 = 300;

// ============================================================================
// API Response Types
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxInfo {
    pub id: String,
    pub title: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VmStartResponse {
    pub pitcher_url: String,
    pub pitcher_token: String,
    pub workspace_path: Option<String>,
    /// Pint API URL (newer API, preferred when use_pint is true)
    pub pint_url: Option<String>,
    /// Pint API token (newer API, preferred when use_pint is true)
    pub pint_token: Option<String>,
    /// Whether to use pint_url/pint_token instead of pitcher_url/pitcher_token
    pub use_pint: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecInfo {
    pub id: String,
    pub status: String,
    #[serde(rename = "exitCode")]
    pub exit_code: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileContent {
    pub path: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirEntry {
    pub name: String,
    #[serde(rename = "type")]
    pub entry_type: Option<String>,
}

// ============================================================================
// Persisted Sandbox State (stored in session secrets as JSON)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreviewTokenResponse {
    pub token: PreviewTokenInfo,
    pub sandbox_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreviewTokenInfo {
    pub token: String,
    pub token_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxState {
    pub sandbox_id: String,
    /// Pint API base URL: https://{sandbox_id}-57468.csb.app
    pub pint_url: String,
    pub pitcher_token: String,
    pub preview_token: String,
    pub workspace_path: String,
    pub started_at: String,
}

// NOTE: Template alias resolution was removed. The CodeSandbox `template` field
// caused 500 errors from their API for most template IDs (tested 2026-02-15).
// Sandboxes created without a template work fine. See specs/codesandbox.md for details.

// ============================================================================
// CodeSandboxClient - HTTP client for CodeSandbox APIs
// ============================================================================

pub struct CodeSandboxClient {
    http: reqwest::Client,
    api_key: String,
    base_url: String,
}

impl CodeSandboxClient {
    pub fn new(api_key: String) -> Self {
        Self::with_base_url(api_key, CSB_API_BASE.to_string())
    }

    pub fn with_base_url(api_key: String, base_url: String) -> Self {
        Self {
            http: reqwest::Client::new(),
            api_key,
            base_url,
        }
    }

    // --- Generic request helpers ---

    async fn management_request(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value, String> {
        let url = format!("{}{}", self.base_url, path);
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
            .map_err(|e| format!("Failed to connect to CodeSandbox API: {e}"))?;

        let status = resp.status();
        let body_text = resp
            .text()
            .await
            .map_err(|e| format!("Failed to read response: {e}"))?;

        if !status.is_success() {
            return Err(format!("CodeSandbox API error ({status}): {body_text}"));
        }

        serde_json::from_str(&body_text).map_err(|e| format!("Invalid JSON from CodeSandbox: {e}"))
    }

    /// Make a request to the Pint API (in-sandbox HTTP REST API).
    /// pint_url: https://{sandbox_id}-57468.csb.app
    /// preview_token: required for csb.app port proxy auth (query param)
    /// pitcher_token: required for Pint API auth (Bearer header)
    async fn pint_request(
        &self,
        method: reqwest::Method,
        pint_url: &str,
        path: &str,
        pitcher_token: &str,
        preview_token: &str,
        body: Option<Value>,
    ) -> Result<Value, String> {
        let separator = if path.contains('?') { '&' } else { '?' };
        let url = format!(
            "{}{}{separator}preview_token={preview_token}",
            pint_url, path
        );
        let mut req = self
            .http
            .request(method, &url)
            .bearer_auth(pitcher_token)
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

    // --- Management API ---

    pub async fn create_sandbox(&self, body: Value) -> Result<SandboxInfo, String> {
        let resp = self
            .management_request(reqwest::Method::POST, "/sandbox", Some(body))
            .await?;
        // Response wraps data in { data: { id, ... } }
        let data = resp.get("data").cloned().unwrap_or(resp.clone());
        serde_json::from_value(data).map_err(|e| format!("Failed to parse sandbox info: {e}"))
    }

    pub async fn start_vm(
        &self,
        sandbox_id: &str,
        tier: Option<&str>,
    ) -> Result<VmStartResponse, String> {
        let body = tier.map(|t| json!({ "tier": t }));
        let resp = self
            .management_request(
                reqwest::Method::POST,
                &format!("/vm/{sandbox_id}/start"),
                body,
            )
            .await?;
        let data = resp.get("data").cloned().unwrap_or(resp.clone());
        serde_json::from_value(data).map_err(|e| format!("Failed to parse VM start response: {e}"))
    }

    pub async fn create_preview_token(
        &self,
        sandbox_id: &str,
    ) -> Result<PreviewTokenResponse, String> {
        let resp = self
            .management_request(
                reqwest::Method::POST,
                &format!("/sandbox/{sandbox_id}/tokens"),
                None,
            )
            .await?;
        let data = resp.get("data").cloned().unwrap_or(resp.clone());
        serde_json::from_value(data)
            .map_err(|e| format!("Failed to parse preview token response: {e}"))
    }

    /// Wait for the Pint API to become ready after VM start.
    /// The VM may report as "RUNNING" before the internal Pint HTTP service has booted.
    /// Polls the execs endpoint with backoff until it gets a non-502 response.
    pub async fn wait_for_pint_ready(
        &self,
        pint_url: &str,
        pitcher_token: &str,
        preview_token: &str,
    ) -> Result<(), String> {
        let start = std::time::Instant::now();
        let mut interval = PINT_READY_POLL_INTERVAL;

        while start.elapsed() < PINT_READY_MAX_WAIT {
            let url = format!("{pint_url}/api/v1/execs?preview_token={preview_token}");

            match self
                .http
                .get(&url)
                .bearer_auth(pitcher_token)
                .timeout(Duration::from_secs(5))
                .send()
                .await
            {
                Ok(resp) => {
                    let status = resp.status();
                    // 502/503 = Pint service still booting; anything else means it's up
                    if status.as_u16() != 502 && status.as_u16() != 503 {
                        debug!(
                            "Pint API ready (status: {status}) after {:?}",
                            start.elapsed()
                        );
                        return Ok(());
                    }
                    debug!("Pint API not ready yet (status: {status}), retrying...");
                }
                Err(e) => {
                    debug!("Pint API not reachable yet: {e}, retrying...");
                }
            }

            tokio::time::sleep(interval).await;
            // Cap backoff at 4s
            interval = std::cmp::min(interval * 2, Duration::from_secs(4));
        }

        Err(format!(
            "Pint API did not become ready within {}s",
            PINT_READY_MAX_WAIT.as_secs()
        ))
    }

    pub async fn shutdown_vm(&self, sandbox_id: &str) -> Result<(), String> {
        self.management_request(
            reqwest::Method::POST,
            &format!("/vm/{sandbox_id}/shutdown"),
            None,
        )
        .await?;
        Ok(())
    }

    pub async fn hibernate_vm(&self, sandbox_id: &str) -> Result<(), String> {
        self.management_request(
            reqwest::Method::POST,
            &format!("/vm/{sandbox_id}/hibernate"),
            None,
        )
        .await?;
        Ok(())
    }

    pub async fn delete_vm(&self, sandbox_id: &str) -> Result<(), String> {
        self.management_request(reqwest::Method::DELETE, &format!("/vm/{sandbox_id}"), None)
            .await?;
        Ok(())
    }

    // --- Pint API: Exec ---

    pub async fn exec_create(
        &self,
        state: &SandboxState,
        command: &str,
        args: Vec<String>,
    ) -> Result<ExecInfo, String> {
        let resp = self
            .pint_request(
                reqwest::Method::POST,
                &state.pint_url,
                "/api/v1/execs",
                &state.pitcher_token,
                &state.preview_token,
                Some(json!({ "command": command, "args": args })),
            )
            .await?;
        serde_json::from_value(resp).map_err(|e| format!("Failed to parse exec info: {e}"))
    }

    pub async fn exec_get(&self, state: &SandboxState, exec_id: &str) -> Result<ExecInfo, String> {
        let resp = self
            .pint_request(
                reqwest::Method::GET,
                &state.pint_url,
                &format!("/api/v1/execs/{exec_id}"),
                &state.pitcher_token,
                &state.preview_token,
                None,
            )
            .await?;
        serde_json::from_value(resp).map_err(|e| format!("Failed to parse exec info: {e}"))
    }

    pub async fn exec_get_output(
        &self,
        state: &SandboxState,
        exec_id: &str,
    ) -> Result<String, String> {
        let url = format!(
            "{}/api/v1/execs/{exec_id}/io?preview_token={}",
            state.pint_url, state.preview_token
        );
        let resp = self
            .http
            .get(&url)
            .bearer_auth(&state.pitcher_token)
            .timeout(SSE_READ_TIMEOUT)
            .send()
            .await
            .map_err(|e| format!("Failed to connect for exec output: {e}"))?;

        if !resp.status().is_success() {
            return Err(format!("Exec output error ({})", resp.status()));
        }

        let body = resp
            .text()
            .await
            .map_err(|e| format!("Failed to read exec output: {e}"))?;

        // The /io endpoint returns plain text output, not SSE.
        // Trim trailing whitespace but preserve internal newlines.
        Ok(body.trim_end().to_string())
    }

    pub async fn exec_kill(&self, state: &SandboxState, exec_id: &str) -> Result<(), String> {
        self.pint_request(
            reqwest::Method::DELETE,
            &state.pint_url,
            &format!("/api/v1/execs/{exec_id}"),
            &state.pitcher_token,
            &state.preview_token,
            None,
        )
        .await?;
        Ok(())
    }

    // --- Pint API: Files ---

    pub async fn file_read(&self, state: &SandboxState, path: &str) -> Result<FileContent, String> {
        let encoded_path = encode_path(path);
        let resp = self
            .pint_request(
                reqwest::Method::GET,
                &state.pint_url,
                &format!("/api/v1/files/{encoded_path}"),
                &state.pitcher_token,
                &state.preview_token,
                None,
            )
            .await?;
        serde_json::from_value(resp).map_err(|e| format!("Failed to parse file content: {e}"))
    }

    pub async fn file_write(
        &self,
        state: &SandboxState,
        path: &str,
        content: &str,
    ) -> Result<(), String> {
        let encoded_path = encode_path(path);
        self.pint_request(
            reqwest::Method::POST,
            &state.pint_url,
            &format!("/api/v1/files/{encoded_path}"),
            &state.pitcher_token,
            &state.preview_token,
            Some(json!({ "content": content })),
        )
        .await?;
        Ok(())
    }

    // --- Pint API: Directories ---

    pub async fn dir_list(
        &self,
        state: &SandboxState,
        path: &str,
    ) -> Result<Vec<DirEntry>, String> {
        let encoded_path = encode_path(path);
        let resp = self
            .pint_request(
                reqwest::Method::GET,
                &state.pint_url,
                &format!("/api/v1/directories/{encoded_path}"),
                &state.pitcher_token,
                &state.preview_token,
                None,
            )
            .await?;
        // Response may be an array directly or wrapped
        if let Some(arr) = resp.as_array() {
            serde_json::from_value(Value::Array(arr.clone()))
                .map_err(|e| format!("Failed to parse directory listing: {e}"))
        } else {
            serde_json::from_value(resp)
                .map_err(|e| format!("Failed to parse directory listing: {e}"))
        }
    }
}

// ============================================================================
// Utility Functions
// ============================================================================

/// URL-encode a file path for use in Pint API URLs, preserving slash separators.
fn encode_path(path: &str) -> String {
    // Strip leading slash and pass through — CodeSandbox Pint API accepts
    // unencoded paths in practice. Only encode spaces as %20.
    path.trim_start_matches('/').replace(' ', "%20")
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
        .get_secret(context.session_id, CSB_API_KEY_SECRET)
        .await
        .map_err(|e| {
            error!("Failed to read CSB_API_KEY secret: {e}");
            ToolExecutionResult::internal_error_msg(format!("Failed to read API key: {e}"))
        })?
        .ok_or_else(|| {
            ToolExecutionResult::tool_error(
                "CSB_API_KEY not set. Use `secret_store set CSB_API_KEY <your-key>` first. \
                 Get your key at https://codesandbox.io/t/api",
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

    let secret_name = format!("{CSB_SANDBOX_SECRET_PREFIX}{sandbox_id}");
    let json_str = storage
        .get_secret(context.session_id, &secret_name)
        .await
        .map_err(|e| {
            error!("Failed to read sandbox state: {e}");
            ToolExecutionResult::internal_error_msg(format!("Failed to read sandbox state: {e}"))
        })?
        .ok_or_else(|| {
            ToolExecutionResult::tool_error(format!(
                "Sandbox '{sandbox_id}' not found. Create one first with csb_create_sandbox."
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

    let secret_name = format!("{CSB_SANDBOX_SECRET_PREFIX}{}", state.sandbox_id);
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

    let secret_name = format!("{CSB_SANDBOX_SECRET_PREFIX}{sandbox_id}");
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
        if let Some(sandbox_id) = secret_info.name.strip_prefix(CSB_SANDBOX_SECRET_PREFIX) {
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
// CodeSandboxCapability
// ============================================================================

pub struct CodeSandboxCapability;

impl Capability for CodeSandboxCapability {
    fn id(&self) -> &str {
        "codesandbox"
    }

    fn name(&self) -> &str {
        "[Experimental] CodeSandbox"
    }

    fn description(&self) -> &str {
        "Run code in cloud-based sandbox VMs powered by CodeSandbox. \
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
            r#"## CodeSandbox (Experimental)

Cloud-based sandbox VMs via CodeSandbox. Each sandbox is an isolated Firecracker microVM with full Linux and network access.

Prerequisite: CSB_API_KEY must be set in session secrets before using any sandbox tool.

Tools:
- `csb_create_sandbox` - Create and start a new sandbox VM
- `csb_exec` - Run a shell command (`wait: true` for output, `wait: false` for async)
- `csb_exec_status` - Poll async execution status/output
- `csb_read_file` / `csb_write_file` - Read/write files in sandbox
- `csb_download_workspace` - Download sandbox workspace to session storage
- `csb_list_sandboxes` - List session sandboxes
- `csb_manage_sandbox` - Shutdown, hibernate, or delete a sandbox

All tools except `csb_create_sandbox` and `csb_list_sandboxes` require a `sandbox_id`.
Sandboxes auto-hibernate after 5 minutes of inactivity.
Always DELETE sandboxes when done (shutdown/hibernate leave them on the dashboard)."#,
        )
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![
            Box::new(CsbCreateSandboxTool),
            Box::new(CsbExecTool),
            Box::new(CsbExecStatusTool),
            Box::new(CsbReadFileTool),
            Box::new(CsbWriteFileTool),
            Box::new(CsbDownloadWorkspaceTool),
            Box::new(CsbListSandboxesTool),
            Box::new(CsbManageSandboxTool),
        ]
    }

    fn dependencies(&self) -> Vec<&'static str> {
        vec!["session_storage"]
    }
}

// ============================================================================
// CsbCreateSandboxTool
// ============================================================================

pub struct CsbCreateSandboxTool;

#[async_trait]
impl Tool for CsbCreateSandboxTool {
    fn name(&self) -> &str {
        "csb_create_sandbox"
    }

    fn description(&self) -> &str {
        "Create a new CodeSandbox cloud VM. Optionally upload files from session storage."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "title": {
                    "type": "string",
                    "description": "Sandbox title (optional)"
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
            "csb_create_sandbox requires context. This tool must be executed with session context.",
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

        let client = CodeSandboxClient::new(api_key);

        // Build create request
        let title = arguments
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("Everruns Sandbox");
        let create_body = json!({
            "title": title,
            "privacy": 2,
            "runtime": "vm",
            "settings": { "use_pint": true },
            "hibernationTimeoutSeconds": HIBERNATE_TIMEOUT_SECS,
        });

        // Create sandbox
        debug!("Creating CodeSandbox: {title}");
        let sandbox_info = match client.create_sandbox(create_body).await {
            Ok(info) => info,
            Err(e) => return ToolExecutionResult::tool_error(e),
        };

        let sandbox_id = &sandbox_info.id;

        // Start VM
        debug!("Starting VM for sandbox: {sandbox_id}");
        let vm_info = match client.start_vm(sandbox_id, None).await {
            Ok(info) => info,
            Err(e) => return ToolExecutionResult::tool_error(e),
        };

        let workspace_path = vm_info
            .workspace_path
            .unwrap_or_else(|| "/project".to_string());

        // Create preview token (required for Pint API port proxy auth)
        debug!("Creating preview token for sandbox: {sandbox_id}");
        let preview_token_resp = match client.create_preview_token(sandbox_id).await {
            Ok(resp) => resp,
            Err(e) => {
                return ToolExecutionResult::tool_error(format!(
                    "Failed to create preview token: {e}"
                ));
            }
        };
        let preview_token = preview_token_resp.token.token;

        // Pint API is accessed via port forwarding: https://{sandbox_id}-57468.csb.app
        let pint_url = format!("https://{sandbox_id}-57468.csb.app");

        // Wait for Pint API to be ready (VM service may still be booting)
        debug!("Waiting for Pint API to become ready...");
        if let Err(e) = client
            .wait_for_pint_ready(&pint_url, &vm_info.pitcher_token, &preview_token)
            .await
        {
            warn!("Pint API readiness check failed: {e}");
            // Continue anyway — the sandbox was created, agent can retry later
        }

        // Save state
        let state = SandboxState {
            sandbox_id: sandbox_id.clone(),
            pint_url: pint_url.clone(),
            pitcher_token: vm_info.pitcher_token.clone(),
            preview_token: preview_token.clone(),
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
                        // Write to sandbox
                        let content = file.content.unwrap_or_default();
                        if let Err(e) = client.file_write(&state, sandbox_path, &content).await {
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
// CsbExecTool
// ============================================================================

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

// ============================================================================
// CsbExecStatusTool
// ============================================================================

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
        "Read a file from a CodeSandbox VM filesystem."
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
                    "description": "Absolute path to file in sandbox (e.g., '/project/main.py')"
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
        let state = match get_sandbox_state(context, sandbox_id).await {
            Ok(s) => s,
            Err(e) => return e,
        };

        let client = CodeSandboxClient::new(api_key);

        match client.file_read(&state, path).await {
            Ok(content) => ToolExecutionResult::success(json!({
                "path": content.path,
                "content": content.content
            })),
            Err(e) => ToolExecutionResult::tool_error(e),
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
        "Write content to a file in a CodeSandbox VM filesystem."
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
                    "description": "Absolute path for file in sandbox (e.g., '/project/main.py')"
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
        let state = match get_sandbox_state(context, sandbox_id).await {
            Ok(s) => s,
            Err(e) => return e,
        };

        let client = CodeSandboxClient::new(api_key);

        match client.file_write(&state, path, content).await {
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
// CsbDownloadWorkspaceTool
// ============================================================================

pub struct CsbDownloadWorkspaceTool;

#[async_trait]
impl Tool for CsbDownloadWorkspaceTool {
    fn name(&self) -> &str {
        "csb_download_workspace"
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
            "csb_download_workspace requires context. This tool must be executed with session context.",
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

        let client = CodeSandboxClient::new(api_key);

        // Recursively list and download files
        let mut downloaded = 0u64;
        let mut skipped = 0u64;
        let mut errors = Vec::new();

        let mut dirs_to_visit = vec![sandbox_root.to_string()];

        while let Some(dir_path) = dirs_to_visit.pop() {
            let entries = match client.dir_list(&state, &dir_path).await {
                Ok(e) => e,
                Err(e) => {
                    errors.push(format!("Failed to list {dir_path}: {e}"));
                    continue;
                }
            };

            for entry in entries {
                let full_path = if dir_path.ends_with('/') {
                    format!("{}{}", dir_path, entry.name)
                } else {
                    format!("{}/{}", dir_path, entry.name)
                };

                let is_dir = entry
                    .entry_type
                    .as_deref()
                    .map(|t| t == "directory" || t == "dir")
                    .unwrap_or(false);

                if is_dir {
                    dirs_to_visit.push(full_path);
                } else {
                    // Download file
                    match client.file_read(&state, &full_path).await {
                        Ok(content) => {
                            // Compute session destination path
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
                                .write_file(
                                    context.session_id,
                                    &session_dest,
                                    &content.content,
                                    "utf-8",
                                )
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
// CsbListSandboxesTool
// ============================================================================

pub struct CsbListSandboxesTool;

#[async_trait]
impl Tool for CsbListSandboxesTool {
    fn name(&self) -> &str {
        "csb_list_sandboxes"
    }

    fn description(&self) -> &str {
        "List all CodeSandbox VMs created in this session."
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
            "csb_list_sandboxes requires context. This tool must be executed with session context.",
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
// CsbManageSandboxTool
// ============================================================================

pub struct CsbManageSandboxTool;

#[async_trait]
impl Tool for CsbManageSandboxTool {
    fn name(&self) -> &str {
        "csb_manage_sandbox"
    }

    fn description(&self) -> &str {
        "Manage sandbox lifecycle: shutdown, hibernate, or delete a CodeSandbox VM."
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
                    "enum": ["shutdown", "hibernate", "delete"],
                    "description": "Action to perform: shutdown (stop and delete VM), hibernate (save state, keeps on dashboard), delete (same as shutdown)"
                }
            },
            "required": ["sandbox_id", "action"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "csb_manage_sandbox requires context. This tool must be executed with session context.",
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

        let client = CodeSandboxClient::new(api_key);

        let result = match action {
            "shutdown" | "delete" => {
                // Both shutdown and delete fully remove the sandbox.
                // Plain shutdown leaves sandboxes lingering on the dashboard,
                // so we always delete to clean up.
                let r = client.delete_vm(sandbox_id).await;
                if r.is_ok() {
                    let _ = delete_sandbox_state(context, sandbox_id).await;
                }
                r
            }
            "hibernate" => client.hibernate_vm(sandbox_id).await,
            _ => {
                return ToolExecutionResult::tool_error(format!(
                    "Invalid action: '{action}'. Must be one of: shutdown, hibernate, delete"
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
        let cap = CodeSandboxCapability;
        assert_eq!(cap.id(), "codesandbox");
        assert_eq!(cap.name(), "[Experimental] CodeSandbox");
        assert_eq!(cap.status(), CapabilityStatus::Available);
        assert_eq!(cap.icon(), Some("cloud"));
        assert_eq!(cap.category(), Some("Execution"));
    }

    #[test]
    fn test_capability_has_all_tools() {
        let cap = CodeSandboxCapability;
        let tools = cap.tools();
        assert_eq!(tools.len(), 8);

        let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
        assert!(names.contains(&"csb_create_sandbox"));
        assert!(names.contains(&"csb_exec"));
        assert!(names.contains(&"csb_exec_status"));
        assert!(names.contains(&"csb_read_file"));
        assert!(names.contains(&"csb_write_file"));
        assert!(names.contains(&"csb_download_workspace"));
        assert!(names.contains(&"csb_list_sandboxes"));
        assert!(names.contains(&"csb_manage_sandbox"));
    }

    #[test]
    fn test_capability_has_system_prompt() {
        let cap = CodeSandboxCapability;
        let prompt = cap.system_prompt_addition().unwrap();
        assert!(prompt.contains("csb_create_sandbox"));
        assert!(prompt.contains("CSB_API_KEY"));
        assert!(prompt.contains("Experimental"));
        assert!(prompt.contains("csb_exec"));
        assert!(prompt.contains("csb_download_workspace"));
        assert!(prompt.contains("Prerequisite"));
        // Should NOT duplicate workflow steps (that's the agent's job)
        assert!(!prompt.contains("Set API key (once per session)"));
    }

    #[test]
    fn test_all_tools_require_context() {
        let cap = CodeSandboxCapability;
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
        let cap = CodeSandboxCapability;
        assert_eq!(cap.dependencies(), vec!["session_storage"]);
    }

    // --- State serialization tests ---

    #[test]
    fn test_sandbox_state_roundtrip() {
        let state = SandboxState {
            sandbox_id: "sb_123".to_string(),
            pint_url: "https://sb_123-57468.csb.app".to_string(),
            pitcher_token: "tok_abc".to_string(),
            preview_token: "prv_v1_test123".to_string(),
            workspace_path: "/project".to_string(),
            started_at: "2026-02-13T10:00:00Z".to_string(),
        };
        let json = serde_json::to_string(&state).unwrap();
        let deserialized: SandboxState = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.sandbox_id, "sb_123");
        assert_eq!(deserialized.pint_url, "https://sb_123-57468.csb.app");
        assert_eq!(deserialized.pitcher_token, "tok_abc");
        assert_eq!(deserialized.preview_token, "prv_v1_test123");
        assert_eq!(deserialized.workspace_path, "/project");
    }

    #[test]
    fn test_sandbox_state_with_special_chars() {
        let state = SandboxState {
            sandbox_id: "sb-test_123".to_string(),
            pint_url: "https://sb-test_123-57468.csb.app".to_string(),
            pitcher_token: "tok+abc/def==".to_string(),
            preview_token: "prv_v1_special+chars==".to_string(),
            workspace_path: "/home/user/my project".to_string(),
            started_at: "2026-02-13T10:00:00+05:30".to_string(),
        };
        let json = serde_json::to_string(&state).unwrap();
        let deserialized: SandboxState = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.pint_url, "https://sb-test_123-57468.csb.app");
        assert_eq!(deserialized.pitcher_token, "tok+abc/def==");
    }

    // --- SSE parsing tests ---

    // --- URL encoding tests ---

    #[test]
    fn test_encode_path_simple() {
        assert_eq!(encode_path("/project/main.py"), "project/main.py");
    }

    #[test]
    fn test_encode_path_with_spaces() {
        let encoded = encode_path("/my project/file name.txt");
        assert!(encoded.contains("my%20project"));
        assert!(encoded.contains("file%20name.txt"));
    }

    #[test]
    fn test_encode_path_preserves_slashes() {
        assert_eq!(encode_path("/a/b/c/d.txt"), "a/b/c/d.txt");
    }

    // --- Error path tests ---

    #[tokio::test]
    async fn test_csb_exec_without_context() {
        let tool = CsbExecTool;
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
    async fn test_csb_create_sandbox_without_context() {
        let tool = CsbCreateSandboxTool;
        let result = tool.execute(json!({})).await;
        match result {
            ToolExecutionResult::ToolError(msg) => {
                assert!(msg.contains("requires context"));
            }
            _ => panic!("Expected tool error"),
        }
    }

    #[tokio::test]
    async fn test_csb_exec_status_without_context() {
        let tool = CsbExecStatusTool;
        let result = tool
            .execute(json!({"sandbox_id": "test", "exec_id": "e1"}))
            .await;
        match result {
            ToolExecutionResult::ToolError(msg) => {
                assert!(msg.contains("requires context"));
            }
            _ => panic!("Expected tool error"),
        }
    }

    #[tokio::test]
    async fn test_csb_read_file_without_context() {
        let tool = CsbReadFileTool;
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
    async fn test_csb_write_file_without_context() {
        let tool = CsbWriteFileTool;
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
    async fn test_csb_download_workspace_without_context() {
        let tool = CsbDownloadWorkspaceTool;
        let result = tool.execute(json!({"sandbox_id": "test"})).await;
        match result {
            ToolExecutionResult::ToolError(msg) => {
                assert!(msg.contains("requires context"));
            }
            _ => panic!("Expected tool error"),
        }
    }

    #[tokio::test]
    async fn test_csb_list_sandboxes_without_context() {
        let tool = CsbListSandboxesTool;
        let result = tool.execute(json!({})).await;
        match result {
            ToolExecutionResult::ToolError(msg) => {
                assert!(msg.contains("requires context"));
            }
            _ => panic!("Expected tool error"),
        }
    }

    #[tokio::test]
    async fn test_csb_manage_sandbox_without_context() {
        let tool = CsbManageSandboxTool;
        let result = tool
            .execute(json!({"sandbox_id": "test", "action": "shutdown"}))
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
    fn test_csb_exec_schema_has_required_fields() {
        let tool = CsbExecTool;
        let schema = tool.parameters_schema();
        let required = schema["required"].as_array().unwrap();
        let required_strs: Vec<&str> = required.iter().map(|v| v.as_str().unwrap()).collect();
        assert!(required_strs.contains(&"sandbox_id"));
        assert!(required_strs.contains(&"command"));
    }

    #[test]
    fn test_csb_create_sandbox_schema_no_required() {
        let tool = CsbCreateSandboxTool;
        let schema = tool.parameters_schema();
        // create has no required fields
        assert!(schema.get("required").is_none());
    }

    #[test]
    fn test_csb_manage_sandbox_schema_has_enum() {
        let tool = CsbManageSandboxTool;
        let schema = tool.parameters_schema();
        let action_enum = &schema["properties"]["action"]["enum"];
        let values: Vec<&str> = action_enum
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert!(values.contains(&"shutdown"));
        assert!(values.contains(&"hibernate"));
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
                    "data": {
                        "id": "sb_test123",
                        "title": "Test Sandbox"
                    }
                })))
                .mount(&mock_server)
                .await;

            let client =
                CodeSandboxClient::with_base_url("test_key".to_string(), mock_server.uri());
            let result = client.create_sandbox(json!({"title": "Test"})).await;
            assert!(result.is_ok());
            let info = result.unwrap();
            assert_eq!(info.id, "sb_test123");
            assert_eq!(info.title, Some("Test Sandbox".to_string()));
        }

        #[tokio::test]
        async fn test_client_start_vm() {
            let mock_server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/vm/sb_test/start"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "data": {
                        "pitcher_url": "https://pitcher.test.csb.app",
                        "pitcher_token": "tok_test",
                        "workspace_path": "/project"
                    }
                })))
                .mount(&mock_server)
                .await;

            let client =
                CodeSandboxClient::with_base_url("test_key".to_string(), mock_server.uri());
            let result = client.start_vm("sb_test", None).await;
            assert!(result.is_ok());
            let info = result.unwrap();
            assert_eq!(info.pitcher_url, "https://pitcher.test.csb.app");
            assert_eq!(info.pitcher_token, "tok_test");
            assert_eq!(info.workspace_path, Some("/project".to_string()));
        }

        #[tokio::test]
        async fn test_client_shutdown_vm() {
            let mock_server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/vm/sb_test/shutdown"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
                .mount(&mock_server)
                .await;

            let client =
                CodeSandboxClient::with_base_url("test_key".to_string(), mock_server.uri());
            let result = client.shutdown_vm("sb_test").await;
            assert!(result.is_ok());
        }

        #[tokio::test]
        async fn test_client_hibernate_vm() {
            let mock_server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/vm/sb_test/hibernate"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
                .mount(&mock_server)
                .await;

            let client =
                CodeSandboxClient::with_base_url("test_key".to_string(), mock_server.uri());
            let result = client.hibernate_vm("sb_test").await;
            assert!(result.is_ok());
        }

        #[tokio::test]
        async fn test_client_delete_vm() {
            let mock_server = MockServer::start().await;
            Mock::given(method("DELETE"))
                .and(path("/vm/sb_test"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
                .mount(&mock_server)
                .await;

            let client =
                CodeSandboxClient::with_base_url("test_key".to_string(), mock_server.uri());
            let result = client.delete_vm("sb_test").await;
            assert!(result.is_ok());
        }

        fn mock_state(pint_url: &str) -> SandboxState {
            SandboxState {
                sandbox_id: "sb_test".to_string(),
                pint_url: pint_url.to_string(),
                pitcher_token: "tok_test".to_string(),
                preview_token: "prv_test".to_string(),
                workspace_path: "/project".to_string(),
                started_at: "2026-01-01T00:00:00Z".to_string(),
            }
        }

        #[tokio::test]
        async fn test_client_exec_create() {
            let mock_server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/api/v1/execs"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "id": "exec_1",
                    "status": "running",
                    "exitCode": null
                })))
                .mount(&mock_server)
                .await;

            let client =
                CodeSandboxClient::with_base_url("test_key".to_string(), mock_server.uri());
            let state = mock_state(&mock_server.uri());
            let result = client
                .exec_create(&state, "bash", vec!["-c".into(), "echo hi".into()])
                .await;
            assert!(result.is_ok());
            let info = result.unwrap();
            assert_eq!(info.id, "exec_1");
            assert_eq!(info.status, "running");
        }

        #[tokio::test]
        async fn test_client_exec_get() {
            let mock_server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path("/api/v1/execs/exec_1"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "id": "exec_1",
                    "status": "exited",
                    "exitCode": 0
                })))
                .mount(&mock_server)
                .await;

            let client =
                CodeSandboxClient::with_base_url("test_key".to_string(), mock_server.uri());
            let state = mock_state(&mock_server.uri());
            let result = client.exec_get(&state, "exec_1").await;
            assert!(result.is_ok());
            let info = result.unwrap();
            assert_eq!(info.status, "exited");
            assert_eq!(info.exit_code, Some(0));
        }

        #[tokio::test]
        async fn test_client_exec_get_output() {
            let mock_server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path("/api/v1/execs/exec_1/io"))
                .respond_with(
                    // The real CodeSandbox /io endpoint returns plain text, not SSE
                    ResponseTemplate::new(200).set_body_string("hello world\n"),
                )
                .mount(&mock_server)
                .await;

            let client =
                CodeSandboxClient::with_base_url("test_key".to_string(), mock_server.uri());
            let state = mock_state(&mock_server.uri());
            let result = client.exec_get_output(&state, "exec_1").await;
            assert!(result.is_ok());
            let output = result.unwrap();
            assert_eq!(output, "hello world");
        }

        #[tokio::test]
        async fn test_client_file_read() {
            let mock_server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path("/api/v1/files/project/main.py"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "path": "/project/main.py",
                    "content": "print('hello')"
                })))
                .mount(&mock_server)
                .await;

            let client =
                CodeSandboxClient::with_base_url("test_key".to_string(), mock_server.uri());
            let state = mock_state(&mock_server.uri());
            let result = client.file_read(&state, "/project/main.py").await;
            assert!(result.is_ok());
            let content = result.unwrap();
            assert_eq!(content.content, "print('hello')");
        }

        #[tokio::test]
        async fn test_client_file_write() {
            let mock_server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/api/v1/files/project/main.py"))
                .respond_with(ResponseTemplate::new(201).set_body_json(json!({
                    "path": "/project/main.py",
                    "content": "print('hello')"
                })))
                .mount(&mock_server)
                .await;

            let client =
                CodeSandboxClient::with_base_url("test_key".to_string(), mock_server.uri());
            let state = mock_state(&mock_server.uri());
            let result = client
                .file_write(&state, "/project/main.py", "print('hello')")
                .await;
            assert!(result.is_ok());
        }

        #[tokio::test]
        async fn test_client_dir_list() {
            let mock_server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path("/api/v1/directories/project"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!([
                    {"name": "main.py", "type": "file"},
                    {"name": "src", "type": "directory"}
                ])))
                .mount(&mock_server)
                .await;

            let client =
                CodeSandboxClient::with_base_url("test_key".to_string(), mock_server.uri());
            let state = mock_state(&mock_server.uri());
            let result = client.dir_list(&state, "/project").await;
            assert!(result.is_ok());
            let entries = result.unwrap();
            assert_eq!(entries.len(), 2);
            assert_eq!(entries[0].name, "main.py");
            assert_eq!(entries[1].name, "src");
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

            let client =
                CodeSandboxClient::with_base_url("test_key".to_string(), mock_server.uri());
            let result = client
                .management_request(reqwest::Method::GET, "/sandbox/nonexistent", None)
                .await;
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

            let client = CodeSandboxClient::with_base_url("bad_key".to_string(), mock_server.uri());
            let result = client.create_sandbox(json!({"title": "Test"})).await;
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert!(err.contains("401"));
        }

        #[tokio::test]
        async fn test_client_500_error() {
            let mock_server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/vm/sb_test/start"))
                .respond_with(ResponseTemplate::new(500).set_body_string("Internal Server Error"))
                .mount(&mock_server)
                .await;

            let client =
                CodeSandboxClient::with_base_url("test_key".to_string(), mock_server.uri());
            let result = client.start_vm("sb_test", None).await;
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

            let client =
                CodeSandboxClient::with_base_url("test_key".to_string(), mock_server.uri());
            let result = client.create_sandbox(json!({"title": "Test"})).await;
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert!(err.contains("Invalid JSON"));
        }
    }
}

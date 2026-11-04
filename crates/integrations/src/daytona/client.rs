//! Daytona API client and URL encoding utilities.
//!
//! Decision: All command execution goes through the Session API
//! (`/process/session/{id}/exec`). The session provides a persistent shell
//! so commands always get proper shell interpretation (pipes, redirects,
//! variable expansion, etc.). The old stateless `/process/execute` endpoint
//! is NOT used — it doesn't support async execution or output streaming.
//!
//! Decision: Every command is prefixed with a shell-profile preamble that
//! sources common non-interactive profile files (~/.profile, ~/.cargo/env,
//! ~/.nvm/nvm.sh). `~/.bashrc` is intentionally not sourced because most
//! distros guard it with an interactive-shell check that returns early in
//! non-interactive exec contexts.
//! Daytona sessions use a bare shell (no login flag), so tools installed
//! mid-session (e.g. rustup writing ~/.cargo/env) aren't visible in PATH.
//! Unlike E2B, which spawns `bash -l -c` per command, Daytona's
//! persistent session means even a login shell at creation time wouldn't
//! pick up later installs. The preamble runs on every exec (~2ms overhead)
//! and largely eliminates "command not found after install" bugs.

use serde_json::{Value, json};
use std::time::Duration;
use tracing::debug;

use crate::daytona::api::{management, toolbox};
use crate::daytona::state::{
    ExecOutputChunk, ExecResult, ExecStream, SandboxInfo, SnapshotInfo, VolumeInfo,
};
use crate::daytona::{EXEC_POLL_INTERVAL, SANDBOX_READY_MAX_WAIT, SANDBOX_READY_POLL_INTERVAL};

/// Fixed session ID used for all command execution in a sandbox.
/// One session per sandbox, created on first exec, reused thereafter.
const EXEC_SESSION_ID: &str = "everruns-exec";

/// Separate session used exclusively for heartbeat probes.
/// Using a dedicated session avoids false positives: the exec session's shell
/// is blocked while a foreground command runs and cannot respond to a heartbeat,
/// but a separate shell in the same sandbox can. See EVE-255.
const HEARTBEAT_SESSION_ID: &str = "everruns-heartbeat";

/// Shell-profile preamble sourced before every command.
///
/// Daytona sessions use a bare (non-login) shell, so tools installed
/// mid-session (rustup, nvm, etc.) write profile files that never get
/// sourced. This preamble sources common profile files if they exist,
/// making newly-installed tools available immediately.
///
/// Deliberately excludes `~/.bashrc` — most distros guard it with a
/// `case $-` interactive-mode check that causes it to return early in
/// non-interactive exec contexts. The files listed here are all designed
/// for non-interactive sourcing (PATH/env exports only).
///
/// Profiles are sourced with stdout and stderr redirected to `/dev/null`
/// to suppress any unexpected noise (motd, banners, etc.).
const SHELL_PROFILE_PREAMBLE: &str = concat!(
    "for __f in \"$HOME/.profile\" \"$HOME/.cargo/env\" \"$HOME/.nvm/nvm.sh\"; do ",
    "[ -f \"$__f\" ] && . \"$__f\" >/dev/null 2>&1; ",
    "done; unset __f; ",
);

// ============================================================================
// DaytonaClient - HTTP client for Daytona APIs
// ============================================================================

/// Daytona Management and Toolbox API client.
///
/// Decision: JSON operations go through the clients generated from Daytona's
/// OpenAPI specs (`super::api`), so a field or path Daytona renames fails the
/// build or the spec drift check instead of a live call. The raw requests
/// left here are the ones the generator cannot express or that must stay
/// untyped: multipart upload, byte downloads (files and command logs), and
/// the agent-driven `daytona_api_call` passthrough.
pub struct DaytonaClient {
    http: reqwest::Client,
    api_key: String,
    api_base: String,
    toolbox_base: String,
    management: management::Client,
    download_limit: Option<usize>,
}

impl DaytonaClient {
    pub fn new(api_key: String) -> Self {
        Self::with_base_urls(
            api_key,
            crate::daytona::DAYTONA_API_BASE.to_string(),
            crate::daytona::DAYTONA_TOOLBOX_BASE.to_string(),
        )
    }

    pub fn with_base_urls(api_key: String, api_base: String, toolbox_base: String) -> Self {
        let mut headers = reqwest::header::HeaderMap::new();
        if let Ok(mut value) = reqwest::header::HeaderValue::from_str(&format!("Bearer {api_key}"))
        {
            value.set_sensitive(true);
            headers.insert(reqwest::header::AUTHORIZATION, value);
        }
        let http = reqwest::Client::builder()
            .default_headers(headers)
            .build()
            .unwrap_or_default();
        let management = management::Client::new_with_client(&api_base, http.clone());
        Self {
            http,
            api_key,
            api_base,
            toolbox_base,
            management,
            download_limit: None,
        }
    }

    /// Bound untrusted toolbox output before allocating the complete response.
    /// Build scripts may alter even the binaries used to truncate their logs.
    pub fn with_download_limit(mut self, bytes: usize) -> Self {
        self.download_limit = Some(bytes);
        self
    }

    /// Generated Toolbox client for one sandbox.
    fn toolbox(&self, sandbox_id: &str) -> toolbox::Client {
        toolbox::Client::new_with_client(
            &format!("{}/{sandbox_id}", self.toolbox_base),
            self.http.clone(),
        )
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

    /// Raw toolbox GET that returns bytes (file download, command logs).
    async fn toolbox_download(&self, sandbox_id: &str, path: &str) -> Result<Vec<u8>, String> {
        let url = format!("{}/{sandbox_id}{path}", self.toolbox_base);
        let mut resp = self
            .http
            .get(&url)
            .bearer_auth(&self.api_key)
            .send()
            .await
            .map_err(|e| format!("Failed to connect to sandbox: {e}"))?;

        let status = resp.status();
        if let Some(limit) = self.download_limit {
            let mut body = Vec::new();
            while let Some(chunk) = resp
                .chunk()
                .await
                .map_err(|e| format!("Failed to read sandbox output: {e}"))?
            {
                if chunk.len() > limit.saturating_sub(body.len()) {
                    return Err("Sandbox output exceeds the configured byte limit".into());
                }
                body.extend_from_slice(&chunk);
            }
            if !status.is_success() {
                return Err(format!("Sandbox API error ({status})"));
            }
            return Ok(body);
        }
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

    /// Multipart file upload to the toolbox API (`UploadFile`, which creates
    /// missing parent directories). Hand-written: the generator has no
    /// multipart support.
    async fn toolbox_upload(
        &self,
        sandbox_id: &str,
        remote_path: &str,
        content: &[u8],
        filename: &str,
    ) -> Result<(), String> {
        let url = format!(
            "{}/{sandbox_id}/files/upload-v2?path={}",
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

    // --- Generic API call (for daytona_api_call tool) ---

    /// Generic Daytona API call. Routes to Management or Toolbox API based on path prefix.
    ///
    /// - Paths starting with `/toolbox/{sandbox_id}/...` → Toolbox API
    /// - All other paths → Management API
    ///
    /// Headers (Authorization, Content-Type) are controlled internally.
    pub async fn api_call(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value, String> {
        if let Some(rest) = path.strip_prefix("/toolbox/") {
            // Extract sandbox_id from /toolbox/{sandbox_id}/...
            let (sandbox_id, toolbox_path) = rest
                .split_once('/')
                .map(|(id, p)| (id, format!("/{p}")))
                .unwrap_or((rest, String::new()));
            if sandbox_id.is_empty() {
                return Err("Invalid toolbox path: expected /toolbox/{sandbox_id}/...".to_string());
            }
            self.toolbox_request(method, sandbox_id, &toolbox_path, body)
                .await
        } else {
            self.management_request(method, path, body).await
        }
    }

    // --- Management API ---

    pub async fn create_sandbox(
        &self,
        body: management::types::CreateSandbox,
    ) -> Result<SandboxInfo, String> {
        let sandbox = self
            .management
            .create_sandbox()
            .body(body)
            .send()
            .await
            .or_api_error(DAYTONA)
            .await?;
        Ok(sandbox_info(sandbox))
    }

    /// Replace a running sandbox's outbound network rules.
    pub async fn update_network_settings(
        &self,
        sandbox_id: &str,
        body: management::types::UpdateSandboxNetworkSettings,
    ) -> Result<(), String> {
        self.management
            .update_network_settings()
            .sandbox_id_or_name(sandbox_id)
            .body(body)
            .send()
            .await
            .or_api_error_ignoring_body(DAYTONA)
            .await?;
        Ok(())
    }

    pub async fn get_sandbox(&self, sandbox_id: &str) -> Result<SandboxInfo, String> {
        let sandbox = self
            .management
            .get_sandbox()
            .sandbox_id_or_name(sandbox_id)
            .send()
            .await
            .or_api_error(DAYTONA)
            .await?;
        Ok(sandbox_info(sandbox))
    }

    /// Bake the stopped sandbox's filesystem, never its memory or live sessions.
    pub async fn create_sandbox_snapshot(
        &self,
        sandbox_id: &str,
        name: &str,
    ) -> Result<(), String> {
        self.management
            .create_sandbox_snapshot()
            .sandbox_id_or_name(sandbox_id)
            .body(management::types::CreateSandboxSnapshot {
                name: name.to_string(),
                include_memory: false,
            })
            .send()
            .await
            .or_api_error_ignoring_body(DAYTONA)
            .await?;
        Ok(())
    }

    pub async fn get_snapshot(&self, id: &str) -> Result<SnapshotInfo, String> {
        let snapshot = self
            .management
            .get_snapshot()
            .id(id)
            .send()
            .await
            .or_api_error(DAYTONA)
            .await?;
        Ok(snapshot_info(snapshot))
    }

    pub async fn delete_snapshot(&self, id: &str) -> Result<(), String> {
        self.management
            .remove_snapshot()
            .id(id)
            .send()
            .await
            .or_api_error_ignoring_body(DAYTONA)
            .await?;
        Ok(())
    }

    /// Every sandbox carrying all of `labels`, following `nextCursor` until
    /// the last page.
    pub async fn list_sandboxes_by_labels(
        &self,
        labels: &str,
    ) -> Result<Vec<management::types::SandboxListItem>, String> {
        let mut items = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let mut request = self.management.list_sandboxes().labels(labels);
            if let Some(cursor) = &cursor {
                request = request.cursor(cursor.as_str());
            }
            let page = request.send().await.or_api_error(DAYTONA).await?;
            items.extend(page.items.unwrap_or_default());
            match page.next_cursor.filter(|next| !next.is_empty()) {
                // A cursor that does not move would loop forever.
                Some(next) if cursor.as_deref() != Some(next.as_str()) => cursor = Some(next),
                _ => return Ok(items),
            }
        }
    }

    pub async fn create_volume(&self, name: &str) -> Result<VolumeInfo, String> {
        let volume = self
            .management
            .create_volume()
            .body(management::types::CreateVolume {
                name: name.to_string(),
            })
            .send()
            .await
            .or_api_error(DAYTONA)
            .await?;
        Ok(volume_info(volume))
    }

    pub async fn get_volume_by_name(&self, name: &str) -> Result<VolumeInfo, String> {
        let volume = self
            .management
            .get_volume_by_name()
            .name(name)
            .send()
            .await
            .or_api_error(DAYTONA)
            .await?;
        Ok(volume_info(volume))
    }

    pub async fn get_volume(&self, volume_id: &str) -> Result<VolumeInfo, String> {
        let volume = self
            .management
            .get_volume()
            .volume_id(volume_id)
            .send()
            .await
            .or_api_error(DAYTONA)
            .await?;
        Ok(volume_info(volume))
    }

    pub async fn start_sandbox(&self, sandbox_id: &str) -> Result<(), String> {
        self.management
            .start_sandbox()
            .sandbox_id_or_name(sandbox_id)
            .send()
            .await
            .or_api_error_ignoring_body(DAYTONA)
            .await?;
        Ok(())
    }

    pub async fn stop_sandbox(&self, sandbox_id: &str) -> Result<(), String> {
        self.management
            .stop_sandbox()
            .sandbox_id_or_name(sandbox_id)
            .send()
            .await
            .or_api_error_ignoring_body(DAYTONA)
            .await?;
        Ok(())
    }

    pub async fn delete_sandbox(&self, sandbox_id: &str) -> Result<(), String> {
        self.management
            .delete_sandbox()
            .sandbox_id_or_name(sandbox_id)
            .send()
            .await
            .or_api_error_ignoring_body(DAYTONA)
            .await?;
        Ok(())
    }

    pub async fn set_autostop(
        &self,
        sandbox_id: &str,
        interval_minutes: u64,
    ) -> Result<(), String> {
        let interval = f64::from(u32::try_from(interval_minutes).unwrap_or(u32::MAX));
        self.management
            .set_autostop_interval()
            .sandbox_id_or_name(sandbox_id)
            .interval(interval)
            .send()
            .await
            .or_api_error_ignoring_body(DAYTONA)
            .await?;
        Ok(())
    }

    // --- Snapshots API ---

    /// Every snapshot visible to the account, across all pages.
    pub async fn list_snapshots(&self) -> Result<Vec<SnapshotInfo>, String> {
        let mut snapshots = Vec::new();
        let mut page = 1;
        loop {
            let response = self
                .management
                .get_all_snapshots()
                .page(std::num::NonZeroU64::new(page).unwrap_or(std::num::NonZeroU64::MIN))
                .send()
                .await
                .or_api_error(DAYTONA)
                .await?;
            let page_items = response.items.unwrap_or_default();
            let fetched = page_items.len();
            snapshots.extend(page_items.into_iter().map(snapshot_info));
            let total_pages = response.total_pages.unwrap_or(1);
            if fetched == 0 || i64::try_from(page).unwrap_or(i64::MAX) >= total_pages {
                return Ok(snapshots);
            }
            page += 1;
        }
    }

    /// Wait for sandbox to reach "started" state after creation.
    pub async fn wait_for_ready(&self, sandbox_id: &str) -> Result<(), String> {
        poll_sandbox_ready(|| async { self.get_sandbox(sandbox_id).await.map(|info| info.state) })
            .await
    }

    // --- Toolbox API: Process (Session-based) ---

    /// Create a persistent shell session in the sandbox. Idempotent: 409
    /// means the session already exists.
    async fn create_shell_session(&self, sandbox_id: &str, session_id: &str) -> Result<(), String> {
        let result = self
            .toolbox(sandbox_id)
            .create_session()
            .body(toolbox::types::CreateSessionRequest {
                session_id: session_id.to_string(),
            })
            .send()
            .await;
        match result {
            Ok(_) => Ok(()),
            Err(error) if error.status().is_some_and(|s| s.as_u16() == 409) => Ok(()),
            Err(error) => Err(api_error(CREATE_SESSION, error).await),
        }
    }

    /// Start `command` asynchronously in a session and return its command id.
    async fn start_session_command(
        &self,
        sandbox_id: &str,
        session_id: &str,
        command: String,
    ) -> Result<String, String> {
        self.toolbox(sandbox_id)
            .session_execute_command()
            .session_id(session_id)
            .body(toolbox::types::SessionExecuteRequest {
                command,
                run_async: Some(true),
                async_: None,
                suppress_input_echo: None,
            })
            .send()
            .await
            .or_api_error(SANDBOX)
            .await?
            .cmd_id
            .ok_or_else(|| "Missing cmdId in session exec response".to_string())
    }

    /// Exit code of a session command, `None` while it still runs.
    async fn session_command_exit_code(
        &self,
        sandbox_id: &str,
        session_id: &str,
        command_id: &str,
    ) -> Result<Option<i64>, String> {
        Ok(self
            .toolbox(sandbox_id)
            .get_session_command()
            .session_id(session_id)
            .command_id(command_id)
            .send()
            .await
            .or_api_error(SANDBOX)
            .await?
            .exit_code)
    }

    /// Ensure a persistent shell session exists for the sandbox.
    async fn ensure_session(&self, sandbox_id: &str) -> Result<(), String> {
        self.create_shell_session(sandbox_id, EXEC_SESSION_ID).await
    }

    /// Probe sandbox health by running a trivial command in a **separate**
    /// heartbeat session. Returns `true` if the sandbox shell environment
    /// is functional, `false` if the probe stalls (sandbox is dead).
    ///
    /// We intentionally avoid probing the exec session because its shell is
    /// blocked by the foreground command and cannot respond — leading to
    /// false "dead session" detection for long-running commands that produce
    /// no stdout. See EVE-255.
    async fn session_heartbeat(&self, sandbox_id: &str) -> bool {
        if self
            .create_shell_session(sandbox_id, HEARTBEAT_SESSION_ID)
            .await
            .is_err()
        {
            return false;
        }

        let Ok(cmd_id) = self
            .start_session_command(sandbox_id, HEARTBEAT_SESSION_ID, "true".to_string())
            .await
        else {
            return false;
        };

        // Poll for up to SESSION_HEARTBEAT_TIMEOUT.
        let deadline = tokio::time::Instant::now()
            + Duration::from_millis(crate::daytona::SESSION_HEARTBEAT_TIMEOUT_MS);

        while tokio::time::Instant::now() < deadline {
            tokio::time::sleep(EXEC_POLL_INTERVAL).await;
            if let Ok(Some(_)) = self
                .session_command_exit_code(sandbox_id, HEARTBEAT_SESSION_ID, &cmd_id)
                .await
            {
                return true;
            }
        }

        false
    }

    /// Delete the persistent session. Used to reset a dead session whose
    /// shell was terminated by a command like `exit`.
    async fn delete_session(&self, sandbox_id: &str) -> Result<(), String> {
        match self
            .toolbox(sandbox_id)
            .delete_session()
            .session_id(EXEC_SESSION_ID)
            .send()
            .await
        {
            Ok(_) => Ok(()),
            Err(error) if error.status().is_some_and(|s| s.as_u16() == 404) => Ok(()),
            Err(error) => Err(api_error(DELETE_SESSION, error).await),
        }
    }

    /// Reset a dead session by deleting and re-creating it.
    async fn reset_session(&self, sandbox_id: &str) -> Result<(), String> {
        self.delete_session(sandbox_id).await?;
        self.ensure_session(sandbox_id).await
    }

    /// Execute a command in the sandbox via the Session API.
    ///
    /// Creates a persistent shell session (if needed), runs the command
    /// asynchronously, and polls the session logs endpoint for output.
    /// Calls `on_output` with each new chunk as it becomes available.
    /// Pass `|_| {}` if streaming is not needed.
    pub async fn exec<F>(
        &self,
        sandbox_id: &str,
        command: &str,
        cwd: Option<&str>,
        timeout_ms: Option<u64>,
        mut on_output: F,
    ) -> Result<ExecResult, String>
    where
        F: FnMut(ExecOutputChunk),
    {
        self.ensure_session(sandbox_id).await?;

        let cmd = format_exec_command(command, cwd);

        let timeout = timeout_ms.unwrap_or(crate::daytona::EXEC_TIMEOUT_MS);

        // Start async execution in the persistent session.
        // runAsync: true returns immediately with a cmdId.
        let cmd_id = self
            .start_session_command(sandbox_id, EXEC_SESSION_ID, cmd)
            .await?;

        // Poll for output and completion.
        //
        // Dead-session detection: When a command like `exit` kills the
        // persistent shell, Daytona keeps the session metadata alive (GET
        // returns 200) but the command never gets an exitCode and logs stay
        // empty. We detect this by tracking consecutive "stale" polls — polls
        // where there is no exitCode and no new output. After enough stale
        // polls we probe with a heartbeat command; if it also stalls, we
        // know the session is dead and reset it.
        let deadline = tokio::time::Instant::now() + Duration::from_millis(timeout);
        let mut raw_bytes_emitted: usize = 0;
        let mut stale_polls: u32 = 0;
        let mut delta_parser = ExecDeltaParserState::new(ExecStream::Stdout);
        let logs_path = format!("/process/session/{EXEC_SESSION_ID}/command/{cmd_id}/logs");

        loop {
            if tokio::time::Instant::now() >= deadline {
                return match self.reset_session(sandbox_id).await {
                    Ok(()) => Err(format!(
                        "Command timed out after {timeout}ms. \
                         The exec session has been automatically reset so \
                         your next command will work. Increase the timeout \
                         parameter or break the command into smaller steps."
                    )),
                    Err(e) => {
                        debug!("Failed to reset session {EXEC_SESSION_ID} after timeout: {e}");
                        Err(format!(
                            "Command timed out after {timeout}ms. \
                             Additionally, failed to reset the exec session: {e}"
                        ))
                    }
                };
            }

            tokio::time::sleep(EXEC_POLL_INTERVAL).await;

            // Fetch session command logs (raw bytes with stream markers).
            let logs_result = self.toolbox_download(sandbox_id, &logs_path).await;
            if logs_result
                .as_ref()
                .is_err_and(|e| e == "Sandbox output exceeds the configured byte limit")
            {
                return Err("Sandbox output exceeds the configured byte limit".into());
            }
            let logs_ok = logs_result.ok();

            let had_new_output = logs_ok
                .as_ref()
                .is_some_and(|bytes| bytes.len() > raw_bytes_emitted);
            if let Some(raw_logs) = logs_ok.as_ref()
                && raw_logs.len() > raw_bytes_emitted
            {
                let delta =
                    parse_exec_output_delta(&raw_logs[raw_bytes_emitted..], &mut delta_parser);
                for chunk in delta.chunks {
                    on_output(chunk);
                }
                raw_bytes_emitted = raw_logs.len();
            }

            // Check if command completed (exitCode is present).
            let status = self
                .session_command_exit_code(sandbox_id, EXEC_SESSION_ID, &cmd_id)
                .await;

            if let Ok(Some(exit_code)) = status {
                // Final log fetch to capture any trailing output.
                let final_result = self.toolbox_download(sandbox_id, &logs_path).await;
                if final_result
                    .as_ref()
                    .is_err_and(|e| e == "Sandbox output exceeds the configured byte limit")
                {
                    return Err("Sandbox output exceeds the configured byte limit".into());
                }
                let final_logs = final_result.unwrap_or_default();

                if final_logs.len() > raw_bytes_emitted {
                    let delta = parse_exec_output_delta(
                        &final_logs[raw_bytes_emitted..],
                        &mut delta_parser,
                    );
                    for chunk in delta.chunks {
                        on_output(chunk);
                    }
                }
                for chunk in finish_exec_output_delta(&mut delta_parser).chunks {
                    on_output(chunk);
                }
                let final_output = split_stream_markers(&final_logs);

                return Ok(ExecResult {
                    stdout: final_output.stdout,
                    stderr: final_output.stderr,
                    result: final_output.combined,
                    exit_code: exit_code as i32,
                });
            }

            // Track stale polls: no exitCode and no new output.
            // Only count polls where both log and status calls succeeded —
            // transient HTTP failures should not trigger dead-session detection.
            if had_new_output {
                stale_polls = 0;
            } else if status.is_ok() && logs_ok.is_some() {
                stale_polls += 1;
            }

            // After several stale polls, probe session health with a
            // heartbeat. If the heartbeat doesn't complete quickly, the
            // session shell is dead. Skip the probe if remaining time is
            // less than the heartbeat timeout to avoid exceeding deadline.
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if stale_polls >= crate::daytona::SESSION_STALE_THRESHOLD
                && remaining > Duration::from_millis(crate::daytona::SESSION_HEARTBEAT_TIMEOUT_MS)
            {
                debug!(
                    "Command {cmd_id} stale for {stale_polls} polls, \
                     probing session health"
                );
                if self.session_heartbeat(sandbox_id).await {
                    // Session is alive; command is just slow. Reset counter
                    // so we don't probe again for a while.
                    stale_polls = 0;
                } else {
                    debug!(
                        "Session {EXEC_SESSION_ID} is dead (heartbeat failed); \
                         resetting session"
                    );
                    return match self.reset_session(sandbox_id).await {
                        Ok(()) => Err("Session shell terminated unexpectedly. Common causes: \
                             the command ran `exit`, a subshell crashed, or the process \
                             was OOM-killed. The session has been automatically reset — \
                             your next command will work. To avoid this, do not use \
                             `exit` in commands and avoid excessive memory usage."
                            .to_string()),
                        Err(e) => {
                            debug!("Failed to reset session {EXEC_SESSION_ID}: {e}");
                            Err(format!(
                                "Session shell terminated unexpectedly. Common causes: \
                                 the command ran `exit`, a subshell crashed, or the process \
                                 was OOM-killed. Additionally, failed to reset session: {e}"
                            ))
                        }
                    };
                }
            }
        }
    }

    // --- Toolbox API: Files ---

    pub async fn file_download(&self, sandbox_id: &str, path: &str) -> Result<Vec<u8>, String> {
        let encoded = urlencoding::encode(path);
        self.toolbox_download(sandbox_id, &format!("/files/download?path={encoded}"))
            .await
    }

    /// [`Self::file_download`] as a stream of chunks, so a large file (a
    /// desktop recording) is never held whole in memory.
    pub async fn file_download_stream(
        &self,
        sandbox_id: &str,
        path: &str,
    ) -> Result<futures::stream::BoxStream<'static, Result<Vec<u8>, String>>, String> {
        use futures::StreamExt;
        let encoded = urlencoding::encode(path);
        let url = format!(
            "{}/{sandbox_id}/files/download?path={encoded}",
            self.toolbox_base
        );
        let resp = self
            .http
            .get(&url)
            .bearer_auth(&self.api_key)
            .send()
            .await
            .map_err(|e| format!("Failed to connect to sandbox: {e}"))?;
        let status = resp.status();
        if !status.is_success() {
            let body_text = resp.text().await.unwrap_or_default();
            return Err(format!("Sandbox API error ({status}): {body_text}"));
        }
        Ok(resp
            .bytes_stream()
            .map(|chunk| {
                chunk
                    .map(|bytes| bytes.to_vec())
                    .map_err(|e| format!("Failed to read file bytes: {e}"))
            })
            .boxed())
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
        let files = self
            .toolbox(sandbox_id)
            .list_files()
            .path(path)
            .send()
            .await
            .or_api_error(SANDBOX)
            .await?;
        files
            .into_iter()
            .map(|file| serde_json::to_value(file).map_err(|e| format!("Invalid file entry: {e}")))
            .collect()
    }

    pub async fn file_delete(&self, sandbox_id: &str, path: &str) -> Result<(), String> {
        self.toolbox(sandbox_id)
            .delete_file()
            .path(path)
            .send()
            .await
            .or_api_error_ignoring_body(SANDBOX)
            .await?;
        Ok(())
    }

    pub async fn create_folder(
        &self,
        sandbox_id: &str,
        path: &str,
        mode: &str,
    ) -> Result<(), String> {
        self.toolbox(sandbox_id)
            .create_folder()
            .path(path)
            .mode(mode)
            .send()
            .await
            .or_api_error_ignoring_body(SANDBOX)
            .await?;
        Ok(())
    }
}

/// Error text prefixes, kept in the shape callers already match on
/// (`"... error (404 Not Found): body"`).
const DAYTONA: &str = "Daytona API error";
const SANDBOX: &str = "Sandbox API error";
const CREATE_SESSION: &str = "Failed to create session";
const DELETE_SESSION: &str = "Failed to delete session";

/// Turn a generated-client error into the text the tools surface. Error
/// responses are left out of the generated specs (`build.rs`), so every
/// non-2xx answer arrives here with its raw body.
async fn api_error(prefix: &str, error: progenitor_client::Error) -> String {
    match error {
        progenitor_client::Error::UnexpectedResponse(response) => {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            format!("{prefix} ({status}): {body}")
        }
        progenitor_client::Error::CommunicationError(e) => {
            format!("Failed to connect to Daytona: {e}")
        }
        progenitor_client::Error::InvalidResponsePayload(_, e) if prefix == SANDBOX => {
            format!("Invalid JSON from sandbox: {e}")
        }
        progenitor_client::Error::InvalidResponsePayload(_, e) => {
            format!("Invalid JSON from Daytona: {e}")
        }
        other => format!("{prefix}: {other}"),
    }
}

trait ResponseExt<T> {
    async fn or_api_error(self, prefix: &str) -> Result<T, String>;

    /// Success for calls whose response body is not read: a 2xx with a body
    /// the spec did not promise (empty, say) still means the call worked.
    async fn or_api_error_ignoring_body(self, prefix: &str) -> Result<(), String>;
}

impl<T> ResponseExt<T> for Result<progenitor_client::ResponseValue<T>, progenitor_client::Error> {
    async fn or_api_error(self, prefix: &str) -> Result<T, String> {
        match self {
            Ok(value) => Ok(value.into_inner()),
            Err(error) => Err(api_error(prefix, error).await),
        }
    }

    async fn or_api_error_ignoring_body(self, prefix: &str) -> Result<(), String> {
        match self {
            Ok(_) | Err(progenitor_client::Error::InvalidResponsePayload(..)) => Ok(()),
            // Another 2xx than the spec lists (200 for a documented 204, say).
            Err(progenitor_client::Error::UnexpectedResponse(response))
                if response.status().is_success() =>
            {
                Ok(())
            }
            Err(error) => Err(api_error(prefix, error).await),
        }
    }
}

fn sandbox_info(sandbox: management::types::Sandbox) -> SandboxInfo {
    SandboxInfo {
        id: sandbox.id.unwrap_or_default(),
        name: sandbox.name,
        state: sandbox.state.map(|state| state.0).unwrap_or_default(),
    }
}

fn volume_info(volume: management::types::VolumeDto) -> VolumeInfo {
    VolumeInfo {
        id: volume.id.unwrap_or_default(),
        name: volume.name.unwrap_or_default(),
        state: volume.state.map(|state| state.0).unwrap_or_default(),
    }
}

fn snapshot_info(snapshot: management::types::SnapshotDto) -> SnapshotInfo {
    // Daytona reports resources as numbers that are whole in practice.
    let whole = |value: Option<f64>| value.filter(|v| *v >= 0.0).map(|v| v as u64);
    SnapshotInfo {
        id: snapshot.id.unwrap_or_default(),
        name: snapshot.name.unwrap_or_default(),
        state: snapshot.state.map(|state| state.0).unwrap_or_default(),
        cpu: whole(snapshot.cpu),
        mem: whole(snapshot.mem),
        disk: whole(snapshot.disk),
        gpu: whole(snapshot.gpu),
        size: snapshot.size.map(|size| format!("{size:.2}")),
        general: snapshot.general,
        image_name: snapshot.image_name,
        created_at: snapshot.created_at.map(|at| at.to_rfc3339()),
        updated_at: snapshot.updated_at.map(|at| at.to_rfc3339()),
    }
}

fn format_exec_command(command: &str, cwd: Option<&str>) -> String {
    // Wrap the user command in a subshell `( ... )` so that `exit`,
    // SIGPIPE, SIGHUP, and other signals only terminate the subshell,
    // not the persistent session shell. Without this, commands like
    // `bash -lc '...; exit'`, pipes to `head`/`sed` (SIGPIPE), and
    // long `&&` chains that call `exit` would kill the entire session
    // and force expensive re-establishment. See EVE-251.
    match cwd {
        Some(c) => {
            // Cwd is a literal path, even when it contains shell metacharacters.
            let quoted = c.replace('\'', "'\"'\"'");
            format!("{SHELL_PROFILE_PREAMBLE}( cd '{quoted}' && {command} )")
        }
        None => format!("{SHELL_PROFILE_PREAMBLE}( {command} )"),
    }
}

// Keep readiness policy independent of HTTP so time is controlled without
// racing network I/O against an auto-advancing test clock.
async fn poll_sandbox_ready<F, Fut>(mut fetch_state: F) -> Result<(), String>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<String, String>>,
{
    let start = tokio::time::Instant::now();
    let mut interval = SANDBOX_READY_POLL_INTERVAL;
    while start.elapsed() < SANDBOX_READY_MAX_WAIT {
        match fetch_state().await {
            Ok(state) if state == "started" => {
                debug!("Sandbox ready (state: started) after {:?}", start.elapsed());
                return Ok(());
            }
            Ok(state) if state == "error" || state == "build_failed" => {
                return Err(format!("Sandbox entered error state: {state}"));
            }
            Ok(state) => debug!("Sandbox not ready yet (state: {state}), retrying..."),
            Err(error) => debug!("Failed to poll sandbox state: {error}, retrying..."),
        }
        tokio::time::sleep(interval).await;
        interval = std::cmp::min(interval * 2, Duration::from_secs(4));
    }
    Err(format!(
        "Sandbox did not become ready within {}s",
        SANDBOX_READY_MAX_WAIT.as_secs()
    ))
}

const STDOUT_MARKER: [u8; 3] = [0x01, 0x01, 0x01];
const STDERR_MARKER: [u8; 3] = [0x02, 0x02, 0x02];

struct ParsedExecOutput {
    stdout: String,
    stderr: String,
    combined: String,
}

struct ParsedExecDelta {
    chunks: Vec<ExecOutputChunk>,
}

#[derive(Debug, Clone)]
struct ExecDeltaParserState {
    current_stream: ExecStream,
    pending_marker_prefix: Vec<u8>,
    pending_text: Vec<u8>,
}

impl ExecDeltaParserState {
    fn new(starting_stream: ExecStream) -> Self {
        Self {
            current_stream: starting_stream,
            pending_marker_prefix: Vec::new(),
            pending_text: Vec::new(),
        }
    }
}

/// Split Daytona session log stream multiplexing markers into stdout/stderr.
///
/// The session API multiplexes stdout/stderr with 3-byte prefix markers:
/// - `\x01\x01\x01` = stdout data follows
/// - `\x02\x02\x02` = stderr data follows
///
/// The returned `combined` output matches the legacy `ExecResult.result`
/// behavior so existing textual consumers can continue using it.
fn split_stream_markers(raw: &[u8]) -> ParsedExecOutput {
    let mut stdout = Vec::with_capacity(raw.len());
    let mut stderr = Vec::new();
    let mut combined = Vec::with_capacity(raw.len());
    let mut current_stream = ExecStream::Stdout;
    let mut i = 0;

    while i < raw.len() {
        if i + 3 <= raw.len() && raw[i..i + 3] == STDOUT_MARKER {
            current_stream = ExecStream::Stdout;
            i += 3;
            continue;
        }
        if i + 3 <= raw.len() && raw[i..i + 3] == STDERR_MARKER {
            current_stream = ExecStream::Stderr;
            i += 3;
            continue;
        }

        let byte = raw[i];
        combined.push(byte);
        match current_stream {
            ExecStream::Stdout => stdout.push(byte),
            ExecStream::Stderr => stderr.push(byte),
        }
        i += 1;
    }

    ParsedExecOutput {
        stdout: String::from_utf8_lossy(&stdout).into_owned(),
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
        combined: String::from_utf8_lossy(&combined).into_owned(),
    }
}

fn flush_exec_output_buffer(
    chunks: &mut Vec<ExecOutputChunk>,
    stream: ExecStream,
    buffer: &mut Vec<u8>,
) {
    if buffer.is_empty() {
        return;
    }
    let text = String::from_utf8_lossy(buffer).into_owned();
    if let Some(last) = chunks.last_mut()
        && last.stream == stream
    {
        last.text.push_str(&text);
    } else {
        chunks.push(ExecOutputChunk { stream, text });
    }
    buffer.clear();
}

fn is_partial_stream_marker_prefix(bytes: &[u8]) -> bool {
    if bytes.is_empty() || bytes.len() >= STDOUT_MARKER.len() {
        return false;
    }

    bytes.iter().all(|b| *b == STDOUT_MARKER[0]) || bytes.iter().all(|b| *b == STDERR_MARKER[0])
}

fn parse_exec_output_delta(raw: &[u8], state: &mut ExecDeltaParserState) -> ParsedExecDelta {
    let mut chunks: Vec<ExecOutputChunk> = Vec::new();
    let mut buffer = std::mem::take(&mut state.pending_text);
    let mut bytes = Vec::with_capacity(state.pending_marker_prefix.len() + raw.len());
    bytes.extend_from_slice(&state.pending_marker_prefix);
    bytes.extend_from_slice(raw);
    state.pending_marker_prefix.clear();
    let mut i = 0;

    while i < bytes.len() {
        if i + 3 <= bytes.len() && bytes[i..i + 3] == STDOUT_MARKER {
            flush_exec_output_buffer(&mut chunks, state.current_stream, &mut buffer);
            state.current_stream = ExecStream::Stdout;
            i += 3;
            continue;
        }
        if i + 3 <= bytes.len() && bytes[i..i + 3] == STDERR_MARKER {
            flush_exec_output_buffer(&mut chunks, state.current_stream, &mut buffer);
            state.current_stream = ExecStream::Stderr;
            i += 3;
            continue;
        }
        if is_partial_stream_marker_prefix(&bytes[i..]) {
            state.pending_marker_prefix.extend_from_slice(&bytes[i..]);
            break;
        }

        buffer.push(bytes[i]);
        i += 1;
    }

    // A poll can end inside a UTF-8 code point. Keep only that incomplete
    // suffix; malformed complete sequences still use lossy decoding.
    let mut offset = 0;
    while let Err(error) = std::str::from_utf8(&buffer[offset..]) {
        offset += error.valid_up_to();
        if let Some(length) = error.error_len() {
            offset += length;
        } else {
            state.pending_text = buffer.split_off(offset);
            break;
        }
    }
    flush_exec_output_buffer(&mut chunks, state.current_stream, &mut buffer);

    ParsedExecDelta { chunks }
}

fn finish_exec_output_delta(state: &mut ExecDeltaParserState) -> ParsedExecDelta {
    let mut chunks = Vec::new();
    let mut buffer = std::mem::take(&mut state.pending_text);
    buffer.append(&mut state.pending_marker_prefix);
    flush_exec_output_buffer(&mut chunks, state.current_stream, &mut buffer);
    ParsedExecDelta { chunks }
}

pub(crate) mod urlencoding {
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
// Tests
// ============================================================================

#[cfg(test)]
#[path = "client_tests.rs"]
mod tests;

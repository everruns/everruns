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
//! Unlike E2B and Deno which spawn `bash -l -c` per command, Daytona's
//! persistent session means even a login shell at creation time wouldn't
//! pick up later installs. The preamble runs on every exec (~2ms overhead)
//! and largely eliminates "command not found after install" bugs.

use serde_json::{Value, json};
use std::time::Duration;
use tracing::debug;

use crate::state::{
    ExecOutputChunk, ExecResult, ExecStream, SandboxInfo, SnapshotInfo, VolumeInfo,
};
use crate::{EXEC_POLL_INTERVAL, SANDBOX_READY_MAX_WAIT, SANDBOX_READY_POLL_INTERVAL};

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
            crate::DAYTONA_API_BASE.to_string(),
            crate::DAYTONA_TOOLBOX_BASE.to_string(),
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

    pub async fn create_volume(&self, name: &str) -> Result<VolumeInfo, String> {
        let resp = self
            .management_request(
                reqwest::Method::POST,
                "/volumes",
                Some(json!({ "name": name })),
            )
            .await?;
        serde_json::from_value(resp).map_err(|e| format!("Failed to parse volume info: {e}"))
    }

    pub async fn get_volume_by_name(&self, name: &str) -> Result<VolumeInfo, String> {
        let resp = self
            .management_request(
                reqwest::Method::GET,
                &format!("/volumes/by-name/{}", urlencoding::encode(name)),
                None,
            )
            .await?;
        serde_json::from_value(resp).map_err(|e| format!("Failed to parse volume info: {e}"))
    }

    pub async fn get_volume(&self, volume_id: &str) -> Result<VolumeInfo, String> {
        let resp = self
            .management_request(
                reqwest::Method::GET,
                &format!("/volumes/{}", urlencoding::encode(volume_id)),
                None,
            )
            .await?;
        serde_json::from_value(resp).map_err(|e| format!("Failed to parse volume info: {e}"))
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

    // --- Snapshots API ---

    pub async fn list_snapshots(&self) -> Result<Vec<SnapshotInfo>, String> {
        let resp = self
            .management_request(reqwest::Method::GET, "/snapshots", None)
            .await?;

        // API returns an array of snapshot objects.
        let snapshots: Vec<SnapshotInfo> = serde_json::from_value(resp)
            .map_err(|e| format!("Failed to parse snapshots response: {e}"))?;
        Ok(snapshots)
    }

    /// Wait for sandbox to reach "started" state after creation.
    pub async fn wait_for_ready(&self, sandbox_id: &str) -> Result<(), String> {
        poll_sandbox_ready(|| async { self.get_sandbox(sandbox_id).await.map(|info| info.state) })
            .await
    }

    // --- Toolbox API: Process (Session-based) ---

    /// Ensure a persistent shell session exists for the sandbox.
    /// Idempotent — 409 means the session already exists.
    async fn ensure_session(&self, sandbox_id: &str) -> Result<(), String> {
        let url = format!("{}/{sandbox_id}/process/session", self.toolbox_base);
        let resp = self
            .http
            .post(&url)
            .bearer_auth(&self.api_key)
            .header("Content-Type", "application/json")
            .json(&json!({"sessionId": EXEC_SESSION_ID}))
            .send()
            .await
            .map_err(|e| format!("Failed to create session: {e}"))?;

        let status = resp.status();
        if status.is_success() || status.as_u16() == 409 {
            Ok(())
        } else {
            let body = resp.text().await.unwrap_or_default();
            Err(format!("Failed to create session ({status}): {body}"))
        }
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
        // Ensure the heartbeat session exists (idempotent).
        let create_url = format!("{}/{sandbox_id}/process/session", self.toolbox_base);
        let create_resp = self
            .http
            .post(&create_url)
            .bearer_auth(&self.api_key)
            .header("Content-Type", "application/json")
            .json(&json!({"sessionId": HEARTBEAT_SESSION_ID}))
            .send()
            .await;
        match create_resp {
            Ok(r) if r.status().is_success() || r.status().as_u16() == 409 => {}
            _ => return false,
        }

        // Fire a heartbeat command in the dedicated session.
        let resp = self
            .toolbox_request(
                reqwest::Method::POST,
                sandbox_id,
                &format!("/process/session/{HEARTBEAT_SESSION_ID}/exec"),
                Some(json!({
                    "command": "true",
                    "runAsync": true
                })),
            )
            .await;

        let cmd_id = match resp {
            Ok(ref v) => match v.get("cmdId").and_then(|v| v.as_str()) {
                Some(id) => id.to_string(),
                None => return false,
            },
            Err(_) => return false,
        };

        let status_path = format!("/process/session/{HEARTBEAT_SESSION_ID}/command/{cmd_id}");

        // Poll for up to SESSION_HEARTBEAT_TIMEOUT.
        let deadline =
            std::time::Instant::now() + Duration::from_millis(crate::SESSION_HEARTBEAT_TIMEOUT_MS);

        while std::time::Instant::now() < deadline {
            tokio::time::sleep(EXEC_POLL_INTERVAL).await;
            if let Ok(s) = self
                .toolbox_request(reqwest::Method::GET, sandbox_id, &status_path, None)
                .await
                && s.get("exitCode").is_some()
            {
                return true;
            }
        }

        false
    }

    /// Delete the persistent session. Used to reset a dead session whose
    /// shell was terminated by a command like `exit`.
    async fn delete_session(&self, sandbox_id: &str) -> Result<(), String> {
        let url = format!(
            "{}/{sandbox_id}/process/session/{EXEC_SESSION_ID}",
            self.toolbox_base
        );
        let resp = self
            .http
            .delete(&url)
            .bearer_auth(&self.api_key)
            .send()
            .await
            .map_err(|e| format!("Failed to delete session: {e}"))?;

        let status = resp.status();
        if status.is_success() || status.as_u16() == 404 {
            Ok(())
        } else {
            let body = resp.text().await.unwrap_or_default();
            Err(format!("Failed to delete session ({status}): {body}"))
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

        let timeout = timeout_ms.unwrap_or(crate::EXEC_TIMEOUT_MS);

        // Start async execution in the persistent session.
        // runAsync: true returns immediately with a cmdId.
        let resp = self
            .toolbox_request(
                reqwest::Method::POST,
                sandbox_id,
                &format!("/process/session/{EXEC_SESSION_ID}/exec"),
                Some(json!({
                    "command": cmd,
                    "runAsync": true
                })),
            )
            .await?;

        let cmd_id = resp
            .get("cmdId")
            .and_then(|v| v.as_str())
            .ok_or("Missing cmdId in session exec response")?
            .to_string();

        // Poll for output and completion.
        //
        // Dead-session detection: When a command like `exit` kills the
        // persistent shell, Daytona keeps the session metadata alive (GET
        // returns 200) but the command never gets an exitCode and logs stay
        // empty. We detect this by tracking consecutive "stale" polls — polls
        // where there is no exitCode and no new output. After enough stale
        // polls we probe with a heartbeat command; if it also stalls, we
        // know the session is dead and reset it.
        let deadline = std::time::Instant::now() + Duration::from_millis(timeout);
        let mut raw_bytes_emitted: usize = 0;
        let mut stale_polls: u32 = 0;
        let mut delta_parser = ExecDeltaParserState::new(ExecStream::Stdout);
        let logs_path = format!("/process/session/{EXEC_SESSION_ID}/command/{cmd_id}/logs");
        let status_path = format!("/process/session/{EXEC_SESSION_ID}/command/{cmd_id}");

        loop {
            if std::time::Instant::now() >= deadline {
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
            let logs_ok = self.toolbox_download(sandbox_id, &logs_path).await.ok();

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
                .toolbox_request(reqwest::Method::GET, sandbox_id, &status_path, None)
                .await;

            if let Ok(ref s) = status
                && let Some(exit_code) = s.get("exitCode").and_then(|v| v.as_i64())
            {
                // Final log fetch to capture any trailing output.
                let final_logs = self
                    .toolbox_download(sandbox_id, &logs_path)
                    .await
                    .unwrap_or_default();

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
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if stale_polls >= crate::SESSION_STALE_THRESHOLD
                && remaining > Duration::from_millis(crate::SESSION_HEARTBEAT_TIMEOUT_MS)
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
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn test_client_create_sandbox() {
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/sandbox"))
            .and(wiremock::matchers::header(
                "authorization",
                "Bearer test_key",
            ))
            .and(wiremock::matchers::body_json(json!({"name":"Test"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "sb_test123",
                "name": "Test Sandbox",
                "state": "started"
            })))
            .expect(1)
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
        assert_eq!(info.state, "started");
        assert_eq!(info.name, Some("Test Sandbox".to_string()));
    }

    #[tokio::test]
    async fn test_client_get_sandbox() {
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/sandbox/sb_test"))
            .and(wiremock::matchers::header(
                "authorization",
                "Bearer test_key",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "sb_test",
                "name": "Test",
                "state": "started"
            })))
            .expect(1)
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
        assert_eq!(info.id, "sb_test");
        assert_eq!(info.name.as_deref(), Some("Test"));
    }

    #[tokio::test]
    async fn test_client_creates_volume() {
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/volumes"))
            .and(wiremock::matchers::header(
                "authorization",
                "Bearer test_key",
            ))
            .and(wiremock::matchers::body_json(
                json!({"name":"everruns-recovery"}),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "vol_recovery",
                "name": "everruns-recovery",
                "state": "pending_create"
            })))
            .expect(1)
            .mount(&mock_server)
            .await;

        let client = DaytonaClient::with_base_urls(
            "test_key".to_string(),
            mock_server.uri(),
            mock_server.uri(),
        );
        let volume = client.create_volume("everruns-recovery").await.unwrap();

        assert_eq!(volume.id, "vol_recovery");
        assert_eq!(volume.name, "everruns-recovery");
        assert_eq!(volume.state, "pending_create");
    }

    #[tokio::test]
    async fn test_client_gets_volume_by_encoded_name() {
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/volumes/by-name/everruns%20recovery"))
            .and(wiremock::matchers::header(
                "authorization",
                "Bearer test_key",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "vol_recovery",
                "name": "everruns recovery",
                "state": "ready"
            })))
            .expect(1)
            .mount(&mock_server)
            .await;

        let client = DaytonaClient::with_base_urls(
            "test_key".to_string(),
            mock_server.uri(),
            mock_server.uri(),
        );
        let volume = client
            .get_volume_by_name("everruns recovery")
            .await
            .unwrap();

        assert_eq!(volume.id, "vol_recovery");
        assert_eq!(volume.state, "ready");
    }

    #[tokio::test]
    async fn test_client_stop_sandbox() {
        for response in ["", "{}"] {
            let mock_server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/sandbox/sb_test/stop"))
                .and(wiremock::matchers::header(
                    "authorization",
                    "Bearer test_key",
                ))
                .respond_with(ResponseTemplate::new(200).set_body_string(response))
                .expect(1)
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
    }

    #[tokio::test]
    async fn test_client_delete_sandbox() {
        let mock_server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path("/sandbox/sb_test"))
            .and(wiremock::matchers::header(
                "authorization",
                "Bearer test_key",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
            .expect(1)
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

    /// Set up mock responses for the session-based exec flow.
    async fn setup_session_mocks(
        mock_server: &MockServer,
        sandbox_id: &str,
        exit_code: i64,
        output: &str,
    ) {
        // 1. Create session → 201 (or 409 if exists)
        Mock::given(method("POST"))
            .and(wiremock::matchers::header(
                "authorization",
                "Bearer test_key",
            ))
            .and(path(format!("/{sandbox_id}/process/session")))
            .and(wiremock::matchers::body_json(
                json!({"sessionId":"everruns-exec"}),
            ))
            .respond_with(ResponseTemplate::new(201))
            .expect(1)
            .mount(mock_server)
            .await;

        // 2. Async exec → 202 with cmdId
        Mock::given(method("POST"))
            .and(wiremock::matchers::header(
                "authorization",
                "Bearer test_key",
            ))
            .and(path(format!(
                "/{sandbox_id}/process/session/everruns-exec/exec"
            )))
            .respond_with(ResponseTemplate::new(202).set_body_json(json!({
                "cmdId": "cmd_001"
            })))
            .mount(mock_server)
            .await;

        // 3. Logs → output bytes with stdout marker prefix
        let mut log_bytes = vec![0x01, 0x01, 0x01];
        log_bytes.extend_from_slice(output.as_bytes());
        Mock::given(method("GET"))
            .and(path(format!(
                "/{sandbox_id}/process/session/everruns-exec/command/cmd_001/logs"
            )))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(log_bytes))
            .mount(mock_server)
            .await;

        // 4. Command status → exitCode
        Mock::given(method("GET"))
            .and(path(format!(
                "/{sandbox_id}/process/session/everruns-exec/command/cmd_001"
            )))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "cmd_001",
                "command": "test",
                "exitCode": exit_code
            })))
            .mount(mock_server)
            .await;
    }

    #[tokio::test]
    async fn test_client_exec_detects_dead_session() {
        use std::sync::atomic::{AtomicU32, Ordering};

        let mock_server = MockServer::start().await;
        let exec_call_count = std::sync::Arc::new(AtomicU32::new(0));

        // 1. Create session → 201
        Mock::given(method("POST"))
            .and(path("/sb_dead/process/session"))
            .and(wiremock::matchers::body_json(
                json!({"sessionId":"everruns-exec"}),
            ))
            .respond_with(ResponseTemplate::new(201))
            .expect(2)
            .mount(&mock_server)
            .await;

        Mock::given(method("POST"))
            .and(path("/sb_dead/process/session"))
            .and(wiremock::matchers::body_json(
                json!({"sessionId":"everruns-heartbeat"}),
            ))
            .respond_with(ResponseTemplate::new(201))
            .expect(1)
            .mount(&mock_server)
            .await;

        // 2. Exec calls to the main session → 202 with unique cmdId.
        let counter = exec_call_count.clone();
        Mock::given(method("POST"))
            .and(path("/sb_dead/process/session/everruns-exec/exec"))
            .respond_with(move |_: &wiremock::Request| {
                let n = counter.fetch_add(1, Ordering::SeqCst);
                ResponseTemplate::new(202).set_body_json(json!({
                    "cmdId": format!("cmd_{n}")
                }))
            })
            .mount(&mock_server)
            .await;

        // 2b. Heartbeat session exec calls → 202 with unique cmdId.
        //     The heartbeat uses a separate session (everruns-heartbeat).
        //     In a dead-sandbox scenario, this session also can't complete.
        let hb_counter = std::sync::Arc::new(AtomicU32::new(100));
        let hb_counter_clone = hb_counter.clone();
        Mock::given(method("POST"))
            .and(path("/sb_dead/process/session/everruns-heartbeat/exec"))
            .respond_with(move |_: &wiremock::Request| {
                let n = hb_counter_clone.fetch_add(1, Ordering::SeqCst);
                ResponseTemplate::new(202).set_body_json(json!({
                    "cmdId": format!("hb_{n}")
                }))
            })
            .mount(&mock_server)
            .await;

        // 3. All command logs → 200 with empty body (dead shell produces nothing)
        Mock::given(method("GET"))
            .and(wiremock::matchers::path_regex(
                r"/sb_dead/process/session/everruns-exec/command/cmd_\d+/logs",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![]))
            .mount(&mock_server)
            .await;

        // 4. All command status → 200 with no exitCode (dead shell never reports)
        Mock::given(method("GET"))
            .and(wiremock::matchers::path_regex(
                r"/sb_dead/process/session/everruns-exec/command/cmd_\d+$",
            ))
            .respond_with(|req: &wiremock::Request| {
                let path = req.url.path();
                let cmd_id = path.rsplit('/').next().unwrap_or("unknown");
                ResponseTemplate::new(200).set_body_json(json!({
                    "id": cmd_id,
                    "command": "exit 1"
                }))
            })
            .mount(&mock_server)
            .await;

        // 4b. Heartbeat command status → 200 with no exitCode (sandbox is dead)
        Mock::given(method("GET"))
            .and(wiremock::matchers::path_regex(
                r"/sb_dead/process/session/everruns-heartbeat/command/hb_\d+$",
            ))
            .respond_with(|req: &wiremock::Request| {
                let path = req.url.path();
                let cmd_id = path.rsplit('/').next().unwrap_or("unknown");
                ResponseTemplate::new(200).set_body_json(json!({
                    "id": cmd_id,
                    "command": "true"
                }))
            })
            .mount(&mock_server)
            .await;

        // 5. DELETE session → 204
        Mock::given(method("DELETE"))
            .and(path("/sb_dead/process/session/everruns-exec"))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&mock_server)
            .await;

        let client = DaytonaClient::with_base_urls(
            "test_key".to_string(),
            mock_server.uri(),
            mock_server.uri(),
        );
        // Timeout must exceed stale detection window + heartbeat so the
        // dead-session path fires before the exec timeout path.
        let detection_window_ms = (crate::SESSION_STALE_THRESHOLD as u64)
            * crate::EXEC_POLL_INTERVAL.as_millis() as u64
            + crate::SESSION_HEARTBEAT_TIMEOUT_MS
            + 5_000; // buffer
        let result = client
            .exec("sb_dead", "exit 1", None, Some(detection_window_ms), |_| {})
            .await;
        assert!(result.is_err(), "Should detect dead session");
        let err = result.unwrap_err();
        assert!(
            err.contains("Session shell terminated unexpectedly"),
            "Error should mention session termination, got: {err}"
        );

        // Verify the heartbeat was attempted on the dedicated heartbeat session
        // (separate from the exec session). exec_call_count only tracks the
        // exec session; hb_counter tracks the heartbeat session.
        assert_eq!(
            exec_call_count.load(Ordering::SeqCst),
            1,
            "Expected exactly 1 exec call (the original command)"
        );
        assert!(
            hb_counter.load(Ordering::SeqCst) > 100,
            "Expected at least 1 heartbeat probe on the heartbeat session, got {}",
            hb_counter.load(Ordering::SeqCst) - 100
        );
    }

    #[tokio::test]
    async fn test_client_file_list() {
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/sb_test/files/"))
            .and(wiremock::matchers::header(
                "authorization",
                "Bearer test_key",
            ))
            .and(wiremock::matchers::query_param("path", "/sandbox"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([
                {"name": "main.py", "isDir": false, "size": 42},
                {"name": "src", "isDir": true, "size": 0}
            ])))
            .expect(1)
            .mount(&mock_server)
            .await;

        let client = DaytonaClient::with_base_urls(
            "test_key".to_string(),
            mock_server.uri(),
            mock_server.uri(),
        );
        let result = client.file_list("sb_test", "/sandbox").await;
        assert!(result.is_ok());
        let entries = result.unwrap();
        assert_eq!(
            entries,
            vec![
                json!({"name":"main.py","isDir":false,"size":42}),
                json!({"name":"src","isDir":true,"size":0})
            ]
        );
    }

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
        assert_eq!(
            result.unwrap_err(),
            "Daytona API error (404 Not Found): {\"error\":\"Sandbox not found\"}"
        );
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
        assert_eq!(
            result.unwrap_err(),
            "Daytona API error (401 Unauthorized): {\"error\":\"Unauthorized\"}"
        );
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
        assert_eq!(
            result.unwrap_err(),
            "Daytona API error (500 Internal Server Error): Internal Server Error"
        );
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
        assert!(
            result
                .unwrap_err()
                .starts_with("Invalid JSON from Daytona:")
        );
    }

    #[test]
    fn query_encoding_preserves_unreserved_and_escapes_path_bytes() {
        for (input, expected) in [
            ("", ""),
            ("hello", "hello"),
            ("abc-_.~123", "abc-_.~123"),
            ("/sandbox/main.py", "%2Fsandbox%2Fmain.py"),
            (
                "/my project/file name.txt",
                "%2Fmy%20project%2Ffile%20name.txt",
            ),
            ("hello@world#test", "hello%40world%23test"),
            ("&?%=+", "%26%3F%25%3D%2B"),
            ("é/🙂", "%C3%A9%2F%F0%9F%99%82"),
        ] {
            assert_eq!(urlencoding::encode(input), expected, "{input:?}");
        }
    }

    #[tokio::test]
    async fn test_client_start_sandbox() {
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/sandbox/sb_test/start"))
            .and(wiremock::matchers::header(
                "authorization",
                "Bearer test_key",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_string(""))
            .expect(1)
            .mount(&mock_server)
            .await;

        let client = DaytonaClient::with_base_urls(
            "test_key".to_string(),
            mock_server.uri(),
            mock_server.uri(),
        );
        let result = client.start_sandbox("sb_test").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_client_set_autostop() {
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/sandbox/sb_test/autostop/5"))
            .and(wiremock::matchers::header(
                "authorization",
                "Bearer test_key",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_string(""))
            .expect(1)
            .mount(&mock_server)
            .await;

        let client = DaytonaClient::with_base_urls(
            "test_key".to_string(),
            mock_server.uri(),
            mock_server.uri(),
        );
        let result = client.set_autostop("sb_test", 5).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_client_file_download() {
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/sb_test/files/download"))
            .and(wiremock::matchers::header(
                "authorization",
                "Bearer test_key",
            ))
            .and(wiremock::matchers::query_param(
                "path",
                "/my project/file &?#.bin",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![0, 255, 128, 10, 65]))
            .expect(1)
            .mount(&mock_server)
            .await;

        let client = DaytonaClient::with_base_urls(
            "test_key".to_string(),
            mock_server.uri(),
            mock_server.uri(),
        );
        let result = client
            .file_download("sb_test", "/my project/file &?#.bin")
            .await;
        assert!(result.is_ok());
        let bytes = result.unwrap();
        assert_eq!(bytes, vec![0, 255, 128, 10, 65]);
    }

    #[tokio::test]
    async fn test_client_file_delete() {
        let mock_server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path("/sb_test/files"))
            .and(wiremock::matchers::header(
                "authorization",
                "Bearer test_key",
            ))
            .and(wiremock::matchers::query_param("path", "/old &?#.txt"))
            .respond_with(ResponseTemplate::new(200).set_body_string(""))
            .expect(1)
            .mount(&mock_server)
            .await;

        let client = DaytonaClient::with_base_urls(
            "test_key".to_string(),
            mock_server.uri(),
            mock_server.uri(),
        );
        let result = client.file_delete("sb_test", "/old &?#.txt").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_client_create_folder() {
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/sb_test/files/folder"))
            .and(wiremock::matchers::header(
                "authorization",
                "Bearer test_key",
            ))
            .and(wiremock::matchers::query_param("path", "/src &dir"))
            .and(wiremock::matchers::query_param("mode", "0750"))
            .respond_with(ResponseTemplate::new(200).set_body_string(""))
            .expect(1)
            .mount(&mock_server)
            .await;

        let client = DaytonaClient::with_base_urls(
            "test_key".to_string(),
            mock_server.uri(),
            mock_server.uri(),
        );
        let result = client.create_folder("sb_test", "/src &dir", "0750").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_client_file_upload() {
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/sb_test/files/upload"))
            .and(wiremock::matchers::header(
                "authorization",
                "Bearer test_key",
            ))
            .and(wiremock::matchers::query_param(
                "path",
                "/sandbox/test &data.bin",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_string(""))
            .expect(1)
            .mount(&mock_server)
            .await;

        let client = DaytonaClient::with_base_urls(
            "test_key".to_string(),
            mock_server.uri(),
            mock_server.uri(),
        );
        let result = client
            .file_upload("sb_test", "/sandbox/test &data.bin", &[0, 255, 128, 10, 65])
            .await;
        result.unwrap();
        let requests = mock_server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 1);
        let request = &requests[0];
        assert!(
            request
                .body
                .windows(5)
                .any(|bytes| bytes == [0, 255, 128, 10, 65])
        );
        let envelope = String::from_utf8_lossy(&request.body);
        assert!(envelope.contains("name=\"file\"; filename=\"test &data.bin\""));
        assert!(envelope.contains("application/octet-stream"));
    }

    #[tokio::test(start_paused = true)]
    async fn readiness_timeout_preserves_backoff_without_wall_clock_waiting() {
        let start = tokio::time::Instant::now();
        let mut polls = Vec::new();
        let result = tokio::time::timeout(
            Duration::from_secs(63),
            poll_sandbox_ready(|| {
                polls.push(start.elapsed().as_secs());
                std::future::ready(Ok("creating".to_owned()))
            }),
        )
        .await
        .expect("readiness loop must enforce its own timeout");
        assert_eq!(
            result,
            Err("Sandbox did not become ready within 60s".into())
        );
        assert_eq!(
            polls,
            vec![0, 2, 6, 10, 14, 18, 22, 26, 30, 34, 38, 42, 46, 50, 54, 58]
        );
        assert_eq!(start.elapsed(), Duration::from_secs(62));
    }

    #[tokio::test(start_paused = true)]
    async fn readiness_retries_transient_errors_and_stops_when_started() {
        let start = tokio::time::Instant::now();
        let mut polls = Vec::new();
        let mut states = std::collections::VecDeque::from([
            Ok("creating".to_owned()),
            Err("temporary transport failure".to_owned()),
            Ok("started".to_owned()),
        ]);
        let result = poll_sandbox_ready(|| {
            polls.push(start.elapsed().as_secs());
            std::future::ready(states.pop_front().expect("must stop polling after started"))
        })
        .await;
        assert_eq!(result, Ok(()));
        assert_eq!(polls, vec![0, 2, 6]);
    }

    #[tokio::test(start_paused = true)]
    async fn readiness_terminal_states_do_not_sleep_or_retry() {
        for (state, expected) in [
            ("started", Ok(())),
            (
                "error",
                Err("Sandbox entered error state: error".to_owned()),
            ),
            (
                "build_failed",
                Err("Sandbox entered error state: build_failed".to_owned()),
            ),
        ] {
            let start = tokio::time::Instant::now();
            let mut count = 0;
            let result = poll_sandbox_ready(|| {
                count += 1;
                assert_eq!(count, 1, "terminal state must stop polling");
                std::future::ready(Ok(state.to_owned()))
            })
            .await;
            assert_eq!(result, expected);
            assert_eq!(start.elapsed(), Duration::ZERO);
        }
    }

    #[tokio::test]
    async fn test_client_wait_for_ready_already_started() {
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/sandbox/sb_test"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "sb_test",
                "state": "started"
            })))
            .mount(&mock_server)
            .await;

        let client = DaytonaClient::with_base_urls(
            "test_key".to_string(),
            mock_server.uri(),
            mock_server.uri(),
        );
        let result = client.wait_for_ready("sb_test").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_client_wait_for_ready_error_state() {
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/sandbox/sb_test"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "sb_test",
                "state": "error"
            })))
            .mount(&mock_server)
            .await;

        let client = DaytonaClient::with_base_urls(
            "test_key".to_string(),
            mock_server.uri(),
            mock_server.uri(),
        );
        let result = client.wait_for_ready("sb_test").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("error state"));
    }

    #[tokio::test]
    async fn test_client_file_download_error() {
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/sb_test/files/download"))
            .respond_with(ResponseTemplate::new(404).set_body_string("Not Found"))
            .mount(&mock_server)
            .await;

        let client = DaytonaClient::with_base_urls(
            "test_key".to_string(),
            mock_server.uri(),
            mock_server.uri(),
        );
        let result = client.file_download("sb_test", "/missing.txt").await;
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            "Sandbox API error (404 Not Found): Not Found"
        );
    }

    #[tokio::test]
    async fn test_client_file_upload_error() {
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/sb_test/files/upload"))
            .respond_with(ResponseTemplate::new(500).set_body_string("Disk full"))
            .mount(&mock_server)
            .await;

        let client = DaytonaClient::with_base_urls(
            "test_key".to_string(),
            mock_server.uri(),
            mock_server.uri(),
        );
        let result = client.file_upload("sb_test", "/test.txt", b"content").await;
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            "File upload error (500 Internal Server Error): Disk full"
        );
    }

    #[tokio::test]
    async fn test_client_sandbox_info_optional_name() {
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/sandbox/sb_test"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "sb_test",
                "state": "started"
            })))
            .mount(&mock_server)
            .await;

        let client = DaytonaClient::with_base_urls(
            "test_key".to_string(),
            mock_server.uri(),
            mock_server.uri(),
        );
        let info = client.get_sandbox("sb_test").await.unwrap();
        assert_eq!(info.name, None);
    }

    #[tokio::test]
    async fn test_client_file_list_single_entry() {
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/sb_test/files/"))
            .and(wiremock::matchers::header(
                "authorization",
                "Bearer test_key",
            ))
            .and(wiremock::matchers::query_param("path", "/home"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"name": "only.txt", "isDir": false})),
            )
            .expect(1)
            .mount(&mock_server)
            .await;

        let client = DaytonaClient::with_base_urls(
            "test_key".to_string(),
            mock_server.uri(),
            mock_server.uri(),
        );
        let entries = client.file_list("sb_test", "/home").await.unwrap();
        assert_eq!(entries, vec![json!({"name":"only.txt","isDir":false})]);
    }

    #[tokio::test]
    async fn test_client_create_sandbox_forwards_labels() {
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/sandbox"))
            .and(wiremock::matchers::body_json(json!({
                "name": "Labeled Sandbox",
                "autoStopInterval": 5,
                "autoArchiveInterval": 30,
                "autoDeleteInterval": 60,
                "labels": {
                    "everruns": "true",
                    "everruns.session_id": "session_abc",
                    "everruns.org_id": "org_123"
                }
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "sb_labeled",
                "name": "Labeled Sandbox",
                "state": "started"
            })))
            .expect(1)
            .mount(&mock_server)
            .await;

        let client = DaytonaClient::with_base_urls(
            "test_key".to_string(),
            mock_server.uri(),
            mock_server.uri(),
        );
        let result = client
            .create_sandbox(json!({
                "name": "Labeled Sandbox",
                "autoStopInterval": 5,
                "autoArchiveInterval": 30,
                "autoDeleteInterval": 60,
                "labels": {
                    "everruns": "true",
                    "everruns.session_id": "session_abc",
                    "everruns.org_id": "org_123"
                }
            }))
            .await;
        assert!(result.is_ok());
        let info = result.unwrap();
        assert_eq!(info.id, "sb_labeled");
    }

    #[tokio::test]
    async fn test_exec_streaming_collects_output_and_exit_code() {
        use std::sync::{Arc, Mutex};

        let mock_server = MockServer::start().await;
        setup_session_mocks(&mock_server, "sb_test", 0, "hello streaming\n").await;

        let client = DaytonaClient::with_base_urls(
            "test_key".to_string(),
            mock_server.uri(),
            mock_server.uri(),
        );

        let chunks: Arc<Mutex<Vec<ExecOutputChunk>>> = Arc::new(Mutex::new(Vec::new()));
        let chunks_clone = chunks.clone();

        let result = client
            .exec(
                "sb_test",
                "echo hello streaming",
                None,
                Some(30_000),
                |chunk| {
                    chunks_clone.lock().unwrap().push(chunk);
                },
            )
            .await;

        assert!(result.is_ok(), "exec_streaming failed: {:?}", result.err());
        let exec_result = result.unwrap();
        assert_eq!(exec_result.exit_code, 0);
        assert_eq!(exec_result.stdout, "hello streaming\n");
        assert_eq!(exec_result.stderr, "");
        assert_eq!(exec_result.result, "hello streaming\n");

        assert_eq!(
            *chunks.lock().unwrap(),
            vec![ExecOutputChunk {
                stream: ExecStream::Stdout,
                text: "hello streaming\n".into()
            }]
        );
    }

    #[tokio::test]
    async fn test_ensure_session_handles_existing() {
        let mock_server = MockServer::start().await;

        // Return 409 Conflict (session already exists)
        Mock::given(method("POST"))
            .and(path("/sb_test/process/session"))
            .and(wiremock::matchers::body_json(
                json!({"sessionId":"everruns-exec"}),
            ))
            .respond_with(ResponseTemplate::new(409))
            .expect(1)
            .mount(&mock_server)
            .await;

        let client = DaytonaClient::with_base_urls(
            "test_key".to_string(),
            mock_server.uri(),
            mock_server.uri(),
        );
        let result = client.ensure_session("sb_test").await;
        assert!(result.is_ok(), "409 should be treated as success");
    }

    #[tokio::test]
    async fn test_exec_timeout_resets_session_and_next_command_succeeds() {
        use std::sync::atomic::{AtomicU32, Ordering};

        let mock_server = MockServer::start().await;
        let exec_call_count = std::sync::Arc::new(AtomicU32::new(0));

        Mock::given(method("POST"))
            .and(path("/sb_timeout/process/session"))
            .and(wiremock::matchers::body_json(
                json!({"sessionId":"everruns-exec"}),
            ))
            .respond_with(ResponseTemplate::new(201))
            .expect(3)
            .mount(&mock_server)
            .await;

        let counter = exec_call_count.clone();
        Mock::given(method("POST"))
            .and(path("/sb_timeout/process/session/everruns-exec/exec"))
            .respond_with(move |_: &wiremock::Request| {
                let n = counter.fetch_add(1, Ordering::SeqCst);
                let cmd_id = if n == 0 { "cmd_timeout" } else { "cmd_ok" };
                ResponseTemplate::new(202).set_body_json(json!({ "cmdId": cmd_id }))
            })
            .mount(&mock_server)
            .await;

        Mock::given(method("GET"))
            .and(path(
                "/sb_timeout/process/session/everruns-exec/command/cmd_timeout/logs",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![]))
            .mount(&mock_server)
            .await;
        Mock::given(method("GET"))
            .and(path(
                "/sb_timeout/process/session/everruns-exec/command/cmd_timeout",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "cmd_timeout",
                "command": "sleep 999"
            })))
            .mount(&mock_server)
            .await;

        let mut ok_log_bytes = vec![0x01, 0x01, 0x01];
        ok_log_bytes.extend_from_slice(b"still works\n");
        Mock::given(method("GET"))
            .and(path(
                "/sb_timeout/process/session/everruns-exec/command/cmd_ok/logs",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(ok_log_bytes))
            .mount(&mock_server)
            .await;
        Mock::given(method("GET"))
            .and(path(
                "/sb_timeout/process/session/everruns-exec/command/cmd_ok",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "cmd_ok",
                "command": "echo still works",
                "exitCode": 0
            })))
            .mount(&mock_server)
            .await;

        Mock::given(method("DELETE"))
            .and(path("/sb_timeout/process/session/everruns-exec"))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&mock_server)
            .await;

        let client = DaytonaClient::with_base_urls(
            "test_key".to_string(),
            mock_server.uri(),
            mock_server.uri(),
        );

        let timeout_err = client
            .exec("sb_timeout", "sleep 999", None, Some(1_200), |_| {})
            .await
            .unwrap_err();
        assert!(timeout_err.contains("Command timed out after"));
        assert!(timeout_err.contains("automatically reset"));

        let result = client
            .exec("sb_timeout", "echo still works", None, Some(5_000), |_| {})
            .await
            .unwrap();
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.stdout, "still works\n");
        assert_eq!(result.stderr, "");
        assert_eq!(result.result, "still works\n");
        assert_eq!(exec_call_count.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn test_strip_stream_markers_stdout_only() {
        let mut raw = vec![0x01, 0x01, 0x01];
        raw.extend_from_slice(b"hello world\n");
        let parsed = split_stream_markers(&raw);
        assert_eq!(parsed.stdout, "hello world\n");
        assert_eq!(parsed.stderr, "");
        assert_eq!(parsed.combined, "hello world\n");
    }

    #[test]
    fn test_strip_stream_markers_mixed_streams() {
        let mut raw = Vec::new();
        raw.extend_from_slice(&[0x01, 0x01, 0x01]);
        raw.extend_from_slice(b"stdout line\n");
        raw.extend_from_slice(&[0x02, 0x02, 0x02]);
        raw.extend_from_slice(b"stderr line\n");
        raw.extend_from_slice(&[0x01, 0x01, 0x01]);
        raw.extend_from_slice(b"more stdout\n");
        let parsed = split_stream_markers(&raw);
        assert_eq!(parsed.stdout, "stdout line\nmore stdout\n");
        assert_eq!(parsed.stderr, "stderr line\n");
        assert_eq!(parsed.combined, "stdout line\nstderr line\nmore stdout\n");
    }

    #[test]
    fn test_strip_stream_markers_no_markers() {
        let parsed = split_stream_markers(b"plain text\n");
        assert_eq!(parsed.stdout, "plain text\n");
        assert_eq!(parsed.stderr, "");
        assert_eq!(parsed.combined, "plain text\n");
    }

    #[test]
    fn test_strip_stream_markers_empty() {
        let parsed = split_stream_markers(b"");
        assert_eq!(parsed.stdout, "");
        assert_eq!(parsed.stderr, "");
        assert_eq!(parsed.combined, "");
    }

    #[test]
    fn test_parse_exec_output_delta_preserves_streams() {
        let mut raw = Vec::new();
        raw.extend_from_slice(&STDOUT_MARKER);
        raw.extend_from_slice(b"stdout line\n");
        raw.extend_from_slice(&STDERR_MARKER);
        raw.extend_from_slice(b"stderr line\n");

        let mut parser = ExecDeltaParserState::new(ExecStream::Stdout);
        let parsed = parse_exec_output_delta(&raw, &mut parser);
        assert_eq!(
            parsed.chunks,
            vec![
                ExecOutputChunk {
                    stream: ExecStream::Stdout,
                    text: "stdout line\n".to_string(),
                },
                ExecOutputChunk {
                    stream: ExecStream::Stderr,
                    text: "stderr line\n".to_string(),
                },
            ]
        );
        assert_eq!(parser.current_stream, ExecStream::Stderr);
        assert!(parser.pending_marker_prefix.is_empty());
    }

    #[test]
    fn test_parse_exec_output_delta_handles_split_marker_across_polls() {
        let mut parser = ExecDeltaParserState::new(ExecStream::Stdout);

        let mut first = Vec::new();
        first.extend_from_slice(&STDOUT_MARKER);
        first.extend_from_slice(b"stdout line\n");
        first.extend_from_slice(&STDERR_MARKER[..2]);

        let parsed_first = parse_exec_output_delta(&first, &mut parser);
        assert_eq!(
            parsed_first.chunks,
            vec![ExecOutputChunk {
                stream: ExecStream::Stdout,
                text: "stdout line\n".to_string(),
            }]
        );
        assert_eq!(parser.current_stream, ExecStream::Stdout);
        assert_eq!(parser.pending_marker_prefix, STDERR_MARKER[..2].to_vec());

        let mut second = Vec::new();
        second.extend_from_slice(&STDERR_MARKER[2..]);
        second.extend_from_slice(b"stderr line\n");

        let parsed_second = parse_exec_output_delta(&second, &mut parser);
        assert_eq!(
            parsed_second.chunks,
            vec![ExecOutputChunk {
                stream: ExecStream::Stderr,
                text: "stderr line\n".to_string(),
            }]
        );
        assert_eq!(parser.current_stream, ExecStream::Stderr);
        assert!(parser.pending_marker_prefix.is_empty());
    }
    #[test]
    fn streaming_output_preserves_utf8_at_every_poll_boundary() {
        let raw = "\x01\x01\x01hé🙂\x02\x02\x02érror界\x01\x01\x01fin".as_bytes();
        for cut in 0..=raw.len() {
            let mut state = ExecDeltaParserState::new(ExecStream::Stdout);
            let mut chunks = parse_exec_output_delta(&raw[..cut], &mut state).chunks;
            chunks.extend(parse_exec_output_delta(&raw[cut..], &mut state).chunks);
            chunks.extend(finish_exec_output_delta(&mut state).chunks);
            let stdout: String = chunks
                .iter()
                .filter(|c| c.stream == ExecStream::Stdout)
                .map(|c| c.text.as_str())
                .collect();
            let stderr: String = chunks
                .iter()
                .filter(|c| c.stream == ExecStream::Stderr)
                .map(|c| c.text.as_str())
                .collect();
            assert_eq!(stdout, "hé🙂fin", "poll split {cut}");
            assert_eq!(stderr, "érror界", "poll split {cut}");
        }
    }

    #[test]
    fn streaming_output_flushes_incomplete_text_once_on_finish_or_stream_change() {
        for suffix in [b"".as_slice(), b"\x01", b"\x02\x02"] {
            let mut state = ExecDeltaParserState::new(ExecStream::Stdout);
            let mut raw = b"ok\xff\xe2\x82".to_vec();
            raw.extend_from_slice(suffix);
            let first = parse_exec_output_delta(&raw, &mut state);
            assert_eq!(
                first.chunks,
                vec![ExecOutputChunk {
                    stream: ExecStream::Stdout,
                    text: "ok�".into()
                }]
            );
            let last = finish_exec_output_delta(&mut state);
            assert_eq!(
                last.chunks,
                vec![ExecOutputChunk {
                    stream: ExecStream::Stdout,
                    text: format!("�{}", String::from_utf8_lossy(suffix))
                }]
            );
            assert!(finish_exec_output_delta(&mut state).chunks.is_empty());
        }
        let mut state = ExecDeltaParserState::new(ExecStream::Stdout);
        assert!(
            parse_exec_output_delta(b"\xe2", &mut state)
                .chunks
                .is_empty()
        );
        let next = parse_exec_output_delta(b"\x02\x02\x02err", &mut state);
        assert_eq!(
            next.chunks,
            vec![
                ExecOutputChunk {
                    stream: ExecStream::Stdout,
                    text: "�".into()
                },
                ExecOutputChunk {
                    stream: ExecStream::Stderr,
                    text: "err".into()
                },
            ]
        );
    }
    #[test]
    fn exec_shell_loads_profiles_silently_and_keeps_cwd_literal_and_exit_local() {
        let home = tempfile::tempdir().unwrap();
        for (path, variable, value) in [
            (".profile", "DAYTONA_TEST_PROFILE", "profile"),
            (".cargo/env", "DAYTONA_TEST_CARGO", "cargo"),
            (".nvm/nvm.sh", "DAYTONA_TEST_NVM", "nvm"),
        ] {
            let path = home.path().join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(
                path,
                format!("export {variable}={value}; echo noise; echo error >&2\n"),
            )
            .unwrap();
        }
        std::fs::write(home.path().join(".bashrc"), "echo must-not-source-bashrc\n").unwrap();
        let cwd = home.path().join("project's $(printf substituted) files");
        std::fs::create_dir(&cwd).unwrap();
        let command = "printf '%s|%s|%s\\n' \"$DAYTONA_TEST_PROFILE\" \"$DAYTONA_TEST_CARGO\" \"$DAYTONA_TEST_NVM\"; pwd -P; exit 7";
        for requested_cwd in [None, Some(cwd.to_str().unwrap())] {
            let generated = format_exec_command(command, requested_cwd);
            let output = std::process::Command::new("/bin/sh")
                .arg("-c")
                .arg(format!("{generated}; printf 'session-alive\\n'"))
                .env("HOME", home.path())
                .env_remove("DAYTONA_TEST_PROFILE")
                .env_remove("DAYTONA_TEST_CARGO")
                .env_remove("DAYTONA_TEST_NVM")
                .current_dir(home.path())
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "cwd={requested_cwd:?}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(
                output.stderr.is_empty(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let expected_cwd = if requested_cwd.is_some() {
                &cwd
            } else {
                &home.path().to_path_buf()
            };
            assert_eq!(
                String::from_utf8(output.stdout).unwrap(),
                format!(
                    "profile|cargo|nvm\n{}\nsession-alive\n",
                    expected_cwd.canonicalize().unwrap().display()
                )
            );
        }
    }
    #[tokio::test]
    async fn exec_preserves_status_output_and_command_options() {
        for (command, cwd, timeout, exit_code, output, suffix) in [
            ("echo hello", None, None, 0, "hello\n", "( echo hello )"),
            (
                "ls",
                Some("/my project's files"),
                Some(5000),
                1,
                "failed\n",
                "( cd '/my project'\"'\"'s files' && ls )",
            ),
            (
                "nonexistent",
                None,
                Some(5000),
                127,
                "command not found",
                "( nonexistent )",
            ),
        ] {
            let server = MockServer::start().await;
            setup_session_mocks(&server, "sb_test", exit_code, output).await;
            let client =
                DaytonaClient::with_base_urls("test_key".into(), server.uri(), server.uri());
            let result = client
                .exec("sb_test", command, cwd, timeout, |_| {})
                .await
                .unwrap();
            assert_eq!(i64::from(result.exit_code), exit_code);
            assert_eq!(result.stdout, output);
            assert_eq!(result.stderr, "");
            assert_eq!(result.result, output);
            let requests = server.received_requests().await.unwrap();
            let exec: Vec<_> = requests
                .iter()
                .filter(|r| r.url.path().ends_with("/exec"))
                .collect();
            assert_eq!(exec.len(), 1);
            // Profile behavior has an independent real-shell oracle below.
            assert_eq!(
                exec[0].body_json::<Value>().unwrap(),
                json!({
                    "command": format!("{SHELL_PROFILE_PREAMBLE}{suffix}"), "runAsync": true
                })
            );
        }
    }
}

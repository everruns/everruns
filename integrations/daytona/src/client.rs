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

use crate::state::{ExecResult, SandboxInfo, SnapshotInfo};
use crate::{EXEC_POLL_INTERVAL, SANDBOX_READY_MAX_WAIT, SANDBOX_READY_POLL_INTERVAL};

/// Fixed session ID used for all command execution in a sandbox.
/// One session per sandbox, created on first exec, reused thereafter.
const EXEC_SESSION_ID: &str = "everruns-exec";

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

    /// Probe session health by running a trivial command and checking if
    /// it completes within a few seconds. Returns `true` if the session
    /// shell is responsive, `false` if the heartbeat stalls (dead shell).
    async fn session_heartbeat(&self, sandbox_id: &str) -> bool {
        // Fire a heartbeat command.
        let resp = self
            .toolbox_request(
                reqwest::Method::POST,
                sandbox_id,
                &format!("/process/session/{EXEC_SESSION_ID}/exec"),
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

        let status_path = format!("/process/session/{EXEC_SESSION_ID}/command/{cmd_id}");

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
        F: FnMut(&str),
    {
        self.ensure_session(sandbox_id).await?;

        let cmd = match cwd {
            Some(c) => format!("{SHELL_PROFILE_PREAMBLE}cd {c} && {command}"),
            None => format!("{SHELL_PROFILE_PREAMBLE}{command}"),
        };

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
        let mut output_emitted: usize = 0;
        let mut stale_polls: u32 = 0;
        let logs_path = format!("/process/session/{EXEC_SESSION_ID}/command/{cmd_id}/logs");
        let status_path = format!("/process/session/{EXEC_SESSION_ID}/command/{cmd_id}");

        loop {
            if std::time::Instant::now() >= deadline {
                return Err(format!(
                    "Command timed out after {timeout}ms. \
                     Increase the timeout parameter or break the command into smaller steps."
                ));
            }

            tokio::time::sleep(EXEC_POLL_INTERVAL).await;

            // Fetch session command logs (raw bytes with stream markers).
            let logs_ok = self.toolbox_download(sandbox_id, &logs_path).await.ok();

            let text = logs_ok
                .as_deref()
                .map(strip_stream_markers)
                .unwrap_or_default();
            let had_new_output = text.len() > output_emitted;
            if had_new_output {
                on_output(&text[output_emitted..]);
                output_emitted = text.len();
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

                let final_text = strip_stream_markers(&final_logs);
                if final_text.len() > output_emitted {
                    on_output(&final_text[output_emitted..]);
                }

                return Ok(ExecResult {
                    result: final_text,
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

/// Strip Daytona session log stream multiplexing markers from raw output.
///
/// The session API multiplexes stdout/stderr with 3-byte prefix markers:
/// - `\x01\x01\x01` = stdout data follows
/// - `\x02\x02\x02` = stderr data follows
///
/// We strip the markers and return combined text (matching the previous
/// behavior where `ExecResult.result` contains combined stdout+stderr).
pub(crate) fn strip_stream_markers(raw: &[u8]) -> String {
    let mut result = Vec::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        if i + 3 <= raw.len()
            && ((raw[i] == 0x01 && raw[i + 1] == 0x01 && raw[i + 2] == 0x01)
                || (raw[i] == 0x02 && raw[i + 1] == 0x02 && raw[i + 2] == 0x02))
        {
            i += 3;
            continue;
        }
        result.push(raw[i]);
        i += 1;
    }
    String::from_utf8_lossy(&result).into_owned()
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

    /// Set up mock responses for the session-based exec flow.
    async fn setup_session_mocks(
        mock_server: &MockServer,
        sandbox_id: &str,
        exit_code: i64,
        output: &str,
    ) {
        // 1. Create session → 201 (or 409 if exists)
        Mock::given(method("POST"))
            .and(path(format!("/{sandbox_id}/process/session")))
            .respond_with(ResponseTemplate::new(201))
            .mount(mock_server)
            .await;

        // 2. Async exec → 202 with cmdId
        Mock::given(method("POST"))
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
    async fn test_client_exec() {
        let mock_server = MockServer::start().await;
        setup_session_mocks(&mock_server, "sb_test", 0, "hello world\n").await;

        let client = DaytonaClient::with_base_urls(
            "test_key".to_string(),
            mock_server.uri(),
            mock_server.uri(),
        );
        let result = client
            .exec("sb_test", "echo hello world", None, None, |_| {})
            .await;
        assert!(result.is_ok());
        let exec_result = result.unwrap();
        assert_eq!(exec_result.exit_code, 0);
        assert_eq!(exec_result.result, "hello world\n");
    }

    /// Simulates Daytona's real behavior when `exit` kills the session shell:
    /// - Session metadata stays alive (GET /session → 200)
    /// - Command status returns 200 with no exitCode (forever)
    /// - Logs return 200 with empty body
    /// - Heartbeat exec accepts (202) but never completes
    /// After enough stale polls + failed heartbeat, the client detects
    /// the dead session, resets it (DELETE + POST), and returns an error.
    #[tokio::test]
    async fn test_client_exec_detects_dead_session() {
        use std::sync::atomic::{AtomicU32, Ordering};

        let mock_server = MockServer::start().await;
        let exec_call_count = std::sync::Arc::new(AtomicU32::new(0));

        // 1. Create session → 201
        Mock::given(method("POST"))
            .and(path("/sb_dead/process/session"))
            .respond_with(ResponseTemplate::new(201))
            .mount(&mock_server)
            .await;

        // 2. All exec calls → 202 with unique cmdId (both the real command
        //    and the heartbeat probe). Use a counter to generate unique IDs.
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

        // 5. DELETE session → 204
        Mock::given(method("DELETE"))
            .and(path("/sb_dead/process/session/everruns-exec"))
            .respond_with(ResponseTemplate::new(204))
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

        // Verify the heartbeat was attempted (at least 2 exec calls: the
        // original command + the heartbeat probe).
        assert!(
            exec_call_count.load(Ordering::SeqCst) >= 2,
            "Expected at least 2 exec calls (command + heartbeat), got {}",
            exec_call_count.load(Ordering::SeqCst)
        );
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
        let result = client.file_list("sb_test", "/sandbox").await;
        assert!(result.is_ok());
        let entries = result.unwrap();
        assert_eq!(entries.len(), 2);
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
        assert!(result.unwrap_err().contains("404"));
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
        assert!(result.unwrap_err().contains("401"));
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
        assert!(result.unwrap_err().contains("500"));
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
        assert!(result.unwrap_err().contains("Invalid JSON"));
    }

    #[test]
    fn test_encode_simple() {
        assert_eq!(urlencoding::encode("hello"), "hello");
    }

    #[test]
    fn test_encode_path_with_slashes() {
        assert_eq!(
            urlencoding::encode("/sandbox/main.py"),
            "%2Fsandbox%2Fmain.py"
        );
    }

    #[test]
    fn test_encode_path_with_spaces() {
        let encoded = urlencoding::encode("/my project/file name.txt");
        assert!(encoded.contains("%20"));
    }

    #[test]
    fn test_encode_empty_string() {
        assert_eq!(urlencoding::encode(""), "");
    }

    #[test]
    fn test_encode_preserves_unreserved() {
        assert_eq!(urlencoding::encode("abc-_.~123"), "abc-_.~123");
    }

    #[test]
    fn test_encode_special_chars() {
        let encoded = urlencoding::encode("hello@world#test");
        assert_eq!(encoded, "hello%40world%23test");
    }

    #[tokio::test]
    async fn test_client_start_sandbox() {
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/sandbox/sb_test/start"))
            .respond_with(ResponseTemplate::new(200).set_body_string(""))
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
            .respond_with(ResponseTemplate::new(200).set_body_string(""))
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
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"file content here".to_vec()))
            .mount(&mock_server)
            .await;

        let client = DaytonaClient::with_base_urls(
            "test_key".to_string(),
            mock_server.uri(),
            mock_server.uri(),
        );
        let result = client.file_download("sb_test", "/main.py").await;
        assert!(result.is_ok());
        let bytes = result.unwrap();
        assert_eq!(String::from_utf8_lossy(&bytes), "file content here");
    }

    #[tokio::test]
    async fn test_client_file_delete() {
        let mock_server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path("/sb_test/files"))
            .respond_with(ResponseTemplate::new(200).set_body_string(""))
            .mount(&mock_server)
            .await;

        let client = DaytonaClient::with_base_urls(
            "test_key".to_string(),
            mock_server.uri(),
            mock_server.uri(),
        );
        let result = client.file_delete("sb_test", "/old.txt").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_client_create_folder() {
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/sb_test/files/folder"))
            .respond_with(ResponseTemplate::new(200).set_body_string(""))
            .mount(&mock_server)
            .await;

        let client = DaytonaClient::with_base_urls(
            "test_key".to_string(),
            mock_server.uri(),
            mock_server.uri(),
        );
        let result = client.create_folder("sb_test", "/src", "755").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_client_file_upload() {
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/sb_test/files/upload"))
            .respond_with(ResponseTemplate::new(200).set_body_string(""))
            .mount(&mock_server)
            .await;

        let client = DaytonaClient::with_base_urls(
            "test_key".to_string(),
            mock_server.uri(),
            mock_server.uri(),
        );
        let result = client
            .file_upload("sb_test", "/sandbox/test.py", b"print('hello')")
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_client_exec_with_cwd_and_timeout() {
        let mock_server = MockServer::start().await;
        setup_session_mocks(&mock_server, "sb_test", 1, "output").await;

        let client = DaytonaClient::with_base_urls(
            "test_key".to_string(),
            mock_server.uri(),
            mock_server.uri(),
        );
        let result = client
            .exec("sb_test", "ls", Some("/tmp"), Some(5000), |_| {})
            .await;
        assert!(result.is_ok());
        let exec_result = result.unwrap();
        assert_eq!(exec_result.exit_code, 1);
        assert_eq!(exec_result.result, "output");
    }

    #[tokio::test]
    async fn test_client_exec_nonzero_exit() {
        let mock_server = MockServer::start().await;
        setup_session_mocks(&mock_server, "sb_test", 127, "command not found").await;

        let client = DaytonaClient::with_base_urls(
            "test_key".to_string(),
            mock_server.uri(),
            mock_server.uri(),
        );
        let result = client
            .exec("sb_test", "nonexistent", None, None, |_| {})
            .await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap().exit_code, 127);
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
        assert!(result.unwrap_err().contains("404"));
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
        assert!(result.unwrap_err().contains("500"));
    }

    #[tokio::test]
    async fn test_client_empty_response_body() {
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/sandbox/sb_test/stop"))
            .respond_with(ResponseTemplate::new(200).set_body_string(""))
            .mount(&mock_server)
            .await;

        let client = DaytonaClient::with_base_urls(
            "test_key".to_string(),
            mock_server.uri(),
            mock_server.uri(),
        );
        // Empty body should be treated as success for void operations
        let result = client.stop_sandbox("sb_test").await;
        assert!(result.is_ok());
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
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([
                {"name": "only.txt", "isDir": false}
            ])))
            .mount(&mock_server)
            .await;

        let client = DaytonaClient::with_base_urls(
            "test_key".to_string(),
            mock_server.uri(),
            mock_server.uri(),
        );
        let entries = client.file_list("sb_test", "/home").await.unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["name"], "only.txt");
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

        let chunks: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let chunks_clone = chunks.clone();

        let result = client
            .exec(
                "sb_test",
                "echo hello streaming",
                None,
                Some(30_000),
                |chunk| {
                    chunks_clone.lock().unwrap().push(chunk.to_string());
                },
            )
            .await;

        assert!(result.is_ok(), "exec_streaming failed: {:?}", result.err());
        let exec_result = result.unwrap();
        assert_eq!(exec_result.exit_code, 0);
        assert_eq!(exec_result.result, "hello streaming\n");

        let collected = chunks.lock().unwrap();
        assert!(!collected.is_empty(), "should have received output chunks");
    }

    #[tokio::test]
    async fn test_ensure_session_handles_existing() {
        let mock_server = MockServer::start().await;

        // Return 409 Conflict (session already exists)
        Mock::given(method("POST"))
            .and(path("/sb_test/process/session"))
            .respond_with(ResponseTemplate::new(409))
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
    async fn test_exec_prepends_shell_profile_preamble() {
        use wiremock::matchers::body_json;

        let mock_server = MockServer::start().await;

        // Session create
        Mock::given(method("POST"))
            .and(path("/sb_preamble/process/session"))
            .respond_with(ResponseTemplate::new(201))
            .mount(&mock_server)
            .await;

        // Exec — match the exact command body to verify preamble is prepended.
        let expected_cmd = format!("{SHELL_PROFILE_PREAMBLE}echo hello");
        Mock::given(method("POST"))
            .and(path("/sb_preamble/process/session/everruns-exec/exec"))
            .and(body_json(json!({
                "command": expected_cmd,
                "runAsync": true
            })))
            .respond_with(ResponseTemplate::new(202).set_body_json(json!({"cmdId": "cmd_p1"})))
            .expect(1)
            .mount(&mock_server)
            .await;

        // Logs + status
        Mock::given(method("GET"))
            .and(path(
                "/sb_preamble/process/session/everruns-exec/command/cmd_p1/logs",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_bytes({
                let mut b = vec![0x01, 0x01, 0x01];
                b.extend_from_slice(b"hello\n");
                b
            }))
            .mount(&mock_server)
            .await;
        Mock::given(method("GET"))
            .and(path(
                "/sb_preamble/process/session/everruns-exec/command/cmd_p1",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"exitCode": 0})))
            .mount(&mock_server)
            .await;

        let client = DaytonaClient::with_base_urls(
            "test_key".to_string(),
            mock_server.uri(),
            mock_server.uri(),
        );
        let result = client
            .exec("sb_preamble", "echo hello", None, None, |_| {})
            .await;
        assert!(result.is_ok(), "exec failed: {:?}", result.err());
    }

    #[tokio::test]
    async fn test_exec_preamble_with_cwd() {
        use wiremock::matchers::body_json;

        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/sb_cwd/process/session"))
            .respond_with(ResponseTemplate::new(201))
            .mount(&mock_server)
            .await;

        // With cwd, command should be: PREAMBLE + cd /tmp && ls
        let expected_cmd = format!("{SHELL_PROFILE_PREAMBLE}cd /tmp && ls");
        Mock::given(method("POST"))
            .and(path("/sb_cwd/process/session/everruns-exec/exec"))
            .and(body_json(json!({
                "command": expected_cmd,
                "runAsync": true
            })))
            .respond_with(ResponseTemplate::new(202).set_body_json(json!({"cmdId": "cmd_c1"})))
            .expect(1)
            .mount(&mock_server)
            .await;

        Mock::given(method("GET"))
            .and(path(
                "/sb_cwd/process/session/everruns-exec/command/cmd_c1/logs",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_bytes({
                let mut b = vec![0x01, 0x01, 0x01];
                b.extend_from_slice(b"output\n");
                b
            }))
            .mount(&mock_server)
            .await;
        Mock::given(method("GET"))
            .and(path("/sb_cwd/process/session/everruns-exec/command/cmd_c1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"exitCode": 0})))
            .mount(&mock_server)
            .await;

        let client = DaytonaClient::with_base_urls(
            "test_key".to_string(),
            mock_server.uri(),
            mock_server.uri(),
        );
        let result = client
            .exec("sb_cwd", "ls", Some("/tmp"), None, |_| {})
            .await;
        assert!(result.is_ok(), "exec with cwd failed: {:?}", result.err());
    }

    #[test]
    fn test_strip_stream_markers_stdout_only() {
        let mut raw = vec![0x01, 0x01, 0x01];
        raw.extend_from_slice(b"hello world\n");
        assert_eq!(strip_stream_markers(&raw), "hello world\n");
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
        assert_eq!(
            strip_stream_markers(&raw),
            "stdout line\nstderr line\nmore stdout\n"
        );
    }

    #[test]
    fn test_strip_stream_markers_no_markers() {
        assert_eq!(strip_stream_markers(b"plain text\n"), "plain text\n");
    }

    #[test]
    fn test_strip_stream_markers_empty() {
        assert_eq!(strip_stream_markers(b""), "");
    }
}

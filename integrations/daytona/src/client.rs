//! Daytona API client and URL encoding utilities.

use serde_json::{Value, json};
use std::time::Duration;
use tracing::debug;

use crate::state::{ExecResult, SandboxInfo};
use crate::{EXEC_POLL_INTERVAL, SANDBOX_READY_MAX_WAIT, SANDBOX_READY_POLL_INTERVAL};

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

    /// Execute a command with real-time output streaming via background process + file polling.
    ///
    /// Returns `(exit_code, full_output)`. Calls `on_output` with each new chunk of output
    /// as it becomes available.
    pub async fn exec_streaming<F>(
        &self,
        sandbox_id: &str,
        command: &str,
        cwd: Option<&str>,
        timeout_ms: u64,
        mut on_output: F,
    ) -> Result<ExecResult, String>
    where
        F: FnMut(&str),
    {
        let exec_id = format!(
            "{:x}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        let out_file = format!("/tmp/everruns_exec_{exec_id}.out");
        let exit_file = format!("/tmp/everruns_exec_{exec_id}.exit");
        let pid_file = format!("/tmp/everruns_exec_{exec_id}.pid");

        // Write command to a temp script to avoid shell quoting issues (commands
        // frequently contain single/double quotes, backticks, $(), etc.).
        let script_file = format!("/tmp/everruns_exec_{exec_id}.sh");
        let cwd_part = cwd.map(|c| format!("cd {c} && ")).unwrap_or_default();
        let script_body = format!("{cwd_part}({command}) > {out_file} 2>&1; echo $? > {exit_file}");

        // Write the script, make executable, run in background, capture PID.
        let write_result = self
            .exec(
                sandbox_id,
                &format!(
                    "cat > {script_file} << 'EVERRUNS_EXEC_EOF'\n{script_body}\nEVERRUNS_EXEC_EOF\nchmod +x {script_file}"
                ),
                None,
                Some(10_000),
            )
            .await;
        if let Err(e) = write_result {
            return Err(format!("Failed to write exec script: {e}"));
        }

        let start_result = self
            .exec(
                sandbox_id,
                &format!("nohup sh {script_file} > /dev/null 2>&1 & echo $! > {pid_file}"),
                None,
                Some(10_000),
            )
            .await;
        if let Err(e) = start_result {
            return Err(format!("Failed to start background command: {e}"));
        }

        // Poll for output
        let deadline = std::time::Instant::now() + Duration::from_millis(timeout_ms);
        let mut bytes_read: usize = 0;

        loop {
            if std::time::Instant::now() >= deadline {
                // Timeout — kill the process by PID and clean up
                let _ = self
                    .exec(
                        sandbox_id,
                        &format!(
                            "if [ -f {pid_file} ]; then kill $(cat {pid_file}) 2>/dev/null; fi"
                        ),
                        None,
                        Some(5_000),
                    )
                    .await;
                let _ = self
                    .exec(
                        sandbox_id,
                        &format!("rm -f {out_file} {exit_file} {script_file} {pid_file}"),
                        None,
                        Some(5_000),
                    )
                    .await;
                return Err(format!("Command timed out after {}ms", timeout_ms));
            }

            tokio::time::sleep(EXEC_POLL_INTERVAL).await;

            // Check for new output
            let new_output = self
                .exec(
                    sandbox_id,
                    &format!("tail -c +{} {out_file} 2>/dev/null", bytes_read + 1),
                    None,
                    Some(10_000),
                )
                .await;

            if let Ok(ref chunk) = new_output
                && !chunk.result.is_empty()
            {
                bytes_read += chunk.result.len();
                on_output(&chunk.result);
            }

            // Check if the process has completed (exit file exists)
            let done_check = self
                .exec(
                    sandbox_id,
                    &format!("cat {exit_file} 2>/dev/null"),
                    None,
                    Some(5_000),
                )
                .await;

            if let Ok(ref done) = done_check {
                let trimmed = done.result.trim();
                if !trimmed.is_empty() {
                    // Process finished — read any remaining output
                    let final_output = self
                        .exec(
                            sandbox_id,
                            &format!("tail -c +{} {out_file} 2>/dev/null", bytes_read + 1),
                            None,
                            Some(10_000),
                        )
                        .await;
                    if let Ok(ref tail) = final_output
                        && !tail.result.is_empty()
                    {
                        on_output(&tail.result);
                    }

                    // Read the full output for the final result
                    let full_output = self
                        .exec(
                            sandbox_id,
                            &format!("cat {out_file} 2>/dev/null"),
                            None,
                            Some(10_000),
                        )
                        .await
                        .map(|r| r.result)
                        .unwrap_or_default();

                    let exit_code = trimmed.parse::<i32>().unwrap_or(-1);

                    // Clean up temp files
                    let _ = self
                        .exec(
                            sandbox_id,
                            &format!("rm -f {out_file} {exit_file} {script_file} {pid_file}"),
                            None,
                            Some(5_000),
                        )
                        .await;

                    return Ok(ExecResult {
                        result: full_output,
                        exit_code,
                    });
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
        Mock::given(method("POST"))
            .and(path("/sb_test/process/execute"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "result": "output",
                "exitCode": 1
            })))
            .mount(&mock_server)
            .await;

        let client = DaytonaClient::with_base_urls(
            "test_key".to_string(),
            mock_server.uri(),
            mock_server.uri(),
        );
        let result = client.exec("sb_test", "ls", Some("/tmp"), Some(5000)).await;
        assert!(result.is_ok());
        let exec_result = result.unwrap();
        assert_eq!(exec_result.exit_code, 1);
        assert_eq!(exec_result.result, "output");
    }

    #[tokio::test]
    async fn test_client_exec_nonzero_exit() {
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/sb_test/process/execute"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "result": "command not found",
                "exitCode": 127
            })))
            .mount(&mock_server)
            .await;

        let client = DaytonaClient::with_base_urls(
            "test_key".to_string(),
            mock_server.uri(),
            mock_server.uri(),
        );
        let result = client.exec("sb_test", "nonexistent", None, None).await;
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
        use wiremock::matchers::body_string_contains;

        let mock_server = MockServer::start().await;
        let exec_path = "/sb_test/process/execute";

        // 1. Write script → success
        Mock::given(method("POST"))
            .and(path(exec_path))
            .and(body_string_contains("cat >"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "result": "", "exitCode": 0
            })))
            .mount(&mock_server)
            .await;

        // 2. Start script → success
        Mock::given(method("POST"))
            .and(path(exec_path))
            .and(body_string_contains("nohup sh"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "result": "", "exitCode": 0
            })))
            .mount(&mock_server)
            .await;

        // 3. Poll output via tail → return a chunk
        Mock::given(method("POST"))
            .and(path(exec_path))
            .and(body_string_contains("tail -c"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "result": "hello streaming\n", "exitCode": 0
            })))
            .mount(&mock_server)
            .await;

        // 4. Check exit file → return exit code (process done)
        Mock::given(method("POST"))
            .and(path(exec_path))
            .and(body_string_contains(".exit"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "result": "0\n", "exitCode": 0
            })))
            .mount(&mock_server)
            .await;

        // 5. Full output cat → return complete output
        Mock::given(method("POST"))
            .and(path(exec_path))
            .and(body_string_contains("cat /tmp"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "result": "hello streaming\n", "exitCode": 0
            })))
            .mount(&mock_server)
            .await;

        // 6. Cleanup rm → success
        Mock::given(method("POST"))
            .and(path(exec_path))
            .and(body_string_contains("rm -f"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "result": "", "exitCode": 0
            })))
            .mount(&mock_server)
            .await;

        let client = DaytonaClient::with_base_urls(
            "test_key".to_string(),
            mock_server.uri(),
            mock_server.uri(),
        );

        let chunks: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let chunks_clone = chunks.clone();

        let result = client
            .exec_streaming("sb_test", "echo hello streaming", None, 30_000, |chunk| {
                chunks_clone.lock().unwrap().push(chunk.to_string());
            })
            .await;

        assert!(result.is_ok(), "exec_streaming failed: {:?}", result.err());
        let exec_result = result.unwrap();
        assert_eq!(exec_result.exit_code, 0);
        assert_eq!(exec_result.result, "hello streaming\n");

        let collected = chunks.lock().unwrap();
        assert!(!collected.is_empty(), "should have received output chunks");
    }
}

//! HTTP client for CodeSandbox Management and Pint APIs.

use crate::types::*;

use serde_json::{Value, json};
use std::time::Duration;
use tracing::debug;

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

    pub async fn management_request(
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
    pub async fn pint_request(
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
        // Pint API returns {"files": [...], "path": "..."} or an error object
        if resp.get("files").is_some() {
            let listing: DirListResponse = serde_json::from_value(resp)
                .map_err(|e| format!("Failed to parse directory listing: {e}"))?;
            Ok(listing.files)
        } else if let Some(msg) = resp.get("message").and_then(|m| m.as_str()) {
            Err(format!("Directory listing error: {msg}"))
        } else {
            Err(format!("Unexpected directory listing response: {resp}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

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

    // --- Management API tests ---

    #[tokio::test]
    async fn test_create_sandbox() {
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

        let client = CodeSandboxClient::with_base_url("test_key".to_string(), mock_server.uri());
        let result = client.create_sandbox(json!({"title": "Test"})).await;
        assert!(result.is_ok());
        let info = result.unwrap();
        assert_eq!(info.id, "sb_test123");
        assert_eq!(info.title, Some("Test Sandbox".to_string()));
    }

    #[tokio::test]
    async fn test_start_vm() {
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

        let client = CodeSandboxClient::with_base_url("test_key".to_string(), mock_server.uri());
        let result = client.start_vm("sb_test", None).await;
        assert!(result.is_ok());
        let info = result.unwrap();
        assert_eq!(info.pitcher_url, "https://pitcher.test.csb.app");
        assert_eq!(info.pitcher_token, "tok_test");
        assert_eq!(info.workspace_path, Some("/project".to_string()));
    }

    #[tokio::test]
    async fn test_start_vm_with_tier() {
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/vm/sb_test/start"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": {
                    "pitcher_url": "https://p.test",
                    "pitcher_token": "tok",
                }
            })))
            .mount(&mock_server)
            .await;

        let client = CodeSandboxClient::with_base_url("test_key".to_string(), mock_server.uri());
        let result = client.start_vm("sb_test", Some("large")).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_shutdown_vm() {
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/vm/sb_test/shutdown"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
            .mount(&mock_server)
            .await;

        let client = CodeSandboxClient::with_base_url("test_key".to_string(), mock_server.uri());
        let result = client.shutdown_vm("sb_test").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_hibernate_vm() {
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/vm/sb_test/hibernate"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
            .mount(&mock_server)
            .await;

        let client = CodeSandboxClient::with_base_url("test_key".to_string(), mock_server.uri());
        let result = client.hibernate_vm("sb_test").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_delete_vm() {
        let mock_server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path("/vm/sb_test"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
            .mount(&mock_server)
            .await;

        let client = CodeSandboxClient::with_base_url("test_key".to_string(), mock_server.uri());
        let result = client.delete_vm("sb_test").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_create_preview_token() {
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/sandbox/sb_test/tokens"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": {
                    "token": {"token": "prv_tok", "token_id": "tid_1"},
                    "sandbox_id": "sb_test"
                }
            })))
            .mount(&mock_server)
            .await;

        let client = CodeSandboxClient::with_base_url("test_key".to_string(), mock_server.uri());
        let result = client.create_preview_token("sb_test").await;
        assert!(result.is_ok());
        let resp = result.unwrap();
        assert_eq!(resp.token.token, "prv_tok");
    }

    // --- Pint API tests ---

    #[tokio::test]
    async fn test_exec_create() {
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

        let client = CodeSandboxClient::with_base_url("test_key".to_string(), mock_server.uri());
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
    async fn test_exec_get() {
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

        let client = CodeSandboxClient::with_base_url("test_key".to_string(), mock_server.uri());
        let state = mock_state(&mock_server.uri());
        let result = client.exec_get(&state, "exec_1").await;
        assert!(result.is_ok());
        let info = result.unwrap();
        assert_eq!(info.status, "exited");
        assert_eq!(info.exit_code, Some(0));
    }

    #[tokio::test]
    async fn test_exec_get_output() {
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/execs/exec_1/io"))
            .respond_with(ResponseTemplate::new(200).set_body_string("hello world\n"))
            .mount(&mock_server)
            .await;

        let client = CodeSandboxClient::with_base_url("test_key".to_string(), mock_server.uri());
        let state = mock_state(&mock_server.uri());
        let result = client.exec_get_output(&state, "exec_1").await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "hello world");
    }

    #[tokio::test]
    async fn test_exec_kill() {
        let mock_server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path("/api/v1/execs/exec_1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
            .mount(&mock_server)
            .await;

        let client = CodeSandboxClient::with_base_url("test_key".to_string(), mock_server.uri());
        let state = mock_state(&mock_server.uri());
        let result = client.exec_kill(&state, "exec_1").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_file_read() {
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/files/project/main.py"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "path": "/project/main.py",
                "content": "print('hello')"
            })))
            .mount(&mock_server)
            .await;

        let client = CodeSandboxClient::with_base_url("test_key".to_string(), mock_server.uri());
        let state = mock_state(&mock_server.uri());
        let result = client.file_read(&state, "/project/main.py").await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap().content, "print('hello')");
    }

    #[tokio::test]
    async fn test_file_write() {
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/files/project/main.py"))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({
                "path": "/project/main.py",
                "content": "print('hello')"
            })))
            .mount(&mock_server)
            .await;

        let client = CodeSandboxClient::with_base_url("test_key".to_string(), mock_server.uri());
        let state = mock_state(&mock_server.uri());
        let result = client
            .file_write(&state, "/project/main.py", "print('hello')")
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_dir_list() {
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/directories/project"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "files": [
                    {"name": "main.py", "path": "/project/main.py", "isDir": false, "size": 42},
                    {"name": "src", "path": "/project/src", "isDir": true, "size": 0}
                ],
                "path": "/project"
            })))
            .mount(&mock_server)
            .await;

        let client = CodeSandboxClient::with_base_url("test_key".to_string(), mock_server.uri());
        let state = mock_state(&mock_server.uri());
        let result = client.dir_list(&state, "/project").await;
        assert!(result.is_ok());
        let entries = result.unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "main.py");
        assert!(!entries[0].is_dir);
        assert!(entries[1].is_dir);
    }

    #[tokio::test]
    async fn test_dir_list_error_message() {
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/directories/nonexistent"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"message": "Not found"})))
            .mount(&mock_server)
            .await;

        let client = CodeSandboxClient::with_base_url("test_key".to_string(), mock_server.uri());
        let state = mock_state(&mock_server.uri());
        let result = client.dir_list(&state, "/nonexistent").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Not found"));
    }

    // --- Error response tests ---

    #[tokio::test]
    async fn test_404_error() {
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/sandbox/nonexistent"))
            .respond_with(
                ResponseTemplate::new(404).set_body_string("{\"error\":\"Sandbox not found\"}"),
            )
            .mount(&mock_server)
            .await;

        let client = CodeSandboxClient::with_base_url("test_key".to_string(), mock_server.uri());
        let result = client
            .management_request(reqwest::Method::GET, "/sandbox/nonexistent", None)
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("404"));
    }

    #[tokio::test]
    async fn test_401_error() {
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
        assert!(result.unwrap_err().contains("401"));
    }

    #[tokio::test]
    async fn test_500_error() {
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/vm/sb_test/start"))
            .respond_with(ResponseTemplate::new(500).set_body_string("Internal Server Error"))
            .mount(&mock_server)
            .await;

        let client = CodeSandboxClient::with_base_url("test_key".to_string(), mock_server.uri());
        let result = client.start_vm("sb_test", None).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("500"));
    }

    #[tokio::test]
    async fn test_malformed_response() {
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/sandbox"))
            .respond_with(ResponseTemplate::new(200).set_body_string("not valid json"))
            .mount(&mock_server)
            .await;

        let client = CodeSandboxClient::with_base_url("test_key".to_string(), mock_server.uri());
        let result = client.create_sandbox(json!({"title": "Test"})).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid JSON"));
    }

    #[tokio::test]
    async fn test_pint_request_empty_body() {
        let mock_server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path("/api/v1/execs/e1"))
            .respond_with(ResponseTemplate::new(200).set_body_string(""))
            .mount(&mock_server)
            .await;

        let client = CodeSandboxClient::with_base_url("test_key".to_string(), mock_server.uri());
        let result = client
            .pint_request(
                reqwest::Method::DELETE,
                &mock_server.uri(),
                "/api/v1/execs/e1",
                "tok",
                "prv",
                None,
            )
            .await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), json!({}));
    }
}

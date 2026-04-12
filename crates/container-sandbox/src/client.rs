//! Docker Engine REST API client.
//!
//! Decision: Uses reqwest for HTTP/TCP connections to Docker Engine API v1.47.
//! Docker host resolution: config → env `CONTAINER_SANDBOX_DOCKER_HOST` → default `http://localhost:2375`.
//! Unix socket support is a future enhancement (requires hyper unix transport).
//!
//! Decision: All methods return `Result<T, String>` for consistency with the Daytona client
//! pattern and easy mapping to ToolExecutionResult errors.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::io::Read as _;
use tracing::{debug, warn};

/// Docker Engine API version prefix.
const API_VERSION: &str = "/v1.47";

/// Default Docker host when no config or env is set.
const DEFAULT_DOCKER_HOST: &str = "http://localhost:2375";

/// Environment variable for Docker host override.
const DOCKER_HOST_ENV: &str = "CONTAINER_SANDBOX_DOCKER_HOST";

// ============================================================================
// Types
// ============================================================================

/// Container creation configuration.
#[derive(Debug, Clone, Serialize)]
pub struct ContainerCreateConfig {
    #[serde(rename = "Image")]
    pub image: String,
    #[serde(rename = "Cmd")]
    pub cmd: Vec<String>,
    #[serde(rename = "WorkingDir")]
    pub working_dir: String,
    #[serde(rename = "Labels")]
    pub labels: std::collections::HashMap<String, String>,
    #[serde(rename = "HostConfig")]
    pub host_config: HostConfig,
}

/// Container host configuration (resource limits, networking).
#[derive(Debug, Clone, Serialize)]
pub struct HostConfig {
    #[serde(rename = "Runtime", skip_serializing_if = "String::is_empty")]
    pub runtime: String,
    #[serde(rename = "NetworkMode")]
    pub network_mode: String,
    #[serde(rename = "Memory")]
    pub memory: i64,
    #[serde(rename = "NanoCpus")]
    pub nano_cpus: i64,
    #[serde(rename = "PidsLimit")]
    pub pids_limit: i64,
    #[serde(rename = "Init")]
    pub init: bool,
}

/// Response from container create.
#[derive(Debug, Deserialize)]
pub struct ContainerCreateResponse {
    #[serde(rename = "Id")]
    pub id: String,
    #[serde(rename = "Warnings", default)]
    pub warnings: Vec<String>,
}

/// Container inspect response (subset of fields we need).
#[derive(Debug, Deserialize)]
pub struct ContainerInspect {
    #[serde(rename = "Id")]
    pub id: String,
    #[serde(rename = "State")]
    pub state: ContainerState,
    #[serde(rename = "Name")]
    pub name: String,
}

/// Container state from inspect.
#[derive(Debug, Deserialize)]
pub struct ContainerState {
    #[serde(rename = "Status")]
    pub status: String,
    #[serde(rename = "Running")]
    pub running: bool,
    #[serde(rename = "ExitCode")]
    pub exit_code: i64,
}

/// Container list item.
#[derive(Debug, Deserialize)]
pub struct ContainerListItem {
    #[serde(rename = "Id")]
    pub id: String,
    #[serde(rename = "Names")]
    pub names: Vec<String>,
    #[serde(rename = "State")]
    pub state: String,
    #[serde(rename = "Status")]
    pub status: String,
    #[serde(rename = "Labels")]
    pub labels: std::collections::HashMap<String, String>,
}

/// Exec create response.
#[derive(Debug, Deserialize)]
pub struct ExecCreateResponse {
    #[serde(rename = "Id")]
    pub id: String,
}

/// Exec inspect response.
#[derive(Debug, Deserialize)]
pub struct ExecInspect {
    #[serde(rename = "ExitCode")]
    pub exit_code: Option<i64>,
    #[serde(rename = "Running")]
    pub running: bool,
}

/// Result of exec start (captured output).
pub struct ExecOutput {
    pub output: String,
    pub exit_code: i64,
}

// ============================================================================
// DockerClient
// ============================================================================

pub struct DockerClient {
    http: reqwest::Client,
    base_url: String,
}

impl DockerClient {
    /// Create a new DockerClient.
    ///
    /// Host resolution priority:
    /// 1. Explicit `docker_host` parameter (from config)
    /// 2. `CONTAINER_SANDBOX_DOCKER_HOST` env var
    /// 3. Default: `http://localhost:2375`
    pub fn new(docker_host: Option<&str>) -> Self {
        let base_url = docker_host
            .map(String::from)
            .or_else(|| std::env::var(DOCKER_HOST_ENV).ok())
            .unwrap_or_else(|| DEFAULT_DOCKER_HOST.to_string())
            .trim_end_matches('/')
            .to_string();

        debug!(base_url = %base_url, "DockerClient initialized");

        let http = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(30))
            .timeout(std::time::Duration::from_secs(300))
            .build()
            .expect("failed to build reqwest client");

        Self { http, base_url }
    }

    /// For tests: create with explicit base URL.
    #[cfg(test)]
    pub fn with_base_url(base_url: String) -> Self {
        let base_url = base_url.trim_end_matches('/').to_string();
        let http = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(30))
            .timeout(std::time::Duration::from_secs(300))
            .build()
            .expect("failed to build reqwest client");
        Self { http, base_url }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{API_VERSION}{path}", self.base_url)
    }

    // --- Generic request helpers ---

    async fn request(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value, String> {
        let url = self.url(path);
        let mut req = self
            .http
            .request(method, &url)
            .header("Content-Type", "application/json");

        if let Some(b) = body {
            req = req.json(&b);
        }

        let resp = req
            .send()
            .await
            .map_err(|e| format!("Docker API connection failed: {e}"))?;

        let status = resp.status();
        let body_text = resp
            .text()
            .await
            .map_err(|e| format!("Failed to read Docker response: {e}"))?;

        if !status.is_success() {
            return Err(map_docker_error(status.as_u16(), &body_text));
        }

        if body_text.is_empty() {
            return Ok(json!({}));
        }

        serde_json::from_str(&body_text).map_err(|e| format!("Invalid JSON from Docker: {e}"))
    }

    async fn request_typed<T: serde::de::DeserializeOwned>(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<T, String> {
        let url = self.url(path);
        let mut req = self
            .http
            .request(method, &url)
            .header("Content-Type", "application/json");

        if let Some(b) = body {
            req = req.json(&b);
        }

        let resp = req
            .send()
            .await
            .map_err(|e| format!("Docker API connection failed: {e}"))?;

        let status = resp.status();
        let body_text = resp
            .text()
            .await
            .map_err(|e| format!("Failed to read Docker response: {e}"))?;

        if !status.is_success() {
            return Err(map_docker_error(status.as_u16(), &body_text));
        }

        serde_json::from_str(&body_text).map_err(|e| format!("Invalid JSON from Docker: {e}"))
    }

    async fn request_bytes(&self, method: reqwest::Method, path: &str) -> Result<Vec<u8>, String> {
        let url = self.url(path);
        let resp = self
            .http
            .request(method, &url)
            .send()
            .await
            .map_err(|e| format!("Docker API connection failed: {e}"))?;

        let status = resp.status();
        if !status.is_success() {
            let body_text = resp
                .text()
                .await
                .map_err(|e| format!("Failed to read response: {e}"))?;
            return Err(map_docker_error(status.as_u16(), &body_text));
        }

        resp.bytes()
            .await
            .map(|b| b.to_vec())
            .map_err(|e| format!("Failed to read bytes: {e}"))
    }

    async fn request_no_content(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<(), String> {
        let url = self.url(path);
        let mut req = self
            .http
            .request(method, &url)
            .header("Content-Type", "application/json");

        if let Some(b) = body {
            req = req.json(&b);
        }

        let resp = req
            .send()
            .await
            .map_err(|e| format!("Docker API connection failed: {e}"))?;

        let status = resp.status();
        if !status.is_success() {
            let body_text = resp
                .text()
                .await
                .map_err(|e| format!("Failed to read response: {e}"))?;
            return Err(map_docker_error(status.as_u16(), &body_text));
        }

        Ok(())
    }

    // ========================================================================
    // Network operations
    // ========================================================================

    /// Create an isolated bridge network for a sandbox.
    pub async fn create_network(
        &self,
        name: &str,
        labels: std::collections::HashMap<String, String>,
    ) -> Result<String, String> {
        debug!(name = %name, "Creating Docker network");
        let body = json!({
            "Name": name,
            "Driver": "bridge",
            "Labels": labels,
        });
        let resp: Value = self
            .request(reqwest::Method::POST, "/networks/create", Some(body))
            .await?;
        resp.get("Id")
            .and_then(|v| v.as_str())
            .map(String::from)
            .ok_or_else(|| "Missing network Id in response".to_string())
    }

    /// Remove a network by ID.
    pub async fn remove_network(&self, id: &str) -> Result<(), String> {
        debug!(id = %id, "Removing Docker network");
        self.request_no_content(reqwest::Method::DELETE, &format!("/networks/{id}"), None)
            .await
    }

    // ========================================================================
    // Container lifecycle
    // ========================================================================

    /// Create a container with the given configuration.
    pub async fn create_container(
        &self,
        name: &str,
        config: &ContainerCreateConfig,
    ) -> Result<ContainerCreateResponse, String> {
        debug!(name = %name, image = %config.image, "Creating container");
        let body =
            serde_json::to_value(config).map_err(|e| format!("Failed to serialize config: {e}"))?;
        let encoded_name = urlencoding::encode(name);
        self.request_typed(
            reqwest::Method::POST,
            &format!("/containers/create?name={encoded_name}"),
            Some(body),
        )
        .await
    }

    /// Start a stopped container.
    pub async fn start_container(&self, id: &str) -> Result<(), String> {
        debug!(id = %id, "Starting container");
        self.request_no_content(
            reqwest::Method::POST,
            &format!("/containers/{id}/start"),
            None,
        )
        .await
    }

    /// Gracefully stop a running container.
    pub async fn stop_container(&self, id: &str, timeout_seconds: u32) -> Result<(), String> {
        debug!(id = %id, timeout = timeout_seconds, "Stopping container");
        self.request_no_content(
            reqwest::Method::POST,
            &format!("/containers/{id}/stop?t={timeout_seconds}"),
            None,
        )
        .await
    }

    /// Remove a container (force remove if running).
    pub async fn remove_container(&self, id: &str) -> Result<(), String> {
        debug!(id = %id, "Removing container");
        self.request_no_content(
            reqwest::Method::DELETE,
            &format!("/containers/{id}?force=true"),
            None,
        )
        .await
    }

    /// Inspect a container (get detailed state).
    pub async fn inspect_container(&self, id: &str) -> Result<ContainerInspect, String> {
        self.request_typed(
            reqwest::Method::GET,
            &format!("/containers/{id}/json"),
            None,
        )
        .await
    }

    /// List containers matching label filters.
    pub async fn list_containers(
        &self,
        label_filters: &[(&str, &str)],
    ) -> Result<Vec<ContainerListItem>, String> {
        let filters: std::collections::HashMap<&str, Vec<String>> =
            std::collections::HashMap::from([(
                "label",
                label_filters
                    .iter()
                    .map(|(k, v)| format!("{k}={v}"))
                    .collect(),
            )]);

        let filters_json =
            serde_json::to_string(&filters).map_err(|e| format!("Filter encoding error: {e}"))?;

        self.request_typed(
            reqwest::Method::GET,
            &format!(
                "/containers/json?all=true&filters={}",
                urlencoding::encode(&filters_json)
            ),
            None,
        )
        .await
    }

    // ========================================================================
    // Exec operations
    // ========================================================================

    /// Create an exec instance in a container.
    pub async fn exec_create(
        &self,
        container_id: &str,
        cmd: &[&str],
        working_dir: Option<&str>,
    ) -> Result<String, String> {
        debug!(container_id = %container_id, cmd = ?cmd, "Creating exec instance");
        let mut body = json!({
            "AttachStdout": true,
            "AttachStderr": true,
            "Cmd": cmd,
        });
        if let Some(wd) = working_dir {
            body["WorkingDir"] = json!(wd);
        }

        let resp: ExecCreateResponse = self
            .request_typed(
                reqwest::Method::POST,
                &format!("/containers/{container_id}/exec"),
                Some(body),
            )
            .await?;
        Ok(resp.id)
    }

    /// Start an exec instance and capture output.
    ///
    /// Docker multiplexes stdout/stderr into a single stream with 8-byte headers.
    /// We demux the stream and combine stdout+stderr into a single string.
    pub async fn exec_start(&self, exec_id: &str) -> Result<String, String> {
        debug!(exec_id = %exec_id, "Starting exec");
        let url = self.url(&format!("/exec/{exec_id}/start"));
        let resp = self
            .http
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&json!({"Detach": false, "Tty": false}))
            .send()
            .await
            .map_err(|e| format!("Docker exec start failed: {e}"))?;

        let status = resp.status();
        if !status.is_success() {
            let body_text = resp
                .text()
                .await
                .map_err(|e| format!("Failed to read response: {e}"))?;
            return Err(map_docker_error(status.as_u16(), &body_text));
        }

        let raw = resp
            .bytes()
            .await
            .map_err(|e| format!("Failed to read exec output: {e}"))?;

        Ok(demux_docker_stream(&raw))
    }

    /// Get the exit code of a completed exec instance.
    pub async fn exec_inspect(&self, exec_id: &str) -> Result<ExecInspect, String> {
        self.request_typed(reqwest::Method::GET, &format!("/exec/{exec_id}/json"), None)
            .await
    }

    /// Execute a command and return combined output + exit code.
    ///
    /// After `exec_start` returns (which blocks until the command finishes),
    /// `exec_inspect` should report the exit code. In rare cases the exit code
    /// may not yet be populated, so we retry up to 3 times with a brief delay.
    pub async fn exec(
        &self,
        container_id: &str,
        cmd: &[&str],
        working_dir: Option<&str>,
    ) -> Result<ExecOutput, String> {
        let exec_id = self.exec_create(container_id, cmd, working_dir).await?;
        let output = self.exec_start(&exec_id).await?;

        // Retry inspect up to 3 times to handle rare race where exit_code is not yet set.
        let mut exit_code: Option<i64> = None;
        for attempt in 0..3 {
            let inspect = self.exec_inspect(&exec_id).await?;
            if let Some(code) = inspect.exit_code {
                exit_code = Some(code);
                break;
            }
            if attempt < 2 {
                debug!(exec_id = %exec_id, attempt = attempt + 1, "exec_inspect returned no exit_code, retrying");
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        }

        Ok(ExecOutput {
            output,
            exit_code: exit_code.unwrap_or(-1),
        })
    }

    // ========================================================================
    // File operations (tar-based archive API)
    // ========================================================================

    /// Read a file from a container via the archive API.
    /// Returns the file content as bytes.
    pub async fn get_archive(&self, container_id: &str, path: &str) -> Result<Vec<u8>, String> {
        debug!(container_id = %container_id, path = %path, "Reading file from container");
        let encoded_path = urlencoding::encode(path);
        let tar_bytes = self
            .request_bytes(
                reqwest::Method::GET,
                &format!("/containers/{container_id}/archive?path={encoded_path}"),
            )
            .await?;

        extract_file_from_tar(&tar_bytes)
    }

    /// Write a file to a container via the archive API.
    /// Content is wrapped in a tar archive and PUT to the container.
    pub async fn put_archive(
        &self,
        container_id: &str,
        dir_path: &str,
        filename: &str,
        content: &[u8],
    ) -> Result<(), String> {
        sanitize_filename(filename)?;
        debug!(container_id = %container_id, dir_path = %dir_path, filename = %filename, "Writing file to container");
        let tar_bytes = create_tar_with_file(filename, content)?;
        let encoded_path = urlencoding::encode(dir_path);
        let url = self.url(&format!(
            "/containers/{container_id}/archive?path={encoded_path}"
        ));

        let resp = self
            .http
            .put(&url)
            .header("Content-Type", "application/x-tar")
            .body(tar_bytes)
            .send()
            .await
            .map_err(|e| format!("Docker archive upload failed: {e}"))?;

        let status = resp.status();
        if !status.is_success() {
            let body_text = resp
                .text()
                .await
                .map_err(|e| format!("Failed to read response: {e}"))?;
            return Err(map_docker_error(status.as_u16(), &body_text));
        }

        Ok(())
    }
}

// ============================================================================
// Helpers
// ============================================================================

/// Validate that a filename is safe for use inside a tar archive.
/// Rejects path traversal (`..`), absolute paths (`/`), and backslash separators.
fn sanitize_filename(filename: &str) -> Result<(), String> {
    if filename.is_empty() {
        return Err("Filename must not be empty".to_string());
    }
    if filename.contains("..") {
        return Err(format!("Filename contains path traversal (..): {filename}"));
    }
    if filename.contains('/') {
        return Err(format!(
            "Filename contains directory separator (/): {filename}"
        ));
    }
    if filename.contains('\\') {
        return Err(format!(
            "Filename contains backslash separator (\\): {filename}"
        ));
    }
    Ok(())
}

/// Map Docker API HTTP errors to human-readable messages.
fn map_docker_error(status: u16, body: &str) -> String {
    // Try to extract Docker's error message from JSON response
    let detail = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| v.get("message").and_then(|m| m.as_str()).map(String::from))
        .unwrap_or_else(|| body.to_string());

    match status {
        304 => format!("Container already in requested state: {detail}"),
        404 => format!("Not found: {detail}"),
        409 => format!("Conflict (container/network already exists): {detail}"),
        500 => {
            if detail.contains("runtime not found") {
                format!("Container runtime not found. Is sysbox installed? ({detail})")
            } else {
                format!("Docker Engine error: {detail}")
            }
        }
        _ => format!("Docker API error (HTTP {status}): {detail}"),
    }
}

/// Demultiplex Docker's multiplexed stdout/stderr stream.
///
/// Docker exec with `Tty: false` returns a stream where each frame has:
/// - byte 0: stream type (1 = stdout, 2 = stderr)
/// - bytes 1-3: reserved (zero)
/// - bytes 4-7: big-endian payload size
/// - bytes 8..8+size: payload
fn demux_docker_stream(raw: &[u8]) -> String {
    let mut output = String::new();
    let mut pos = 0;

    while pos + 8 <= raw.len() {
        // Stream type at byte 0 (we combine stdout+stderr)
        let size =
            u32::from_be_bytes([raw[pos + 4], raw[pos + 5], raw[pos + 6], raw[pos + 7]]) as usize;
        pos += 8;

        if pos + size > raw.len() {
            warn!(
                expected = size,
                available = raw.len() - pos,
                "Truncated Docker stream frame"
            );
            // Append whatever we have
            output.push_str(&String::from_utf8_lossy(&raw[pos..]));
            break;
        }

        output.push_str(&String::from_utf8_lossy(&raw[pos..pos + size]));
        pos += size;
    }

    output
}

/// Extract the first regular file's content from a tar archive (Docker GET /archive response).
/// Skips directory entries to find the actual file content.
fn extract_file_from_tar(tar_bytes: &[u8]) -> Result<Vec<u8>, String> {
    let mut archive = tar::Archive::new(tar_bytes);
    let entries = archive
        .entries()
        .map_err(|e| format!("Failed to read tar archive: {e}"))?;

    for entry_result in entries {
        let mut entry = entry_result.map_err(|e| format!("Failed to read tar entry: {e}"))?;
        let entry_type = entry.header().entry_type();
        if entry_type.is_dir() {
            continue;
        }
        let mut content = Vec::new();
        entry
            .read_to_end(&mut content)
            .map_err(|e| format!("Failed to read file from tar: {e}"))?;
        return Ok(content);
    }

    Err("No regular file found in tar archive".to_string())
}

/// Create a tar archive containing a single file.
fn create_tar_with_file(filename: &str, content: &[u8]) -> Result<Vec<u8>, String> {
    let mut builder = tar::Builder::new(Vec::new());
    let mut header = tar::Header::new_gnu();
    header.set_size(content.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    builder
        .append_data(&mut header, filename, content)
        .map_err(|e| format!("Failed to create tar archive: {e}"))?;
    builder
        .into_inner()
        .map_err(|e| format!("Failed to finalize tar archive: {e}"))
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_url_construction() {
        let client = DockerClient::with_base_url("http://10.0.0.3:2375".to_string());
        assert_eq!(
            client.url("/containers/json"),
            "http://10.0.0.3:2375/v1.47/containers/json"
        );
    }

    #[test]
    fn test_demux_docker_stream_stdout() {
        // stdout frame: type=1, size=5, payload="hello"
        let mut frame = vec![1, 0, 0, 0, 0, 0, 0, 5]; // header
        frame.extend_from_slice(b"hello");
        assert_eq!(demux_docker_stream(&frame), "hello");
    }

    #[test]
    fn test_demux_docker_stream_mixed() {
        // stdout "hello", stderr " world"
        let mut frame = vec![1, 0, 0, 0, 0, 0, 0, 5];
        frame.extend_from_slice(b"hello");
        frame.extend_from_slice(&[2, 0, 0, 0, 0, 0, 0, 6]);
        frame.extend_from_slice(b" world");
        assert_eq!(demux_docker_stream(&frame), "hello world");
    }

    #[test]
    fn test_demux_empty_stream() {
        assert_eq!(demux_docker_stream(&[]), "");
    }

    #[test]
    fn test_demux_truncated_frame() {
        // Frame claims 100 bytes but only 3 available
        let mut frame = vec![1, 0, 0, 0, 0, 0, 0, 100];
        frame.extend_from_slice(b"abc");
        assert_eq!(demux_docker_stream(&frame), "abc");
    }

    #[test]
    fn test_create_and_extract_tar() {
        let content = b"Hello, container!";
        let tar_bytes = create_tar_with_file("test.txt", content).unwrap();
        let extracted = extract_file_from_tar(&tar_bytes).unwrap();
        assert_eq!(extracted, content);
    }

    #[test]
    fn test_map_docker_error_not_found() {
        let msg = map_docker_error(404, r#"{"message":"No such container: abc123"}"#);
        assert!(msg.contains("Not found"));
        assert!(msg.contains("No such container"));
    }

    #[test]
    fn test_map_docker_error_conflict() {
        let msg = map_docker_error(409, r#"{"message":"name already in use"}"#);
        assert!(msg.contains("Conflict"));
    }

    #[test]
    fn test_map_docker_error_runtime_not_found() {
        let msg = map_docker_error(500, r#"{"message":"OCI runtime not found: sysbox-runc"}"#);
        assert!(msg.contains("runtime not found"));
        assert!(msg.contains("sysbox"));
    }

    #[test]
    fn test_container_create_config_serialization() {
        let config = ContainerCreateConfig {
            image: "ubuntu:24.04".to_string(),
            cmd: vec![
                "tail".to_string(),
                "-f".to_string(),
                "/dev/null".to_string(),
            ],
            working_dir: "/workspace".to_string(),
            labels: std::collections::HashMap::from([(
                "managed-by".to_string(),
                "everruns".to_string(),
            )]),
            host_config: HostConfig {
                runtime: String::new(),
                network_mode: "bridge".to_string(),
                memory: 2_147_483_648,
                nano_cpus: 1_000_000_000,
                pids_limit: 256,
                init: true,
            },
        };

        let json = serde_json::to_value(&config).unwrap();
        assert_eq!(json["Image"], "ubuntu:24.04");
        assert_eq!(json["HostConfig"]["Memory"], 2_147_483_648_i64);
        assert_eq!(json["HostConfig"]["Init"], true);
        // Empty runtime should not appear in serialized output
        assert!(json["HostConfig"].get("Runtime").is_none());
    }

    #[test]
    fn test_label_filter_encoding() {
        let filters: std::collections::HashMap<&str, Vec<String>> =
            std::collections::HashMap::from([(
                "label",
                vec![
                    "managed-by=everruns".to_string(),
                    "session=abc123".to_string(),
                ],
            )]);
        let json = serde_json::to_string(&filters).unwrap();
        assert!(json.contains("managed-by=everruns"));
        assert!(json.contains("session=abc123"));
    }

    #[test]
    fn test_url_trailing_slash_trimmed() {
        let client = DockerClient::with_base_url("http://10.0.0.3:2375/".to_string());
        assert_eq!(
            client.url("/containers/json"),
            "http://10.0.0.3:2375/v1.47/containers/json"
        );
    }

    #[test]
    fn test_sanitize_filename_rejects_dot_dot() {
        let err = sanitize_filename("../etc/passwd").unwrap_err();
        assert!(err.contains("path traversal"));
    }

    #[test]
    fn test_sanitize_filename_rejects_embedded_dot_dot() {
        let err = sanitize_filename("foo/../bar").unwrap_err();
        assert!(err.contains("path traversal"));
    }

    #[test]
    fn test_sanitize_filename_rejects_slash() {
        let err = sanitize_filename("subdir/file.txt").unwrap_err();
        assert!(err.contains("directory separator"));
    }

    #[test]
    fn test_sanitize_filename_rejects_backslash() {
        let err = sanitize_filename("subdir\\file.txt").unwrap_err();
        assert!(err.contains("backslash"));
    }

    #[test]
    fn test_sanitize_filename_rejects_empty() {
        let err = sanitize_filename("").unwrap_err();
        assert!(err.contains("empty"));
    }

    #[test]
    fn test_sanitize_filename_accepts_valid() {
        assert!(sanitize_filename("hello.txt").is_ok());
        assert!(sanitize_filename("my-file_v2.tar.gz").is_ok());
    }

    #[test]
    fn test_extract_tar_skips_directories() {
        // Build a tar with a directory entry followed by a regular file.
        let mut builder = tar::Builder::new(Vec::new());

        // Add a directory entry
        let mut dir_header = tar::Header::new_gnu();
        dir_header.set_entry_type(tar::EntryType::Directory);
        dir_header.set_size(0);
        dir_header.set_mode(0o755);
        dir_header.set_cksum();
        builder
            .append_data(&mut dir_header, "mydir/", &[] as &[u8])
            .unwrap();

        // Add a regular file
        let content = b"file content";
        let mut file_header = tar::Header::new_gnu();
        file_header.set_size(content.len() as u64);
        file_header.set_mode(0o644);
        file_header.set_cksum();
        builder
            .append_data(&mut file_header, "mydir/file.txt", &content[..])
            .unwrap();

        let tar_bytes = builder.into_inner().unwrap();
        let extracted = extract_file_from_tar(&tar_bytes).unwrap();
        assert_eq!(extracted, content);
    }
}

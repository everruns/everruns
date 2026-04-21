//! Sprites API client.
//!
//! Decision: Single API tier — all operations go through api.sprites.dev/v1
//! Decision: Bearer token auth via Authorization header

use serde_json::{Value, json};
use tracing::debug;

use crate::state::{CheckpointInfo, ExecResult, SpriteInfo};

// ============================================================================
// SpritesClient - HTTP client for Sprites API
// ============================================================================

pub struct SpritesClient {
    http: reqwest::Client,
    api_token: String,
    api_base: String,
}

impl SpritesClient {
    pub fn new(api_token: String) -> Self {
        Self::with_base_url(api_token, crate::SPRITES_API_BASE.to_string())
    }

    pub fn with_base_url(api_token: String, api_base: String) -> Self {
        Self {
            http: reqwest::Client::new(),
            api_token,
            api_base,
        }
    }

    // --- Generic request helpers ---

    async fn request(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value, String> {
        let url = format!("{}{}", self.api_base, path);
        let mut req = self
            .http
            .request(method, &url)
            .bearer_auth(&self.api_token)
            .header("Content-Type", "application/json");

        if let Some(b) = body {
            req = req.json(&b);
        }

        let resp = req
            .send()
            .await
            .map_err(|e| format!("Failed to connect to Sprites API: {e}"))?;

        let status = resp.status();
        let body_text = resp
            .text()
            .await
            .map_err(|e| format!("Failed to read response: {e}"))?;

        if !status.is_success() {
            return Err(format!("Sprites API error ({status}): {body_text}"));
        }

        if body_text.is_empty() {
            return Ok(json!({}));
        }

        serde_json::from_str(&body_text).map_err(|e| format!("Invalid JSON from Sprites: {e}"))
    }

    async fn request_text(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<String, String> {
        let url = format!("{}{}", self.api_base, path);
        let mut req = self
            .http
            .request(method, &url)
            .bearer_auth(&self.api_token)
            .header("Content-Type", "application/json");

        if let Some(b) = body {
            req = req.json(&b);
        }

        let resp = req
            .send()
            .await
            .map_err(|e| format!("Failed to connect to Sprites API: {e}"))?;

        let status = resp.status();
        let body_text = resp
            .text()
            .await
            .map_err(|e| format!("Failed to read response: {e}"))?;

        if !status.is_success() {
            return Err(format!("Sprites API error ({status}): {body_text}"));
        }

        Ok(body_text)
    }

    // --- Sprite Lifecycle ---

    pub async fn create_sprite(&self, name: &str, body: Value) -> Result<SpriteInfo, String> {
        let mut body = body;
        let Some(obj) = body.as_object_mut() else {
            return Err("Sprites create request must be a JSON object".to_string());
        };
        obj.insert("name".to_string(), json!(name));
        let resp = self
            .request(reqwest::Method::POST, "/sprites", Some(body))
            .await?;
        serde_json::from_value(resp).map_err(|e| format!("Failed to parse sprite info: {e}"))
    }

    pub async fn get_sprite(&self, name: &str) -> Result<SpriteInfo, String> {
        let resp = self
            .request(reqwest::Method::GET, &format!("/sprites/{name}"), None)
            .await?;
        serde_json::from_value(resp).map_err(|e| format!("Failed to parse sprite info: {e}"))
    }

    pub async fn list_sprites(&self) -> Result<Vec<SpriteInfo>, String> {
        let resp = self.request(reqwest::Method::GET, "/sprites", None).await?;
        // Response may be an array or an object with a "sprites" key
        if let Some(arr) = resp.as_array() {
            let mut sprites = Vec::new();
            for item in arr {
                match serde_json::from_value::<SpriteInfo>(item.clone()) {
                    Ok(s) => sprites.push(s),
                    Err(e) => debug!("Skipping unparseable sprite: {e}"),
                }
            }
            Ok(sprites)
        } else if let Some(arr) = resp.get("sprites").and_then(|v| v.as_array()) {
            let mut sprites = Vec::new();
            for item in arr {
                match serde_json::from_value::<SpriteInfo>(item.clone()) {
                    Ok(s) => sprites.push(s),
                    Err(e) => debug!("Skipping unparseable sprite: {e}"),
                }
            }
            Ok(sprites)
        } else {
            Ok(vec![])
        }
    }

    pub async fn delete_sprite(&self, name: &str) -> Result<(), String> {
        self.request(reqwest::Method::DELETE, &format!("/sprites/{name}"), None)
            .await?;
        Ok(())
    }

    // --- Exec ---

    pub async fn exec(
        &self,
        name: &str,
        command: &str,
        _timeout_ms: Option<u64>,
    ) -> Result<ExecResult, String> {
        let url = format!(
            "{}/sprites/{name}/exec?cmd={}&cmd={}&cmd={}",
            self.api_base,
            urlencoding::encode("sh"),
            urlencoding::encode("-lc"),
            urlencoding::encode(command)
        );
        let resp = self
            .http
            .post(&url)
            .bearer_auth(&self.api_token)
            .send()
            .await
            .map_err(|e| format!("Failed to connect to Sprites API: {e}"))?;
        let status = resp.status();
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| format!("Failed to read exec response: {e}"))?;
        if !status.is_success() {
            let body_text = String::from_utf8_lossy(&bytes);
            return Err(format!("Sprites API error ({status}): {body_text}"));
        }

        parse_exec_result(&bytes)
    }

    // --- Filesystem ---

    pub async fn read_file(&self, name: &str, path: &str) -> Result<Vec<u8>, String> {
        let url = format!(
            "{}/sprites/{name}/fs/read?path={}",
            self.api_base,
            urlencoding::encode(path)
        );
        let resp = self
            .http
            .get(&url)
            .bearer_auth(&self.api_token)
            .send()
            .await
            .map_err(|e| format!("Failed to connect to Sprites API: {e}"))?;

        let status = resp.status();
        if !status.is_success() {
            let body_text = resp
                .text()
                .await
                .map_err(|e| format!("Failed to read response: {e}"))?;
            return Err(format!("Sprites API error ({status}): {body_text}"));
        }

        resp.bytes()
            .await
            .map(|b| b.to_vec())
            .map_err(|e| format!("Failed to read file bytes: {e}"))
    }

    pub async fn write_file(&self, name: &str, path: &str, content: &[u8]) -> Result<(), String> {
        let url = format!(
            "{}/sprites/{name}/fs/write?path={}&mkdir=true",
            self.api_base,
            urlencoding::encode(path)
        );

        let resp = self
            .http
            .put(&url)
            .bearer_auth(&self.api_token)
            .header("Content-Type", "application/octet-stream")
            .body(content.to_vec())
            .send()
            .await
            .map_err(|e| format!("Failed to write file: {e}"))?;

        let status = resp.status();
        if !status.is_success() {
            let body_text = resp
                .text()
                .await
                .map_err(|e| format!("Failed to read response: {e}"))?;
            return Err(format!("Sprites API error ({status}): {body_text}"));
        }

        Ok(())
    }

    // --- Checkpoints ---

    pub async fn create_checkpoint(&self, name: &str) -> Result<CheckpointInfo, String> {
        let stream = self
            .request_text(
                reqwest::Method::POST,
                &format!("/sprites/{name}/checkpoint"),
                Some(json!({})),
            )
            .await?;
        ensure_ndjson_success(&stream)?;

        if let Some(checkpoint) = checkpoint_from_stream(&stream) {
            return Ok(checkpoint);
        }

        self.list_checkpoints(name)
            .await?
            .into_iter()
            .find(|checkpoint| checkpoint.id != "Current")
            .ok_or_else(|| {
                "Sprites checkpoint completed but no new checkpoint id was found".to_string()
            })
    }

    pub async fn list_checkpoints(&self, name: &str) -> Result<Vec<CheckpointInfo>, String> {
        let resp = self
            .request(
                reqwest::Method::GET,
                &format!("/sprites/{name}/checkpoints"),
                None,
            )
            .await?;
        if let Some(arr) = resp.as_array() {
            let mut checkpoints = Vec::new();
            for item in arr {
                match serde_json::from_value::<CheckpointInfo>(item.clone()) {
                    Ok(c) => checkpoints.push(c),
                    Err(e) => debug!("Skipping unparseable checkpoint: {e}"),
                }
            }
            Ok(checkpoints)
        } else {
            Ok(vec![])
        }
    }

    pub async fn restore_checkpoint(&self, name: &str, checkpoint_id: &str) -> Result<(), String> {
        let stream = self
            .request_text(
                reqwest::Method::POST,
                &format!("/sprites/{name}/checkpoints/{checkpoint_id}/restore"),
                None,
            )
            .await?;
        ensure_ndjson_success(&stream)?;
        Ok(())
    }

    // --- Services ---

    pub async fn list_services(&self, name: &str) -> Result<Vec<Value>, String> {
        let resp = self
            .request(
                reqwest::Method::GET,
                &format!("/sprites/{name}/services"),
                None,
            )
            .await?;
        match resp.as_array() {
            Some(arr) => Ok(arr.clone()),
            None => Ok(vec![resp]),
        }
    }
}

fn parse_exec_result(bytes: &[u8]) -> Result<ExecResult, String> {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut exit_bytes = Vec::new();
    let mut stream = None;

    for &byte in bytes {
        match byte {
            0x01 => stream = Some(0x01),
            0x02 => stream = Some(0x02),
            0x03 => stream = Some(0x03),
            _ => match stream {
                Some(0x01) => stdout.push(byte),
                Some(0x02) => stderr.push(byte),
                Some(0x03) => exit_bytes.push(byte),
                _ => {}
            },
        }
    }

    let exit_code = exit_bytes.first().copied().unwrap_or(0) as i32;
    Ok(ExecResult {
        stdout: String::from_utf8(stdout)
            .map_err(|e| format!("Sprites exec stdout was not valid UTF-8: {e}"))?,
        stderr: String::from_utf8(stderr)
            .map_err(|e| format!("Sprites exec stderr was not valid UTF-8: {e}"))?,
        exit_code,
    })
}

fn ensure_ndjson_success(stream: &str) -> Result<(), String> {
    for line in stream.lines().filter(|line| !line.trim().is_empty()) {
        let event: Value =
            serde_json::from_str(line).map_err(|e| format!("Invalid NDJSON from Sprites: {e}"))?;
        if event.get("type").and_then(|v| v.as_str()) == Some("error") {
            if let Some(error) = event.get("error").and_then(|v| v.as_str()) {
                return Err(format!("Sprites stream error: {error}"));
            }
            if let Some(message) = event.get("message").and_then(|v| v.as_str()) {
                return Err(format!("Sprites stream error: {message}"));
            }
            return Err(format!("Sprites stream error: {event}"));
        }
    }
    Ok(())
}

fn checkpoint_from_stream(stream: &str) -> Option<CheckpointInfo> {
    for line in stream.lines().filter(|line| !line.trim().is_empty()) {
        let event: Value = serde_json::from_str(line).ok()?;
        if event.get("type").and_then(|v| v.as_str()) != Some("complete") {
            continue;
        }
        let data = event.get("data").and_then(|v| v.as_str())?;
        let id = data
            .split_whitespace()
            .nth(1)
            .map(|token| token.trim_matches(|ch: char| !ch.is_ascii_alphanumeric()))?;
        return Some(CheckpointInfo {
            id: id.to_string(),
            created_at: event
                .get("time")
                .and_then(|v| v.as_str())
                .map(ToOwned::to_owned),
            comment: None,
        });
    }
    None
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
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn test_client_create_sprite() {
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/sprites"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "name": "test-sprite",
                "status": "running"
            })))
            .mount(&mock_server)
            .await;

        let client = SpritesClient::with_base_url("test_token".to_string(), mock_server.uri());
        let result = client.create_sprite("test-sprite", json!({})).await;
        assert!(result.is_ok());
        let info = result.unwrap();
        assert_eq!(info.name, "test-sprite");
    }

    #[tokio::test]
    async fn test_client_get_sprite() {
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/sprites/my-sprite"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "name": "my-sprite",
                "status": "running"
            })))
            .mount(&mock_server)
            .await;

        let client = SpritesClient::with_base_url("test_token".to_string(), mock_server.uri());
        let result = client.get_sprite("my-sprite").await;
        assert!(result.is_ok());
        let info = result.unwrap();
        assert_eq!(info.status, "running");
    }

    #[tokio::test]
    async fn test_client_delete_sprite() {
        let mock_server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path("/sprites/old-sprite"))
            .respond_with(ResponseTemplate::new(204).set_body_string(""))
            .mount(&mock_server)
            .await;

        let client = SpritesClient::with_base_url("test_token".to_string(), mock_server.uri());
        let result = client.delete_sprite("old-sprite").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_client_exec() {
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/sprites/my-sprite/exec"))
            .and(query_param("cmd", "sh"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![
                0x01, b'h', b'e', b'l', b'l', b'o', b' ', b'w', b'o', b'r', b'l', b'd', b'\n',
                0x03, 0,
            ]))
            .mount(&mock_server)
            .await;

        let client = SpritesClient::with_base_url("test_token".to_string(), mock_server.uri());
        let result = client.exec("my-sprite", "echo hello world", None).await;
        assert!(result.is_ok());
        let exec_result = result.unwrap();
        assert_eq!(exec_result.exit_code, 0);
        assert_eq!(exec_result.stdout, "hello world\n");
    }

    #[tokio::test]
    async fn test_client_create_checkpoint() {
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/sprites/my-sprite/checkpoint"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                "{\"type\":\"info\",\"data\":\"Creating checkpoint...\",\"time\":\"2026-03-23T10:05:00Z\"}\n{\"type\":\"complete\",\"data\":\"Checkpoint v1 created successfully\",\"time\":\"2026-03-23T10:05:01Z\"}\n",
            ))
            .expect(1)
            .mount(&mock_server)
            .await;

        let client = SpritesClient::with_base_url("test_token".to_string(), mock_server.uri());
        let result = client.create_checkpoint("my-sprite").await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap().id, "v1");
    }

    #[tokio::test]
    async fn test_client_restore_checkpoint() {
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/sprites/my-sprite/checkpoints/cp_abc123/restore"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                "{\"type\":\"info\",\"data\":\"Restoring checkpoint...\",\"time\":\"2026-03-23T10:05:00Z\"}\n{\"type\":\"complete\",\"data\":\"Restore complete\",\"time\":\"2026-03-23T10:05:01Z\"}\n",
            ))
            .mount(&mock_server)
            .await;

        let client = SpritesClient::with_base_url("test_token".to_string(), mock_server.uri());
        let result = client.restore_checkpoint("my-sprite", "cp_abc123").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_client_list_sprites() {
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/sprites"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([
                {"name": "sprite-1", "status": "running"},
                {"name": "sprite-2", "status": "stopped"}
            ])))
            .mount(&mock_server)
            .await;

        let client = SpritesClient::with_base_url("test_token".to_string(), mock_server.uri());
        let result = client.list_sprites().await;
        assert!(result.is_ok());
        let sprites = result.unwrap();
        assert_eq!(sprites.len(), 2);
    }

    #[tokio::test]
    async fn test_client_404_error() {
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/sprites/nonexistent"))
            .respond_with(
                ResponseTemplate::new(404).set_body_string("{\"error\":\"Sprite not found\"}"),
            )
            .mount(&mock_server)
            .await;

        let client = SpritesClient::with_base_url("test_token".to_string(), mock_server.uri());
        let result = client.get_sprite("nonexistent").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("404"));
    }

    #[tokio::test]
    async fn test_client_401_error() {
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/sprites"))
            .respond_with(
                ResponseTemplate::new(401).set_body_string("{\"error\":\"Unauthorized\"}"),
            )
            .mount(&mock_server)
            .await;

        let client = SpritesClient::with_base_url("bad_token".to_string(), mock_server.uri());
        let result = client.create_sprite("test", json!({})).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("401"));
    }

    #[tokio::test]
    async fn test_client_read_file() {
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/sprites/my-sprite/fs/read"))
            .and(query_param("path", "/home/sprite/main.py"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"print('hello')\n".to_vec()))
            .mount(&mock_server)
            .await;

        let client = SpritesClient::with_base_url("test_token".to_string(), mock_server.uri());
        let result = client.read_file("my-sprite", "/home/sprite/main.py").await;
        assert!(result.is_ok());
        assert_eq!(
            String::from_utf8_lossy(&result.unwrap()),
            "print('hello')\n"
        );
    }

    #[tokio::test]
    async fn test_client_write_file() {
        let mock_server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/sprites/my-sprite/fs/write"))
            .and(query_param("path", "/home/sprite/test.py"))
            .and(query_param("mkdir", "true"))
            .respond_with(ResponseTemplate::new(200).set_body_string(""))
            .mount(&mock_server)
            .await;

        let client = SpritesClient::with_base_url("test_token".to_string(), mock_server.uri());
        let result = client
            .write_file("my-sprite", "/home/sprite/test.py", b"print('test')")
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_client_list_services() {
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/sprites/my-sprite/services"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([
                {"name": "web", "port": 8080, "url": "https://my-sprite.fly.dev"}
            ])))
            .mount(&mock_server)
            .await;

        let client = SpritesClient::with_base_url("test_token".to_string(), mock_server.uri());
        let result = client.list_services("my-sprite").await;
        assert!(result.is_ok());
        let services = result.unwrap();
        assert_eq!(services.len(), 1);
    }

    #[tokio::test]
    async fn test_client_list_checkpoints() {
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/sprites/my-sprite/checkpoints"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([
                {"id": "Current", "create_time": "2026-03-23T10:05:01Z"},
                {"id": "cp_1", "create_time": "2026-03-23T10:04:01Z"},
                {"id": "cp_2", "create_time": "2026-03-23T10:03:01Z"}
            ])))
            .mount(&mock_server)
            .await;

        let client = SpritesClient::with_base_url("test_token".to_string(), mock_server.uri());
        let result = client.list_checkpoints("my-sprite").await;
        assert!(result.is_ok());
        let checkpoints = result.unwrap();
        assert_eq!(checkpoints.len(), 3);
        assert_eq!(checkpoints[1].id, "cp_1");
    }

    #[tokio::test]
    async fn test_client_empty_response() {
        let mock_server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path("/sprites/my-sprite"))
            .respond_with(ResponseTemplate::new(200).set_body_string(""))
            .mount(&mock_server)
            .await;

        let client = SpritesClient::with_base_url("test_token".to_string(), mock_server.uri());
        let result = client.delete_sprite("my-sprite").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_client_malformed_response() {
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/sprites/bad"))
            .respond_with(ResponseTemplate::new(200).set_body_string("not valid json"))
            .mount(&mock_server)
            .await;

        let client = SpritesClient::with_base_url("test_token".to_string(), mock_server.uri());
        let result = client.get_sprite("bad").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid JSON"));
    }

    #[tokio::test]
    async fn test_client_sends_bearer_auth() {
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/sprites/auth-test"))
            .and(wiremock::matchers::header(
                "Authorization",
                "Bearer secret_token_123",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "name": "auth-test",
                "status": "running"
            })))
            .expect(1)
            .mount(&mock_server)
            .await;

        let client =
            SpritesClient::with_base_url("secret_token_123".to_string(), mock_server.uri());
        let info = client.get_sprite("auth-test").await.unwrap();
        assert_eq!(info.name, "auth-test");
    }

    #[test]
    fn test_parse_exec_stdout_and_exit_code() {
        let result = parse_exec_result(&[0x01, b'h', b'i', b'\n', 0x03, 0]).unwrap();
        assert_eq!(result.stdout, "hi\n");
        assert_eq!(result.stderr, "");
        assert_eq!(result.exit_code, 0);
    }

    #[test]
    fn test_parse_exec_stderr_and_nonzero_exit() {
        let result = parse_exec_result(&[0x02, b'e', b'r', b'r', 0x03, 42]).unwrap();
        assert_eq!(result.stdout, "");
        assert_eq!(result.stderr, "err");
        assert_eq!(result.exit_code, 42);
    }

    #[test]
    fn test_parse_exec_mixed_streams() {
        let result =
            parse_exec_result(&[0x02, b'e', b'r', b'r', 0x01, b'o', b'u', b't', 0x03, 7]).unwrap();
        assert_eq!(result.stdout, "out");
        assert_eq!(result.stderr, "err");
        assert_eq!(result.exit_code, 7);
    }

    #[test]
    fn test_checkpoint_from_stream_prefers_complete_event() {
        let checkpoint = checkpoint_from_stream(
            "{\"type\":\"info\",\"data\":\"Creating checkpoint...\",\"time\":\"2026-03-23T10:05:00Z\"}\n{\"type\":\"complete\",\"data\":\"Checkpoint v9 created successfully\",\"time\":\"2026-03-23T10:05:01Z\"}\n",
        )
        .unwrap();
        assert_eq!(checkpoint.id, "v9");
        assert_eq!(
            checkpoint.created_at.as_deref(),
            Some("2026-03-23T10:05:01Z")
        );
    }
}

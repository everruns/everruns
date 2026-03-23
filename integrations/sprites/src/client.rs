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

    /// Raw GET that returns bytes (for file download).
    async fn download(&self, path: &str) -> Result<Vec<u8>, String> {
        let url = format!("{}{}", self.api_base, path);
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

    // --- Sprite Lifecycle ---

    pub async fn create_sprite(&self, name: &str, body: Value) -> Result<SpriteInfo, String> {
        let resp = self
            .request(
                reqwest::Method::PUT,
                &format!("/sprites/{name}"),
                Some(body),
            )
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
        timeout_ms: Option<u64>,
    ) -> Result<ExecResult, String> {
        let mut body = json!({ "command": command });
        if let Some(t) = timeout_ms {
            body["timeout"] = json!(t);
        }
        let resp = self
            .request(
                reqwest::Method::POST,
                &format!("/sprites/{name}/exec"),
                Some(body),
            )
            .await?;
        serde_json::from_value(resp).map_err(|e| format!("Failed to parse exec result: {e}"))
    }

    // --- Filesystem ---

    pub async fn read_file(&self, name: &str, path: &str) -> Result<Vec<u8>, String> {
        let encoded = urlencoding::encode(path);
        self.download(&format!("/sprites/{name}/files/{encoded}"))
            .await
    }

    pub async fn write_file(&self, name: &str, path: &str, content: &[u8]) -> Result<(), String> {
        let encoded = urlencoding::encode(path);
        let url = format!("{}/sprites/{name}/files/{encoded}", self.api_base);

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
        let resp = self
            .request(
                reqwest::Method::POST,
                &format!("/sprites/{name}/checkpoints"),
                None,
            )
            .await?;
        serde_json::from_value(resp).map_err(|e| format!("Failed to parse checkpoint info: {e}"))
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
        self.request(
            reqwest::Method::POST,
            &format!("/sprites/{name}/checkpoints/{checkpoint_id}/restore"),
            None,
        )
        .await?;
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

pub(crate) mod urlencoding {
    /// Percent-encode a string for use in URL path segments.
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
    async fn test_client_create_sprite() {
        let mock_server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/sprites/test-sprite"))
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
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
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
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "stdout": "hello world\n",
                "stderr": "",
                "exit_code": 0
            })))
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
            .and(path("/sprites/my-sprite/checkpoints"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "cp_abc123"
            })))
            .mount(&mock_server)
            .await;

        let client = SpritesClient::with_base_url("test_token".to_string(), mock_server.uri());
        let result = client.create_checkpoint("my-sprite").await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap().id, "cp_abc123");
    }

    #[tokio::test]
    async fn test_client_restore_checkpoint() {
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/sprites/my-sprite/checkpoints/cp_abc123/restore"))
            .respond_with(ResponseTemplate::new(200).set_body_string(""))
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
        Mock::given(method("PUT"))
            .and(path("/sprites/test"))
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
            .and(path("/sprites/my-sprite/files/%2Fhome%2Fuser%2Fmain.py"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"print('hello')\n".to_vec()))
            .mount(&mock_server)
            .await;

        let client = SpritesClient::with_base_url("test_token".to_string(), mock_server.uri());
        let result = client.read_file("my-sprite", "/home/user/main.py").await;
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
            .and(path("/sprites/my-sprite/files/%2Fhome%2Fuser%2Ftest.py"))
            .respond_with(ResponseTemplate::new(200).set_body_string(""))
            .mount(&mock_server)
            .await;

        let client = SpritesClient::with_base_url("test_token".to_string(), mock_server.uri());
        let result = client
            .write_file("my-sprite", "/home/user/test.py", b"print('test')")
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
                {"id": "cp_1"},
                {"id": "cp_2"}
            ])))
            .mount(&mock_server)
            .await;

        let client = SpritesClient::with_base_url("test_token".to_string(), mock_server.uri());
        let result = client.list_checkpoints("my-sprite").await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap().len(), 2);
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
    fn test_encode_simple() {
        assert_eq!(urlencoding::encode("hello"), "hello");
    }

    #[test]
    fn test_encode_path_with_slashes() {
        assert_eq!(
            urlencoding::encode("/home/user/main.py"),
            "%2Fhome%2Fuser%2Fmain.py"
        );
    }

    #[test]
    fn test_encode_preserves_unreserved() {
        assert_eq!(urlencoding::encode("abc-_.~123"), "abc-_.~123");
    }

    #[test]
    fn test_encode_empty_string() {
        assert_eq!(urlencoding::encode(""), "");
    }
}

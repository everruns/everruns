// Remote session filesystem API client
//
// TODO(sdk): Replace all raw reqwest calls with SDK session_files() methods.
// SDK v0.1.5 ships session filesystem support (https://github.com/everruns/sdk/issues/60 resolved).
// Migration is tracked separately — involves adapting to SDK's FileInfo/SessionFile types.
// Affected methods: list, read_file, write_file, create_dir, delete.
//
// Design Decision: Binary detection mirrors server logic (null bytes in first 8KB).

use anyhow::{Context, Result};
use base64::Engine;
use serde::{Deserialize, Serialize};

/// Represents a file entry returned by the remote API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteFileEntry {
    pub path: String,
    pub is_directory: bool,
    #[serde(default)]
    pub size_bytes: i64,
    #[serde(default)]
    pub content_hash: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
    #[serde(default)]
    pub is_readonly: bool,
}

/// File content with encoding info.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteFileContent {
    pub path: String,
    pub content: String,
    #[serde(default = "default_encoding")]
    pub encoding: String,
    #[serde(default)]
    pub content_hash: Option<String>,
    #[serde(default)]
    pub is_directory: bool,
    #[serde(default)]
    pub updated_at: Option<String>,
}

fn default_encoding() -> String {
    "text".to_string()
}

/// Client for the session filesystem API.
pub struct RemoteClient {
    http: reqwest::Client,
    base_url: String,
    session_id: String,
    api_key: String,
}

impl RemoteClient {
    pub fn new(api_url: &str, api_key: &str, session_id: &str) -> Self {
        Self {
            http: reqwest::Client::new(),
            base_url: api_url.trim_end_matches('/').to_string(),
            session_id: session_id.to_string(),
            api_key: api_key.to_string(),
        }
    }

    fn fs_url(&self, path: &str) -> String {
        let clean = path.trim_start_matches('/');
        if clean.is_empty() {
            format!("{}/v1/sessions/{}/fs", self.base_url, self.session_id)
        } else {
            format!(
                "{}/v1/sessions/{}/fs/{}",
                self.base_url, self.session_id, clean
            )
        }
    }

    /// List files at a path. If recursive, lists the entire tree.
    pub async fn list(&self, path: &str, recursive: bool) -> Result<Vec<RemoteFileEntry>> {
        let mut url = self.fs_url(path);
        if recursive {
            let sep = if url.contains('?') { '&' } else { '?' };
            url = format!("{}{}recursive=true", url, sep);
        }

        let resp = self
            .http
            .get(&url)
            .header("Authorization", &self.api_key)
            .send()
            .await
            .context("Failed to list remote files")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("List remote files failed: {} {}", status, text);
        }

        // The API returns either a file object or a directory listing.
        // Directory listing has a "data" array.
        let body: serde_json::Value = resp.json().await?;

        if let Some(data) = body
            .get("data")
            .and_then(|d| d.as_array())
            .or_else(|| body.get("entries").and_then(|e| e.as_array()))
        {
            let files: Vec<RemoteFileEntry> =
                serde_json::from_value(serde_json::Value::Array(data.clone()))?;
            Ok(files)
        } else {
            // Single file
            let entry: RemoteFileEntry = serde_json::from_value(body)?;
            Ok(vec![entry])
        }
    }

    /// Read file content.
    pub async fn read_file(&self, path: &str) -> Result<RemoteFileContent> {
        let url = self.fs_url(path);
        let resp = self
            .http
            .get(&url)
            .header("Authorization", &self.api_key)
            .send()
            .await
            .context("Failed to read remote file")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("Read remote file failed: {} {}", status, text);
        }

        let content: RemoteFileContent = resp.json().await?;
        Ok(content)
    }

    /// Decode file content bytes from the remote response.
    pub fn decode_content(content: &RemoteFileContent) -> Result<Vec<u8>> {
        if content.encoding == "base64" {
            base64::engine::general_purpose::STANDARD
                .decode(&content.content)
                .context("Failed to decode base64 content")
        } else {
            Ok(content.content.as_bytes().to_vec())
        }
    }

    /// Create or update a file on remote.
    pub async fn write_file(&self, path: &str, content: &[u8], create: bool) -> Result<()> {
        let url = self.fs_url(path);

        // Detect binary: null bytes in first 8KB
        let is_binary = content.iter().take(8192).any(|&b| b == 0);
        let (encoded, encoding) = if is_binary {
            (
                base64::engine::general_purpose::STANDARD.encode(content),
                "base64",
            )
        } else {
            (String::from_utf8_lossy(content).into_owned(), "text")
        };

        let body = serde_json::json!({
            "content": encoded,
            "encoding": encoding,
        });

        let resp = if create {
            self.http
                .post(&url)
                .header("Authorization", &self.api_key)
                .header("Content-Type", "application/json")
                .json(&body)
                .send()
                .await
                .context("Failed to create remote file")?
        } else {
            self.http
                .put(&url)
                .header("Authorization", &self.api_key)
                .header("Content-Type", "application/json")
                .json(&body)
                .send()
                .await
                .context("Failed to update remote file")?
        };

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            // If create fails with 409 (conflict/exists), fall back to update
            if create && status == reqwest::StatusCode::CONFLICT {
                return self.update_file(path, &body).await;
            }
            anyhow::bail!("Write remote file failed: {} {}", status, text);
        }

        Ok(())
    }

    /// Update-only helper (avoids async recursion in write_file).
    async fn update_file(&self, path: &str, body: &serde_json::Value) -> Result<()> {
        let url = self.fs_url(path);
        let resp = self
            .http
            .put(&url)
            .header("Authorization", &self.api_key)
            .header("Content-Type", "application/json")
            .json(body)
            .send()
            .await
            .context("Failed to update remote file")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("Update remote file failed: {} {}", status, text);
        }
        Ok(())
    }

    /// Delete a file or directory on remote.
    pub async fn delete(&self, path: &str, recursive: bool) -> Result<()> {
        let mut url = self.fs_url(path);
        if recursive {
            url = format!("{}?recursive=true", url);
        }

        let resp = self
            .http
            .delete(&url)
            .header("Authorization", &self.api_key)
            .send()
            .await
            .context("Failed to delete remote file")?;

        if !resp.status().is_success() && resp.status() != reqwest::StatusCode::NOT_FOUND {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("Delete remote file failed: {} {}", status, text);
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_client() -> RemoteClient {
        RemoteClient::new("https://api.example.com", "test-key", "ses_123")
    }

    #[test]
    fn test_fs_url_root() {
        let client = test_client();
        assert_eq!(
            client.fs_url("/"),
            "https://api.example.com/v1/sessions/ses_123/fs"
        );
    }

    #[test]
    fn test_fs_url_empty() {
        let client = test_client();
        assert_eq!(
            client.fs_url(""),
            "https://api.example.com/v1/sessions/ses_123/fs"
        );
    }

    #[test]
    fn test_fs_url_file() {
        let client = test_client();
        assert_eq!(
            client.fs_url("/src/main.rs"),
            "https://api.example.com/v1/sessions/ses_123/fs/src/main.rs"
        );
    }

    #[test]
    fn test_fs_url_nested() {
        let client = test_client();
        assert_eq!(
            client.fs_url("/a/b/c/d.txt"),
            "https://api.example.com/v1/sessions/ses_123/fs/a/b/c/d.txt"
        );
    }

    #[test]
    fn test_fs_url_trailing_slash_stripped() {
        let client = RemoteClient::new("https://api.example.com/", "key", "ses_1");
        assert_eq!(
            client.fs_url("/file.txt"),
            "https://api.example.com/v1/sessions/ses_1/fs/file.txt"
        );
    }

    #[test]
    fn test_decode_content_text() {
        let content = RemoteFileContent {
            path: "/hello.txt".to_string(),
            content: "hello world".to_string(),
            encoding: "text".to_string(),
            content_hash: None,
            is_directory: false,
            updated_at: None,
        };
        let bytes = RemoteClient::decode_content(&content).unwrap();
        assert_eq!(bytes, b"hello world");
    }

    #[test]
    fn test_decode_content_base64() {
        let content = RemoteFileContent {
            path: "/binary.bin".to_string(),
            content: base64::engine::general_purpose::STANDARD.encode(b"\x00\x01\x02\x03"),
            encoding: "base64".to_string(),
            content_hash: None,
            is_directory: false,
            updated_at: None,
        };
        let bytes = RemoteClient::decode_content(&content).unwrap();
        assert_eq!(bytes, &[0x00, 0x01, 0x02, 0x03]);
    }

    #[test]
    fn test_decode_content_invalid_base64() {
        let content = RemoteFileContent {
            path: "/bad.bin".to_string(),
            content: "not-valid-base64!!!".to_string(),
            encoding: "base64".to_string(),
            content_hash: None,
            is_directory: false,
            updated_at: None,
        };
        assert!(RemoteClient::decode_content(&content).is_err());
    }

    #[test]
    fn test_default_encoding() {
        assert_eq!(default_encoding(), "text");
    }

    #[test]
    fn test_remote_file_entry_deserialize() {
        let json = serde_json::json!({
            "path": "/src/lib.rs",
            "is_directory": false,
            "size_bytes": 1024,
            "content_hash": "sha256:abc123",
            "updated_at": "2026-03-20T12:00:00Z"
        });
        let entry: RemoteFileEntry = serde_json::from_value(json).unwrap();
        assert_eq!(entry.path, "/src/lib.rs");
        assert!(!entry.is_directory);
        assert_eq!(entry.size_bytes, 1024);
        assert_eq!(entry.content_hash.as_deref(), Some("sha256:abc123"));
        assert!(!entry.is_readonly);
    }

    #[test]
    fn test_remote_file_entry_deserialize_minimal() {
        let json = serde_json::json!({
            "path": "/test",
            "is_directory": true
        });
        let entry: RemoteFileEntry = serde_json::from_value(json).unwrap();
        assert_eq!(entry.path, "/test");
        assert!(entry.is_directory);
        assert_eq!(entry.size_bytes, 0); // default
        assert!(entry.content_hash.is_none());
        assert!(entry.updated_at.is_none());
    }

    #[test]
    fn test_remote_file_content_deserialize() {
        let json = serde_json::json!({
            "path": "/hello.txt",
            "content": "hello world",
        });
        let content: RemoteFileContent = serde_json::from_value(json).unwrap();
        assert_eq!(content.path, "/hello.txt");
        assert_eq!(content.content, "hello world");
        assert_eq!(content.encoding, "text"); // default
        assert!(!content.is_directory);
    }
}

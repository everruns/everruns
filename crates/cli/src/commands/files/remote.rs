// Remote session filesystem API client
//
// Design Decision: Direct reqwest calls since session filesystem is not in the SDK yet.
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
        // Directory listing has an "entries" array.
        let body: serde_json::Value = resp.json().await?;

        if let Some(entries) = body.get("entries").and_then(|e| e.as_array()) {
            let files: Vec<RemoteFileEntry> =
                serde_json::from_value(serde_json::Value::Array(entries.clone()))?;
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

    /// Create a directory on remote (used by sync for mkdir-on-demand).
    #[allow(dead_code)]
    pub async fn create_dir(&self, path: &str) -> Result<()> {
        let url = self.fs_url(path);
        let body = serde_json::json!({ "is_directory": true });

        let resp = self
            .http
            .post(&url)
            .header("Authorization", &self.api_key)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .context("Failed to create remote directory")?;

        // Ignore 409 (already exists)
        if !resp.status().is_success() && resp.status() != reqwest::StatusCode::CONFLICT {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("Create remote dir failed: {} {}", status, text);
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

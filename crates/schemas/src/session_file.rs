// Session File domain types (Virtual Filesystem)
//
// These types represent files and directories stored within a session's
// virtual filesystem. Each session has its own isolated filesystem.

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

/// File metadata without content
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct FileInfo {
    pub id: Uuid,
    pub session_id: Uuid,
    pub path: String,
    pub name: String,
    pub is_directory: bool,
    pub is_readonly: bool,
    pub size_bytes: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl FileInfo {
    /// Extract file name from path
    pub fn name_from_path(path: &str) -> String {
        if path == "/" {
            "/".to_string()
        } else {
            path.rsplit('/').next().unwrap_or(path).to_string()
        }
    }

    /// Get parent directory path
    pub fn parent_path(path: &str) -> Option<String> {
        if path == "/" {
            None
        } else {
            let parent = path.rsplit_once('/').map(|(p, _)| p).unwrap_or("/");
            Some(if parent.is_empty() { "/" } else { parent }.to_string())
        }
    }
}

/// Complete file with content
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct SessionFile {
    pub id: Uuid,
    pub session_id: Uuid,
    pub path: String,
    pub name: String,
    /// Base64-encoded content for binary files, plain text for text files
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    /// Content encoding: "text" or "base64"
    #[serde(default = "default_encoding")]
    pub encoding: String,
    pub is_directory: bool,
    pub is_readonly: bool,
    pub size_bytes: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

fn default_encoding() -> String {
    "text".to_string()
}

impl SessionFile {
    /// Check if content is likely text based on bytes
    pub fn is_text_content(bytes: &[u8]) -> bool {
        // Quick heuristic: check first 8KB for null bytes
        let check_len = bytes.len().min(8192);
        !bytes[..check_len].contains(&0)
    }

    /// Convert raw bytes to content string with appropriate encoding
    pub fn encode_content(bytes: &[u8]) -> (String, String) {
        if Self::is_text_content(bytes) {
            match String::from_utf8(bytes.to_vec()) {
                Ok(text) => (text, "text".to_string()),
                Err(_) => (BASE64.encode(bytes), "base64".to_string()),
            }
        } else {
            (BASE64.encode(bytes), "base64".to_string())
        }
    }

    /// Decode content string to raw bytes
    pub fn decode_content(content: &str, encoding: &str) -> Result<Vec<u8>, base64::DecodeError> {
        match encoding {
            "base64" => BASE64.decode(content),
            _ => Ok(content.as_bytes().to_vec()),
        }
    }
}

/// File stat information
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct FileStat {
    pub path: String,
    pub name: String,
    pub is_directory: bool,
    pub is_readonly: bool,
    pub size_bytes: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Grep match result
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct GrepMatch {
    pub path: String,
    pub line_number: usize,
    pub line: String,
}

/// Grep result for a file
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct GrepResult {
    pub path: String,
    pub matches: Vec<GrepMatch>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_name_from_path() {
        assert_eq!(FileInfo::name_from_path("/"), "/");
        assert_eq!(FileInfo::name_from_path("/foo"), "foo");
        assert_eq!(FileInfo::name_from_path("/foo/bar"), "bar");
    }

    #[test]
    fn test_parent_path() {
        assert_eq!(FileInfo::parent_path("/"), None);
        assert_eq!(FileInfo::parent_path("/foo"), Some("/".to_string()));
        assert_eq!(FileInfo::parent_path("/foo/bar"), Some("/foo".to_string()));
    }

    #[test]
    fn test_is_text_content() {
        assert!(SessionFile::is_text_content(b"hello world"));
        assert!(!SessionFile::is_text_content(b"hello\0world"));
    }

    #[test]
    fn test_encode_content_text() {
        let (content, encoding) = SessionFile::encode_content(b"hello world");
        assert_eq!(content, "hello world");
        assert_eq!(encoding, "text");
    }
}

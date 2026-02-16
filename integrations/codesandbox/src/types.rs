//! Data types, constants, and utility functions for the CodeSandbox integration.

use serde::{Deserialize, Serialize};
use std::time::Duration;

// ============================================================================
// Constants
// ============================================================================

pub const CSB_API_BASE: &str = "https://api.codesandbox.io";
pub const CSB_API_KEY_SECRET: &str = "CSB_API_KEY";
pub const CSB_SANDBOX_SECRET_PREFIX: &str = "csb_sandbox:";
pub const EXEC_POLL_INTERVAL: Duration = Duration::from_millis(500);
pub const EXEC_POLL_MAX_WAIT: Duration = Duration::from_secs(120);
pub const SSE_READ_TIMEOUT: Duration = Duration::from_secs(5);
pub const PINT_READY_POLL_INTERVAL: Duration = Duration::from_secs(2);
pub const PINT_READY_MAX_WAIT: Duration = Duration::from_secs(30);
/// Auto-hibernate after 5 minutes of inactivity (safety net)
pub const HIBERNATE_TIMEOUT_SECS: u64 = 300;

// ============================================================================
// API Response Types
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxInfo {
    pub id: String,
    pub title: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VmStartResponse {
    pub pitcher_url: String,
    pub pitcher_token: String,
    pub workspace_path: Option<String>,
    /// Pint API URL (newer API, preferred when use_pint is true)
    pub pint_url: Option<String>,
    /// Pint API token (newer API, preferred when use_pint is true)
    pub pint_token: Option<String>,
    /// Whether to use pint_url/pint_token instead of pitcher_url/pitcher_token
    pub use_pint: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecInfo {
    pub id: String,
    pub status: String,
    #[serde(rename = "exitCode")]
    pub exit_code: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileContent {
    pub path: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DirEntry {
    pub name: String,
    /// Full path of the entry (e.g. "/workspace/src")
    #[serde(default)]
    pub path: String,
    /// Whether this entry is a directory
    #[serde(default)]
    pub is_dir: bool,
    /// File size in bytes
    #[serde(default)]
    pub size: u64,
}

/// Wrapper for the Pint directory listing response: `{"files": [...], "path": "..."}`
#[derive(Debug, Clone, Deserialize)]
pub struct DirListResponse {
    pub files: Vec<DirEntry>,
}

// ============================================================================
// Persisted Sandbox State (stored in session secrets as JSON)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreviewTokenResponse {
    pub token: PreviewTokenInfo,
    pub sandbox_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreviewTokenInfo {
    pub token: String,
    pub token_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxState {
    pub sandbox_id: String,
    /// Pint API base URL: https://{sandbox_id}-57468.csb.app
    pub pint_url: String,
    pub pitcher_token: String,
    pub preview_token: String,
    pub workspace_path: String,
    pub started_at: String,
}

// NOTE: Template alias resolution was removed. The CodeSandbox `template` field
// caused 500 errors from their API for most template IDs (tested 2026-02-15).
// Sandboxes created without a template work fine. See specs/codesandbox.md for details.

// ============================================================================
// Utility Functions
// ============================================================================

/// URL-encode a file path for use in Pint API URLs, preserving slash separators.
pub fn encode_path(path: &str) -> String {
    // Strip leading slash and pass through — CodeSandbox Pint API accepts
    // unencoded paths in practice. Only encode spaces as %20.
    path.trim_start_matches('/').replace(' ', "%20")
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- State serialization tests ---

    #[test]
    fn test_sandbox_state_roundtrip() {
        let state = SandboxState {
            sandbox_id: "sb_123".to_string(),
            pint_url: "https://sb_123-57468.csb.app".to_string(),
            pitcher_token: "tok_abc".to_string(),
            preview_token: "prv_v1_test123".to_string(),
            workspace_path: "/project".to_string(),
            started_at: "2026-02-13T10:00:00Z".to_string(),
        };
        let json = serde_json::to_string(&state).unwrap();
        let deserialized: SandboxState = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.sandbox_id, "sb_123");
        assert_eq!(deserialized.pint_url, "https://sb_123-57468.csb.app");
        assert_eq!(deserialized.pitcher_token, "tok_abc");
        assert_eq!(deserialized.preview_token, "prv_v1_test123");
        assert_eq!(deserialized.workspace_path, "/project");
    }

    #[test]
    fn test_sandbox_state_with_special_chars() {
        let state = SandboxState {
            sandbox_id: "sb-test_123".to_string(),
            pint_url: "https://sb-test_123-57468.csb.app".to_string(),
            pitcher_token: "tok+abc/def==".to_string(),
            preview_token: "prv_v1_special+chars==".to_string(),
            workspace_path: "/home/user/my project".to_string(),
            started_at: "2026-02-13T10:00:00+05:30".to_string(),
        };
        let json = serde_json::to_string(&state).unwrap();
        let deserialized: SandboxState = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.pint_url, "https://sb-test_123-57468.csb.app");
        assert_eq!(deserialized.pitcher_token, "tok+abc/def==");
    }

    // --- URL encoding tests ---

    #[test]
    fn test_encode_path_simple() {
        assert_eq!(encode_path("/project/main.py"), "project/main.py");
    }

    #[test]
    fn test_encode_path_with_spaces() {
        let encoded = encode_path("/my project/file name.txt");
        assert!(encoded.contains("my%20project"));
        assert!(encoded.contains("file%20name.txt"));
    }

    #[test]
    fn test_encode_path_preserves_slashes() {
        assert_eq!(encode_path("/a/b/c/d.txt"), "a/b/c/d.txt");
    }

    #[test]
    fn test_encode_path_no_leading_slash() {
        assert_eq!(encode_path("project/main.py"), "project/main.py");
    }

    #[test]
    fn test_encode_path_empty() {
        assert_eq!(encode_path(""), "");
    }

    // --- DirListResponse deserialization ---

    #[test]
    fn test_dir_list_response_deserialize() {
        let json =
            r#"{"files": [{"name": "a.txt", "path": "/a.txt", "isDir": false, "size": 10}]}"#;
        let resp: DirListResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.files.len(), 1);
        assert_eq!(resp.files[0].name, "a.txt");
    }

    #[test]
    fn test_dir_entry_defaults() {
        let json = r#"{"name": "x"}"#;
        let entry: DirEntry = serde_json::from_str(json).unwrap();
        assert_eq!(entry.name, "x");
        assert_eq!(entry.path, "");
        assert!(!entry.is_dir);
        assert_eq!(entry.size, 0);
    }

    #[test]
    fn test_vm_start_response_minimal() {
        let json = r#"{"pitcher_url": "https://p.test", "pitcher_token": "tok"}"#;
        let resp: VmStartResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.pitcher_url, "https://p.test");
        assert!(resp.workspace_path.is_none());
        assert!(resp.pint_url.is_none());
        assert!(resp.use_pint.is_none());
    }

    #[test]
    fn test_exec_info_without_exit_code() {
        let json = r#"{"id": "e1", "status": "running"}"#;
        let info: ExecInfo = serde_json::from_str(json).unwrap();
        assert_eq!(info.id, "e1");
        assert!(info.exit_code.is_none());
    }

    #[test]
    fn test_exec_info_with_exit_code() {
        let json = r#"{"id": "e1", "status": "exited", "exitCode": 0}"#;
        let info: ExecInfo = serde_json::from_str(json).unwrap();
        assert_eq!(info.exit_code, Some(0));
    }

    #[test]
    fn test_preview_token_response_deserialize() {
        let json = r#"{"token": {"token": "tok_abc", "token_id": "tid_1"}, "sandbox_id": "sb1"}"#;
        let resp: PreviewTokenResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.token.token, "tok_abc");
        assert_eq!(resp.sandbox_id, "sb1");
    }
}

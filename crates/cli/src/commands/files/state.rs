// Sync state persistence
//
// Design Decision: State stored in <local-dir>/.everruns-sync/state.json.
// Design Decision: Content hashes used for incremental sync (skip unchanged files).

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Per-file sync state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileSyncState {
    pub local_hash: Option<String>,
    pub remote_hash: Option<String>,
    pub local_mtime: Option<String>,
    pub remote_updated_at: Option<String>,
}

/// Root sync state persisted to disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncState {
    pub session_id: String,
    pub last_sync: Option<String>,
    pub files: HashMap<String, FileSyncState>,
}

impl SyncState {
    pub fn new(session_id: &str) -> Self {
        Self {
            session_id: session_id.to_string(),
            last_sync: None,
            files: HashMap::new(),
        }
    }

    /// Load state from disk, or create fresh if missing/corrupt.
    pub fn load(state_dir: &Path, session_id: &str) -> Result<Self> {
        let path = state_dir.join("state.json");
        if path.exists() {
            let data = std::fs::read_to_string(&path).context("Read sync state")?;
            match serde_json::from_str::<SyncState>(&data) {
                Ok(state) if state.session_id == session_id => Ok(state),
                _ => {
                    eprintln!("Sync state mismatch or corrupt, starting fresh");
                    Ok(Self::new(session_id))
                }
            }
        } else {
            Ok(Self::new(session_id))
        }
    }

    /// Persist state to disk.
    pub fn save(&self, state_dir: &Path) -> Result<()> {
        std::fs::create_dir_all(state_dir).context("Create sync state dir")?;
        let path = state_dir.join("state.json");
        let data = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, data).context("Write sync state")?;
        Ok(())
    }
}

/// Compute sha256 hash of bytes, returning "sha256:<hex>" string.
pub fn content_hash(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let result = hasher.finalize();
    format!("sha256:{}", hex::encode(result))
}

/// Hex encoding for sha256 (inline, avoids hex crate dep).
mod hex {
    pub fn encode(bytes: impl AsRef<[u8]>) -> String {
        bytes
            .as_ref()
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect()
    }
}

/// Return the state directory path for a given local dir.
pub fn state_dir(local_dir: &Path) -> PathBuf {
    local_dir.join(".everruns-sync")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_content_hash_deterministic() {
        let h1 = content_hash(b"hello world");
        let h2 = content_hash(b"hello world");
        assert_eq!(h1, h2);
        assert!(h1.starts_with("sha256:"));
    }

    #[test]
    fn test_content_hash_different_content() {
        let h1 = content_hash(b"hello");
        let h2 = content_hash(b"world");
        assert_ne!(h1, h2);
    }

    #[test]
    fn test_content_hash_empty() {
        let h = content_hash(b"");
        assert!(h.starts_with("sha256:"));
        // SHA256 of empty string is well-known
        assert_eq!(
            h,
            "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn test_content_hash_binary() {
        let h = content_hash(&[0x00, 0x01, 0xff, 0xfe]);
        assert!(h.starts_with("sha256:"));
        assert_eq!(h.len(), 7 + 64); // "sha256:" + 64 hex chars
    }

    #[test]
    fn test_state_new() {
        let state = SyncState::new("ses_123");
        assert_eq!(state.session_id, "ses_123");
        assert!(state.last_sync.is_none());
        assert!(state.files.is_empty());
    }

    #[test]
    fn test_state_save_and_load() {
        let dir = tempfile::tempdir().unwrap();
        let sd = dir.path().join(".everruns-sync");

        let mut state = SyncState::new("ses_abc");
        state.last_sync = Some("2026-01-01T00:00:00Z".to_string());
        state.files.insert(
            "src/main.rs".to_string(),
            FileSyncState {
                local_hash: Some("sha256:aaa".to_string()),
                remote_hash: Some("sha256:bbb".to_string()),
                local_mtime: None,
                remote_updated_at: None,
            },
        );

        state.save(&sd).unwrap();

        let loaded = SyncState::load(&sd, "ses_abc").unwrap();
        assert_eq!(loaded.session_id, "ses_abc");
        assert_eq!(loaded.last_sync, Some("2026-01-01T00:00:00Z".to_string()));
        assert!(loaded.files.contains_key("src/main.rs"));
        let file = &loaded.files["src/main.rs"];
        assert_eq!(file.local_hash.as_deref(), Some("sha256:aaa"));
        assert_eq!(file.remote_hash.as_deref(), Some("sha256:bbb"));
    }

    #[test]
    fn test_state_load_missing_returns_fresh() {
        let dir = tempfile::tempdir().unwrap();
        let sd = dir.path().join(".nonexistent");

        let state = SyncState::load(&sd, "ses_xyz").unwrap();
        assert_eq!(state.session_id, "ses_xyz");
        assert!(state.files.is_empty());
    }

    #[test]
    fn test_state_load_session_mismatch_returns_fresh() {
        let dir = tempfile::tempdir().unwrap();
        let sd = dir.path().join(".everruns-sync");

        let state = SyncState::new("ses_old");
        state.save(&sd).unwrap();

        // Load with different session_id
        let loaded = SyncState::load(&sd, "ses_new").unwrap();
        assert_eq!(loaded.session_id, "ses_new");
        assert!(loaded.files.is_empty());
    }

    #[test]
    fn test_state_load_corrupt_returns_fresh() {
        let dir = tempfile::tempdir().unwrap();
        let sd = dir.path().join(".everruns-sync");
        fs::create_dir_all(&sd).unwrap();
        fs::write(sd.join("state.json"), "not json at all").unwrap();

        let state = SyncState::load(&sd, "ses_abc").unwrap();
        assert_eq!(state.session_id, "ses_abc");
        assert!(state.files.is_empty());
    }

    #[test]
    fn test_state_dir_path() {
        let p = state_dir(Path::new("/home/user/project"));
        assert_eq!(p, PathBuf::from("/home/user/project/.everruns-sync"));
    }

    #[test]
    fn test_hex_encode() {
        assert_eq!(hex::encode([0xde, 0xad, 0xbe, 0xef]), "deadbeef");
        assert_eq!(hex::encode([0x00, 0xff]), "00ff");
        assert_eq!(hex::encode([]), "");
    }
}

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

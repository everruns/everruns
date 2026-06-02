// Storage quota limits for session files (TM-FS-008 / TM-DOS-005).
//
// Defaults are generous to avoid throttling legitimate long-horizon agentic
// runs, while bounding Postgres BYTEA exhaustion. Operators can tighten via
// env without redeploying code.
//
// Limits are read from env once at service construction time (via QuotaLimits::from_env),
// not on every write, to avoid per-call overhead and process-global env mutation in tests.

use crate::storage::StorageBackend;
use anyhow::{Result, anyhow};
use uuid::Uuid;

const DEFAULT_SESSION_MAX: i64 = 500 * 1024 * 1024;
const DEFAULT_PER_FILE_MAX: i64 = 100 * 1024 * 1024;

/// Byte-level quota configuration for session file writes.
#[derive(Clone, Copy, Debug)]
pub struct QuotaLimits {
    /// Maximum bytes for a single file write.
    pub per_file_bytes: i64,
    /// Maximum total bytes stored as session files per session.
    pub per_session_bytes: i64,
}

impl Default for QuotaLimits {
    fn default() -> Self {
        Self::from_env()
    }
}

impl QuotaLimits {
    /// Read limits from env vars (`SESSION_FILE_SINGLE_MAX_BYTES`,
    /// `SESSION_FILE_MAX_BYTES`), falling back to generous defaults.
    pub fn from_env() -> Self {
        Self {
            per_file_bytes: read_bytes_env("SESSION_FILE_SINGLE_MAX_BYTES", DEFAULT_PER_FILE_MAX),
            per_session_bytes: read_bytes_env("SESSION_FILE_MAX_BYTES", DEFAULT_SESSION_MAX),
        }
    }
}

fn read_bytes_env(var: &str, default: i64) -> i64 {
    std::env::var(var)
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .filter(|&v| v > 0)
        .unwrap_or(default)
}

/// Check quota before writing `incoming_bytes` to a session file.
///
/// Pass `existing_bytes = 0` for new files. The session-total DB query is
/// skipped when the write would not increase net storage (delta ≤ 0).
pub async fn check_write_quota(
    db: &StorageBackend,
    session_id: Uuid,
    incoming_bytes: i64,
    existing_bytes: i64,
    limits: &QuotaLimits,
) -> Result<()> {
    if incoming_bytes > limits.per_file_bytes {
        return Err(anyhow!(
            "File too large: {} bytes exceeds the per-file limit of {} bytes",
            incoming_bytes,
            limits.per_file_bytes
        ));
    }
    let delta = incoming_bytes - existing_bytes;
    if delta > 0 {
        let current_total = db.total_session_file_bytes(session_id).await?;
        let new_total = current_total.saturating_add(delta);
        if new_total > limits.per_session_bytes {
            return Err(anyhow!(
                "Session storage quota exceeded: writing {} bytes would bring total to {} bytes (limit {} bytes)",
                incoming_bytes,
                new_total,
                limits.per_session_bytes
            ));
        }
    }
    Ok(())
}

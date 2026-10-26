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
// Decision: with object storage (`STORAGE_BLOB_BACKEND=s3`) the bytes never
// touch PostgreSQL and uploads and downloads stream, so the defaults fit
// recordings and media. The inline backend keeps the smaller ones above.
const OBJECT_STORAGE_SESSION_MAX: i64 = 10 * 1024 * 1024 * 1024;
const OBJECT_STORAGE_PER_FILE_MAX: i64 = 2 * 1024 * 1024 * 1024;

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
    /// `SESSION_FILE_MAX_BYTES`), falling back to defaults that depend on
    /// where file bytes are stored (`STORAGE_BLOB_BACKEND`).
    pub fn from_env() -> Self {
        let object_storage = std::env::var("STORAGE_BLOB_BACKEND").is_ok_and(|v| v == "s3");
        Self::from_env_with_defaults(object_storage)
    }

    fn from_env_with_defaults(object_storage: bool) -> Self {
        let (per_file, per_session) = if object_storage {
            (OBJECT_STORAGE_PER_FILE_MAX, OBJECT_STORAGE_SESSION_MAX)
        } else {
            (DEFAULT_PER_FILE_MAX, DEFAULT_SESSION_MAX)
        };
        Self {
            per_file_bytes: read_bytes_env("SESSION_FILE_SINGLE_MAX_BYTES", per_file),
            per_session_bytes: read_bytes_env("SESSION_FILE_MAX_BYTES", per_session),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn object_storage_raises_default_limits() {
        // Env overrides are process-global; only assert when none is set.
        if std::env::var("SESSION_FILE_SINGLE_MAX_BYTES").is_ok()
            || std::env::var("SESSION_FILE_MAX_BYTES").is_ok()
        {
            return;
        }
        let inline = QuotaLimits::from_env_with_defaults(false);
        assert_eq!(inline.per_file_bytes, DEFAULT_PER_FILE_MAX);
        assert_eq!(inline.per_session_bytes, DEFAULT_SESSION_MAX);
        let offloaded = QuotaLimits::from_env_with_defaults(true);
        assert_eq!(offloaded.per_file_bytes, OBJECT_STORAGE_PER_FILE_MAX);
        assert_eq!(offloaded.per_session_bytes, OBJECT_STORAGE_SESSION_MAX);
    }
}

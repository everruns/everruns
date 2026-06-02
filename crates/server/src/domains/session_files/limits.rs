// Storage quota limits for session files (TM-FS-008 / TM-DOS-005).
//
// Defaults are generous to avoid throttling legitimate long-horizon agentic
// runs, while bounding Postgres BYTEA exhaustion. Operators can tighten via
// env without redeploying code.

/// Max total bytes stored as session files per session (default 500 MB).
pub fn max_session_file_bytes() -> i64 {
    std::env::var("SESSION_FILE_MAX_BYTES")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .filter(|&v| v > 0)
        .unwrap_or(500 * 1024 * 1024)
}

/// Max bytes for a single file write (default 100 MB).
pub fn max_single_file_bytes() -> i64 {
    std::env::var("SESSION_FILE_SINGLE_MAX_BYTES")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .filter(|&v| v > 0)
        .unwrap_or(100 * 1024 * 1024)
}

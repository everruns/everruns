// Size and resource limits for session SQL databases

/// Maximum size of a single database in bytes (50 MB).
pub const MAX_DB_SIZE_BYTES: i64 = 50 * 1024 * 1024;

/// Maximum total database storage per session in bytes (100 MB).
pub const MAX_SESSION_TOTAL_SIZE_BYTES: i64 = 100 * 1024 * 1024;

/// Maximum number of databases per session.
pub const MAX_DATABASES_PER_SESSION: usize = 10;

/// SQLite page size in bytes.
pub const PAGE_SIZE: usize = 4096;

/// Maximum rows returned by a single query.
pub const MAX_RESULT_ROWS: usize = 1000;

/// Maximum result payload size in bytes (1 MB).
pub const MAX_RESULT_SIZE_BYTES: usize = 1024 * 1024;

/// Query timeout in seconds.
pub const QUERY_TIMEOUT_SECS: u64 = 30;

/// SQLite progress handler callback interval (VM instructions).
/// Check timeout every 10,000 instructions.
pub const PROGRESS_HANDLER_INTERVAL: i32 = 10_000;

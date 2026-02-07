// Session SQL Database crate
//
// Provides session-scoped SQLite databases backed by PostgreSQL page-level
// storage (production) or in-memory serialize/deserialize (DEV_MODE).
//
// Architecture:
// - types.rs: Shared types (DatabaseInfo, SqlQueryResult, etc.)
// - executor.rs: Shared query execution on rusqlite::Connection
// - authorizer.rs: SQLite authorizer blocking dangerous operations
// - validation.rs: Database name validation
// - limits.rs: Size and resource limit constants
// - memory.rs: In-memory backend for DEV_MODE
// - store.rs: Async SessionSqlDbStore trait implementation
// - error.rs: Error types

pub mod authorizer;
pub mod error;
pub mod executor;
pub mod limits;
pub mod memory;
pub mod store;
pub mod types;
pub mod validation;

// Re-exports
pub use error::SqlDbError;
pub use memory::InMemorySqlDbBackend;
pub use store::InMemorySqlDbStore;
pub use types::{ColumnSchema, DatabaseInfo, SqlExecuteResult, SqlQueryResult, TableSchema};

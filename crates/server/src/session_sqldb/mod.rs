//! Session-scoped SQL databases for Everruns agents.
//!
//! This server module gives each session its own SQLite-compatible database, backed
//! by PostgreSQL page-level storage in production (or an in-memory backend in
//! dev mode), so agents can create tables, run queries, and persist structured
//! data with multi-tenant isolation.
//!
//! # Example
//!
//! ```
//! use everruns_provider::typed_id::SessionId;
//! use everruns_server::session_sqldb::InMemorySqlDbBackend;
//!
//! let backend = InMemorySqlDbBackend::new();
//! let session = SessionId::new();
//!
//! // `execute` auto-creates the database on first use.
//! backend.execute(session, "notes", "CREATE TABLE notes (id INTEGER, body TEXT)").unwrap();
//! backend.execute(session, "notes", "INSERT INTO notes VALUES (1, 'hello')").unwrap();
//!
//! let result = backend.query(session, "notes", "SELECT body FROM notes").unwrap();
//! assert_eq!(result.row_count, 1);
//! ```

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

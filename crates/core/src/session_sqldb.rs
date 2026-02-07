// Session SQL Database types and trait
//
// Types and async trait for session-scoped SQL databases.
// Defined in core so capability tools can depend on them.
// Implementations live in the session-sqldb crate.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::typed_id::SessionId;

/// Metadata about a session database (no content/pages).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseInfo {
    pub name: String,
    pub size_bytes: i64,
    pub page_count: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Result of a SELECT query.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SqlQueryResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<serde_json::Value>>,
    pub row_count: usize,
    /// True if results were truncated due to limits
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub truncated: bool,
}

/// Result of an INSERT/UPDATE/DELETE/DDL statement.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SqlExecuteResult {
    pub rows_affected: u64,
}

/// Schema of a table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableSchema {
    pub name: String,
    pub columns: Vec<ColumnSchema>,
    pub row_count: i64,
}

/// Schema of a column.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColumnSchema {
    pub name: String,
    #[serde(rename = "type")]
    pub column_type: String,
    pub notnull: bool,
    pub pk: bool,
    pub default_value: Option<String>,
}

/// Error type for session SQL database operations.
///
/// Tool-visible errors (validation, not found, limits) vs internal errors
/// are distinguished by the caller when converting to ToolExecutionResult.
#[derive(Debug, thiserror::Error)]
pub enum SessionSqlDbError {
    #[error("database not found: {0}")]
    DatabaseNotFound(String),

    #[error("database already exists: {0}")]
    DatabaseAlreadyExists(String),

    #[error("invalid database name: {0}")]
    InvalidDatabaseName(String),

    #[error("database limit exceeded: {0}")]
    LimitExceeded(String),

    #[error("query error: {0}")]
    QueryError(String),

    #[error("query timeout after {0} seconds")]
    QueryTimeout(u64),

    #[error("result too large: {0}")]
    ResultTooLarge(String),

    #[error("operation blocked by authorizer: {0}")]
    AuthorizerBlocked(String),

    #[error("internal error: {0}")]
    Internal(String),
}

impl SessionSqlDbError {
    /// Whether this error should be shown to the LLM as a tool error (vs internal error).
    pub fn is_tool_error(&self) -> bool {
        matches!(
            self,
            Self::DatabaseNotFound(_)
                | Self::DatabaseAlreadyExists(_)
                | Self::InvalidDatabaseName(_)
                | Self::LimitExceeded(_)
                | Self::QueryError(_)
                | Self::QueryTimeout(_)
                | Self::ResultTooLarge(_)
                | Self::AuthorizerBlocked(_)
        )
    }
}

/// Async trait for session-scoped SQL database operations.
///
/// Implementations:
/// - InMemorySqlDbBackend (DEV_MODE) in session-sqldb crate
/// - Future: PostgreSQL VFS backend
#[async_trait]
pub trait SessionSqlDbStore: Send + Sync {
    /// Create a named database in the session.
    async fn create_database(
        &self,
        session_id: SessionId,
        name: &str,
    ) -> Result<DatabaseInfo, SessionSqlDbError>;

    /// List all databases for a session.
    async fn list_databases(
        &self,
        session_id: SessionId,
    ) -> Result<Vec<DatabaseInfo>, SessionSqlDbError>;

    /// Get metadata for a specific database.
    async fn get_database(
        &self,
        session_id: SessionId,
        name: &str,
    ) -> Result<Option<DatabaseInfo>, SessionSqlDbError>;

    /// Delete a database and all its pages.
    async fn delete_database(
        &self,
        session_id: SessionId,
        name: &str,
    ) -> Result<bool, SessionSqlDbError>;

    /// Execute DDL/DML SQL. Auto-creates database if it doesn't exist.
    async fn sql_execute(
        &self,
        session_id: SessionId,
        db_name: &str,
        sql: &str,
    ) -> Result<SqlExecuteResult, SessionSqlDbError>;

    /// Execute read-only SQL query. Returns columns and rows.
    async fn sql_query(
        &self,
        session_id: SessionId,
        db_name: &str,
        sql: &str,
    ) -> Result<SqlQueryResult, SessionSqlDbError>;

    /// Get schema for all tables (or a specific table) in a database.
    async fn sql_schema(
        &self,
        session_id: SessionId,
        db_name: &str,
        table: Option<&str>,
    ) -> Result<Vec<TableSchema>, SessionSqlDbError>;
}

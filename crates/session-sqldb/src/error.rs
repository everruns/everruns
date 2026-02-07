// Session SQL database error types

use thiserror::Error;

#[derive(Error, Debug)]
pub enum SqlDbError {
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

    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("internal error: {0}")]
    Internal(String),
}

impl From<SqlDbError> for everruns_core::session_sqldb::SessionSqlDbError {
    fn from(e: SqlDbError) -> Self {
        use everruns_core::session_sqldb::SessionSqlDbError;
        match e {
            SqlDbError::DatabaseNotFound(s) => SessionSqlDbError::DatabaseNotFound(s),
            SqlDbError::DatabaseAlreadyExists(s) => SessionSqlDbError::DatabaseAlreadyExists(s),
            SqlDbError::InvalidDatabaseName(s) => SessionSqlDbError::InvalidDatabaseName(s),
            SqlDbError::LimitExceeded(s) => SessionSqlDbError::LimitExceeded(s),
            SqlDbError::QueryError(s) => SessionSqlDbError::QueryError(s),
            SqlDbError::QueryTimeout(s) => SessionSqlDbError::QueryTimeout(s),
            SqlDbError::ResultTooLarge(s) => SessionSqlDbError::ResultTooLarge(s),
            SqlDbError::AuthorizerBlocked(s) => SessionSqlDbError::AuthorizerBlocked(s),
            SqlDbError::Sqlite(e) => SessionSqlDbError::Internal(e.to_string()),
            SqlDbError::Internal(s) => SessionSqlDbError::Internal(s),
        }
    }
}

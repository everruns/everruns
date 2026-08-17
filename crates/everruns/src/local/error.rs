// Error type for the local crate. Converts cleanly into the core
// `AgentLoopError` so trait implementations can return `everruns_provider::error::Result`.

use everruns_provider::error::AgentLoopError;

#[derive(Debug, thiserror::Error)]
/// Failure produced while configuring or operating local persistence.
pub enum LocalError {
    /// SQLite storage failed.
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    /// A persisted JSON value could not be encoded or decoded.
    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),

    /// A filesystem operation failed.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// The requested local configuration is invalid.
    #[error("configuration error: {0}")]
    Config(String),

    /// Another local backend operation failed.
    #[error("{0}")]
    Other(String),
}

/// Result returned by local backend operations.
pub type LocalResult<T> = std::result::Result<T, LocalError>;

impl From<LocalError> for AgentLoopError {
    fn from(err: LocalError) -> Self {
        match err {
            LocalError::Config(msg) => AgentLoopError::config(msg),
            other => AgentLoopError::store(other.to_string()),
        }
    }
}

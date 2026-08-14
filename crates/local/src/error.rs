// Error type for the local crate. Converts cleanly into the core
// `AgentLoopError` so trait implementations can return `everruns_provider::error::Result`.

use everruns_provider::error::AgentLoopError;

#[derive(Debug, thiserror::Error)]
pub enum LocalError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("configuration error: {0}")]
    Config(String),

    #[error("{0}")]
    Other(String),
}

pub type LocalResult<T> = std::result::Result<T, LocalError>;

impl From<LocalError> for AgentLoopError {
    fn from(err: LocalError) -> Self {
        match err {
            LocalError::Config(msg) => AgentLoopError::config(msg),
            other => AgentLoopError::store(other.to_string()),
        }
    }
}

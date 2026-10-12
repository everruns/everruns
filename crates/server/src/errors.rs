// Shared application error types.
// Decision: keep resource-level errors typed so API handlers can map them
// to precise HTTP responses instead of collapsing validation failures to 500s.

use thiserror::Error;

pub(crate) const ALREADY_EXISTS_CODE: &str = "already_exists";
pub(crate) const ALREADY_EXISTS_DETAIL: &str = "Resource already exists";
pub(crate) const DATABASE_POOL_EXHAUSTED_CODE: &str = "database_pool_exhausted";
pub(crate) const DATABASE_POOL_EXHAUSTED_DETAIL: &str =
    "Database capacity is temporarily unavailable";
pub(crate) const DATABASE_POOL_RETRY_AFTER_SECONDS: u32 = 1;

pub(crate) fn is_database_unique_violation(error: &anyhow::Error) -> bool {
    if error.chain().any(|source| {
        source
            .downcast_ref::<sqlx::Error>()
            .and_then(sqlx::Error::as_database_error)
            .and_then(sqlx::error::DatabaseError::code)
            .is_some_and(|code| code == "23505")
    }) {
        return true;
    }

    error.chain().any(|source| {
        source
            .to_string()
            .to_ascii_lowercase()
            .contains("duplicate key value violates unique constraint")
    })
}

/// Name of the unique constraint (or unique index) a Postgres 23505 violated,
/// if the error chain carries one. Lets a caller recognize an *expected*
/// conflict — one a constraint exists to settle — before the generic
/// `classify_anyhow` path logs it as a warning.
pub(crate) fn violated_unique_constraint(error: &anyhow::Error) -> Option<String> {
    if let Some(name) = error.chain().find_map(|source| {
        source
            .downcast_ref::<sqlx::Error>()
            .and_then(sqlx::Error::as_database_error)
            .filter(|db| db.code().is_some_and(|code| code == "23505"))
            .and_then(sqlx::error::DatabaseError::constraint)
            .map(str::to_string)
    }) {
        return Some(name);
    }

    // Errors flattened to text keep Postgres' message shape:
    // `duplicate key value violates unique constraint "<name>"`.
    const PREFIX: &str = "duplicate key value violates unique constraint \"";
    error.chain().find_map(|source| {
        let text = source.to_string();
        let start = text.to_ascii_lowercase().find(PREFIX)? + PREFIX.len();
        let rest = &text[start..];
        let end = rest.find('"')?;
        Some(rest[..end].to_string())
    })
}

/// Name of the foreign-key constraint a Postgres 23503 violated, if the error
/// chain carries a database error that names one.
pub(crate) fn violated_foreign_key_constraint(error: &anyhow::Error) -> Option<String> {
    error.chain().find_map(|source| {
        source
            .downcast_ref::<sqlx::Error>()
            .and_then(sqlx::Error::as_database_error)
            .filter(|db| db.is_foreign_key_violation())
            .and_then(sqlx::error::DatabaseError::constraint)
            .map(str::to_string)
    })
}

pub(crate) fn is_database_pool_exhausted(error: &anyhow::Error) -> bool {
    error.chain().any(|source| {
        matches!(
            source.downcast_ref::<sqlx::Error>(),
            Some(sqlx::Error::PoolTimedOut)
        ) || everruns_core::DatabaseFailureKind::classify(&source.to_string())
            == everruns_core::DatabaseFailureKind::PoolExhausted
    })
}
pub(crate) fn is_domain_already_exists_message(message: &str) -> bool {
    message.to_ascii_lowercase().contains("already exists")
}

#[derive(Debug, Error)]
#[error("{message}")]
pub struct BadRequestError {
    message: String,
}

impl BadRequestError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

#[derive(Debug, Error)]
#[error("{message}")]
pub struct ConflictError {
    message: String,
    code: Option<&'static str>,
}

impl ConflictError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code: None,
        }
    }

    /// Attach a stable machine-readable error code.
    pub fn with_code(mut self, code: &'static str) -> Self {
        self.code = Some(code);
        self
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn code(&self) -> Option<&'static str> {
        self.code
    }

    /// The `409` command error, carrying the code when one is attached.
    pub fn to_command_error(&self) -> crate::domains::common::CommandError {
        let error = crate::domains::common::CommandError::conflict(self.message.clone());
        match self.code {
            Some(code) => error.with_code(code),
            None => error,
        }
    }
}

/// Stable code of a refused fork of a session that ran on the OpenAI Agents
/// API backend (EVE-1126).
pub const AGENTS_API_SESSION_NOT_FORKABLE_CODE: &str = "agents_api_session_not_forkable";
/// Why such a fork is refused.
pub const AGENTS_API_SESSION_NOT_FORKABLE_DETAIL: &str = "This session ran on the OpenAI Agents API backend, whose provider-held context cannot be copied, so it cannot be forked. Start a new session instead; this session's record stays readable.";

pub type ResourceLimitError = ConflictError;

#[derive(Debug, Error)]
#[error("{resource} not found")]
pub struct ResourceNotFoundError {
    resource: &'static str,
}

impl ResourceNotFoundError {
    pub const fn new(resource: &'static str) -> Self {
        Self { resource }
    }

    pub const fn resource(&self) -> &'static str {
        self.resource
    }
}

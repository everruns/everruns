// Shared application error types.
// Decision: keep resource-level errors typed so API handlers can map them
// to precise HTTP responses instead of collapsing validation failures to 500s.

use thiserror::Error;

pub(crate) const ALREADY_EXISTS_CODE: &str = "already_exists";
pub(crate) const ALREADY_EXISTS_DETAIL: &str = "Resource already exists";

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
}

impl ConflictError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

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

// Shared application error types.
// Decision: keep resource-level errors typed so API handlers can map them
// to precise HTTP responses instead of collapsing validation failures to 500s.

use thiserror::Error;

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
pub struct ResourceLimitError {
    message: String,
}

impl ResourceLimitError {
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

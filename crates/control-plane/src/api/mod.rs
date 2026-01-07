// HTTP API routes
//
// This module contains all HTTP route handlers for the public API.
// Each submodule handles a specific resource type with its own AppState.

pub mod agents;
pub mod capabilities;
pub mod common;
pub mod events;
pub mod llm_models;
pub mod llm_providers;
pub mod messages;
pub mod session_files;
pub mod sessions;
pub mod users;
pub mod validation;

// Re-export common types
pub use common::{ErrorResponse, ListResponse};

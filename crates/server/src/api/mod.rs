// HTTP API routes
//
// This module contains all HTTP route handlers for the public API.
// Each submodule handles a specific resource type with its own AppState.

pub mod agents;
pub mod capabilities;
pub mod common;
pub mod durable;
pub mod events;
pub mod images;
pub mod llm_models;
pub mod llm_providers;
pub mod mcp_servers;
pub mod messages;
pub mod organizations;
pub mod session_files;
pub mod session_storage;
pub mod sessions;
pub mod sse;
pub mod users;
pub mod validation;

// Re-export common types
pub use common::{ErrorResponse, ListResponse, PaginatedResponse};

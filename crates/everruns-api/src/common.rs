// Common DTOs for public API
//
// These types are shared across multiple API endpoints.

use serde::{Deserialize, Serialize};
use serde_json::json;
use utoipa::ToSchema;

/// Response wrapper for list endpoints.
/// All list endpoints return responses wrapped in a `data` field.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ListResponse<T> {
    /// Array of items returned by the list operation.
    pub data: Vec<T>,
}

impl<T> ListResponse<T> {
    pub fn new(data: Vec<T>) -> Self {
        Self { data }
    }
}

impl<T> From<Vec<T>> for ListResponse<T> {
    fn from(data: Vec<T>) -> Self {
        Self { data }
    }
}

/// Request to create an event (for internal use)
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateEventRequest {
    /// The type of event (e.g., "message", "tool_call", "error").
    #[schema(example = "message")]
    pub event_type: String,
    /// Event payload as JSON. Structure depends on event_type.
    pub data: serde_json::Value,
}

/// Request to update agent capabilities.
/// Replaces the agent's current capabilities with the provided list.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpdateAgentCapabilitiesRequest {
    /// List of capability IDs in desired order.
    /// Position is determined by array index.
    #[schema(value_type = Vec<String>, example = json!(["file_operations", "web_search"]))]
    pub capabilities: Vec<everruns_core::CapabilityId>,
}

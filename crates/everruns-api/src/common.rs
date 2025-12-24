// Common DTOs for public API
//
// These types are shared across multiple API endpoints.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Response wrapper for list endpoints
/// All list endpoints return responses wrapped in a `data` field
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ListResponse<T> {
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
    pub event_type: String,
    pub data: serde_json::Value,
}

/// Request to update agent capabilities
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpdateAgentCapabilitiesRequest {
    /// List of capability IDs in desired order
    /// Position is determined by array index
    #[schema(value_type = Vec<String>)]
    pub capabilities: Vec<everruns_core::CapabilityId>,
}

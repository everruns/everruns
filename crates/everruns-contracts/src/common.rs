// Common DTOs for public API (M2)

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

// Frozen App compatibility types.
//
// The App storage row remains for archival reads and compatibility fixtures.

use serde::Deserialize;
use utoipa::IntoParams;

pub use crate::storage::AppRow;

/// Query parameters for deprecated archival App listings.
#[derive(Debug, Clone, Deserialize, IntoParams)]
pub struct ListAppsQuery {
    /// Search by name or description (case-insensitive substring match).
    pub search: Option<String>,
    /// Include archived apps. Deleted apps never appear in lists.
    pub include_archived: Option<bool>,
}

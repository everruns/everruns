// Frozen App compatibility types.
//
// Storage row types remain for archival reads and compatibility fixtures.

use serde::Deserialize;
use utoipa::IntoParams;

pub use crate::storage::models::{AppChannelRow, AppRow};

/// Query parameters for deprecated archival App listings.
#[derive(Debug, Clone, Deserialize, IntoParams)]
pub struct ListAppsQuery {
    /// Search by name or description (case-insensitive substring match).
    pub search: Option<String>,
    /// Include archived apps. Deleted apps never appear in lists.
    pub include_archived: Option<bool>,
}

// Virtual user domain types — canonical definitions for request shapes.
//
// Storage row types are re-exported from `storage::models` so domain code
// has a single import path.

use crate::records::VirtualUserStatus;
use everruns_db::UpdateField;
use serde::Deserialize;
use utoipa::{IntoParams, ToSchema};

pub use crate::storage::models::{CreateVirtualUserRow, UpdateVirtualUser, VirtualUserRow};

/// Create an organization-scoped runtime account.
#[derive(Debug, Clone, Deserialize, ToSchema, serde::Serialize)]
pub struct CreateVirtualUserRequest {
    #[serde(default)]
    /// Runtime purpose; service accounts may be attached to agents.
    #[schema(example = "end_user")]
    pub usage: crate::records::VirtualUserUsage,
    #[schema(example = "Ops Bot")]
    /// Human-readable name. Safe to render in user-facing messages.
    pub name: String,
    /// Human-readable description. Safe to render in user-facing messages.
    pub description: Option<String>,
    /// Profile image URL.
    #[schema(example = "https://example.com/avatar.png")]
    pub avatar_url: Option<String>,
    #[schema(example = "en-US")]
    /// Locale used for agent-facing defaults.
    pub locale: Option<String>,
    #[schema(example = "America/Los_Angeles")]
    /// IANA time zone used for agent-facing defaults.
    pub timezone: Option<String>,
}

/// Update a runtime account profile or its management lifecycle.
#[derive(Debug, Clone, Deserialize, ToSchema, serde::Serialize)]
pub struct UpdateVirtualUserRequest {
    /// Human-readable name. Safe to render in user-facing messages.
    #[schema(example = "Alex")]
    pub name: Option<String>,
    #[serde(default, with = "crate::domains::change_history::update_field")]
    #[schema(value_type = Option<String>, nullable = true)]
    /// Human-readable description. Safe to render in user-facing messages.
    #[schema(example = "Support team member")]
    pub description: UpdateField<String>,
    #[serde(default, with = "crate::domains::change_history::update_field")]
    #[schema(value_type = Option<String>, nullable = true)]
    /// Profile image URL.
    #[schema(example = "https://example.com/avatar.png")]
    pub avatar_url: UpdateField<String>,
    #[serde(default, with = "crate::domains::change_history::update_field")]
    #[schema(value_type = Option<String>, nullable = true)]
    /// Locale used for agent-facing defaults.
    pub locale: UpdateField<String>,
    #[serde(default, with = "crate::domains::change_history::update_field")]
    #[schema(value_type = Option<String>, nullable = true)]
    /// IANA time zone used for agent-facing defaults.
    pub timezone: UpdateField<String>,
    /// Current lifecycle status.
    pub status: Option<VirtualUserStatus>,
}

#[derive(Debug, Clone, Deserialize, IntoParams)]
pub struct ListVirtualUsersQuery {
    pub usage: Option<crate::records::VirtualUserUsage>,
    pub search: Option<String>,
    pub include_archived: Option<bool>,
    /// Zero-based page offset.
    pub offset: Option<u32>,
    /// Maximum results per page.
    pub limit: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_virtual_user_request_defaults_to_unchanged() {
        let req: UpdateVirtualUserRequest = serde_json::from_str("{}").unwrap();
        assert_eq!(req.description, UpdateField::Unchanged);
        assert_eq!(req.avatar_url, UpdateField::Unchanged);
        assert_eq!(req.locale, UpdateField::Unchanged);
        assert_eq!(req.timezone, UpdateField::Unchanged);
    }

    #[test]
    fn update_virtual_user_request_supports_clear() {
        let req: UpdateVirtualUserRequest =
            serde_json::from_str(r#"{"description":null,"locale":null}"#).unwrap();
        assert_eq!(req.description, UpdateField::Clear);
        assert_eq!(req.locale, UpdateField::Clear);
    }
}

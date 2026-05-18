// Agent identity domain types — canonical definitions for request shapes.
//
// Storage row types are re-exported from `storage::models` so domain code
// has a single import path.

use crate::api::common::deserialize_nullable_update_field;
use everruns_core::AgentIdentityStatus;
use everruns_durable::UpdateField;
use serde::Deserialize;
use utoipa::{IntoParams, ToSchema};

pub use crate::storage::models::{AgentIdentityRow, CreateAgentIdentityRow, UpdateAgentIdentity};

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateAgentIdentityRequest {
    #[schema(example = "Ops Bot")]
    /// Human-readable name. Safe to render in user-facing messages.
    pub name: String,
    /// Human-readable description. Safe to render in user-facing messages.
    pub description: Option<String>,
    pub avatar_url: Option<String>,
    #[schema(example = "en-US")]
    pub locale: Option<String>,
    #[schema(example = "America/Los_Angeles")]
    pub timezone: Option<String>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpdateAgentIdentityRequest {
    /// Human-readable name. Safe to render in user-facing messages.
    pub name: Option<String>,
    #[serde(default, deserialize_with = "deserialize_nullable_update_field")]
    #[schema(value_type = Option<String>, nullable = true)]
    /// Human-readable description. Safe to render in user-facing messages.
    pub description: UpdateField<String>,
    #[serde(default, deserialize_with = "deserialize_nullable_update_field")]
    #[schema(value_type = Option<String>, nullable = true)]
    pub avatar_url: UpdateField<String>,
    #[serde(default, deserialize_with = "deserialize_nullable_update_field")]
    #[schema(value_type = Option<String>, nullable = true)]
    pub locale: UpdateField<String>,
    #[serde(default, deserialize_with = "deserialize_nullable_update_field")]
    #[schema(value_type = Option<String>, nullable = true)]
    pub timezone: UpdateField<String>,
    /// Current lifecycle status.
    pub status: Option<AgentIdentityStatus>,
}

#[derive(Debug, Clone, Deserialize, IntoParams)]
pub struct ListAgentIdentitiesQuery {
    pub search: Option<String>,
    pub include_archived: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_agent_identity_request_roundtrip() {
        let json = r#"{"name":"Ops Bot","locale":"en-US","timezone":"America/Los_Angeles"}"#;
        let req: CreateAgentIdentityRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.name, "Ops Bot");
        assert_eq!(req.locale.as_deref(), Some("en-US"));
        assert_eq!(req.timezone.as_deref(), Some("America/Los_Angeles"));
    }

    #[test]
    fn test_update_agent_identity_request_status() {
        let json = r#"{"status":"archived"}"#;
        let req: UpdateAgentIdentityRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.status, Some(AgentIdentityStatus::Archived));
    }

    #[test]
    fn update_agent_identity_request_defaults_to_unchanged() {
        let req: UpdateAgentIdentityRequest = serde_json::from_str("{}").unwrap();
        assert_eq!(req.description, UpdateField::Unchanged);
        assert_eq!(req.avatar_url, UpdateField::Unchanged);
        assert_eq!(req.locale, UpdateField::Unchanged);
        assert_eq!(req.timezone, UpdateField::Unchanged);
    }

    #[test]
    fn update_agent_identity_request_supports_clear() {
        let req: UpdateAgentIdentityRequest =
            serde_json::from_str(r#"{"description":null,"locale":null}"#).unwrap();
        assert_eq!(req.description, UpdateField::Clear);
        assert_eq!(req.locale, UpdateField::Clear);
    }
}

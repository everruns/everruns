// Apps domain types — canonical definitions for request shapes.
//
// Storage row types are re-exported from `storage::models` so domain code
// has a single import path.

use crate::api::common::deserialize_nullable_update_field;
use crate::domains::common::deserialize_opt_json_value_lenient;
use everruns_core::typed_id::{AgentId, AgentIdentityId, HarnessId};
use everruns_core::{AppStatus, ChannelType};
use everruns_durable::UpdateField;
use serde::Deserialize;
use utoipa::{IntoParams, ToSchema};

pub use crate::storage::models::{
    AppChannelRow, AppRow, CreateAppChannelRow, CreateAppRow, UpdateApp, UpdateAppChannel,
};

/// Request to create a new app
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateAppRequest {
    /// Display name of the app.
    #[schema(example = "Support Bot")]
    pub name: String,
    /// Description of what the app does.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "Customer support bot connected to Slack")]
    pub description: Option<String>,
    /// ID of the harness to use.
    #[schema(value_type = String, example = "harness_01933b5a00007000800000000000001")]
    pub harness_id: HarnessId,
    /// Optional ID of the agent to use.
    #[serde(default)]
    #[schema(value_type = Option<String>, example = "agent_01933b5a00007000800000000000001")]
    pub agent_id: Option<AgentId>,
    /// Optional resident agent identity for unattended/channel execution.
    #[serde(default)]
    #[schema(value_type = Option<String>, example = "identity_01933b5a00007000800000000000001")]
    pub agent_identity_id: Option<AgentIdentityId>,
    /// Optional initial channel type (creates the first channel on the app).
    #[serde(default)]
    pub channel_type: Option<ChannelType>,
    /// Initial channel configuration.
    #[serde(default, deserialize_with = "deserialize_opt_json_value_lenient")]
    pub channel_config: Option<serde_json::Value>,
}

/// Request to update an app. Only provided fields will be updated.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpdateAppRequest {
    /// Display name of the app.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Description of what the app does.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// ID of the harness to use.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>)]
    pub harness_id: Option<HarnessId>,
    /// ID of the agent to use.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>)]
    pub agent_id: Option<AgentId>,
    /// Optional resident agent identity for unattended/channel execution.
    #[serde(default, deserialize_with = "deserialize_nullable_update_field")]
    #[schema(value_type = Option<String>, nullable = true)]
    pub agent_identity_id: UpdateField<AgentIdentityId>,
    /// Lifecycle status (draft or archived). Use publish/unpublish endpoints for publishing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<AppStatus>,
}

/// Request to add a channel to an app.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct AddChannelRequest {
    /// Channel type.
    pub channel_type: ChannelType,
    /// Channel-specific configuration.
    #[serde(default, deserialize_with = "deserialize_opt_json_value_lenient")]
    pub channel_config: Option<serde_json::Value>,
    /// Whether the channel is enabled (default: true).
    #[serde(default)]
    pub enabled: Option<bool>,
}

/// Request to update a channel.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpdateChannelRequest {
    /// Channel type.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_type: Option<ChannelType>,
    /// Channel-specific configuration.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_opt_json_value_lenient"
    )]
    pub channel_config: Option<serde_json::Value>,
    /// Whether the channel is enabled.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
}

/// Query parameters for listing apps.
#[derive(Debug, Clone, Deserialize, IntoParams)]
pub struct ListAppsQuery {
    /// Search by name or description (case-insensitive substring match).
    pub search: Option<String>,
    /// Include archived apps. Deleted apps never appear in lists.
    pub include_archived: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_HARNESS_ID: &str = "harness_550e8400e29b41d4a716446655440001";
    const TEST_AGENT_IDENTITY_ID: &str = "identity_550e8400e29b41d4a716446655440000";

    #[test]
    fn create_app_request_allows_missing_agent_and_channel() {
        let req: CreateAppRequest = serde_json::from_str(&format!(
            r#"{{"name":"Draft App","harness_id":"{}"}}"#,
            TEST_HARNESS_ID
        ))
        .unwrap();

        assert_eq!(req.name, "Draft App");
        assert_eq!(req.agent_id, None);
        assert_eq!(req.channel_type, None);
        assert_eq!(req.channel_config, None);
    }

    #[test]
    fn create_app_request_accepts_stringified_channel_config() {
        let req: CreateAppRequest = serde_json::from_str(&format!(
            r#"{{"name":"Draft App","harness_id":"{}","channel_type":"webhook","channel_config":"{{\"token\":\"secret\",\"message\":\"run\"}}"}}"#,
            TEST_HARNESS_ID
        ))
        .unwrap();

        assert_eq!(req.channel_type, Some(ChannelType::Webhook));
        assert_eq!(
            req.channel_config,
            Some(serde_json::json!({"token": "secret", "message": "run"}))
        );
    }

    #[test]
    fn update_app_request_leaves_agent_identity_unchanged_when_omitted() {
        let req: UpdateAppRequest = serde_json::from_str("{}").unwrap();
        assert_eq!(req.agent_identity_id, UpdateField::Unchanged);
    }

    #[test]
    fn update_app_request_clears_agent_identity_when_null() {
        let req: UpdateAppRequest = serde_json::from_str(r#"{"agent_identity_id":null}"#).unwrap();
        assert_eq!(req.agent_identity_id, UpdateField::Clear);
    }

    #[test]
    fn update_app_request_sets_agent_identity_when_present() {
        let req: UpdateAppRequest = serde_json::from_str(&format!(
            r#"{{"agent_identity_id":"{}"}}"#,
            TEST_AGENT_IDENTITY_ID
        ))
        .unwrap();
        let expected: AgentIdentityId = TEST_AGENT_IDENTITY_ID.parse().unwrap();
        assert_eq!(req.agent_identity_id, UpdateField::Set(expected));
    }

    #[test]
    fn add_channel_request_accepts_stringified_channel_config() {
        let req: AddChannelRequest = serde_json::from_str(
            r#"{"channel_type":"schedule","channel_config":"{\"cron_expression\":\"0 * * * * * *\",\"message\":\"run\"}"}"#,
        )
        .unwrap();

        assert_eq!(req.channel_type, ChannelType::Schedule);
        assert_eq!(
            req.channel_config,
            Some(serde_json::json!({"cron_expression": "0 * * * * * *", "message": "run"}))
        );
    }
}

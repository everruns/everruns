// Apps domain types — canonical definitions for request shapes.
//
// Storage row types are re-exported from `storage::models` so domain code
// has a single import path.

use crate::api::common::deserialize_nullable_update_field;
use crate::domains::common::deserialize_opt_json_value_lenient;
use chrono::{DateTime, Utc};
use everruns_durable::UpdateField;
use everruns_platform::{AgentVersionPolicy, AppStatus, ChannelType};
use everruns_provider::typed_id::{AgentId, AgentIdentityId, AgentVersionId, HarnessId};
use serde::{Deserialize, Serialize};
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
    /// ID of the agent to use. The command validation requires this field even
    /// though it remains optional in the wire shape for a clear domain error.
    #[serde(default)]
    #[schema(value_type = Option<String>, example = "agent_01933b5a00007000800000000000001")]
    pub agent_id: Option<AgentId>,
    /// How an App resolves the Agent version it runs.
    /// Example shape is defined on `AgentVersionPolicy`.
    #[serde(default)]
    pub agent_version_policy: AgentVersionPolicy,
    #[serde(default)]
    #[schema(value_type = Option<String>, example = "agentver_01933b5a00007000800000000000001")]
    pub agent_version_id: Option<AgentVersionId>,
    /// Optional resident agent identity for unattended/channel execution.
    #[serde(default)]
    #[schema(value_type = Option<String>, example = "identity_01933b5a00007000800000000000001")]
    pub agent_identity_id: Option<AgentIdentityId>,
    /// Optional initial channel type (creates the first channel on the app).
    /// Example shape is defined on `ChannelType`.
    #[serde(default)]
    pub channel_type: Option<ChannelType>,
    /// Initial channel configuration. Shape depends on `channel_type`, for
    /// example `{"token": "whk_redacted", "message": "Run support triage"}`
    /// for `webhook`. New schedule channels are rejected; use agent triggers.
    #[serde(default, deserialize_with = "deserialize_opt_json_value_lenient")]
    #[schema(value_type = Option<Object>)]
    pub channel_config: Option<serde_json::Value>,
}

/// Request to update an app. Only provided fields will be updated.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpdateAppRequest {
    /// Display name of the app.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "Support Bot")]
    pub name: Option<String>,
    /// Description of what the app does.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "Customer support bot connected to Slack")]
    pub description: Option<String>,
    /// ID of the harness to use.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, example = "harness_01933b5a00007000800000000000001")]
    pub harness_id: Option<HarnessId>,
    /// ID of the agent to use.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, example = "agent_01933b5a00007000800000000000001")]
    pub agent_id: Option<AgentId>,
    /// How the App resolves the agent version. Example shape is defined on `AgentVersionPolicy`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_version_policy: Option<AgentVersionPolicy>,
    #[serde(default, deserialize_with = "deserialize_nullable_update_field")]
    #[schema(value_type = Option<String>, nullable = true)]
    pub agent_version_id: UpdateField<AgentVersionId>,
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

/// Single app-channel invocation record (one run = one channel-side event).
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AppRunEvent {
    /// Prefixed public identifier of this run event.
    pub id: String,
    /// App that received the invocation.
    pub app_id: String,
    /// Channel that originated the invocation.
    pub channel_id: String,
    /// Channel kind (slack, ag_ui, webhook, a2a, fcp, public_chat).
    pub channel_type: ChannelType,
    /// Human-readable channel name when set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_name: Option<String>,
    /// Terminal status (`ok`, `err`, `running`).
    pub status: String,
    /// Timestamp the run started (RFC 3339).
    pub created_at: DateTime<Utc>,
    /// Timestamp the run finished, if any (RFC 3339).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<DateTime<Utc>>,
}

/// One bucket of the run-history histogram (per-hour aggregate).
#[derive(Debug, Clone, Serialize, Default, ToSchema)]
pub struct AppRunBucket {
    /// Bucket start (top of the hour, RFC 3339).
    pub hour: DateTime<Utc>,
    /// Count of runs that completed successfully in this bucket.
    pub ok: u32,
    /// Count of runs that failed in this bucket.
    pub err: u32,
    /// Count of runs still running when the bucket was queried.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub running: Option<u32>,
}

/// Paged response for the app run history endpoint, optionally including a per-hour histogram.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AppRunListResponse {
    /// Page of run events ordered newest first.
    pub data: Vec<AppRunEvent>,
    /// Per-hour aggregate buckets when the caller asked for them.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub buckets: Option<Vec<AppRunBucket>>,
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

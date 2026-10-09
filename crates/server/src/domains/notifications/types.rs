// Notification domain types.
//
// Decision: request/response DTOs are defined here, not in the HTTP layer, so
// the domain never imports `api`. The `api` module re-exports them, keeping
// OpenAPI schema names and JSON shapes unchanged.

use crate::kernel_imports::contracts::typed_id::NotificationId;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ListNotificationsResponse {
    /// Page of items returned by this query.
    pub data: Vec<Notification>,
    /// Number of notifications the current user has not viewed yet.
    #[schema(example = 3)]
    pub unviewed_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Notification {
    #[schema(value_type = String, example = "notification_01933b5a00007000800000000000001")]
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
    pub id: NotificationId,
    /// Discriminator selecting the variant of this resource.
    #[schema(example = "health.issue")]
    pub kind: String,
    /// Human-readable title. Safe to render in user-facing messages.
    #[schema(example = "Agent run failed")]
    pub title: String,
    /// Plain-text summary of what happened.
    pub body: String,
    /// What sent the notification. Absent on notifications written before
    /// senders were recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<NotificationSource>,
    /// Kind of resource the notification is about, when it has one.
    #[schema(example = "session")]
    pub target_type: Option<String>,
    /// Prefixed public identifier of the resource the notification is about.
    #[schema(example = "session_01933b5a00007000800000000000001")]
    pub target_id: Option<String>,
    /// UI link that opens the resource the notification is about.
    #[schema(example = "/chat/session_01933b5a00007000800000000000001")]
    pub href: Option<String>,
    /// Kind-specific structured detail.
    #[schema(value_type = Object)]
    pub payload: serde_json::Value,
    /// How many times the same event recurred into this notification.
    #[schema(example = 1)]
    pub occurrence_count: i32,
    /// When the current user marked the notification as viewed; null while unviewed.
    pub viewed_at: Option<DateTime<Utc>>,
    /// Timestamp when this resource was created (RFC 3339).
    pub created_at: DateTime<Utc>,
    /// Timestamp when this resource was last updated (RFC 3339).
    pub updated_at: DateTime<Utc>,
}

/// What sent a notification. Kinds share one shape so new senders (a shared
/// agent, an integration) render without a client change.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
pub struct NotificationSource {
    /// `agent` or `system`.
    #[serde(rename = "type")]
    #[schema(example = "agent")]
    pub source_type: String,
    /// Public ID of the sender, when it has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "agent_01933b5a00007000800000000000001")]
    pub id: Option<String>,
    /// Display name of the sender.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "Platform Assistant")]
    pub name: Option<String>,
}

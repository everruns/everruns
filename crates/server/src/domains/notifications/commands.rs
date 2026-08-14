use super::queries as q;
use super::types::{ListNotificationsResponse, Notification};
use crate::domains::common::*;
use everruns_provider::typed_id::NotificationId;
use serde::Deserialize;
use utoipa::ToSchema;

fn require_user_id(ctx: &Ctx) -> Result<uuid::Uuid, CommandError> {
    ctx.caller
        .user_id
        .ok_or_else(|| CommandError::forbidden("Notifications require an authenticated user"))
}

#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct ListNotifications {
    /// Maximum number of items returned in this page.
    pub limit: Option<i64>,
}

impl Command for ListNotifications {
    type Output = ListNotificationsResponse;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "list_notifications",
            category: "notifications",
            description: "List notifications for the current user.",
            method: "GET",
            path: "/v1/notifications",
        }
    }

    async fn execute(self, ctx: &Ctx) -> Result<ListNotificationsResponse, CommandError> {
        let user_id = require_user_id(ctx)?;
        let limit = self.limit.unwrap_or(50).clamp(1, 100);
        let service = crate::domains::notifications::NotificationService::new(ctx.db.clone());
        let notifications = service
            .list(ctx.org_id(), user_id, limit)
            .await
            .map_err(classify_anyhow)?;
        let unviewed_count = service
            .count_unviewed(ctx.org_id(), user_id)
            .await
            .map_err(classify_anyhow)?;

        Ok(ListNotificationsResponse {
            data: notifications
                .into_iter()
                .map(q::row_to_notification)
                .collect(),
            unviewed_count,
        })
    }
}

inventory::submit! { CommandDescriptor::of::<ListNotifications>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct MarkNotificationViewed {
    pub notification_id: String,
}

impl Command for MarkNotificationViewed {
    type Output = Notification;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "mark_notification_viewed",
            category: "notifications",
            description: "Mark a notification as viewed.",
            method: "POST",
            path: "/v1/notifications/{notification_id}/view",
        }
    }

    fn positional_arg() -> Option<&'static str> {
        Some("notification_id")
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&super::NOTIFICATION_ACCESS)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Notification, CommandError> {
        let user_id = require_user_id(ctx)?;
        let notification_id: NotificationId = self
            .notification_id
            .parse()
            .map_err(|e| CommandError::bad_request(format!("Invalid notification ID: {e}")))?;
        let notification = crate::domains::notifications::NotificationService::new(ctx.db.clone())
            .mark_viewed(ctx.org_id(), user_id, notification_id)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Notification"))?;
        Ok(q::row_to_notification(notification))
    }
}

inventory::submit! { CommandDescriptor::of::<MarkNotificationViewed>() }

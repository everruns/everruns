use super::queries as q;
use super::types::{ListNotificationsResponse, Notification};
use crate::domains::common::*;
use everruns_contracts::typed_id::NotificationId;
use serde::Deserialize;
use utoipa::ToSchema;

fn require_user_id(ctx: &Ctx) -> Result<uuid::Uuid, CommandError> {
    ctx.caller
        .user_id
        .ok_or_else(|| CommandError::forbidden("Notifications require an authenticated user"))
}

#[derive(Debug, Default, Deserialize, ToSchema, utoipa::IntoParams, serde::Serialize)]
#[into_params(parameter_in = Query)]
pub struct ListNotifications {
    /// Maximum number of items returned in this page.
    pub limit: Option<i64>,
}

#[command(
    name = "list_notifications",
    category = "notifications",
    description = "List notifications for the current user.",
    method = "GET",
    path = "/v1/notifications",
    http = plain,
    params(ListNotifications),
)]
impl Command for ListNotifications {
    type Output = ListNotificationsResponse;

    async fn execute(self, ctx: &Ctx) -> Result<ListNotificationsResponse, CommandError> {
        let user_id = require_user_id(ctx)?;
        let limit = self.limit.unwrap_or(50).clamp(1, 100);
        let service = crate::domains::notifications::NotificationService::new(ctx.db.clone());
        let notifications = service.list(ctx.org_id(), user_id, limit).await?;
        let unviewed_count = service.count_unviewed(ctx.org_id(), user_id).await?;

        let health_count = ctx
            .db
            .count_unviewed_notifications_by_kind(ctx.org_id(), user_id, "health.issue")
            .await?;
        let health_visible = if crate::domains::health_issues::can_receive_health(ctx).await? {
            ctx.db
                .count_visible_health_notifications(ctx.org_id(), user_id)
                .await?
        } else {
            0
        };
        let notifications =
            crate::domains::health_issues::filter_notifications(ctx, notifications).await?;
        let unviewed_count = unviewed_count
            .saturating_sub(health_count)
            .saturating_add(health_visible);
        Ok(ListNotificationsResponse {
            data: notifications
                .into_iter()
                .map(q::row_to_notification)
                .collect(),
            unviewed_count,
        })
    }
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct MarkNotificationViewed {
    /// Prefixed public identifier of the notification to mark as viewed.
    pub notification_id: String,
}

#[command(
    name = "mark_notification_viewed",
    category = "notifications",
    description = "Mark a notification as viewed.",
    method = "POST",
    path = "/v1/notifications/{notification_id}/view",
    policy = super::NOTIFICATION_ACCESS,
    positional = "notification_id",
    http = plain,
    responses((status = 404, description = "Notification not found")),
)]
impl Command for MarkNotificationViewed {
    type Output = Notification;

    async fn execute(self, ctx: &Ctx) -> Result<Notification, CommandError> {
        let user_id = require_user_id(ctx)?;
        let notification_id: NotificationId = self
            .notification_id
            .parse()
            .map_err(|e| CommandError::bad_request(format!("Invalid notification ID: {e}")))?;
        let existing = ctx
            .db
            .get_notification(ctx.org_id(), user_id, notification_id)
            .await?
            .ok_or_else(|| CommandError::not_found("Notification"))?;
        if crate::domains::health_issues::filter_notifications(ctx, vec![existing])
            .await?
            .is_empty()
        {
            return Err(CommandError::not_found("Notification"));
        }
        let notification = crate::domains::notifications::NotificationService::new(ctx.db.clone())
            .mark_viewed(ctx.org_id(), user_id, notification_id)
            .await?
            .ok_or_else(|| CommandError::not_found("Notification"))?;
        Ok(q::row_to_notification(notification))
    }
}

use super::{
    active_turns,
    service::{SlackHealthService, issue_copy},
    types::{HealthIssue, HealthIssueList},
};
use crate::domains::sessions::limits::OrgCaps;
use crate::domains::{
    agents::{AGENT_MANAGE, AGENT_VIEW},
    common::*,
};
use crate::storage::{CreateNotificationRow, HealthIssueRow};
use chrono::Utc;
use serde::Deserialize;
use utoipa::ToSchema;
use uuid::Uuid;

// THREAT[TM-SLACK-012]: historical announcements cannot preserve revoked access.
/// Re-evaluate current membership as well as the deployment's active resolver.
/// Used on delivery too, so a stale notification cannot preserve revoked access.
pub async fn can_receive_health(ctx: &Ctx) -> Result<bool, CommandError> {
    let mut caller = ctx.caller.clone();
    if let Some(user) = caller.user_id {
        let Some(member) = ctx.db.get_organization_member(ctx.org_id(), user).await? else {
            return Ok(false);
        };
        caller.role = member
            .role
            .parse()
            .map_err(|_| CommandError::forbidden("Invalid membership"))?;
    }
    Ok(AGENT_VIEW
        .evaluate_with(ctx.permission_resolver.as_ref(), &caller)
        .is_ok())
}

pub async fn present(ctx: &Ctx, row: HealthIssueRow) -> Result<HealthIssue, CommandError> {
    let (title, body) = issue_copy(&row);
    let href = format!("/settings/health?issue={}", row.id);
    let user = ctx.caller.user_id;
    let notification_id = if ctx.feature_flags.notifications
        && matches!(row.status.as_str(), "open" | "needs_check")
        && let Some(user_id) = user
    {
        let notification = ctx
            .db
            .health_notification(CreateNotificationRow {
                org_id: ctx.org_id(),
                user_id,
                kind: "health.issue".into(),
                title: title.clone(),
                body: match &row.agent_name {
                    Some(agent) => format!("{agent}: {body}"),
                    None => body.clone(),
                },
                target_type: Some("health_issue".into()),
                target_id: Some(row.id.to_string()),
                href: Some(href.clone()),
                payload: serde_json::json!({"issue_id":row.id,"episode_id":row.episode_id}),
                dedupe_key: Some(format!("health:{}:{}", row.id, row.episode_id)),
                source: Some(crate::storage::NotificationSourceRow {
                    source_type: "agent".into(),
                    source_id: Some(row.agent_public_id.clone()),
                    source_name: Some(row.agent_name.clone()),
                }),
            })
            .await?;
        notification.map(|n| n.id.to_string())
    } else {
        None
    };
    let snoozed_until = if let Some(user) = user {
        ctx.db
            .health_issue_snooze(row.id, user, row.episode_id, None)
            .await?
    } else {
        None
    };
    // An organization-level issue has no channel revision to compare.
    let revision_changed = match &row.channel_public_id {
        Some(channel) => ctx
            .db
            .get_ingress_channel_by_public_id(channel)
            .await?
            .is_none_or(|e| Some(e.updated_at) != row.channel_revision),
        None => false,
    };
    let stale = revision_changed
        || Utc::now() - row.last_checked_at > chrono::Duration::minutes(15)
        || row.error_code.as_deref() == Some("verification_unavailable");
    Ok(HealthIssue {
        id: row.id,
        code: row.code,
        agent_id: row.agent_public_id,
        agent_name: row.agent_name,
        channel_id: row.channel_public_id,
        status: row.status,
        title,
        body,
        missing_scopes: row.missing_scopes,
        error_code: row.error_code,
        first_detected_at: row.first_detected_at,
        last_checked_at: row.last_checked_at,
        stale,
        notification_id,
        snoozed_until,
        href,
    })
}

async fn require_access(ctx: &Ctx) -> Result<(), CommandError> {
    if !can_receive_health(ctx).await? {
        return Err(CommandError::forbidden(
            "Agent access is required to view health issues",
        ));
    }
    Ok(())
}
async fn issue(ctx: &Ctx, id: Uuid) -> Result<HealthIssueRow, CommandError> {
    require_access(ctx).await?;
    let mut row = ctx
        .db
        .get_health_issue(ctx.org_id(), id)
        .await?
        .ok_or_else(|| CommandError::not_found("Health issue"))?;
    if let Some(channel_public_id) = row.channel_public_id.as_deref()
        && let Some(channel) = ctx
            .db
            .get_ingress_channel_by_public_id(channel_public_id)
            .await?
        && (!channel.enabled || channel.channel_status == "disabled")
    {
        row.status = "inapplicable".into();
    }
    Ok(row)
}

/// Pagination and optional channel filter for pending health issues.
#[derive(Debug, Default, Deserialize, ToSchema, serde::Serialize)]
pub struct ListHealthIssues {
    /// Restrict results to this public channel identifier.
    #[schema(example = "appchan_550e8400e29b41d4a716446655440000")]
    pub channel_id: Option<String>,
    /// Number of matching issues to skip; defaults to zero.
    #[schema(example = 0)]
    pub offset: Option<i64>,
    /// Page size, clamped to 1 through 100; defaults to 20.
    #[schema(example = 20)]
    pub limit: Option<i64>,
}
#[command(
    name = "list_health_issues",
    category = "health_issues",
    description = "List pending operational health issues.",
    method = "GET",
    path = "/v1/health-issues",
    policy = AGENT_VIEW,
)]
impl Command for ListHealthIssues {
    type Output = HealthIssueList;
    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        require_access(ctx).await?;
        let offset = self.offset.unwrap_or(0).clamp(0, 1_000_000);
        let limit = self.limit.unwrap_or(20).clamp(1, 100);
        let rows = ctx
            .db
            .list_health_issues(ctx.org_id(), offset, limit, self.channel_id.as_deref())
            .await?;
        let mut data = Vec::with_capacity(rows.len());
        for row in rows {
            data.push(present(ctx, row).await?);
        }
        let total = ctx
            .db
            .count_health_issues(ctx.org_id(), self.channel_id.as_deref())
            .await?;
        Ok(HealthIssueList {
            data,
            total,
            offset,
            limit,
        })
    }
}
/// Read the current evidence and recovery guidance for one health issue.
#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct GetHealthIssue {
    /// Stable identifier of the issue in the current organization.
    #[schema(example = "550e8400-e29b-41d4-a716-446655440000")]
    pub issue_id: Uuid,
}
#[command(
    name = "get_health_issue",
    category = "health_issues",
    description = "Get an operational health issue.",
    method = "GET",
    path = "/v1/health-issues/{issue_id}",
    policy = AGENT_VIEW,
)]
impl Command for GetHealthIssue {
    type Output = HealthIssue;
    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        present(ctx, issue(ctx, self.issue_id).await?).await
    }
}
/// Request fresh, non-mutating verification of one health issue.
#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct CheckHealthIssue {
    /// Stable identifier of the issue in the current organization.
    #[schema(example = "550e8400-e29b-41d4-a716-446655440000")]
    pub issue_id: Uuid,
}
#[command(
    name = "check_health_issue",
    category = "health_issues",
    description = "Verify an installation's current health without modifying provider data.",
    method = "POST",
    path = "/v1/health-issues/{issue_id}/check",
    policy = AGENT_MANAGE,
)]
impl Command for CheckHealthIssue {
    type Output = HealthIssue;
    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        let row = issue(ctx, self.issue_id).await?;
        let checked = match row.channel_public_id.as_deref() {
            Some(channel_public_id) => {
                SlackHealthService::new(ctx.db.clone(), ctx.encryption.clone())
                    .check_requested(channel_public_id, self.issue_id)
                    .await?
            }
            // Same cooldown as a channel check.
            None if Utc::now() - row.last_checked_at < chrono::Duration::seconds(15) => false,
            None if row.code == active_turns::ACTIVE_TURN_LIMIT => {
                active_turns::recheck(&ctx.db, ctx.org_id(), OrgCaps::from_env()).await?;
                true
            }
            None => true,
        };
        if !checked {
            return Err(
                CommandError::rate_limited("Wait a few seconds before checking again")
                    .with_retry_after(15),
            );
        }
        present(ctx, issue(ctx, self.issue_id).await?).await
    }
}
/// Suppress the current user's reminders for one day without resolving the issue.
#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct SnoozeHealthIssue {
    /// Stable identifier of the issue in the current organization.
    #[schema(example = "550e8400-e29b-41d4-a716-446655440000")]
    pub issue_id: Uuid,
}
#[command(
    name = "snooze_health_issue",
    category = "health_issues",
    description = "Snooze health reminders for the current user for one day.",
    method = "POST",
    path = "/v1/health-issues/{issue_id}/snooze",
    policy = AGENT_VIEW,
)]
impl Command for SnoozeHealthIssue {
    type Output = HealthIssue;
    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        let row = issue(ctx, self.issue_id).await?;
        let user = ctx
            .caller
            .user_id
            .ok_or_else(|| CommandError::forbidden("An authenticated user is required"))?;
        ctx.db
            .health_issue_snooze(
                row.id,
                user,
                row.episode_id,
                Some(Utc::now() + chrono::Duration::days(1)),
            )
            .await?;
        present(ctx, row).await
    }
}
pub async fn filter_notifications(
    ctx: &Ctx,
    rows: Vec<crate::storage::NotificationRow>,
) -> Result<Vec<crate::storage::NotificationRow>, CommandError> {
    let allowed = can_receive_health(ctx).await?;
    let mut visible = Vec::with_capacity(rows.len());
    for row in rows {
        if row.kind == "health.issue" {
            if !allowed {
                continue;
            }
            let Some(id) = row
                .target_id
                .as_deref()
                .and_then(|id| id.parse::<Uuid>().ok())
            else {
                continue;
            };
            let Some(issue) = ctx.db.get_health_issue(ctx.org_id(), id).await? else {
                continue;
            };
            if issue.status == "inapplicable" {
                continue;
            }
            if let Some(channel_public_id) = issue.channel_public_id.as_deref() {
                let channel = ctx
                    .db
                    .get_ingress_channel_by_public_id(channel_public_id)
                    .await?;
                if channel.is_none_or(|e| !e.enabled || e.channel_status == "disabled") {
                    continue;
                }
            }
        }
        visible.push(row);
    }
    Ok(visible)
}

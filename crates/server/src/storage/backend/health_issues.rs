use super::*;
use crate::storage::{HealthIssueRow, ObserveHealthIssue};

impl StorageBackend {
    pub async fn count_visible_health_notifications(
        &self,
        org_id: i64,
        user_id: Uuid,
    ) -> Result<u32> {
        dispatch!(self, count_visible_health_notifications, org_id, user_id)
    }
    pub async fn observe_health_issue(&self, input: ObserveHealthIssue) -> Result<()> {
        dispatch!(self, observe_health_issue, input)
    }
    pub async fn observe_org_health_issue(
        &self,
        org_id: i64,
        code: &str,
        status: &str,
        checked_at: DateTime<Utc>,
        resolved_copy: (&str, &str),
    ) -> Result<()> {
        dispatch!(
            self,
            observe_org_health_issue,
            org_id,
            code,
            status,
            checked_at,
            resolved_copy
        )
    }
    pub async fn orgs_with_open_org_health_issue(&self, code: &str) -> Result<Vec<i64>> {
        dispatch!(self, orgs_with_open_org_health_issue, code)
    }
    pub async fn count_org_active_turns(&self, org_id: i64) -> Result<i64> {
        dispatch!(self, count_org_active_turns, org_id)
    }
    pub async fn list_health_issues(
        &self,
        org_id: i64,
        offset: i64,
        limit: i64,
        channel_public_id: Option<&str>,
    ) -> Result<Vec<HealthIssueRow>> {
        dispatch!(
            self,
            list_health_issues,
            org_id,
            offset,
            limit,
            channel_public_id
        )
    }
    pub async fn get_health_issue(&self, org_id: i64, id: Uuid) -> Result<Option<HealthIssueRow>> {
        dispatch!(self, get_health_issue, org_id, id)
    }
    pub async fn count_health_issues(
        &self,
        org_id: i64,
        channel_public_id: Option<&str>,
    ) -> Result<i64> {
        dispatch!(self, count_health_issues, org_id, channel_public_id)
    }
    pub async fn health_issue_snooze(
        &self,
        issue_id: Uuid,
        user_id: Uuid,
        episode_id: Uuid,
        until: Option<DateTime<Utc>>,
    ) -> Result<Option<DateTime<Utc>>> {
        dispatch!(
            self,
            health_issue_snooze,
            issue_id,
            user_id,
            episode_id,
            until
        )
    }
    pub async fn health_notification(
        &self,
        input: CreateNotificationRow,
    ) -> Result<Option<NotificationRow>> {
        dispatch!(self, health_notification, input)
    }
    pub async fn health_sweep_channels(&self, after: Uuid, limit: i64) -> Result<Vec<String>> {
        dispatch!(self, health_sweep_channels, after, limit)
    }
}

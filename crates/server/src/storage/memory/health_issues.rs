use super::InMemoryDatabase;
use crate::storage::{CreateNotificationRow, HealthIssueRow, NotificationRow, ObserveHealthIssue};
use anyhow::Result;
use chrono::{DateTime, Utc};
use everruns_contracts::typed_id::NotificationId;
use uuid::Uuid;

impl InMemoryDatabase {
    pub async fn count_visible_health_notifications(
        &self,
        org_id: i64,
        user_id: Uuid,
    ) -> Result<u32> {
        let rows: Vec<_> = self
            .notifications
            .read()
            .values()
            .filter(|n| {
                n.org_id == org_id
                    && n.user_id == user_id
                    && n.kind == "health.issue"
                    && n.viewed_at.is_none()
            })
            .cloned()
            .collect();
        let mut count = 0;
        for row in rows {
            if let Some(id) = row.target_id.as_deref().and_then(|id| id.parse().ok())
                && let Some(issue) = self.get_health_issue(org_id, id).await?
                && issue.status != "inapplicable"
                && self
                    .ingress_channels
                    .read()
                    .get(&issue.channel_id)
                    .is_some_and(|e| e.enabled && e.channel_status != "disabled")
            {
                count += 1;
            }
        }
        Ok(count)
    }

    pub async fn observe_health_issue(&self, input: ObserveHealthIssue) -> Result<()> {
        let channels = self.ingress_channels.read();
        let Some(channel) = channels
            .get(&input.channel_id)
            .filter(|e| e.org_id == input.org_id && e.updated_at == input.channel_revision)
        else {
            return Ok(());
        };
        let mut issues = self.health_issues.write();
        if let Some(row) = issues
            .values_mut()
            .find(|r| r.org_id == input.org_id && r.channel_id == input.channel_id)
        {
            if row.last_checked_at > input.checked_at {
                return Ok(());
            }
            if matches!(row.status.as_str(), "resolved" | "inapplicable")
                && matches!(input.status.as_str(), "open" | "needs_check")
            {
                row.episode_id = Uuid::now_v7();
                row.first_detected_at = input.checked_at;
            }
            if input.status != "needs_check" {
                row.missing_scopes = input.missing_scopes;
            }
            if input.status != "needs_check" || row.status != "open" {
                row.status = input.status;
            }
            row.error_code = input.error_code;
            row.channel_revision = input.channel_revision;
            row.last_checked_at = input.checked_at;
            if matches!(row.status.as_str(), "resolved" | "inapplicable") {
                let key = format!("health:{}:{}", row.id, row.episode_id);
                for n in self.notifications.write().values_mut().filter(|n| {
                    n.org_id == input.org_id
                        && n.kind == "health.issue"
                        && n.dedupe_key.as_deref() == Some(key.as_str())
                }) {
                    if n.payload["status"] != row.status {
                        n.title = if row.status == "resolved" {
                            "Slack permissions verified"
                        } else {
                            "Slack issue no longer applies"
                        }
                        .into();
                        n.body = if row.status == "resolved" {
                            "This installation has the required permissions."
                        } else {
                            "The affected integration is disabled."
                        }
                        .into();
                        n.viewed_at = Some(n.viewed_at.unwrap_or_else(Utc::now));
                        n.updated_at = Utc::now();
                        n.payload["status"] = row.status.clone().into();
                    }
                }
            }
            return Ok(());
        }
        let row = HealthIssueRow {
            id: Uuid::now_v7(),
            org_id: input.org_id,
            channel_id: input.channel_id,
            channel_public_id: channel.channel_public_id.clone(),
            agent_public_id: channel.agent_public_id.clone(),
            agent_name: channel.agent_name.clone(),
            code: "slack.permissions".into(),
            episode_id: Uuid::now_v7(),
            status: input.status,
            missing_scopes: input.missing_scopes,
            error_code: input.error_code,
            channel_revision: input.channel_revision,
            first_detected_at: input.checked_at,
            last_checked_at: input.checked_at,
        };
        issues.insert(row.id, row);
        Ok(())
    }

    pub async fn list_health_issues(
        &self,
        org_id: i64,
        offset: i64,
        limit: i64,
        channel_public_id: Option<&str>,
    ) -> Result<Vec<HealthIssueRow>> {
        let channels = self.ingress_channels.read();
        let mut rows: Vec<_> = self
            .health_issues
            .read()
            .values()
            .filter(|r| {
                r.org_id == org_id
                    && channel_public_id.is_none_or(|id| r.channel_public_id == id)
                    && matches!(r.status.as_str(), "open" | "needs_check")
                    && channels.get(&r.channel_id).is_some_and(|e| {
                        e.enabled && e.channel_status != "disabled" && e.agent_status == "active"
                    })
            })
            .cloned()
            .collect();
        rows.sort_by_key(|r| std::cmp::Reverse((r.first_detected_at, r.id)));
        Ok(rows
            .into_iter()
            .skip(offset as usize)
            .take(limit as usize)
            .collect())
    }

    pub async fn get_health_issue(&self, org_id: i64, id: Uuid) -> Result<Option<HealthIssueRow>> {
        let row = self
            .health_issues
            .read()
            .get(&id)
            .filter(|r| r.org_id == org_id)
            .cloned();
        Ok(row.filter(|r| {
            self.ingress_channels
                .read()
                .get(&r.channel_id)
                .is_some_and(|e| e.agent_status == "active")
        }))
    }
    pub async fn count_health_issues(
        &self,
        org_id: i64,
        channel_public_id: Option<&str>,
    ) -> Result<i64> {
        Ok(self
            .list_health_issues(org_id, 0, i64::MAX, channel_public_id)
            .await?
            .len() as i64)
    }
    pub async fn health_issue_snooze(
        &self,
        issue_id: Uuid,
        user_id: Uuid,
        episode_id: Uuid,
        until: Option<DateTime<Utc>>,
    ) -> Result<Option<DateTime<Utc>>> {
        let mut reminders = self.health_reminders.write();
        if let Some(until) = until {
            reminders.insert((issue_id, user_id), (episode_id, until));
        }
        Ok(reminders
            .get(&(issue_id, user_id))
            .filter(|(episode, until)| *episode == episode_id && *until > Utc::now())
            .map(|(_, until)| *until))
    }
    pub async fn health_notification(
        &self,
        input: CreateNotificationRow,
    ) -> Result<Option<NotificationRow>> {
        let issues = self.health_issues.read();
        let Some(issue) = input
            .target_id
            .as_deref()
            .and_then(|id| id.parse::<Uuid>().ok())
            .and_then(|id| issues.get(&id))
        else {
            return Ok(None);
        };
        if !matches!(issue.status.as_str(), "open" | "needs_check")
            || input.payload["episode_id"] != issue.episode_id.to_string()
        {
            return Ok(None);
        }
        let mut reminders = self.health_reminders.write();
        let due = reminders
            .get(&(issue.id, input.user_id))
            .is_some_and(|(episode, until)| *episode == issue.episode_id && *until <= Utc::now());
        if due {
            reminders.remove(&(issue.id, input.user_id));
        }
        let mut notifications = self.notifications.write();
        if let Some(row) = notifications.values_mut().find(|r| {
            r.org_id == input.org_id
                && r.user_id == input.user_id
                && r.kind == "health.issue"
                && r.dedupe_key == input.dedupe_key
        }) {
            if due {
                row.viewed_at = None;
                row.updated_at = Utc::now();
            }
            return Ok(Some(row.clone()));
        }
        let row = NotificationRow {
            id: NotificationId::new(),
            org_id: input.org_id,
            user_id: input.user_id,
            kind: input.kind,
            title: input.title,
            body: input.body,
            target_type: input.target_type,
            target_id: input.target_id,
            href: input.href,
            payload: input.payload,
            dedupe_key: input.dedupe_key,
            occurrence_count: 1,
            viewed_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        notifications.insert(row.id, row.clone());
        Ok(Some(row))
    }
    pub async fn health_sweep_channels(&self, after: Uuid, limit: i64) -> Result<Vec<String>> {
        let mut rows: Vec<_> = self
            .ingress_channels
            .read()
            .values()
            .filter(|e| {
                e.channel_id > after && e.channel_type == "slack" && e.agent_status == "active"
            })
            .map(|e| (e.channel_id, e.channel_public_id.clone()))
            .collect();
        rows.sort();
        Ok(rows
            .into_iter()
            .take(limit as usize)
            .map(|(_, id)| id)
            .collect())
    }
}

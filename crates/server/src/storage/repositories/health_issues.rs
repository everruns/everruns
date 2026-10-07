use super::Database;
use crate::storage::{HealthIssueRow, ObserveHealthIssue};
use anyhow::Result;
use chrono::{DateTime, Utc};
use uuid::Uuid;

impl Database {
    pub async fn count_visible_health_notifications(
        &self,
        org_id: i64,
        user_id: Uuid,
    ) -> Result<u32> {
        let count:i64=sqlx::query_scalar("SELECT COUNT(*) FROM notifications n JOIN health_issues h ON n.target_id = h.id::text AND n.org_id=h.org_id JOIN agent_channels ae ON ae.id=h.channel_id JOIN agents a ON a.id=ae.agent_id WHERE n.org_id=$1 AND n.user_id=$2 AND n.kind='health.issue' AND n.viewed_at IS NULL AND a.status='active' AND ae.enabled AND ae.status<>'disabled' AND h.status <> 'inapplicable'")
            .bind(org_id).bind(user_id).fetch_one(&self.pool).await?;
        Ok(count as u32)
    }

    pub async fn observe_health_issue(&self, input: ObserveHealthIssue) -> Result<()> {
        // Compare the observed channel revision in the same statement that records
        // the result. A slow probe must not resolve a newly replaced credential.
        let mut tx = self.pool.begin().await?;
        let current: Option<Uuid> = sqlx::query_scalar("SELECT ae.id FROM agent_channels ae JOIN agents a ON a.id=ae.agent_id WHERE ae.id=$1 AND a.org_id=$2 AND ae.updated_at=$3 FOR UPDATE OF ae")
            .bind(input.channel_id).bind(input.org_id).bind(input.channel_revision)
            .fetch_optional(&mut *tx).await?;
        if current.is_none() {
            return Ok(());
        }
        let changed=sqlx::query_as::<_,(Uuid,Uuid,String)>(
            "INSERT INTO health_issues (org_id, channel_id, code, status, missing_scopes,
                error_code, channel_revision, last_checked_at)
             SELECT $1, ae.id, 'slack.permissions', $4, $5, $6, $3, $7
             FROM agent_channels ae JOIN agents a ON a.id = ae.agent_id
             WHERE ae.id = $2 AND a.org_id = $1 AND ae.updated_at = $3
             ON CONFLICT (org_id, channel_id, code) DO UPDATE SET
                episode_id = CASE WHEN health_issues.status IN ('resolved', 'inapplicable')
                    AND EXCLUDED.status IN ('open', 'needs_check') THEN uuidv7()
                    ELSE health_issues.episode_id END,
                first_detected_at = CASE WHEN health_issues.status IN ('resolved', 'inapplicable')
                    AND EXCLUDED.status IN ('open', 'needs_check') THEN EXCLUDED.last_checked_at
                    ELSE health_issues.first_detected_at END,
                status = CASE WHEN EXCLUDED.status = 'needs_check' AND health_issues.status = 'open'
                    THEN 'open' ELSE EXCLUDED.status END,
                missing_scopes = CASE WHEN EXCLUDED.status = 'needs_check'
                    THEN health_issues.missing_scopes ELSE EXCLUDED.missing_scopes END,
                error_code = EXCLUDED.error_code,
                channel_revision = EXCLUDED.channel_revision,
                last_checked_at = EXCLUDED.last_checked_at
             WHERE health_issues.last_checked_at <= EXCLUDED.last_checked_at RETURNING id,episode_id,status",
        )
        .bind(input.org_id).bind(input.channel_id).bind(input.channel_revision)
        .bind(input.status).bind(input.missing_scopes).bind(input.error_code)
        .bind(input.checked_at).fetch_optional(&mut *tx).await?;
        if let Some((id, episode, status)) = changed
            && matches!(status.as_str(), "resolved" | "inapplicable")
        {
            sqlx::query("UPDATE notifications SET title=$4, body=$5, viewed_at=COALESCE(viewed_at,NOW()),payload=payload || jsonb_build_object('status',$6::text) WHERE org_id=$1 AND kind='health.issue' AND target_id=$2 AND dedupe_key=$3 AND payload->>'status' IS DISTINCT FROM $6")
                .bind(input.org_id).bind(id.to_string()).bind(format!("health:{id}:{episode}"))
                .bind(if status=="resolved" {"Slack permissions verified"} else {"Slack issue no longer applies"})
                .bind(if status=="resolved" {"This installation has the required permissions."} else {"The affected integration is disabled."})
                .bind(status).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn list_health_issues(
        &self,
        org_id: i64,
        offset: i64,
        limit: i64,
        channel_public_id: Option<&str>,
    ) -> Result<Vec<HealthIssueRow>> {
        sqlx::query_as("SELECT h.*, ae.public_id AS channel_public_id, a.public_id AS agent_public_id, COALESCE(a.display_name, a.name) AS agent_name FROM health_issues h JOIN agent_channels ae ON ae.id = h.channel_id JOIN agents a ON a.id = ae.agent_id WHERE h.org_id = $1
            AND ($4::text IS NULL OR ae.public_id = $4) AND h.status IN ('open', 'needs_check') AND ae.enabled
            AND ae.status <> 'disabled' AND a.status = 'active'
            ORDER BY h.first_detected_at DESC, h.id DESC OFFSET $2 LIMIT $3")
            .bind(org_id).bind(offset).bind(limit).bind(channel_public_id).fetch_all(&self.pool).await.map_err(Into::into)
    }

    pub async fn get_health_issue(&self, org_id: i64, id: Uuid) -> Result<Option<HealthIssueRow>> {
        sqlx::query_as("SELECT h.*, ae.public_id AS channel_public_id, a.public_id AS agent_public_id, COALESCE(a.display_name, a.name) AS agent_name FROM health_issues h JOIN agent_channels ae ON ae.id = h.channel_id JOIN agents a ON a.id = ae.agent_id WHERE h.org_id = $1 AND h.id = $2
            AND a.status = 'active'")
            .bind(org_id).bind(id).fetch_optional(&self.pool).await.map_err(Into::into)
    }

    pub async fn count_health_issues(
        &self,
        org_id: i64,
        channel_public_id: Option<&str>,
    ) -> Result<i64> {
        sqlx::query_scalar(
            "SELECT COUNT(*) FROM health_issues h
            JOIN agent_channels ae ON ae.id = h.channel_id JOIN agents a ON a.id = ae.agent_id
            WHERE h.org_id = $1 AND ($2::text IS NULL OR ae.public_id = $2) AND h.status IN ('open', 'needs_check')
            AND ae.enabled AND ae.status <> 'disabled' AND a.status = 'active'",
        )
        .bind(org_id)
        .bind(channel_public_id)
        .fetch_one(&self.pool)
        .await
        .map_err(Into::into)
    }

    pub async fn health_issue_snooze(
        &self,
        issue_id: Uuid,
        user_id: Uuid,
        episode_id: Uuid,
        until: Option<DateTime<Utc>>,
    ) -> Result<Option<DateTime<Utc>>> {
        if let Some(until) = until {
            sqlx::query(
                "INSERT INTO health_issue_reminders (issue_id, user_id, episode_id, snoozed_until)
                VALUES ($1, $2, $3, $4) ON CONFLICT (issue_id, user_id) DO UPDATE
                SET episode_id = EXCLUDED.episode_id, snoozed_until = EXCLUDED.snoozed_until",
            )
            .bind(issue_id)
            .bind(user_id)
            .bind(episode_id)
            .bind(until)
            .execute(&self.pool)
            .await?;
        }
        sqlx::query_scalar(
            "SELECT snoozed_until FROM health_issue_reminders
            WHERE issue_id = $1 AND user_id = $2 AND episode_id = $3 AND snoozed_until > NOW()",
        )
        .bind(issue_id)
        .bind(user_id)
        .bind(episode_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(Into::into)
    }

    pub async fn health_notification(
        &self,
        input: crate::storage::CreateNotificationRow,
    ) -> Result<Option<crate::storage::NotificationRow>> {
        let Some(issue_id) = input
            .target_id
            .as_deref()
            .and_then(|id| id.parse::<Uuid>().ok())
        else {
            return Ok(None);
        };
        let mut tx = self.pool.begin().await?;
        let current: Option<(String, Uuid)> = sqlx::query_as(
            "SELECT status,episode_id FROM health_issues WHERE org_id=$1 AND id=$2 FOR UPDATE",
        )
        .bind(input.org_id)
        .bind(issue_id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some((status, episode)) = current else {
            return Ok(None);
        };
        if !matches!(status.as_str(), "open" | "needs_check")
            || input.payload["episode_id"] != episode.to_string()
        {
            return Ok(None);
        }
        let source = input.source.clone();
        sqlx::query("INSERT INTO notifications (org_id,user_id,kind,title,body,target_type,target_id,href,payload,dedupe_key,source_type,source_id,source_name)
            VALUES ($1,$2,'health.issue',$3,$4,'health_issue',$5,$6,$7,$8,$9,$10,$11) ON CONFLICT DO NOTHING")
            .bind(input.org_id).bind(input.user_id).bind(input.title).bind(input.body)
            .bind(&input.target_id).bind(input.href).bind(input.payload).bind(&input.dedupe_key)
            .bind(source.as_ref().map(|s| s.source_type.clone()))
            .bind(source.as_ref().and_then(|s| s.source_id.clone()))
            .bind(source.and_then(|s| s.source_name)).execute(&mut *tx).await?;
        let due:Option<Uuid>=sqlx::query_scalar("DELETE FROM health_issue_reminders WHERE issue_id=$1 AND user_id=$2 AND episode_id=$3 AND snoozed_until<=NOW() RETURNING issue_id")
            .bind(issue_id).bind(input.user_id).bind(episode).fetch_optional(&mut *tx).await?;
        if due.is_some() {
            sqlx::query("UPDATE notifications SET viewed_at=NULL,updated_at=NOW() WHERE org_id=$1 AND user_id=$2 AND kind='health.issue' AND dedupe_key=$3")
                .bind(input.org_id).bind(input.user_id).bind(&input.dedupe_key).execute(&mut *tx).await?;
        }
        let row=sqlx::query_as("SELECT * FROM notifications WHERE org_id=$1 AND user_id=$2 AND kind='health.issue' AND dedupe_key=$3")
            .bind(input.org_id).bind(input.user_id).bind(input.dedupe_key).fetch_optional(&mut *tx).await?;
        tx.commit().await?;
        Ok(row)
    }

    /// Small batches across organizations; no credential data in the sweep cursor.
    pub async fn health_sweep_channels(&self, after: Uuid, limit: i64) -> Result<Vec<String>> {
        sqlx::query_scalar(
            "SELECT ae.public_id FROM agent_channels ae JOIN agents a ON a.id = ae.agent_id
            WHERE ae.id > $1 AND ae.channel_type = 'slack' AND a.status = 'active'
            ORDER BY ae.id LIMIT $2",
        )
        .bind(after)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(Into::into)
    }
}

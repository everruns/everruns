//! What the Inspector reads and records (`domains/health_issues/inspector.rs`).
//!
//! Decisions:
//! - One investigation per finding episode, claimed with an insert before the
//!   model is called, so replicas never pay twice. A failed claim older than
//!   the retry interval can be taken again, up to a few attempts.
//! - A cluster is chosen by the Inspector from the org's recent clusters of
//!   the same kind, or created from the title it proposes. Clusters are per
//!   org and kind: findings from different orgs are never grouped together.
//! - Dismissal is recorded on the episode's investigation row, creating one
//!   when the Inspector never ran, and keeps the reason.
use super::StorageBackend;
use anyhow::Result;
use chrono::{DateTime, Utc};
use uuid::Uuid;

/// An open finding the Inspector has not written up yet.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct InvestigationCandidateRow {
    pub health_issue_id: Uuid,
    pub episode_id: Uuid,
    pub org_id: i64,
    pub code: String,
    pub session_id: Option<Uuid>,
    pub agent_id: Option<Uuid>,
    pub evidence: Option<serde_json::Value>,
    pub first_detected_at: DateTime<Utc>,
}

/// A cluster the Inspector may file a finding under.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct FindingClusterRow {
    pub id: Uuid,
    pub title: String,
}

/// The Inspector's write-up of a finding episode, with its cluster.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct FindingInvestigationRow {
    pub status: String,
    pub what_happened: Option<String>,
    pub severity: Option<String>,
    pub suggested_action: Option<String>,
    pub cluster_id: Option<Uuid>,
    pub cluster_title: Option<String>,
    /// Open findings in the cluster, this one included.
    pub cluster_open: Option<i64>,
    pub model: Option<String>,
    pub completed_at: Option<DateTime<Utc>>,
    pub dismissed_at: Option<DateTime<Utc>>,
    pub dismiss_reason: Option<String>,
}

/// An open finding of the platform's own concern, across organizations.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PlatformDetectionRow {
    pub health_issue_id: Uuid,
    pub org_public_id: String,
    pub org_name: String,
    pub code: String,
    pub session_id: Option<Uuid>,
    pub agent_name: Option<String>,
    pub evidence: Option<serde_json::Value>,
    pub first_detected_at: DateTime<Utc>,
    pub what_happened: Option<String>,
    pub severity: Option<String>,
    pub suggested_action: Option<String>,
    pub cluster_title: Option<String>,
}

/// What the Inspector concluded about one finding.
#[derive(Debug, Clone)]
pub struct CompletedInvestigation<'a> {
    pub what_happened: &'a str,
    pub severity: &'a str,
    pub suggested_action: &'a str,
    pub cluster_id: Uuid,
    pub model: &'a str,
    pub cost_usd: Option<f64>,
}

impl StorageBackend {
    /// Open findings of `codes` whose current episode has no investigation,
    /// or only a failed one old enough to retry, oldest first.
    pub async fn findings_to_investigate(
        &self,
        codes: &[&str],
        retry_secs: i64,
        max_attempts: i32,
        limit: i64,
    ) -> Result<Vec<InvestigationCandidateRow>> {
        let codes: Vec<String> = codes.iter().map(|c| c.to_string()).collect();
        Ok(sqlx::query_as(
            "SELECT h.id AS health_issue_id, h.episode_id, h.org_id, h.code, h.session_id,
                    COALESCE(h.agent_id, s.agent_id) AS agent_id, h.evidence, h.first_detected_at
             FROM health_issues h
             LEFT JOIN sessions s ON s.id = h.session_id
             LEFT JOIN finding_investigations i
               ON i.health_issue_id = h.id AND i.episode_id = h.episode_id
             WHERE h.code = ANY($1) AND h.status IN ('open', 'needs_check')
               AND (i.health_issue_id IS NULL
                    OR (i.status = 'failed' AND i.dismissed_at IS NULL AND i.attempts < $3
                        AND i.claimed_at < NOW() - make_interval(secs => $2)))
             ORDER BY h.first_detected_at
             LIMIT $4",
        )
        .bind(codes)
        .bind(retry_secs as f64)
        .bind(max_attempts)
        .bind(limit)
        .fetch_all(self.database().pool())
        .await?)
    }

    /// Claim a finding episode for investigation. `false` when another
    /// replica holds it.
    pub async fn claim_finding_investigation(
        &self,
        health_issue_id: Uuid,
        episode_id: Uuid,
        org_id: i64,
        retry_secs: i64,
    ) -> Result<bool> {
        let claimed: Option<Uuid> = sqlx::query_scalar(
            "INSERT INTO finding_investigations (health_issue_id, episode_id, org_id)
             VALUES ($1, $2, $3)
             ON CONFLICT (health_issue_id, episode_id) DO UPDATE
                SET status = 'running', attempts = finding_investigations.attempts + 1,
                    claimed_at = NOW(), error = NULL
                WHERE finding_investigations.status = 'failed'
                  AND finding_investigations.dismissed_at IS NULL
                  AND finding_investigations.claimed_at < NOW() - make_interval(secs => $4)
             RETURNING health_issue_id",
        )
        .bind(health_issue_id)
        .bind(episode_id)
        .bind(org_id)
        .bind(retry_secs as f64)
        .fetch_optional(self.database().pool())
        .await?;
        Ok(claimed.is_some())
    }

    /// The org's clusters of `code` seen in the last `days`, newest first.
    pub async fn recent_finding_clusters(
        &self,
        org_id: i64,
        code: &str,
        days: i32,
        limit: i64,
    ) -> Result<Vec<FindingClusterRow>> {
        Ok(sqlx::query_as(
            "SELECT id, title FROM finding_clusters
             WHERE org_id = $1 AND code = $2
               AND last_seen_at >= NOW() - make_interval(days => $3)
             ORDER BY last_seen_at DESC LIMIT $4",
        )
        .bind(org_id)
        .bind(code)
        .bind(days)
        .bind(limit)
        .fetch_all(self.database().pool())
        .await?)
    }

    /// File a finding under `existing` when it is one of the org's clusters
    /// of `code`, else under a new cluster titled `title`.
    pub async fn file_finding_cluster(
        &self,
        org_id: i64,
        code: &str,
        existing: Option<Uuid>,
        title: &str,
    ) -> Result<Uuid> {
        if let Some(id) = existing {
            let touched: Option<Uuid> = sqlx::query_scalar(
                "UPDATE finding_clusters SET last_seen_at = NOW()
                 WHERE id = $1 AND org_id = $2 AND code = $3 RETURNING id",
            )
            .bind(id)
            .bind(org_id)
            .bind(code)
            .fetch_optional(self.database().pool())
            .await?;
            if let Some(id) = touched {
                return Ok(id);
            }
        }
        Ok(sqlx::query_scalar(
            "INSERT INTO finding_clusters (org_id, code, title) VALUES ($1, $2, $3) RETURNING id",
        )
        .bind(org_id)
        .bind(code)
        .bind(title)
        .fetch_one(self.database().pool())
        .await?)
    }

    pub async fn complete_finding_investigation(
        &self,
        health_issue_id: Uuid,
        episode_id: Uuid,
        done: CompletedInvestigation<'_>,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE finding_investigations
             SET status = 'done', completed_at = NOW(), what_happened = $3, severity = $4,
                 suggested_action = $5, cluster_id = $6, model = $7, cost_usd = $8, error = NULL
             WHERE health_issue_id = $1 AND episode_id = $2",
        )
        .bind(health_issue_id)
        .bind(episode_id)
        .bind(done.what_happened)
        .bind(done.severity)
        .bind(done.suggested_action)
        .bind(done.cluster_id)
        .bind(done.model)
        .bind(done.cost_usd)
        .execute(self.database().pool())
        .await?;
        Ok(())
    }

    /// Record why an investigation did not finish: `failed` is retried,
    /// `unavailable` (the org has no model) is not.
    pub async fn fail_finding_investigation(
        &self,
        health_issue_id: Uuid,
        episode_id: Uuid,
        status: &str,
        error: &str,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE finding_investigations SET status = $3, completed_at = NOW(), error = $4
             WHERE health_issue_id = $1 AND episode_id = $2",
        )
        .bind(health_issue_id)
        .bind(episode_id)
        .bind(status)
        .bind(error)
        .execute(self.database().pool())
        .await?;
        Ok(())
    }

    /// The write-up of a finding's current episode, if there is one.
    pub async fn finding_investigation(
        &self,
        health_issue_id: Uuid,
        episode_id: Uuid,
    ) -> Result<Option<FindingInvestigationRow>> {
        Ok(sqlx::query_as(
            "SELECT i.status, i.what_happened, i.severity, i.suggested_action, i.cluster_id,
                    c.title AS cluster_title,
                    (SELECT COUNT(*) FROM finding_investigations o
                       JOIN health_issues oh ON oh.id = o.health_issue_id
                                            AND oh.episode_id = o.episode_id
                      WHERE o.cluster_id = i.cluster_id
                        AND oh.status IN ('open', 'needs_check')) AS cluster_open,
                    i.model, i.completed_at, i.dismissed_at, i.dismiss_reason
             FROM finding_investigations i
             LEFT JOIN finding_clusters c ON c.id = i.cluster_id
             WHERE i.health_issue_id = $1 AND i.episode_id = $2",
        )
        .bind(health_issue_id)
        .bind(episode_id)
        .fetch_optional(self.database().pool())
        .await?)
    }

    /// Keep a dismissal and its reason on the episode's investigation row.
    pub async fn dismiss_finding(
        &self,
        health_issue_id: Uuid,
        episode_id: Uuid,
        org_id: i64,
        user_id: Option<Uuid>,
        reason: &str,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO finding_investigations
                (health_issue_id, episode_id, org_id, status, error,
                 dismissed_at, dismissed_by, dismiss_reason)
             VALUES ($1, $2, $3, 'unavailable', 'dismissed before investigation', NOW(), $4, $5)
             ON CONFLICT (health_issue_id, episode_id) DO UPDATE
                SET dismissed_at = NOW(), dismissed_by = $4, dismiss_reason = $5",
        )
        .bind(health_issue_id)
        .bind(episode_id)
        .bind(org_id)
        .bind(user_id)
        .bind(reason)
        .execute(self.database().pool())
        .await?;
        Ok(())
    }

    /// Whether a person dismissed a `code` finding about `subject_id`, so the
    /// detectors leave it closed.
    pub async fn finding_dismissed(
        &self,
        org_id: i64,
        code: &str,
        subject_id: &str,
    ) -> Result<bool> {
        Ok(sqlx::query_scalar(
            "SELECT EXISTS (
                SELECT 1 FROM finding_investigations i
                JOIN health_issues h ON h.id = i.health_issue_id
                WHERE h.org_id = $1 AND h.code = $2 AND h.subject_id = $3
                  AND i.dismissed_at IS NOT NULL)",
        )
        .bind(org_id)
        .bind(code)
        .bind(subject_id)
        .fetch_one(self.database().pool())
        .await?)
    }

    /// Open findings of `codes` across every organization, the newest first,
    /// for the platform operator.
    pub async fn platform_detections(
        &self,
        codes: &[&str],
        offset: i64,
        limit: i64,
    ) -> Result<Vec<PlatformDetectionRow>> {
        let codes: Vec<String> = codes.iter().map(|c| c.to_string()).collect();
        Ok(sqlx::query_as(
            "SELECT h.id AS health_issue_id, o.public_id AS org_public_id, o.name AS org_name, h.code, h.session_id,
                    COALESCE(a.display_name, a.name) AS agent_name, h.evidence,
                    h.first_detected_at, i.what_happened, i.severity, i.suggested_action,
                    c.title AS cluster_title
             FROM health_issues h
             JOIN organizations o ON o.org_id = h.org_id
             LEFT JOIN agents a ON a.id = h.agent_id
             LEFT JOIN finding_investigations i
               ON i.health_issue_id = h.id AND i.episode_id = h.episode_id
             LEFT JOIN finding_clusters c ON c.id = i.cluster_id
             WHERE h.code = ANY($1) AND h.status IN ('open', 'needs_check')
             ORDER BY h.first_detected_at DESC, h.id DESC
             OFFSET $2 LIMIT $3",
        )
        .bind(codes)
        .bind(offset)
        .bind(limit)
        .fetch_all(self.database().pool())
        .await?)
    }

    pub async fn count_platform_detections(&self, codes: &[&str]) -> Result<i64> {
        let codes: Vec<String> = codes.iter().map(|c| c.to_string()).collect();
        Ok(sqlx::query_scalar(
            "SELECT COUNT(*) FROM health_issues
             WHERE code = ANY($1) AND status IN ('open', 'needs_check')",
        )
        .bind(codes)
        .fetch_one(self.database().pool())
        .await?)
    }
}

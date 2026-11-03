//! Self-healing (`domains/health_issues/self_healing.rs`): the org's policy on
//! `organization_settings.self_healing` and the actions taken on findings.
//!
//! Decisions:
//! - One action of a kind per finding episode: a second pause for the same
//!   episode keeps the first, so the scheduler and a person acting at once
//!   record one action.
//! - Undo is compare-and-set on `undone_at IS NULL`, so it happens once.
use super::StorageBackend;
use anyhow::Result;
use chrono::{DateTime, Utc};
use uuid::Uuid;

/// An action taken on a finding, as stored.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct FindingActionRow {
    pub id: Uuid,
    pub public_id: String,
    pub org_id: i64,
    pub health_issue_id: Uuid,
    pub episode_id: Uuid,
    pub kind: String,
    pub target_id: String,
    pub taken_by: Option<Uuid>,
    pub undone_by: Option<Uuid>,
    pub undone_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

const COLUMNS: &str = "id, public_id, org_id, health_issue_id, episode_id, kind, target_id, \
    taken_by, undone_by, undone_at, created_at";

impl StorageBackend {
    /// The org's self-healing policy as stored; `None` when it never set one.
    pub async fn org_self_healing(&self, org_id: i64) -> Result<Option<serde_json::Value>> {
        let stored: Option<Option<serde_json::Value>> =
            sqlx::query_scalar("SELECT self_healing FROM organization_settings WHERE org_id = $1")
                .bind(org_id)
                .fetch_optional(self.database().pool())
                .await?;
        Ok(stored.flatten())
    }

    /// Replace the org's self-healing policy.
    pub async fn set_org_self_healing(
        &self,
        org_id: i64,
        policy: &serde_json::Value,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO organization_settings (org_id, self_healing) VALUES ($1, $2) \
             ON CONFLICT (org_id) DO UPDATE SET self_healing = EXCLUDED.self_healing, \
             updated_at = NOW()",
        )
        .bind(org_id)
        .bind(policy)
        .execute(self.database().pool())
        .await?;
        Ok(())
    }

    /// Record an action on a finding episode. `None` when the episode already
    /// has one of this kind.
    pub async fn record_finding_action(
        &self,
        org_id: i64,
        health_issue_id: Uuid,
        episode_id: Uuid,
        kind: &str,
        target_id: &str,
        taken_by: Option<Uuid>,
    ) -> Result<Option<FindingActionRow>> {
        let id = Uuid::now_v7();
        let sql = format!(
            "INSERT INTO finding_actions (id, public_id, org_id, health_issue_id, episode_id,
                 kind, target_id, taken_by)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
             ON CONFLICT (health_issue_id, episode_id, kind) DO NOTHING
             RETURNING {COLUMNS}"
        );
        Ok(sqlx::query_as(sqlx::AssertSqlSafe(sql.as_str()))
            .bind(id)
            .bind(format!("fact_{}", id.simple()))
            .bind(org_id)
            .bind(health_issue_id)
            .bind(episode_id)
            .bind(kind)
            .bind(target_id)
            .bind(taken_by)
            .fetch_optional(self.database().pool())
            .await?)
    }

    /// The actions taken on a finding, newest first, every episode included.
    pub async fn list_finding_actions(
        &self,
        org_id: i64,
        health_issue_id: Uuid,
    ) -> Result<Vec<FindingActionRow>> {
        let sql = format!(
            "SELECT {COLUMNS} FROM finding_actions
             WHERE org_id = $1 AND health_issue_id = $2
             ORDER BY created_at DESC LIMIT 50"
        );
        Ok(sqlx::query_as(sqlx::AssertSqlSafe(sql.as_str()))
            .bind(org_id)
            .bind(health_issue_id)
            .fetch_all(self.database().pool())
            .await?)
    }

    pub async fn get_finding_action(
        &self,
        org_id: i64,
        public_id: &str,
    ) -> Result<Option<FindingActionRow>> {
        let sql =
            format!("SELECT {COLUMNS} FROM finding_actions WHERE org_id = $1 AND public_id = $2");
        Ok(sqlx::query_as(sqlx::AssertSqlSafe(sql.as_str()))
            .bind(org_id)
            .bind(public_id)
            .fetch_optional(self.database().pool())
            .await?)
    }

    /// Mark an action undone. `false` when it already was.
    pub async fn undo_finding_action(&self, id: Uuid, undone_by: Option<Uuid>) -> Result<bool> {
        let done = sqlx::query(
            "UPDATE finding_actions SET undone_by = $2, undone_at = NOW()
             WHERE id = $1 AND undone_at IS NULL",
        )
        .bind(id)
        .bind(undone_by)
        .execute(self.database().pool())
        .await?;
        Ok(done.rows_affected() == 1)
    }

    /// Put back an undo that could not be carried out.
    pub async fn redo_finding_action(&self, id: Uuid) -> Result<()> {
        sqlx::query("UPDATE finding_actions SET undone_by = NULL, undone_at = NULL WHERE id = $1")
            .bind(id)
            .execute(self.database().pool())
            .await?;
        Ok(())
    }
}

//! Suggested fixes for findings (`domains/health_issues/fixes.rs`).
//!
//! Decisions:
//! - One fix per finding episode: the Inspector writes it with the write-up,
//!   and a second write for the same episode keeps the first.
//! - Status moves only forward through the transitions the domain allows;
//!   each update names the status it expects, so two people acting at once
//!   cannot both apply or both check the same fix.
use super::StorageBackend;
use anyhow::Result;
use chrono::{DateTime, Utc};
use uuid::Uuid;

/// A suggested fix as stored.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct FindingFixRow {
    pub id: Uuid,
    pub public_id: String,
    pub org_id: i64,
    pub health_issue_id: Uuid,
    pub episode_id: Uuid,
    pub agent_id: Uuid,
    pub agent_public_id: String,
    pub summary: String,
    pub addition: String,
    pub base_instructions_sha256: String,
    pub status: String,
    pub experiment_id: Option<Uuid>,
    pub experiment_public_id: Option<String>,
    pub eval_public_id: Option<String>,
    pub check_result: Option<serde_json::Value>,
    pub decided_at: Option<DateTime<Utc>>,
    pub decision_note: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A fix the Inspector wrote.
#[derive(Debug, Clone)]
pub struct NewFindingFix<'a> {
    pub org_id: i64,
    pub health_issue_id: Uuid,
    pub episode_id: Uuid,
    pub agent_id: Uuid,
    pub summary: &'a str,
    pub addition: &'a str,
    pub base_instructions_sha256: &'a str,
}

const SELECT: &str = "SELECT f.id, f.public_id, f.org_id, f.health_issue_id, f.episode_id,
        f.agent_id, a.public_id AS agent_public_id, f.summary, f.addition,
        f.base_instructions_sha256, f.status, f.experiment_id,
        x.public_id AS experiment_public_id, e.public_id AS eval_public_id,
        f.check_result, f.decided_at, f.decision_note, f.created_at, f.updated_at
    FROM finding_fixes f
    JOIN agents a ON a.id = f.agent_id
    LEFT JOIN eval_experiments x ON x.id = f.experiment_id
    LEFT JOIN evals e ON e.id = x.eval_id";

impl StorageBackend {
    /// Keep the Inspector's fix for a finding episode. `None` when the
    /// episode already has one.
    pub async fn create_finding_fix(&self, fix: NewFindingFix<'_>) -> Result<Option<Uuid>> {
        let id = Uuid::now_v7();
        let public_id = format!("fix_{}", id.simple());
        let row: Option<(Uuid,)> = sqlx::query_as(
            "INSERT INTO finding_fixes (id, public_id, org_id, health_issue_id, episode_id,
                 agent_id, summary, addition, base_instructions_sha256)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
             ON CONFLICT (health_issue_id, episode_id) DO NOTHING
             RETURNING id",
        )
        .bind(id)
        .bind(public_id)
        .bind(fix.org_id)
        .bind(fix.health_issue_id)
        .bind(fix.episode_id)
        .bind(fix.agent_id)
        .bind(fix.summary)
        .bind(fix.addition)
        .bind(fix.base_instructions_sha256)
        .fetch_optional(self.database().pool())
        .await?;
        Ok(row.map(|(id,)| id))
    }

    /// The fix for a finding episode, if the Inspector wrote one.
    pub async fn finding_fix_for(
        &self,
        health_issue_id: Uuid,
        episode_id: Uuid,
    ) -> Result<Option<FindingFixRow>> {
        let sql = format!("{SELECT} WHERE f.health_issue_id = $1 AND f.episode_id = $2");
        Ok(sqlx::query_as(sqlx::AssertSqlSafe(sql.as_str()))
            .bind(health_issue_id)
            .bind(episode_id)
            .fetch_optional(self.database().pool())
            .await?)
    }

    pub async fn get_finding_fix(
        &self,
        org_id: i64,
        public_id: &str,
    ) -> Result<Option<FindingFixRow>> {
        let sql = format!("{SELECT} WHERE f.org_id = $1 AND f.public_id = $2");
        Ok(sqlx::query_as(sqlx::AssertSqlSafe(sql.as_str()))
            .bind(org_id)
            .bind(public_id)
            .fetch_optional(self.database().pool())
            .await?)
    }

    /// An agent's fixes, newest first; only those still waiting on a person
    /// when `open` is set.
    pub async fn list_agent_finding_fixes(
        &self,
        org_id: i64,
        agent_id: Uuid,
        open: bool,
        limit: i64,
    ) -> Result<Vec<FindingFixRow>> {
        let filter = if open {
            " AND f.status IN ('proposed', 'checking', 'verified', 'regressed', 'check_failed')"
        } else {
            ""
        };
        let sql = format!(
            "{SELECT} WHERE f.org_id = $1 AND f.agent_id = $2{filter}
             ORDER BY f.created_at DESC LIMIT $3"
        );
        Ok(sqlx::query_as(sqlx::AssertSqlSafe(sql.as_str()))
            .bind(org_id)
            .bind(agent_id)
            .bind(limit)
            .fetch_all(self.database().pool())
            .await?)
    }

    /// Move a fix from one of `from` to `to`, with what the move carries.
    /// `false` when the fix was not in one of `from` (someone else moved it).
    #[allow(clippy::too_many_arguments)]
    pub async fn move_finding_fix(
        &self,
        id: Uuid,
        from: &[&str],
        to: &str,
        experiment_id: Option<Uuid>,
        check_result: Option<&serde_json::Value>,
        decided_by: Option<Uuid>,
        decision_note: Option<&str>,
    ) -> Result<bool> {
        let from: Vec<String> = from.iter().map(|s| s.to_string()).collect();
        let decided = matches!(to, "applied" | "rejected");
        let done = sqlx::query(
            "UPDATE finding_fixes
             SET status = $3,
                 experiment_id = COALESCE($4, experiment_id),
                 check_result = CASE WHEN $3 IN ('proposed', 'checking') THEN NULL
                                     ELSE COALESCE($5, check_result) END,
                 decided_by = CASE WHEN $6 THEN $7 ELSE decided_by END,
                 decided_at = CASE WHEN $6 THEN NOW() ELSE decided_at END,
                 decision_note = CASE WHEN $6 THEN $8 ELSE decision_note END,
                 updated_at = NOW()
             WHERE id = $1 AND status = ANY($2)",
        )
        .bind(id)
        .bind(from)
        .bind(to)
        .bind(experiment_id)
        .bind(check_result)
        .bind(decided)
        .bind(decided_by)
        .bind(decision_note)
        .execute(self.database().pool())
        .await?;
        Ok(done.rows_affected() == 1)
    }

    /// Replace a fix's text; it must be checked again.
    pub async fn edit_finding_fix(
        &self,
        id: Uuid,
        from: &[&str],
        summary: &str,
        addition: &str,
        edited_by: Option<Uuid>,
    ) -> Result<bool> {
        let from: Vec<String> = from.iter().map(|s| s.to_string()).collect();
        let done = sqlx::query(
            "UPDATE finding_fixes
             SET summary = $3, addition = $4, edited_by = $5, status = 'proposed',
                 experiment_id = NULL, check_result = NULL, updated_at = NOW()
             WHERE id = $1 AND status = ANY($2)",
        )
        .bind(id)
        .bind(from)
        .bind(summary)
        .bind(addition)
        .bind(edited_by)
        .execute(self.database().pool())
        .await?;
        Ok(done.rows_affected() == 1)
    }

    /// Record the instructions a check runs against.
    pub async fn rebase_finding_fix(&self, id: Uuid, base_instructions_sha256: &str) -> Result<()> {
        sqlx::query(
            "UPDATE finding_fixes SET base_instructions_sha256 = $2, updated_at = NOW()
             WHERE id = $1",
        )
        .bind(id)
        .bind(base_instructions_sha256)
        .execute(self.database().pool())
        .await?;
        Ok(())
    }

    /// The agent's evals that can check a fix: active, targeting the agent,
    /// with at least one case. Most cases first.
    pub async fn agent_evals_with_cases(
        &self,
        org_id: i64,
        agent_id: Uuid,
    ) -> Result<Vec<(String, String, i64)>> {
        Ok(sqlx::query_as(
            "SELECT e.public_id, e.name, COUNT(c.id) AS cases
             FROM evals e
             JOIN agents a ON a.id = $2 AND e.target->>'agent_id' = a.public_id
             JOIN eval_cases c ON c.eval_id = e.id
             WHERE e.org_id = $1 AND e.status NOT IN ('archived', 'deleted')
             GROUP BY e.id, e.public_id, e.name
             ORDER BY cases DESC, e.created_at DESC",
        )
        .bind(org_id)
        .bind(agent_id)
        .fetch_all(self.database().pool())
        .await?)
    }
}

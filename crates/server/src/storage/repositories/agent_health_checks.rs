// Agent health check storage (PostgreSQL). See specs/agent-checks.md.

use anyhow::Result;
use uuid::Uuid;

use crate::storage::Database;
use crate::storage::models::*;

impl Database {
    pub async fn create_agent_health_check_run(
        &self,
        org_id: i64,
        input: CreateAgentHealthCheckRunRow,
    ) -> Result<AgentHealthCheckRunRow> {
        let row = sqlx::query_as::<_, AgentHealthCheckRunRow>(
            r#"
            INSERT INTO agent_health_check_runs (org_id, public_id, agent_id, config_hash, model_id)
            VALUES ($1, $2, $3, $4, $5)
            RETURNING id, org_id, public_id, agent_id, config_hash, model_id, status, summary,
                      results, error_message, started_at, completed_at, created_at, updated_at
            "#,
        )
        .bind(org_id)
        .bind(&input.public_id)
        .bind(input.agent_id)
        .bind(&input.config_hash)
        .bind(&input.model_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn get_agent_health_check_run(
        &self,
        org_id: i64,
        public_id: &str,
    ) -> Result<Option<AgentHealthCheckRunRow>> {
        let row = sqlx::query_as::<_, AgentHealthCheckRunRow>(
            r#"
            SELECT id, org_id, public_id, agent_id, config_hash, model_id, status, summary,
                   results, error_message, started_at, completed_at, created_at, updated_at
            FROM agent_health_check_runs
            WHERE org_id = $1 AND public_id = $2
            "#,
        )
        .bind(org_id)
        .bind(public_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn list_agent_health_check_runs(
        &self,
        org_id: i64,
        agent_id: Uuid,
        limit: i64,
    ) -> Result<Vec<AgentHealthCheckRunRow>> {
        let rows = sqlx::query_as::<_, AgentHealthCheckRunRow>(
            r#"
            SELECT id, org_id, public_id, agent_id, config_hash, model_id, status, summary,
                   results, error_message, started_at, completed_at, created_at, updated_at
            FROM agent_health_check_runs
            WHERE org_id = $1 AND agent_id = $2
            ORDER BY created_at DESC
            LIMIT $3
            "#,
        )
        .bind(org_id)
        .bind(agent_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Latest run for a given agent + resolved config hash (badge / cache).
    pub async fn latest_agent_health_check_run(
        &self,
        org_id: i64,
        agent_id: Uuid,
        config_hash: &str,
    ) -> Result<Option<AgentHealthCheckRunRow>> {
        let row = sqlx::query_as::<_, AgentHealthCheckRunRow>(
            r#"
            SELECT id, org_id, public_id, agent_id, config_hash, model_id, status, summary,
                   results, error_message, started_at, completed_at, created_at, updated_at
            FROM agent_health_check_runs
            WHERE org_id = $1 AND agent_id = $2 AND config_hash = $3
            ORDER BY created_at DESC
            LIMIT 1
            "#,
        )
        .bind(org_id)
        .bind(agent_id)
        .bind(config_hash)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    /// Mark every non-terminal run (`pending`/`running`) as `failed`. Called
    /// once on server boot: a fresh process has no run in flight, so any such
    /// row is an orphan left by a previous process that died mid-run and would
    /// otherwise show a perpetual spinner. Returns the number of rows reaped.
    /// See specs/agent-checks.md (durability) and EVE-586.
    pub async fn reap_running_agent_health_check_runs(&self) -> Result<u64> {
        let result = sqlx::query(
            r#"
            UPDATE agent_health_check_runs
            SET status = 'failed',
                error_message = COALESCE(error_message, 'Run interrupted by server restart'),
                completed_at = COALESCE(completed_at, NOW()),
                updated_at = NOW()
            WHERE status IN ('pending', 'running')
            "#,
        )
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }

    pub async fn update_agent_health_check_run(
        &self,
        id: Uuid,
        input: UpdateAgentHealthCheckRunRow,
    ) -> Result<Option<AgentHealthCheckRunRow>> {
        let now = chrono::Utc::now();
        let started = match input.status.as_deref() {
            Some("running") => Some(now),
            _ => None,
        };
        let completed = match input.status.as_deref() {
            Some("completed") | Some("failed") => Some(now),
            _ => None,
        };
        let row = sqlx::query_as::<_, AgentHealthCheckRunRow>(
            r#"
            UPDATE agent_health_check_runs
            SET status = COALESCE($2, status),
                summary = COALESCE($3, summary),
                results = COALESCE($4, results),
                error_message = COALESCE($5, error_message),
                started_at = COALESCE(started_at, $6),
                completed_at = COALESCE(completed_at, $7),
                updated_at = NOW()
            WHERE id = $1
            RETURNING id, org_id, public_id, agent_id, config_hash, model_id, status, summary,
                      results, error_message, started_at, completed_at, created_at, updated_at
            "#,
        )
        .bind(id)
        .bind(&input.status)
        .bind(&input.summary)
        .bind(&input.results)
        .bind(&input.error_message)
        .bind(started)
        .bind(completed)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }
}

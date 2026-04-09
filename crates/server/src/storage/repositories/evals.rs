// Eval storage (PostgreSQL)

use anyhow::Result;
use uuid::Uuid;

use crate::storage::Database;
use crate::storage::models::*;
use crate::storage::repositories::build_search_sql;

impl Database {
    // ============================================
    // Eval CRUD
    // ============================================

    pub async fn create_eval(&self, org_id: i64, input: CreateEvalRow) -> Result<EvalRow> {
        let row = sqlx::query_as::<_, EvalRow>(
            r#"
            INSERT INTO evals (org_id, public_id, name, description, target, model_override, tags)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            RETURNING id, org_id, public_id, name, description, target,
                      model_override, tags, status, created_at, updated_at, archived_at, deleted_at
            "#,
        )
        .bind(org_id)
        .bind(&input.public_id)
        .bind(&input.name)
        .bind(&input.description)
        .bind(&input.target)
        .bind(&input.model_override)
        .bind(&input.tags)
        .fetch_one(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn get_eval_by_public_id(
        &self,
        org_id: i64,
        public_id: &str,
    ) -> Result<Option<EvalRow>> {
        let row = sqlx::query_as::<_, EvalRow>(
            r#"
            SELECT id, org_id, public_id, name, description, target,
                   model_override, tags, status, created_at, updated_at, archived_at, deleted_at
            FROM evals
            WHERE org_id = $1 AND public_id = $2
            "#,
        )
        .bind(org_id)
        .bind(public_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn list_evals(
        &self,
        org_id: i64,
        search: Option<&str>,
        include_archived: bool,
    ) -> Result<Vec<EvalRow>> {
        let (search_sql, patterns) =
            build_search_sql(search, "LOWER(name || ' ' || COALESCE(description, ''))", 2);
        let status_sql = if include_archived {
            " AND status != 'deleted'"
        } else {
            " AND status NOT IN ('archived', 'deleted')"
        };
        let sql = format!(
            r#"SELECT id, org_id, public_id, name, description, target,
                      model_override, tags, status, created_at, updated_at, archived_at, deleted_at
               FROM evals
               WHERE org_id = $1{status_sql}{search_sql}
               ORDER BY created_at DESC"#
        );
        let mut query = sqlx::query_as::<_, EvalRow>(&sql).bind(org_id);
        for pat in &patterns {
            query = query.bind(pat);
        }
        Ok(query.fetch_all(&self.pool).await?)
    }

    pub async fn update_eval(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdateEvalRow,
    ) -> Result<Option<EvalRow>> {
        let row = sqlx::query_as::<_, EvalRow>(
            r#"
            UPDATE evals
            SET
                name = COALESCE($3, name),
                description = COALESCE($4, description),
                target = COALESCE($5, target),
                model_override = COALESCE($6, model_override),
                tags = COALESCE($7, tags),
                status = COALESCE($8, status),
                updated_at = NOW()
            WHERE org_id = $1 AND id = $2
            RETURNING id, org_id, public_id, name, description, target,
                      model_override, tags, status, created_at, updated_at, archived_at, deleted_at
            "#,
        )
        .bind(org_id)
        .bind(id)
        .bind(&input.name)
        .bind(&input.description)
        .bind(&input.target)
        .bind(&input.model_override)
        .bind(&input.tags)
        .bind(&input.status)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn delete_eval(&self, org_id: i64, id: Uuid) -> Result<bool> {
        let result = sqlx::query(
            r#"
            UPDATE evals
            SET status = 'archived', archived_at = COALESCE(archived_at, NOW()), updated_at = NOW()
            WHERE org_id = $1 AND id = $2 AND status = 'active'
            "#,
        )
        .bind(org_id)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    // ============================================
    // Eval Case CRUD
    // ============================================

    pub async fn create_eval_case(
        &self,
        eval_id: Uuid,
        input: CreateEvalCaseRow,
    ) -> Result<EvalCaseRow> {
        let row = sqlx::query_as::<_, EvalCaseRow>(
            r#"
            INSERT INTO eval_cases (eval_id, public_id, name, description, target, tags, conversation, post, scorers, max_turns, timeout_seconds, position)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
            RETURNING id, eval_id, public_id, name, description, target, tags, conversation, post, scorers,
                      max_turns, timeout_seconds, position, created_at, updated_at
            "#,
        )
        .bind(eval_id)
        .bind(&input.public_id)
        .bind(&input.name)
        .bind(&input.description)
        .bind(&input.target)
        .bind(&input.tags)
        .bind(&input.conversation)
        .bind(&input.post)
        .bind(&input.scorers)
        .bind(input.max_turns)
        .bind(input.timeout_seconds)
        .bind(input.position)
        .fetch_one(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn list_eval_cases(&self, eval_id: Uuid) -> Result<Vec<EvalCaseRow>> {
        let rows = sqlx::query_as::<_, EvalCaseRow>(
            r#"
            SELECT id, eval_id, public_id, name, description, target, tags, conversation, post, scorers,
                   max_turns, timeout_seconds, position, created_at, updated_at
            FROM eval_cases
            WHERE eval_id = $1
            ORDER BY position ASC, created_at ASC
            "#,
        )
        .bind(eval_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn get_eval_case(&self, id: Uuid) -> Result<Option<EvalCaseRow>> {
        let row = sqlx::query_as::<_, EvalCaseRow>(
            r#"
            SELECT id, eval_id, public_id, name, description, target, tags, conversation, post, scorers,
                   max_turns, timeout_seconds, position, created_at, updated_at
            FROM eval_cases
            WHERE id = $1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn get_eval_case_by_public_id(
        &self,
        eval_id: Uuid,
        public_id: &str,
    ) -> Result<Option<EvalCaseRow>> {
        let row = sqlx::query_as::<_, EvalCaseRow>(
            r#"
            SELECT id, eval_id, public_id, name, description, target, tags, conversation, post, scorers,
                   max_turns, timeout_seconds, position, created_at, updated_at
            FROM eval_cases
            WHERE eval_id = $1 AND public_id = $2
            "#,
        )
        .bind(eval_id)
        .bind(public_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn update_eval_case(
        &self,
        id: Uuid,
        input: UpdateEvalCaseRow,
    ) -> Result<Option<EvalCaseRow>> {
        let row = sqlx::query_as::<_, EvalCaseRow>(
            r#"
            UPDATE eval_cases
            SET
                name = COALESCE($2, name),
                description = COALESCE($3, description),
                target = COALESCE($4, target),
                tags = COALESCE($5, tags),
                conversation = COALESCE($6, conversation),
                post = COALESCE($7, post),
                scorers = COALESCE($8, scorers),
                max_turns = COALESCE($9, max_turns),
                timeout_seconds = COALESCE($10, timeout_seconds),
                position = COALESCE($11, position),
                updated_at = NOW()
            WHERE id = $1
            RETURNING id, eval_id, public_id, name, description, target, tags, conversation, post, scorers,
                      max_turns, timeout_seconds, position, created_at, updated_at
            "#,
        )
        .bind(id)
        .bind(&input.name)
        .bind(&input.description)
        .bind(&input.target)
        .bind(&input.tags)
        .bind(&input.conversation)
        .bind(&input.post)
        .bind(&input.scorers)
        .bind(input.max_turns)
        .bind(input.timeout_seconds)
        .bind(input.position)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn delete_eval_case(&self, id: Uuid) -> Result<bool> {
        let result = sqlx::query("DELETE FROM eval_cases WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn count_eval_cases(&self, eval_id: Uuid) -> Result<i64> {
        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM eval_cases WHERE eval_id = $1")
            .bind(eval_id)
            .fetch_one(&self.pool)
            .await?;
        Ok(row.0)
    }

    // ============================================
    // Eval Run CRUD
    // ============================================

    pub async fn create_eval_run(
        &self,
        org_id: i64,
        input: CreateEvalRunRow,
    ) -> Result<EvalRunRow> {
        let row = sqlx::query_as::<_, EvalRunRow>(
            r#"
            INSERT INTO eval_runs (eval_id, org_id, public_id, target, model_override, filter_tags, triggered_by)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            RETURNING id, eval_id, org_id, public_id, target, model_override, filter_tags, status,
                      triggered_by, started_at, completed_at, summary, created_at, updated_at
            "#,
        )
        .bind(input.eval_id)
        .bind(org_id)
        .bind(&input.public_id)
        .bind(&input.target)
        .bind(&input.model_override)
        .bind(&input.filter_tags)
        .bind(&input.triggered_by)
        .fetch_one(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn list_eval_runs(&self, eval_id: Uuid) -> Result<Vec<EvalRunRow>> {
        let rows = sqlx::query_as::<_, EvalRunRow>(
            r#"
            SELECT id, eval_id, org_id, public_id, target, model_override, filter_tags, status,
                   triggered_by, started_at, completed_at, summary, created_at, updated_at
            FROM eval_runs
            WHERE eval_id = $1
            ORDER BY created_at DESC
            "#,
        )
        .bind(eval_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn get_eval_run_by_public_id(
        &self,
        org_id: i64,
        public_id: &str,
    ) -> Result<Option<EvalRunRow>> {
        let row = sqlx::query_as::<_, EvalRunRow>(
            r#"
            SELECT id, eval_id, org_id, public_id, target, model_override, filter_tags, status,
                   triggered_by, started_at, completed_at, summary, created_at, updated_at
            FROM eval_runs
            WHERE org_id = $1 AND public_id = $2
            "#,
        )
        .bind(org_id)
        .bind(public_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn update_eval_run_status(
        &self,
        id: Uuid,
        status: &str,
        summary: Option<serde_json::Value>,
    ) -> Result<Option<EvalRunRow>> {
        let now = chrono::Utc::now();
        let started = if status == "running" { Some(now) } else { None };
        let completed = if matches!(status, "completed" | "failed" | "cancelled") {
            Some(now)
        } else {
            None
        };
        let row = sqlx::query_as::<_, EvalRunRow>(
            r#"
            UPDATE eval_runs
            SET
                status = $2,
                started_at = COALESCE(started_at, $3),
                completed_at = COALESCE(completed_at, $4),
                summary = COALESCE($5, summary),
                updated_at = NOW()
            WHERE id = $1
            RETURNING id, eval_id, org_id, public_id, target, model_override, filter_tags, status,
                      triggered_by, started_at, completed_at, summary, created_at, updated_at
            "#,
        )
        .bind(id)
        .bind(status)
        .bind(started)
        .bind(completed)
        .bind(&summary)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn get_latest_eval_run(&self, eval_id: Uuid) -> Result<Option<EvalRunRow>> {
        let row = sqlx::query_as::<_, EvalRunRow>(
            r#"
            SELECT id, eval_id, org_id, public_id, target, model_override, filter_tags, status,
                   triggered_by, started_at, completed_at, summary, created_at, updated_at
            FROM eval_runs
            WHERE eval_id = $1
            ORDER BY created_at DESC
            LIMIT 1
            "#,
        )
        .bind(eval_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    // ============================================
    // Eval Case Result CRUD
    // ============================================

    pub async fn create_eval_case_result(
        &self,
        input: CreateEvalCaseResultRow,
    ) -> Result<EvalCaseResultRow> {
        let row = sqlx::query_as::<_, EvalCaseResultRow>(
            r#"
            INSERT INTO eval_case_results (eval_run_id, eval_case_id, public_id, target, target_snapshot)
            VALUES ($1, $2, $3, $4, $5)
            RETURNING id, eval_run_id, eval_case_id, public_id, session_id, target, target_snapshot,
                      status, scores, turns, latency_ms, input_tokens, output_tokens, error_message,
                      created_at, updated_at
            "#,
        )
        .bind(input.eval_run_id)
        .bind(input.eval_case_id)
        .bind(&input.public_id)
        .bind(&input.target)
        .bind(&input.target_snapshot)
        .fetch_one(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn list_eval_case_results(
        &self,
        eval_run_id: Uuid,
    ) -> Result<Vec<EvalCaseResultRow>> {
        let rows = sqlx::query_as::<_, EvalCaseResultRow>(
            r#"
            SELECT ecr.id, ecr.eval_run_id, ecr.eval_case_id, ecr.public_id, ecr.session_id,
                   ecr.target, ecr.target_snapshot,
                   ecr.status, ecr.scores, ecr.turns, ecr.latency_ms,
                   ecr.input_tokens, ecr.output_tokens, ecr.error_message,
                   ecr.created_at, ecr.updated_at
            FROM eval_case_results ecr
            WHERE ecr.eval_run_id = $1
            ORDER BY ecr.created_at ASC
            "#,
        )
        .bind(eval_run_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn update_eval_case_result(
        &self,
        id: Uuid,
        input: UpdateEvalCaseResultRow,
    ) -> Result<Option<EvalCaseResultRow>> {
        let row = sqlx::query_as::<_, EvalCaseResultRow>(
            r#"
            UPDATE eval_case_results
            SET
                session_id = COALESCE($2, session_id),
                target = COALESCE($3, target),
                target_snapshot = COALESCE($4, target_snapshot),
                status = COALESCE($5, status),
                scores = COALESCE($6, scores),
                turns = COALESCE($7, turns),
                latency_ms = COALESCE($8, latency_ms),
                input_tokens = COALESCE($9, input_tokens),
                output_tokens = COALESCE($10, output_tokens),
                error_message = COALESCE($11, error_message),
                updated_at = NOW()
            WHERE id = $1
            RETURNING id, eval_run_id, eval_case_id, public_id, session_id, target, target_snapshot,
                      status, scores, turns, latency_ms, input_tokens, output_tokens, error_message,
                      created_at, updated_at
            "#,
        )
        .bind(id)
        .bind(input.session_id)
        .bind(&input.target)
        .bind(&input.target_snapshot)
        .bind(&input.status)
        .bind(&input.scores)
        .bind(input.turns)
        .bind(input.latency_ms)
        .bind(input.input_tokens)
        .bind(input.output_tokens)
        .bind(&input.error_message)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }
}

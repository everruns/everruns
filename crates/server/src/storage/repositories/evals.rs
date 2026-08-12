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
                      model_override, tags, status, is_built_in, created_at, updated_at, archived_at, deleted_at
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

    /// Look up the owning org for an eval by its public_id. See
    /// knowledge/security/multitenancy.md (Cross-Org Resource Resolution).
    pub async fn get_eval_organization_id(&self, public_id: &str) -> Result<Option<i64>> {
        let row: Option<(i64,)> =
            sqlx::query_as("SELECT org_id FROM evals WHERE public_id = $1 LIMIT 1")
                .bind(public_id)
                .fetch_optional(&self.pool)
                .await?;
        Ok(row.map(|(org_id,)| org_id))
    }

    pub async fn get_eval_by_public_id(
        &self,
        org_id: i64,
        public_id: &str,
    ) -> Result<Option<EvalRow>> {
        let row = sqlx::query_as::<_, EvalRow>(
            r#"
            SELECT id, org_id, public_id, name, description, target,
                   model_override, tags, status, is_built_in, created_at, updated_at, archived_at, deleted_at
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
                      model_override, tags, status, is_built_in, created_at, updated_at, archived_at, deleted_at
               FROM evals
               WHERE org_id = $1{status_sql}{search_sql}
               ORDER BY created_at DESC"#
        );
        let mut query =
            sqlx::query_as::<_, EvalRow>(sqlx::AssertSqlSafe(sql.as_str())).bind(org_id);
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
                      model_override, tags, status, is_built_in, created_at, updated_at, archived_at, deleted_at
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
            INSERT INTO eval_cases (eval_id, public_id, name, description, target, tags, conversation, post, artifacts, scorers, max_turns, timeout_seconds, position)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
            RETURNING id, eval_id, public_id, name, description, target, tags, conversation, post, artifacts, scorers,
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
        .bind(&input.artifacts)
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
            SELECT id, eval_id, public_id, name, description, target, tags, conversation, post, artifacts, scorers,
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
            SELECT id, eval_id, public_id, name, description, target, tags, conversation, post, artifacts, scorers,
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
            SELECT id, eval_id, public_id, name, description, target, tags, conversation, post, artifacts, scorers,
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
                artifacts = COALESCE($8, artifacts),
                scorers = COALESCE($9, scorers),
                max_turns = COALESCE($10, max_turns),
                timeout_seconds = COALESCE($11, timeout_seconds),
                position = COALESCE($12, position),
                updated_at = NOW()
            WHERE id = $1
            RETURNING id, eval_id, public_id, name, description, target, tags, conversation, post, artifacts, scorers,
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
        .bind(&input.artifacts)
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

    /// Count eval runs in 'pending' or 'running' state for an org.
    pub async fn count_running_eval_runs_for_org(&self, org_id: i64) -> Result<i64> {
        let row: (i64,) = sqlx::query_as(
            "SELECT COUNT(*)::bigint FROM eval_runs WHERE org_id = $1 AND status IN ('pending', 'running')",
        )
        .bind(org_id)
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
                      triggered_by, started_at, completed_at, summary, source, source_run_id, attribution, created_at, updated_at
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

    pub async fn create_eval_run_with_case_results(
        &self,
        org_id: i64,
        input: CreateEvalRunRow,
        eval_target: Option<serde_json::Value>,
        max_concurrent_runs_per_org: usize,
        max_cases_per_run: usize,
    ) -> Result<EvalRunRow> {
        let mut tx = self.pool.begin().await?;

        // Serialize quota checks per org. Without this lock, concurrent callers
        // can all observe the same active-run count before any insert commits.
        sqlx::query(
            "SELECT pg_advisory_xact_lock(hashtextextended('eval_run_quota:' || $1::text, 0))",
        )
        .bind(org_id)
        .execute(&mut *tx)
        .await?;

        let running: (i64,) = sqlx::query_as(
            "SELECT COUNT(*)::bigint FROM eval_runs WHERE org_id = $1 AND status IN ('pending', 'running')",
        )
        .bind(org_id)
        .fetch_one(&mut *tx)
        .await?;
        if running.0 >= max_concurrent_runs_per_org as i64 {
            return Err(CreateEvalRunError::TooManyConcurrentRuns {
                active: running.0,
                limit: max_concurrent_runs_per_org,
            }
            .into());
        }

        let case_count: (i64,) =
            sqlx::query_as("SELECT COUNT(*)::bigint FROM eval_cases WHERE eval_id = $1")
                .bind(input.eval_id)
                .fetch_one(&mut *tx)
                .await?;
        if case_count.0 > max_cases_per_run as i64 {
            return Err(CreateEvalRunError::TooManyCases {
                cases: case_count.0 as usize,
                limit: max_cases_per_run,
            }
            .into());
        }

        let case_ids: Vec<(Uuid,)> = sqlx::query_as(
            r#"
            SELECT id
            FROM eval_cases
            WHERE eval_id = $1
            ORDER BY position ASC, created_at ASC
            LIMIT $2
            "#,
        )
        .bind(input.eval_id)
        .bind(max_cases_per_run as i64 + 1)
        .fetch_all(&mut *tx)
        .await?;

        // Keep the selected case snapshot bounded even if cases are inserted
        // after the count query under PostgreSQL's default read-committed mode.
        if case_ids.len() > max_cases_per_run {
            return Err(CreateEvalRunError::TooManyCases {
                cases: case_ids.len(),
                limit: max_cases_per_run,
            }
            .into());
        }

        let case_ids: Vec<Uuid> = case_ids.into_iter().map(|(id,)| id).collect();
        let cases = sqlx::query_as::<_, EvalCaseRow>(
            r#"
            SELECT id, eval_id, public_id, name, description, target, tags, conversation, post, artifacts, scorers,
                   max_turns, timeout_seconds, position, created_at, updated_at
            FROM eval_cases
            WHERE eval_id = $1 AND id = ANY($2)
            ORDER BY position ASC, created_at ASC
            "#,
        )
        .bind(input.eval_id)
        .bind(&case_ids)
        .fetch_all(&mut *tx)
        .await?;

        let row = sqlx::query_as::<_, EvalRunRow>(
            r#"
            INSERT INTO eval_runs (eval_id, org_id, public_id, target, model_override, filter_tags, triggered_by)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            RETURNING id, eval_id, org_id, public_id, target, model_override, filter_tags, status,
                      triggered_by, started_at, completed_at, summary, source, source_run_id, attribution, created_at, updated_at
            "#,
        )
        .bind(input.eval_id)
        .bind(org_id)
        .bind(&input.public_id)
        .bind(&input.target)
        .bind(&input.model_override)
        .bind(&input.filter_tags)
        .bind(&input.triggered_by)
        .fetch_one(&mut *tx)
        .await?;

        for case in cases {
            let resolved = input
                .target
                .clone()
                .or(case.target.clone())
                .or(eval_target.clone())
                .ok_or(CreateEvalRunError::NoTarget)?;
            let result_uuid = Uuid::now_v7();
            let result_public_id = format!("evalresult_{:032x}", result_uuid.as_u128());
            sqlx::query(
                r#"
                INSERT INTO eval_case_results (eval_run_id, eval_case_id, public_id, target, target_snapshot, artifacts)
                VALUES ($1, $2, $3, $4, $5, $6)
                "#,
            )
            .bind(row.id)
            .bind(case.id)
            .bind(result_public_id)
            .bind(&resolved)
            .bind(&resolved)
            .bind(Option::<serde_json::Value>::None)
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(row)
    }

    /// Ingest one externally-executed eval run. Upserts the eval and its cases
    /// by name, replaces any prior run sharing `source_run_id` (idempotent
    /// re-publish), then writes a completed external run with fully-populated
    /// results. All in one transaction so a failed import leaves no partial run.
    pub async fn import_eval_run(
        &self,
        org_id: i64,
        input: ImportEvalRunInput,
    ) -> Result<EvalRunRow> {
        let mut tx = self.pool.begin().await?;

        // Serialize concurrent imports for the same (org, eval name) so two
        // publishes don't both create the eval.
        sqlx::query(
            "SELECT pg_advisory_xact_lock(hashtextextended('eval_import:' || $1::text || ':' || $2, 0))",
        )
        .bind(org_id)
        .bind(&input.eval_name)
        .execute(&mut *tx)
        .await?;

        // Upsert eval by (org, name), preferring a non-deleted one.
        let existing_eval: Option<(Uuid,)> = sqlx::query_as(
            "SELECT id FROM evals WHERE org_id = $1 AND name = $2 AND status != 'deleted' ORDER BY created_at ASC LIMIT 1",
        )
        .bind(org_id)
        .bind(&input.eval_name)
        .fetch_optional(&mut *tx)
        .await?;

        let eval_id = match existing_eval {
            Some((id,)) => id,
            None => {
                let eval_public_id = format!("eval_{:032x}", Uuid::now_v7().as_u128());
                let row: (Uuid,) = sqlx::query_as(
                    r#"
                    INSERT INTO evals (org_id, public_id, name, description, target, model_override, tags)
                    VALUES ($1, $2, $3, $4, NULL, NULL, $5)
                    RETURNING id
                    "#,
                )
                .bind(org_id)
                .bind(&eval_public_id)
                .bind(&input.eval_name)
                .bind(&input.eval_description)
                .bind(&input.eval_tags)
                .fetch_one(&mut *tx)
                .await?;
                row.0
            }
        };

        // Idempotency: replace any prior run for this eval sharing source_run_id.
        // eval_case_results cascade on eval_runs delete (migration 009).
        sqlx::query(
            "DELETE FROM eval_runs WHERE org_id = $1 AND eval_id = $2 AND source_run_id = $3",
        )
        .bind(org_id)
        .bind(eval_id)
        .bind(&input.source_run_id)
        .execute(&mut *tx)
        .await?;

        let now = chrono::Utc::now();
        let run = sqlx::query_as::<_, EvalRunRow>(
            r#"
            INSERT INTO eval_runs (eval_id, org_id, public_id, status, triggered_by, started_at, completed_at, summary, source, source_run_id, attribution)
            VALUES ($1, $2, $3, 'completed', $4, $5, $5, $6, $7, $8, $9)
            RETURNING id, eval_id, org_id, public_id, target, model_override, filter_tags, status,
                      triggered_by, started_at, completed_at, summary, source, source_run_id, attribution, created_at, updated_at
            "#,
        )
        .bind(eval_id)
        .bind(org_id)
        .bind(&input.run_public_id)
        .bind(&input.triggered_by)
        .bind(now)
        .bind(&input.summary)
        .bind(&input.source)
        .bind(&input.source_run_id)
        .bind(&input.attribution)
        .fetch_one(&mut *tx)
        .await?;

        // New cases append after existing ones for stable display order.
        let mut position =
            sqlx::query_as::<_, (i64,)>("SELECT COUNT(*) FROM eval_cases WHERE eval_id = $1")
                .bind(eval_id)
                .fetch_one(&mut *tx)
                .await?
                .0 as i32;

        for case in input.cases {
            // Upsert case by (eval, name). External cases are identity-only:
            // empty scorers, conversation kept only for display.
            let existing_case: Option<(Uuid,)> = sqlx::query_as(
                "SELECT id FROM eval_cases WHERE eval_id = $1 AND name = $2 LIMIT 1",
            )
            .bind(eval_id)
            .bind(&case.case_name)
            .fetch_optional(&mut *tx)
            .await?;

            let case_id = match existing_case {
                Some((id,)) => id,
                None => {
                    let case_public_id = format!("evalcase_{:032x}", Uuid::now_v7().as_u128());
                    let row: (Uuid,) = sqlx::query_as(
                        r#"
                        INSERT INTO eval_cases (eval_id, public_id, name, description, target, tags, conversation, post, artifacts, scorers, max_turns, timeout_seconds, position)
                        VALUES ($1, $2, $3, $4, NULL, '{}', $5, NULL, NULL, '[]'::jsonb, NULL, NULL, $6)
                        RETURNING id
                        "#,
                    )
                    .bind(eval_id)
                    .bind(&case_public_id)
                    .bind(&case.case_name)
                    .bind(&case.case_description)
                    .bind(&case.conversation)
                    .bind(position)
                    .fetch_one(&mut *tx)
                    .await?;
                    position += 1;
                    row.0
                }
            };

            let result_public_id = format!("evalresult_{:032x}", Uuid::now_v7().as_u128());
            sqlx::query(
                r#"
                INSERT INTO eval_case_results (eval_run_id, eval_case_id, public_id, target, target_snapshot, status, scores, metadata, turns, latency_ms, input_tokens, output_tokens, error_message, artifacts)
                VALUES ($1, $2, $3, $4, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
                "#,
            )
            .bind(run.id)
            .bind(case_id)
            .bind(result_public_id)
            .bind(&case.target_snapshot)
            .bind(&case.status)
            .bind(&case.scores)
            .bind(&case.metadata)
            .bind(case.turns)
            .bind(case.latency_ms)
            .bind(case.input_tokens)
            .bind(case.output_tokens)
            .bind(&case.error_message)
            .bind(&case.artifacts)
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(run)
    }

    pub async fn list_eval_runs(&self, eval_id: Uuid) -> Result<Vec<EvalRunRow>> {
        let rows = sqlx::query_as::<_, EvalRunRow>(
            r#"
            SELECT id, eval_id, org_id, public_id, target, model_override, filter_tags, status,
                   triggered_by, started_at, completed_at, summary, source, source_run_id, attribution, created_at, updated_at
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
                   triggered_by, started_at, completed_at, summary, source, source_run_id, attribution, created_at, updated_at
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
                      triggered_by, started_at, completed_at, summary, source, source_run_id, attribution, created_at, updated_at
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
                   triggered_by, started_at, completed_at, summary, source, source_run_id, attribution, created_at, updated_at
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
            INSERT INTO eval_case_results (eval_run_id, eval_case_id, public_id, target, target_snapshot, artifacts)
            VALUES ($1, $2, $3, $4, $5, $6)
            RETURNING id, eval_run_id, eval_case_id, public_id, session_id, target, target_snapshot,
                      status, scores, metadata, turns, latency_ms, input_tokens, output_tokens, error_message, artifacts,
                      created_at, updated_at
            "#,
        )
        .bind(input.eval_run_id)
        .bind(input.eval_case_id)
        .bind(&input.public_id)
        .bind(&input.target)
        .bind(&input.target_snapshot)
        .bind(&input.artifacts)
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
                   ecr.status, ecr.scores, ecr.metadata, ecr.turns, ecr.latency_ms,
                   ecr.input_tokens, ecr.output_tokens, ecr.error_message, ecr.artifacts,
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
                metadata = COALESCE($7, metadata),
                turns = COALESCE($8, turns),
                latency_ms = COALESCE($9, latency_ms),
                input_tokens = COALESCE($10, input_tokens),
                output_tokens = COALESCE($11, output_tokens),
                error_message = COALESCE($12, error_message),
                artifacts = COALESCE($13, artifacts),
                updated_at = NOW()
            WHERE id = $1
            RETURNING id, eval_run_id, eval_case_id, public_id, session_id, target, target_snapshot,
                      status, scores, metadata, turns, latency_ms, input_tokens, output_tokens, error_message, artifacts,
                      created_at, updated_at
            "#,
        )
        .bind(id)
        .bind(input.session_id)
        .bind(&input.target)
        .bind(&input.target_snapshot)
        .bind(&input.status)
        .bind(&input.scores)
        .bind(&input.metadata)
        .bind(input.turns)
        .bind(input.latency_ms)
        .bind(input.input_tokens)
        .bind(input.output_tokens)
        .bind(&input.error_message)
        .bind(&input.artifacts)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    // ============================================
    // Eval Run Dataset (async export handles — knowledge/evaluation/dataset-export.md)
    // ============================================

    pub async fn create_eval_run_dataset(
        &self,
        org_id: i64,
        input: CreateEvalRunDatasetRow,
    ) -> Result<(EvalRunDatasetRow, bool)> {
        let inserted_id = sqlx::query_scalar::<_, Uuid>(
            r#"
            INSERT INTO eval_run_datasets (org_id, public_id, eval_run_id, request)
            VALUES ($1, $2, $3, $4)
            ON CONFLICT (org_id, eval_run_id, request) WHERE eval_run_id IS NOT NULL
            DO NOTHING
            RETURNING id
            "#,
        )
        .bind(org_id)
        .bind(&input.public_id)
        .bind(input.eval_run_id)
        .bind(&input.request)
        .fetch_optional(&self.pool)
        .await?;
        let created = inserted_id.is_some();
        let row = sqlx::query_as::<_, EvalRunDatasetRow>(
            r#"
            SELECT id, org_id, public_id, eval_run_id, request, status, body, record_count,
                   error_message, started_at, completed_at, created_at, updated_at
            FROM eval_run_datasets
            WHERE org_id = $1 AND eval_run_id = $2 AND request = $3
            "#,
        )
        .bind(org_id)
        .bind(input.eval_run_id)
        .bind(&input.request)
        .fetch_one(&self.pool)
        .await?;
        Ok((row, created))
    }

    pub async fn find_eval_run_dataset_by_request(
        &self,
        org_id: i64,
        eval_run_id: Uuid,
        request: &serde_json::Value,
    ) -> Result<Option<EvalRunDatasetRow>> {
        let row = sqlx::query_as::<_, EvalRunDatasetRow>(
            r#"
            SELECT id, org_id, public_id, eval_run_id, request, status, body, record_count,
                   error_message, started_at, completed_at, created_at, updated_at
            FROM eval_run_datasets
            WHERE org_id = $1 AND eval_run_id = $2 AND request = $3
            "#,
        )
        .bind(org_id)
        .bind(eval_run_id)
        .bind(request)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn get_eval_run_dataset(
        &self,
        org_id: i64,
        public_id: &str,
    ) -> Result<Option<EvalRunDatasetRow>> {
        let row = sqlx::query_as::<_, EvalRunDatasetRow>(
            r#"
            SELECT id, org_id, public_id, eval_run_id, request, status, body, record_count,
                   error_message, started_at, completed_at, created_at, updated_at
            FROM eval_run_datasets
            WHERE org_id = $1 AND public_id = $2
            "#,
        )
        .bind(org_id)
        .bind(public_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn update_eval_run_dataset(
        &self,
        id: Uuid,
        input: UpdateEvalRunDatasetRow,
    ) -> Result<Option<EvalRunDatasetRow>> {
        let now = chrono::Utc::now();
        let started = match input.status.as_deref() {
            Some("running") => Some(now),
            _ => None,
        };
        let completed = match input.status.as_deref() {
            Some("completed") | Some("failed") => Some(now),
            _ => None,
        };
        let row = sqlx::query_as::<_, EvalRunDatasetRow>(
            r#"
            UPDATE eval_run_datasets
            SET status = COALESCE($2, status),
                body = COALESCE($3, body),
                record_count = COALESCE($4, record_count),
                error_message = COALESCE($5, error_message),
                started_at = COALESCE(started_at, $6),
                completed_at = COALESCE(completed_at, $7),
                updated_at = NOW()
            WHERE id = $1
            RETURNING id, org_id, public_id, eval_run_id, request, status, body, record_count,
                      error_message, started_at, completed_at, created_at, updated_at
            "#,
        )
        .bind(id)
        .bind(&input.status)
        .bind(&input.body)
        .bind(input.record_count)
        .bind(&input.error_message)
        .bind(started)
        .bind(completed)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    // ============================================
    // Eval Run Share Tokens (migration 091)
    // ============================================

    pub async fn create_eval_run_share_token(
        &self,
        org_id: i64,
        input: CreateEvalRunShareTokenRow,
    ) -> Result<EvalRunShareTokenRow> {
        let row = sqlx::query_as::<_, EvalRunShareTokenRow>(
            r#"
            INSERT INTO eval_run_share_tokens
                (org_id, public_id, eval_run_id, token_hash, token_prefix, created_by, expires_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            RETURNING id, org_id, public_id, eval_run_id, token_hash, token_prefix, created_by,
                      expires_at, revoked_at, created_at, updated_at
            "#,
        )
        .bind(org_id)
        .bind(&input.public_id)
        .bind(input.eval_run_id)
        .bind(&input.token_hash)
        .bind(&input.token_prefix)
        .bind(input.created_by)
        .bind(input.expires_at)
        .fetch_one(&self.pool)
        .await?;
        Ok(row)
    }

    /// Revoke every currently-active share token for a run. Returns how many were
    /// revoked. Org-scoped so one org can't revoke another's shares.
    pub async fn revoke_eval_run_share_tokens(
        &self,
        org_id: i64,
        eval_run_id: Uuid,
    ) -> Result<u64> {
        let result = sqlx::query(
            r#"
            UPDATE eval_run_share_tokens
            SET revoked_at = NOW(), updated_at = NOW()
            WHERE org_id = $1 AND eval_run_id = $2 AND revoked_at IS NULL
            "#,
        )
        .bind(org_id)
        .bind(eval_run_id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }

    /// Whether a run currently has an active (non-revoked, non-expired) share.
    pub async fn eval_run_has_active_share(&self, org_id: i64, eval_run_id: Uuid) -> Result<bool> {
        let row: (bool,) = sqlx::query_as(
            r#"
            SELECT EXISTS(
                SELECT 1 FROM eval_run_share_tokens
                WHERE org_id = $1 AND eval_run_id = $2
                  AND revoked_at IS NULL AND (expires_at IS NULL OR expires_at > NOW())
            )
            "#,
        )
        .bind(org_id)
        .bind(eval_run_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(row.0)
    }

    /// Resolve a share token by its hash (public path — NOT org-scoped; the org
    /// is read from the row). Returns the row even if revoked/expired so the
    /// caller can distinguish and return a uniform 404.
    pub async fn get_eval_run_share_token_by_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<EvalRunShareTokenRow>> {
        let row = sqlx::query_as::<_, EvalRunShareTokenRow>(
            r#"
            SELECT id, org_id, public_id, eval_run_id, token_hash, token_prefix, created_by,
                   expires_at, revoked_at, created_at, updated_at
            FROM eval_run_share_tokens
            WHERE token_hash = $1
            "#,
        )
        .bind(token_hash)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    /// Load an eval run by internal id (used by the public share resolver, which
    /// has the run id from the token row, not a public id + org).
    pub async fn get_eval_run_by_id(&self, id: Uuid) -> Result<Option<EvalRunRow>> {
        let row = sqlx::query_as::<_, EvalRunRow>(
            r#"
            SELECT id, eval_id, org_id, public_id, target, model_override, filter_tags, status,
                   triggered_by, started_at, completed_at, summary, source, source_run_id, attribution,
                   created_at, updated_at
            FROM eval_runs
            WHERE id = $1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }
}

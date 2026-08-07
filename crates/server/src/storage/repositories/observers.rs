// Observer storage (PostgreSQL) — see knowledge/evaluation/online-evals.md.
//
// trace_scores doubles as the scoring queue: claim_trace_scores uses
// FOR UPDATE SKIP LOCKED so multiple server instances never double-claim,
// and stale 'scoring' rows (crashed worker) are reclaimed until the
// attempt budget is exhausted.

use anyhow::Result;
use uuid::Uuid;

use crate::storage::Database;
use crate::storage::models::*;

impl Database {
    // ============================================
    // Observer CRUD
    // ============================================

    pub async fn create_observer(
        &self,
        org_id: i64,
        input: CreateObserverRow,
    ) -> Result<ObserverRow> {
        let row = sqlx::query_as::<_, ObserverRow>(
            r#"
            INSERT INTO observers (org_id, public_id, name, description, match_config, sampling_rate, scorers)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            RETURNING id, org_id, public_id, name, description, match_config,
                      sampling_rate, scorers, status, created_at, updated_at, archived_at
            "#,
        )
        .bind(org_id)
        .bind(&input.public_id)
        .bind(&input.name)
        .bind(&input.description)
        .bind(&input.match_config)
        .bind(input.sampling_rate)
        .bind(&input.scorers)
        .fetch_one(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn get_observer_by_public_id(
        &self,
        org_id: i64,
        public_id: &str,
    ) -> Result<Option<ObserverRow>> {
        let row = sqlx::query_as::<_, ObserverRow>(
            r#"
            SELECT id, org_id, public_id, name, description, match_config,
                   sampling_rate, scorers, status, created_at, updated_at, archived_at
            FROM observers
            WHERE org_id = $1 AND public_id = $2
            "#,
        )
        .bind(org_id)
        .bind(public_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn get_observer(&self, id: Uuid) -> Result<Option<ObserverRow>> {
        let row = sqlx::query_as::<_, ObserverRow>(
            r#"
            SELECT id, org_id, public_id, name, description, match_config,
                   sampling_rate, scorers, status, created_at, updated_at, archived_at
            FROM observers
            WHERE id = $1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn list_observers(
        &self,
        org_id: i64,
        include_archived: bool,
    ) -> Result<Vec<ObserverRow>> {
        let status_sql = if include_archived {
            " AND status != 'deleted'"
        } else {
            " AND status NOT IN ('archived', 'deleted')"
        };
        let sql = format!(
            r#"SELECT id, org_id, public_id, name, description, match_config,
                      sampling_rate, scorers, status, created_at, updated_at, archived_at
               FROM observers
               WHERE org_id = $1{status_sql}
               ORDER BY created_at DESC"#
        );
        let rows = sqlx::query_as::<_, ObserverRow>(sqlx::AssertSqlSafe(sql.as_str()))
            .bind(org_id)
            .fetch_all(&self.pool)
            .await?;
        Ok(rows)
    }

    /// Active observers for an org — the per-turn matching lookup.
    pub async fn list_active_observers(&self, org_id: i64) -> Result<Vec<ObserverRow>> {
        let rows = sqlx::query_as::<_, ObserverRow>(
            r#"
            SELECT id, org_id, public_id, name, description, match_config,
                   sampling_rate, scorers, status, created_at, updated_at, archived_at
            FROM observers
            WHERE org_id = $1 AND status = 'active'
            "#,
        )
        .bind(org_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn count_active_observers(&self, org_id: i64) -> Result<i64> {
        let row: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM observers WHERE org_id = $1 AND status IN ('active', 'paused')",
        )
        .bind(org_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(row.0)
    }

    pub async fn update_observer(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdateObserverRow,
    ) -> Result<Option<ObserverRow>> {
        let row = sqlx::query_as::<_, ObserverRow>(
            r#"
            UPDATE observers
            SET
                name = COALESCE($3, name),
                description = COALESCE($4, description),
                match_config = COALESCE($5, match_config),
                sampling_rate = COALESCE($6, sampling_rate),
                scorers = COALESCE($7, scorers),
                status = COALESCE($8, status),
                updated_at = NOW()
            WHERE org_id = $1 AND id = $2
            RETURNING id, org_id, public_id, name, description, match_config,
                      sampling_rate, scorers, status, created_at, updated_at, archived_at
            "#,
        )
        .bind(org_id)
        .bind(id)
        .bind(&input.name)
        .bind(&input.description)
        .bind(&input.match_config)
        .bind(input.sampling_rate)
        .bind(&input.scorers)
        .bind(&input.status)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn delete_observer(&self, org_id: i64, id: Uuid) -> Result<bool> {
        let result = sqlx::query(
            r#"
            UPDATE observers
            SET status = 'archived', archived_at = COALESCE(archived_at, NOW()), updated_at = NOW()
            WHERE org_id = $1 AND id = $2 AND status IN ('active', 'paused')
            "#,
        )
        .bind(org_id)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    // ============================================
    // Trace scores (queue + results)
    // ============================================

    /// Enqueue pending scores. Idempotent: re-delivered events hit the
    /// (observer_id, scorer_key, session_id, turn_id) unique key and no-op.
    pub async fn enqueue_trace_scores(
        &self,
        org_id: i64,
        inputs: &[CreateTraceScoreRow],
    ) -> Result<u64> {
        let mut inserted = 0u64;
        for input in inputs {
            let result = sqlx::query(
                r#"
                INSERT INTO trace_scores (org_id, public_id, observer_id, scorer_key, session_id,
                                          turn_id, agent_id, agent_version_id, harness_id)
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
                ON CONFLICT (observer_id, scorer_key, session_id, turn_id) DO NOTHING
                "#,
            )
            .bind(org_id)
            .bind(&input.public_id)
            .bind(input.observer_id)
            .bind(&input.scorer_key)
            .bind(input.session_id)
            .bind(&input.turn_id)
            .bind(input.agent_id)
            .bind(input.agent_version_id)
            .bind(input.harness_id)
            .execute(&self.pool)
            .await?;
            inserted += result.rows_affected();
        }
        Ok(inserted)
    }

    /// Claim up to `limit` queued scores for processing. Picks up fresh
    /// `pending` rows plus stale `scoring` rows (worker died mid-flight)
    /// older than `stale_after_seconds`, while attempts remain. Rows that
    /// exhausted their attempt budget are finalized as errored first.
    pub async fn claim_trace_scores(
        &self,
        limit: i64,
        stale_after_seconds: i64,
        max_attempts: i32,
    ) -> Result<Vec<TraceScoreRow>> {
        sqlx::query(
            r#"
            UPDATE trace_scores
            SET status = 'errored', error_message = 'scoring attempts exhausted', updated_at = NOW()
            WHERE status = 'scoring'
              AND updated_at < NOW() - make_interval(secs => $1)
              AND attempts >= $2
            "#,
        )
        .bind(stale_after_seconds as f64)
        .bind(max_attempts)
        .execute(&self.pool)
        .await?;

        let rows = sqlx::query_as::<_, TraceScoreRow>(
            r#"
            UPDATE trace_scores
            SET status = 'scoring', attempts = attempts + 1, updated_at = NOW()
            WHERE id IN (
                SELECT id FROM trace_scores
                WHERE (status = 'pending'
                       OR (status = 'scoring' AND updated_at < NOW() - make_interval(secs => $1)))
                  AND attempts < $2
                ORDER BY created_at
                LIMIT $3
                FOR UPDATE SKIP LOCKED
            )
            RETURNING id, org_id, public_id, observer_id, scorer_key, session_id,
                      turn_id, agent_id, agent_version_id, harness_id, status, attempts,
                      pass, value, label, reason, judge_input_tokens, judge_output_tokens,
                      judge_cost_usd, error_message, created_at, updated_at
            "#,
        )
        .bind(stale_after_seconds as f64)
        .bind(max_attempts)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn complete_trace_score(
        &self,
        id: Uuid,
        input: CompleteTraceScoreRow,
    ) -> Result<Option<TraceScoreRow>> {
        let row = sqlx::query_as::<_, TraceScoreRow>(
            r#"
            UPDATE trace_scores
            SET status = $2, pass = $3, value = $4, label = $5, reason = $6, judge_input_tokens = $7,
                judge_output_tokens = $8, judge_cost_usd = $9, error_message = $10, updated_at = NOW()
            WHERE id = $1
            RETURNING id, org_id, public_id, observer_id, scorer_key, session_id,
                      turn_id, agent_id, agent_version_id, harness_id, status, attempts,
                      pass, value, label, reason, judge_input_tokens, judge_output_tokens,
                      judge_cost_usd, error_message, created_at, updated_at
            "#,
        )
        .bind(id)
        .bind(&input.status)
        .bind(input.pass)
        .bind(input.value)
        .bind(&input.label)
        .bind(&input.reason)
        .bind(input.judge_input_tokens)
        .bind(input.judge_output_tokens)
        .bind(input.judge_cost_usd)
        .bind(&input.error_message)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn list_trace_scores(
        &self,
        org_id: i64,
        params: ListTraceScoresParams,
    ) -> Result<Vec<TraceScoreRow>> {
        let session_sql = if params.session_id.is_some() {
            " AND session_id = $5"
        } else {
            ""
        };
        let sql = format!(
            r#"SELECT id, org_id, public_id, observer_id, scorer_key, session_id,
                      turn_id, agent_id, agent_version_id, harness_id, status, attempts,
                      pass, value, label, reason, judge_input_tokens, judge_output_tokens,
                      judge_cost_usd, error_message, created_at, updated_at
               FROM trace_scores
               WHERE org_id = $1 AND observer_id = $2{session_sql}
               ORDER BY created_at DESC
               LIMIT $3 OFFSET $4"#
        );
        let mut query = sqlx::query_as::<_, TraceScoreRow>(sqlx::AssertSqlSafe(sql.as_str()))
            .bind(org_id)
            .bind(params.observer_id)
            .bind(params.limit)
            .bind(params.offset);
        if let Some(session_id) = params.session_id {
            query = query.bind(session_id);
        }
        Ok(query.fetch_all(&self.pool).await?)
    }
}

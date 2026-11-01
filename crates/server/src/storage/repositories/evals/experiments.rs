// Experiment storage: one dataset version against several variants, each an
// ordinary eval run tagged with the experiment.

use super::rows::*;
use anyhow::Result;
use everruns_server_macros::sql;
use uuid::Uuid;

use crate::storage::Database;

impl Database {
    pub async fn create_eval_experiment(
        &self,
        org_id: i64,
        input: CreateEvalExperimentRow,
    ) -> Result<EvalExperimentRow> {
        Ok(sqlx::query_as::<_, EvalExperimentRow>(sql!(
            r#"
            INSERT INTO eval_experiments (id, public_id, org_id, eval_id, name, dataset_version,
                trials, max_cost_usd, baseline_label, created_by_user_id)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            RETURNING {EvalExperimentRow}
            "#
        ))
        .bind(input.id)
        .bind(&input.public_id)
        .bind(org_id)
        .bind(input.eval_id)
        .bind(&input.name)
        .bind(input.dataset_version)
        .bind(input.trials)
        .bind(input.max_cost_usd)
        .bind(&input.baseline_label)
        .bind(input.created_by_user_id)
        .fetch_one(&self.pool)
        .await?)
    }

    pub async fn get_eval_experiment_by_public_id(
        &self,
        org_id: i64,
        public_id: &str,
    ) -> Result<Option<EvalExperimentRow>> {
        Ok(sqlx::query_as::<_, EvalExperimentRow>(sql!(
            r#"
            SELECT {EvalExperimentRow} FROM eval_experiments
            WHERE org_id = $1 AND public_id = $2
            "#
        ))
        .bind(org_id)
        .bind(public_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    /// The org's experiments, newest first, optionally of one eval.
    pub async fn list_eval_experiments(
        &self,
        org_id: i64,
        eval_id: Option<Uuid>,
        limit: i64,
    ) -> Result<Vec<EvalExperimentRow>> {
        Ok(sqlx::query_as::<_, EvalExperimentRow>(sql!(
            r#"
            SELECT {EvalExperimentRow} FROM eval_experiments
            WHERE org_id = $1 AND ($2::uuid IS NULL OR eval_id = $2)
            ORDER BY created_at DESC
            LIMIT $3
            "#
        ))
        .bind(org_id)
        .bind(eval_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?)
    }

    /// Tag a run as one variant of an experiment.
    pub async fn tag_eval_run_variant(
        &self,
        run_id: Uuid,
        experiment_id: Uuid,
        label: &str,
    ) -> Result<()> {
        sqlx::query("UPDATE eval_runs SET experiment_id = $2, variant_label = $3 WHERE id = $1")
            .bind(run_id)
            .bind(experiment_id)
            .bind(label)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// The runs of one experiment, oldest (the baseline) first.
    pub async fn list_eval_experiment_runs(&self, experiment_id: Uuid) -> Result<Vec<EvalRunRow>> {
        Ok(sqlx::query_as::<_, EvalRunRow>(sql!(
            r#"
            SELECT {EvalRunRow} FROM eval_runs
            WHERE experiment_id = $1
            ORDER BY created_at, id
            "#
        ))
        .bind(experiment_id)
        .fetch_all(&self.pool)
        .await?)
    }

    /// The runs of several experiments, for list views.
    pub async fn list_runs_of_eval_experiments(
        &self,
        experiment_ids: &[Uuid],
    ) -> Result<Vec<EvalRunRow>> {
        Ok(sqlx::query_as::<_, EvalRunRow>(sql!(
            r#"
            SELECT {EvalRunRow} FROM eval_runs
            WHERE experiment_id = ANY($1)
            ORDER BY created_at, id
            "#
        ))
        .bind(experiment_ids)
        .fetch_all(&self.pool)
        .await?)
    }
}

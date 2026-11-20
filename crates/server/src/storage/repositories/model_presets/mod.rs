// PostgreSQL repository: custom model presets (knowledge/foundations/model-presets.md).

pub(super) mod rows;
use rows::*;

use super::Database;
use anyhow::Result;
use uuid::Uuid;

const COLUMNS: &str = "id, org_id, key, name, description, default_model_id, chat_model_id, \
    coding_model_id, research_model_id, small_model_id, decision_model_id, created_at, updated_at";

impl Database {
    /// Live custom presets of an organization, by name.
    pub async fn list_model_presets(&self, org_id: i64) -> Result<Vec<ModelPresetRow>> {
        Ok(sqlx::query_as::<_, ModelPresetRow>(sqlx::AssertSqlSafe(format!(
            "SELECT {COLUMNS} FROM model_presets WHERE org_id = $1 AND deleted_at IS NULL ORDER BY lower(name), key"
        )))
        .bind(org_id)
        .fetch_all(&self.pool)
        .await?)
    }

    /// A live custom preset by key.
    pub async fn get_model_preset(&self, org_id: i64, key: &str) -> Result<Option<ModelPresetRow>> {
        Ok(sqlx::query_as::<_, ModelPresetRow>(sqlx::AssertSqlSafe(format!(
            "SELECT {COLUMNS} FROM model_presets WHERE org_id = $1 AND key = $2 AND deleted_at IS NULL"
        )))
        .bind(org_id)
        .bind(key)
        .fetch_optional(&self.pool)
        .await?)
    }

    pub async fn create_model_preset(
        &self,
        org_id: i64,
        key: &str,
        name: &str,
        description: Option<&str>,
        jobs: &ModelPresetJobsInput,
    ) -> Result<ModelPresetRow> {
        Ok(
            sqlx::query_as::<_, ModelPresetRow>(sqlx::AssertSqlSafe(format!(
                "INSERT INTO model_presets (id, org_id, key, name, description, default_model_id, \
             chat_model_id, coding_model_id, research_model_id, small_model_id, decision_model_id) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11) RETURNING {COLUMNS}"
            )))
            .bind(Uuid::now_v7())
            .bind(org_id)
            .bind(key)
            .bind(name)
            .bind(description)
            .bind(jobs.default_model_id.map(|id| id.uuid()))
            .bind(jobs.chat_model_id.map(|id| id.uuid()))
            .bind(jobs.coding_model_id.map(|id| id.uuid()))
            .bind(jobs.research_model_id.map(|id| id.uuid()))
            .bind(jobs.small_model_id.map(|id| id.uuid()))
            .bind(jobs.decision_model_id.map(|id| id.uuid()))
            .fetch_one(&self.pool)
            .await?,
        )
    }

    /// Replace a preset's name, description and jobs.
    pub async fn update_model_preset(
        &self,
        org_id: i64,
        key: &str,
        name: &str,
        description: Option<&str>,
        jobs: &ModelPresetJobsInput,
    ) -> Result<Option<ModelPresetRow>> {
        Ok(
            sqlx::query_as::<_, ModelPresetRow>(sqlx::AssertSqlSafe(format!(
                "UPDATE model_presets SET name = $3, description = $4, default_model_id = $5, \
             chat_model_id = $6, coding_model_id = $7, research_model_id = $8, \
             small_model_id = $9, decision_model_id = $10, updated_at = now() \
             WHERE org_id = $1 AND key = $2 AND deleted_at IS NULL RETURNING {COLUMNS}"
            )))
            .bind(org_id)
            .bind(key)
            .bind(name)
            .bind(description)
            .bind(jobs.default_model_id.map(|id| id.uuid()))
            .bind(jobs.chat_model_id.map(|id| id.uuid()))
            .bind(jobs.coding_model_id.map(|id| id.uuid()))
            .bind(jobs.research_model_id.map(|id| id.uuid()))
            .bind(jobs.small_model_id.map(|id| id.uuid()))
            .bind(jobs.decision_model_id.map(|id| id.uuid()))
            .fetch_optional(&self.pool)
            .await?,
        )
    }

    /// Soft delete: the record is kept, its key becomes free.
    pub async fn delete_model_preset(&self, org_id: i64, key: &str) -> Result<bool> {
        Ok(sqlx::query(
            "UPDATE model_presets SET deleted_at = now(), updated_at = now() \
             WHERE org_id = $1 AND key = $2 AND deleted_at IS NULL",
        )
        .bind(org_id)
        .bind(key)
        .execute(&self.pool)
        .await?
        .rows_affected()
            > 0)
    }

    /// How many live agents follow each preset key.
    pub async fn count_agents_by_model_preset(&self, org_id: i64) -> Result<Vec<(String, i64)>> {
        Ok(sqlx::query_as::<_, (String, i64)>(
            "SELECT model_preset, COUNT(*) FROM agents WHERE org_id = $1 \
             AND model_preset IS NOT NULL AND deleted_at IS NULL AND archived_at IS NULL \
             GROUP BY model_preset",
        )
        .bind(org_id)
        .fetch_all(&self.pool)
        .await?)
    }
}

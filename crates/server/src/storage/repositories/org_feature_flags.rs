// PostgreSQL repository: per-organization feature flag opt-ins

use super::Database;
use anyhow::Result;
use std::collections::HashMap;

impl Database {
    pub async fn list_org_feature_flags(&self, org_id: i64) -> Result<HashMap<String, bool>> {
        // Read-through cache (EVE-637): this read happens on essentially every
        // org-scoped request; a hit avoids the SELECT entirely. The write path
        // (`replace_org_feature_flags`) invalidates the same instance.
        let pool = self.pool.clone();
        self.org_feature_flags_cache
            .get_or_load(org_id, move || async move {
                let rows = sqlx::query_as::<_, (String, bool)>(
                    "SELECT flag_name, enabled FROM org_feature_flags WHERE org_id = $1",
                )
                .bind(org_id)
                .fetch_all(&pool)
                .await?;

                Ok(rows.into_iter().collect())
            })
            .await
    }

    pub async fn replace_org_feature_flags(
        &self,
        org_id: i64,
        flags: &HashMap<String, bool>,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;

        for (flag_name, enabled) in flags {
            if *enabled {
                sqlx::query(
                    r#"
                    INSERT INTO org_feature_flags (org_id, flag_name, enabled)
                    VALUES ($1, $2, TRUE)
                    ON CONFLICT (org_id, flag_name) DO UPDATE SET
                        enabled = TRUE,
                        updated_at = NOW()
                    "#,
                )
                .bind(org_id)
                .bind(flag_name)
                .execute(&mut *tx)
                .await?;
            } else {
                sqlx::query("DELETE FROM org_feature_flags WHERE org_id = $1 AND flag_name = $2")
                    .bind(org_id)
                    .bind(flag_name)
                    .execute(&mut *tx)
                    .await?;
            }
        }

        tx.commit().await?;
        // Invalidate the read-through cache so the next read reflects the write
        // on this instance immediately (cross-instance staleness is bounded by
        // the cache TTL). See `storage::org_feature_flags_cache`.
        self.org_feature_flags_cache.invalidate(org_id).await;
        Ok(())
    }
}

// PostgreSQL repository: per-organization feature flag opt-ins

use super::Database;
use anyhow::Result;
use std::collections::HashMap;

impl Database {
    pub async fn list_org_feature_flags(&self, org_id: i64) -> Result<HashMap<String, bool>> {
        let rows = sqlx::query_as::<_, (String, bool)>(
            "SELECT flag_name, enabled FROM org_feature_flags WHERE org_id = $1",
        )
        .bind(org_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().collect())
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
        Ok(())
    }
}

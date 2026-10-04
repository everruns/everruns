use super::*;
impl StorageBackend {
    pub async fn get_decision_default(&self, org_id: i64) -> Result<Option<Uuid>> {
        match self {
            Self::Postgres(db) => Ok(sqlx::query_scalar(
                "SELECT model_id FROM decision_model_defaults WHERE org_id = $1",
            )
            .bind(org_id)
            .fetch_optional(db.pool())
            .await?),
            Self::InMemory(db) => Ok(db.decision_defaults.read().get(&org_id).copied()),
        }
    }
    pub async fn set_decision_default(&self, org_id: i64, model_id: Option<Uuid>) -> Result<()> {
        if let Some(id) = model_id
            && self.get_model(org_id, id).await?.is_none()
        {
            anyhow::bail!("Decision model not found");
        }
        match self {
            Self::Postgres(db) => {
                if let Some(id) = model_id {
                    sqlx::query("INSERT INTO decision_model_defaults (org_id, model_id) VALUES ($1, $2) ON CONFLICT (org_id) DO UPDATE SET model_id = EXCLUDED.model_id").bind(org_id).bind(id).execute(db.pool()).await?;
                } else {
                    sqlx::query("DELETE FROM decision_model_defaults WHERE org_id = $1")
                        .bind(org_id)
                        .execute(db.pool())
                        .await?;
                }
            }
            Self::InMemory(db) => {
                if let Some(id) = model_id {
                    db.decision_defaults.write().insert(org_id, id);
                } else {
                    db.decision_defaults.write().remove(&org_id);
                }
            }
        }
        Ok(())
    }
}

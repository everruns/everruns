use super::*;
impl StorageBackend {
    /// The organization's default image model, if one is chosen.
    pub async fn get_image_default(&self, org_id: i64) -> Result<Option<Uuid>> {
        let db = self.database();
        Ok(
            sqlx::query_scalar("SELECT model_id FROM image_model_defaults WHERE org_id = $1")
                .bind(org_id)
                .fetch_optional(db.tx_pool())
                .await?,
        )
    }

    /// Choose (`Some`) or clear (`None`) the organization's default image model.
    pub async fn set_image_default(&self, org_id: i64, model_id: Option<Uuid>) -> Result<()> {
        if let Some(id) = model_id
            && self.get_model(org_id, id).await?.is_none()
        {
            anyhow::bail!("Image model not found");
        }
        let db = self.database();
        if let Some(id) = model_id {
            sqlx::query("INSERT INTO image_model_defaults (org_id, model_id) VALUES ($1, $2) ON CONFLICT (org_id) DO UPDATE SET model_id = EXCLUDED.model_id")
                .bind(org_id)
                .bind(id)
                .execute(db.tx_pool())
                .await?;
        } else {
            sqlx::query("DELETE FROM image_model_defaults WHERE org_id = $1")
                .bind(org_id)
                .execute(db.tx_pool())
                .await?;
        }
        Ok(())
    }
}

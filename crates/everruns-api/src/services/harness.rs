// Harness service for business logic (M2)

use anyhow::Result;
use everruns_contracts::{Harness, HarnessStatus};
use everruns_storage::{
    models::{CreateHarness, UpdateHarness},
    Database,
};
use std::sync::Arc;
use uuid::Uuid;

pub struct HarnessService {
    db: Arc<Database>,
}

impl HarnessService {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }

    pub async fn create(&self, input: CreateHarness) -> Result<Harness> {
        let row = self.db.create_harness(input).await?;
        Ok(Self::row_to_harness(row))
    }

    pub async fn get(&self, id: Uuid) -> Result<Option<Harness>> {
        let row = self.db.get_harness(id).await?;
        Ok(row.map(Self::row_to_harness))
    }

    pub async fn get_by_slug(&self, slug: &str) -> Result<Option<Harness>> {
        let row = self.db.get_harness_by_slug(slug).await?;
        Ok(row.map(Self::row_to_harness))
    }

    pub async fn list(&self) -> Result<Vec<Harness>> {
        let rows = self.db.list_harnesses().await?;
        Ok(rows.into_iter().map(Self::row_to_harness).collect())
    }

    pub async fn update(&self, id: Uuid, input: UpdateHarness) -> Result<Option<Harness>> {
        let row = self.db.update_harness(id, input).await?;
        Ok(row.map(Self::row_to_harness))
    }

    pub async fn delete(&self, id: Uuid) -> Result<bool> {
        self.db.delete_harness(id).await
    }

    fn row_to_harness(row: everruns_storage::HarnessRow) -> Harness {
        Harness {
            id: row.id,
            slug: row.slug,
            display_name: row.display_name,
            description: row.description,
            system_prompt: row.system_prompt,
            default_model_id: row.default_model_id,
            temperature: row.temperature,
            max_tokens: row.max_tokens,
            tags: row.tags,
            status: row.status.parse().unwrap_or(HarnessStatus::Active),
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

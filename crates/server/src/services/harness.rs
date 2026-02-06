// Harness service for business logic
//
// Manages harness CRUD operations, capability lifecycle.

use crate::api::harnesses::{CreateHarnessRequest, UpdateHarnessRequest};
use crate::storage::{
    HarnessRow, StorageBackend,
    models::{CreateHarnessRow, UpdateHarness},
};
use anyhow::Result;
use everruns_core::{AgentCapabilityConfig, Harness, HarnessId, HarnessStatus};
use std::sync::Arc;
use uuid::Uuid;

pub struct HarnessService {
    db: Arc<StorageBackend>,
}

impl HarnessService {
    pub fn new(db: Arc<StorageBackend>) -> Self {
        Self { db }
    }

    pub async fn create(&self, org_id: i64, req: CreateHarnessRequest) -> Result<Harness> {
        let input = CreateHarnessRow {
            name: req.name,
            description: req.description,
            system_prompt: req.system_prompt,
            default_model_id: req.default_model_id,
            tags: req.tags,
        };
        let row = self.db.create_harness(org_id, input).await?;
        let harness_id = row.id;

        // Set capabilities if provided
        let capabilities = if !req.capabilities.is_empty() {
            let cap_tuples: Vec<(String, i32, serde_json::Value)> = req
                .capabilities
                .iter()
                .enumerate()
                .map(|(idx, cap)| {
                    (
                        cap.capability_ref.to_string(),
                        idx as i32,
                        cap.config.clone(),
                    )
                })
                .collect();
            self.db
                .set_harness_capabilities(harness_id.uuid(), cap_tuples)
                .await?;
            req.capabilities
        } else {
            vec![]
        };

        Ok(Self::row_to_harness(row, capabilities))
    }

    pub async fn get(&self, org_id: i64, id: Uuid) -> Result<Option<Harness>> {
        let row = self
            .db
            .get_harness(org_id, HarnessId::from_uuid(id))
            .await?;
        match row {
            Some(row) => {
                let capabilities = self.get_capabilities(id).await?;
                Ok(Some(Self::row_to_harness(row, capabilities)))
            }
            None => Ok(None),
        }
    }

    pub async fn list(&self, org_id: i64) -> Result<Vec<Harness>> {
        let rows = self.db.list_harnesses(org_id).await?;

        let mut harnesses = Vec::with_capacity(rows.len());
        for row in rows {
            let capabilities = self.get_capabilities(row.id.uuid()).await?;
            harnesses.push(Self::row_to_harness(row, capabilities));
        }

        Ok(harnesses)
    }

    pub async fn update(
        &self,
        org_id: i64,
        id: Uuid,
        req: UpdateHarnessRequest,
    ) -> Result<Option<Harness>> {
        let input = UpdateHarness {
            name: req.name,
            description: req.description,
            system_prompt: req.system_prompt,
            default_model_id: req.default_model_id,
            tags: req.tags,
            status: req.status.map(|s| s.to_string()),
        };
        let row = self
            .db
            .update_harness(org_id, HarnessId::from_uuid(id), input)
            .await?;

        match row {
            Some(row) => {
                let capabilities = if let Some(caps) = req.capabilities {
                    let cap_tuples: Vec<(String, i32, serde_json::Value)> = caps
                        .iter()
                        .enumerate()
                        .map(|(idx, cap)| {
                            (
                                cap.capability_ref.to_string(),
                                idx as i32,
                                cap.config.clone(),
                            )
                        })
                        .collect();
                    self.db.set_harness_capabilities(id, cap_tuples).await?;
                    caps
                } else {
                    self.get_capabilities(id).await?
                };

                Ok(Some(Self::row_to_harness(row, capabilities)))
            }
            None => Ok(None),
        }
    }

    pub async fn delete(&self, org_id: i64, id: Uuid) -> Result<bool> {
        self.db
            .delete_harness(org_id, HarnessId::from_uuid(id))
            .await
    }

    async fn get_capabilities(&self, harness_id: Uuid) -> Result<Vec<AgentCapabilityConfig>> {
        let rows = self.db.get_harness_capabilities(harness_id).await?;
        Ok(rows
            .into_iter()
            .map(|row| AgentCapabilityConfig::with_config(row.capability_id, row.config))
            .collect())
    }

    fn row_to_harness(row: HarnessRow, capabilities: Vec<AgentCapabilityConfig>) -> Harness {
        Harness {
            id: row.id,
            name: row.name,
            description: row.description,
            system_prompt: row.system_prompt,
            default_model_id: row.default_model_id,
            tags: row.tags,
            capabilities,
            status: HarnessStatus::from(row.status.as_str()),
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

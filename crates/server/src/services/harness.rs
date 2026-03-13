// Harness service for business logic
//
// Manages harness CRUD operations, capability lifecycle.
// Policy enforcement via #[policy] macro — see specs/permissions.md.

use crate::api::harnesses::{CreateHarnessRequest, UpdateHarnessRequest};
use crate::storage::{
    HarnessRow, StorageBackend,
    models::{CreateHarnessRow, UpdateHarness},
};
use anyhow::Result;
use everruns_core::{
    AgentCapabilityConfig, Caller, Harness, HarnessId, HarnessStatus, Permission, Policy, Rule,
};
use everruns_macros::policy;
use std::sync::Arc;
use uuid::Uuid;

/// Policy: CRUD on harnesses (create, read, update, copy).
pub const HARNESS_MANAGE: Policy = Policy {
    id: "harness.manage",
    rules: &[Rule::UserHasPermission(Permission::OrgHarnessesManage)],
};

/// Policy: Dangerous harness operations (delete).
pub const HARNESS_DANGEROUS: Policy = Policy {
    id: "harness.dangerous",
    rules: &[
        Rule::UserHasPermission(Permission::OrgHarnessesManage),
        Rule::UserHasPermission(Permission::OrgHarnessesDangerous),
    ],
};

pub struct HarnessService {
    db: Arc<StorageBackend>,
}

impl HarnessService {
    pub fn new(db: Arc<StorageBackend>) -> Self {
        Self { db }
    }

    #[policy(HARNESS_MANAGE)]
    pub async fn create(&self, caller: &Caller, req: CreateHarnessRequest) -> Result<Harness> {
        let input = CreateHarnessRow {
            name: req.name,
            description: req.description,
            system_prompt: req.system_prompt,
            default_model_id: req.default_model_id,
            tags: req.tags,
            is_built_in: false,
        };
        let row = self.db.create_harness(caller.org_id, input).await?;
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

    #[policy(HARNESS_MANAGE)]
    pub async fn get(&self, caller: &Caller, id: Uuid) -> Result<Option<Harness>> {
        let row = self
            .db
            .get_harness(caller.org_id, HarnessId::from_uuid(id))
            .await?;
        match row {
            Some(row) => {
                let capabilities = self.get_capabilities(id).await?;
                Ok(Some(Self::row_to_harness(row, capabilities)))
            }
            None => Ok(None),
        }
    }

    #[policy(HARNESS_MANAGE)]
    pub async fn list(&self, caller: &Caller, search: Option<&str>) -> Result<Vec<Harness>> {
        let rows = self.db.list_harnesses(caller.org_id, search).await?;

        let mut harnesses = Vec::with_capacity(rows.len());
        for row in rows {
            let capabilities = self.get_capabilities(row.id.uuid()).await?;
            harnesses.push(Self::row_to_harness(row, capabilities));
        }

        Ok(harnesses)
    }

    #[policy(HARNESS_MANAGE)]
    pub async fn update(
        &self,
        caller: &Caller,
        id: Uuid,
        req: UpdateHarnessRequest,
    ) -> Result<Option<Harness>> {
        // Reject updates to built-in harnesses
        if self.is_built_in(caller.org_id, id).await? {
            anyhow::bail!(
                "Cannot modify built-in harness. Copy it first to create an editable version."
            );
        }

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
            .update_harness(caller.org_id, HarnessId::from_uuid(id), input)
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

    /// Copy a harness by UUID. Creates a new harness with "{name} (copy)" and
    /// duplicates description, system_prompt, default_model_id, tags, capabilities.
    #[policy(HARNESS_MANAGE)]
    pub async fn copy(&self, caller: &Caller, id: Uuid) -> Result<Option<Harness>> {
        let source = self.get(caller, id).await?;
        let Some(source) = source else {
            return Ok(None);
        };

        let req = CreateHarnessRequest {
            name: format!("{} (copy)", source.name),
            description: source.description,
            system_prompt: source.system_prompt,
            default_model_id: source.default_model_id,
            tags: source.tags,
            capabilities: source.capabilities,
        };

        let harness = self.create(caller, req).await?;
        Ok(Some(harness))
    }

    #[policy(HARNESS_DANGEROUS)]
    pub async fn delete(&self, caller: &Caller, id: Uuid) -> Result<bool> {
        // Reject deletion of built-in harnesses
        if self.is_built_in(caller.org_id, id).await? {
            anyhow::bail!("Cannot delete built-in harness.");
        }

        self.db
            .delete_harness(caller.org_id, HarnessId::from_uuid(id))
            .await
    }

    /// Check if a harness is built-in (system-managed, readonly).
    async fn is_built_in(&self, org_id: i64, id: Uuid) -> Result<bool> {
        let row = self
            .db
            .get_harness(org_id, HarnessId::from_uuid(id))
            .await?;
        Ok(row.map(|r| r.is_built_in).unwrap_or(false))
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
            is_built_in: row.is_built_in,
            status: HarnessStatus::from(row.status.as_str()),
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

// Harness service for business logic
//
// Manages harness CRUD operations, capability lifecycle.
// Policy enforcement via #[policy] macro — see specs/permissions.md.

use crate::api::harnesses::{CreateHarnessRequest, UpdateHarnessRequest};
use crate::errors::ResourceNotFoundError;
use crate::storage::{
    HarnessRow, StorageBackend,
    models::{CreateHarnessRow, UpdateHarness},
};
use anyhow::Result;
use everruns_core::{
    AgentCapabilityConfig, Caller, Harness, HarnessId, HarnessStatus, InitialFile, Permission,
    Policy, Rule,
};
use everruns_macros::policy;
use std::sync::Arc;
use uuid::Uuid;

/// Policy: View harnesses (read-only).
pub const HARNESS_VIEW: Policy = Policy {
    id: "harness.view",
    rules: &[Rule::UserHasPermission(Permission::OrgHarnessesView)],
};

/// Policy: CRUD on harnesses (create, update, copy).
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

fn ensure_file_system_capability(
    mut capabilities: Vec<AgentCapabilityConfig>,
    has_initial_files: bool,
) -> Vec<AgentCapabilityConfig> {
    if has_initial_files
        && !capabilities
            .iter()
            .any(|cap| cap.capability_id() == "session_file_system")
    {
        capabilities.insert(0, AgentCapabilityConfig::new("session_file_system"));
    }
    capabilities
}

impl HarnessService {
    pub fn new(db: Arc<StorageBackend>) -> Self {
        Self { db }
    }

    #[policy(HARNESS_MANAGE)]
    pub async fn create(&self, caller: &Caller, req: CreateHarnessRequest) -> Result<Harness> {
        let capabilities_to_store =
            ensure_file_system_capability(req.capabilities.clone(), !req.initial_files.is_empty());
        let default_model_id = self
            .validate_default_model_id(caller.org_id, req.default_model_id)
            .await?;

        let input = CreateHarnessRow {
            name: req.name,
            description: req.description,
            system_prompt: req.system_prompt,
            default_model_id,
            tags: req.tags,
            initial_files: serde_json::to_value(&req.initial_files).unwrap_or_default(),
            is_built_in: false,
        };
        let row = self.db.create_harness(caller.org_id, input).await?;
        let harness_id = row.id;

        // Set capabilities if provided
        let capabilities = if !capabilities_to_store.is_empty() {
            let cap_tuples: Vec<(String, i32, serde_json::Value)> = capabilities_to_store
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
            capabilities_to_store
        } else {
            vec![]
        };

        Ok(Self::row_to_harness(row, capabilities))
    }

    #[policy(HARNESS_VIEW)]
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

    #[policy(HARNESS_VIEW)]
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
        let existing = self
            .db
            .get_harness(caller.org_id, HarnessId::from_uuid(id))
            .await?
            .ok_or_else(|| anyhow::anyhow!("Harness not found"))?;
        let existing_initial_files: Vec<InitialFile> =
            serde_json::from_value(existing.initial_files.clone()).unwrap_or_default();
        let final_has_initial_files = req
            .initial_files
            .as_ref()
            .map(|files| !files.is_empty())
            .unwrap_or(!existing_initial_files.is_empty());

        let capabilities_override = match req.capabilities.clone() {
            Some(caps) => Some(ensure_file_system_capability(caps, final_has_initial_files)),
            None if final_has_initial_files => Some(ensure_file_system_capability(
                self.get_capabilities(id).await?,
                true,
            )),
            None => None,
        };
        let default_model_id = self
            .validate_default_model_id(caller.org_id, req.default_model_id)
            .await?;

        let input = UpdateHarness {
            name: req.name,
            description: req.description,
            system_prompt: req.system_prompt,
            default_model_id,
            tags: req.tags,
            initial_files: req
                .initial_files
                .map(|files| serde_json::to_value(&files).unwrap_or_default()),
            status: req.status.map(|s| s.to_string()),
        };
        let row = self
            .db
            .update_harness(caller.org_id, HarnessId::from_uuid(id), input)
            .await?;

        match row {
            Some(row) => {
                let capabilities = if let Some(caps) = capabilities_override {
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
            initial_files: source.initial_files,
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

    async fn validate_default_model_id(
        &self,
        org_id: i64,
        default_model_id: Option<everruns_core::ModelId>,
    ) -> Result<Option<everruns_core::ModelId>> {
        let Some(model_id) = default_model_id else {
            return Ok(None);
        };

        self.db
            .get_llm_model(org_id, model_id.uuid())
            .await?
            .ok_or_else(|| ResourceNotFoundError::new("Model"))?;

        Ok(Some(model_id))
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
            initial_files: serde_json::from_value::<Vec<InitialFile>>(row.initial_files)
                .unwrap_or_default(),
            is_built_in: row.is_built_in,
            status: HarnessStatus::from(row.status.as_str()),
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{CreateLlmModelRow, CreateLlmProviderRow, CreateOrganizationRow};
    use everruns_core::DEFAULT_ORG_ID;

    fn build_create_request(
        default_model_id: Option<everruns_core::ModelId>,
    ) -> CreateHarnessRequest {
        CreateHarnessRequest {
            name: "Test Harness".to_string(),
            description: None,
            system_prompt: "Test".to_string(),
            default_model_id,
            tags: vec![],
            capabilities: vec![],
            initial_files: vec![],
        }
    }

    fn build_update_request(
        default_model_id: Option<everruns_core::ModelId>,
    ) -> UpdateHarnessRequest {
        UpdateHarnessRequest {
            name: None,
            description: None,
            system_prompt: None,
            default_model_id,
            tags: None,
            capabilities: None,
            initial_files: None,
            status: None,
        }
    }

    async fn create_second_org(db: &StorageBackend) -> i64 {
        db.create_organization_with_id(
            2,
            CreateOrganizationRow {
                public_id: "org_2".to_string(),
                name: "Org 2".to_string(),
                created_by: None,
            },
        )
        .await
        .unwrap()
        .unwrap()
        .org_id
    }

    async fn create_model(
        db: &StorageBackend,
        org_id: i64,
        model_id: &str,
    ) -> everruns_core::ModelId {
        let provider = db
            .create_llm_provider(
                org_id,
                CreateLlmProviderRow {
                    name: format!("Provider {org_id}"),
                    provider_type: "openai".to_string(),
                    base_url: None,
                    api_key_encrypted: None,
                    settings: None,
                },
            )
            .await
            .unwrap();

        db.create_llm_model(
            org_id,
            CreateLlmModelRow {
                provider_id: provider.id,
                model_id: model_id.to_string(),
                display_name: model_id.to_string(),
                capabilities: vec![],
                installed: true,
                is_favorite: false,
                source: "manual".to_string(),
                provider_metadata: None,
            },
        )
        .await
        .unwrap()
        .id
    }

    #[tokio::test]
    async fn create_rejects_default_model_from_another_org() {
        let db = Arc::new(StorageBackend::in_memory());
        let service = HarnessService::new(db.clone());
        let caller = Caller::internal(DEFAULT_ORG_ID);
        let other_org_id = create_second_org(&db).await;
        let other_model_id = create_model(&db, other_org_id, "cross-org-model").await;

        let err = service
            .create(&caller, build_create_request(Some(other_model_id)))
            .await
            .unwrap_err();

        assert_eq!(err.to_string(), "Model not found");
    }

    #[tokio::test]
    async fn update_rejects_default_model_from_another_org() {
        let db = Arc::new(StorageBackend::in_memory());
        let service = HarnessService::new(db.clone());
        let caller = Caller::internal(DEFAULT_ORG_ID);
        let other_org_id = create_second_org(&db).await;
        let other_model_id = create_model(&db, other_org_id, "cross-org-model").await;

        let harness = service
            .create(&caller, build_create_request(None))
            .await
            .unwrap();

        let err = service
            .update(
                &caller,
                harness.id.uuid(),
                build_update_request(Some(other_model_id)),
            )
            .await
            .unwrap_err();

        assert_eq!(err.to_string(), "Model not found");
    }
}

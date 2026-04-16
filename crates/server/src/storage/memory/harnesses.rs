// In-memory storage: Harnesses, Harness Capabilities

use super::super::models::*;
use super::InMemoryDatabase;
use super::matches_search_tokens;
use anyhow::Result;
use everruns_core::HarnessId;
use uuid::Uuid;

impl InMemoryDatabase {
    // ============================================
    // Harnesses
    // ============================================

    pub async fn create_harness(&self, org_id: i64, input: CreateHarnessRow) -> Result<HarnessRow> {
        let now = Self::now();
        let id = HarnessId::new();
        let row = HarnessRow {
            id,
            org_id,
            name: input.name,
            display_name: input.display_name,
            description: input.description,
            system_prompt: input.system_prompt,
            parent_harness_id: input.parent_harness_id,
            default_model_id: input.default_model_id,
            tags: input.tags,
            initial_files: input.initial_files,
            network_access: input.network_access,
            is_built_in: input.is_built_in,
            status: "active".to_string(),
            created_at: now,
            updated_at: now,
            archived_at: None,
            deleted_at: None,
        };
        self.harnesses.write().insert(id, row.clone());
        Ok(row)
    }

    /// Create harness with a specific ID, idempotent (returns None if exists)
    /// Create or update harness with a specific ID (for seeding).
    /// Returns Some(row) if created or updated, None if unchanged.
    pub async fn create_harness_with_id(
        &self,
        org_id: i64,
        id: HarnessId,
        input: CreateHarnessRow,
    ) -> Result<Option<HarnessRow>> {
        let mut harnesses = self.harnesses.write();
        let now = Self::now();

        if let Some(existing) = harnesses.get(&id) {
            if existing.name == input.name
                && existing.display_name == input.display_name
                && existing.description == input.description
                && existing.system_prompt == input.system_prompt
                && existing.parent_harness_id == input.parent_harness_id
                && existing.tags == input.tags
                && existing.initial_files == input.initial_files
            {
                return Ok(None); // Unchanged
            }
            let row = HarnessRow {
                name: input.name,
                display_name: input.display_name,
                description: input.description,
                system_prompt: input.system_prompt,
                parent_harness_id: input.parent_harness_id,
                tags: input.tags,
                initial_files: input.initial_files,
                updated_at: now,
                ..existing.clone()
            };
            harnesses.insert(id, row.clone());
            return Ok(Some(row));
        }

        let row = HarnessRow {
            id,
            org_id,
            name: input.name,
            display_name: input.display_name,
            description: input.description,
            system_prompt: input.system_prompt,
            parent_harness_id: input.parent_harness_id,
            default_model_id: input.default_model_id,
            tags: input.tags,
            initial_files: input.initial_files,
            network_access: input.network_access,
            is_built_in: input.is_built_in,
            status: "active".to_string(),
            created_at: now,
            updated_at: now,
            archived_at: None,
            deleted_at: None,
        };
        harnesses.insert(id, row.clone());
        Ok(Some(row))
    }

    pub async fn get_harness(&self, org_id: i64, id: HarnessId) -> Result<Option<HarnessRow>> {
        Ok(self
            .harnesses
            .read()
            .get(&id)
            .filter(|h| h.org_id == org_id)
            .cloned())
    }

    pub async fn get_harness_by_name(&self, org_id: i64, name: &str) -> Result<Option<HarnessRow>> {
        Ok(self
            .harnesses
            .read()
            .values()
            .find(|h| h.org_id == org_id && h.name == name && h.status != "deleted")
            .cloned())
    }

    pub async fn list_harnesses(
        &self,
        org_id: i64,
        search: Option<&str>,
        include_archived: bool,
    ) -> Result<Vec<HarnessRow>> {
        let harnesses = self.harnesses.read();
        let mut result: Vec<_> = harnesses
            .values()
            .filter(|h| {
                h.org_id == org_id
                    && if include_archived {
                        h.status != "deleted"
                    } else {
                        h.status == "active"
                    }
            })
            .filter(|h| {
                matches_search_tokens(
                    search,
                    &[
                        h.display_name.as_deref().unwrap_or(""),
                        &h.name,
                        h.description.as_deref().unwrap_or(""),
                    ],
                )
            })
            .cloned()
            .collect();
        result.sort_by_key(|harness| std::cmp::Reverse(harness.created_at));
        Ok(result)
    }

    pub async fn update_harness(
        &self,
        org_id: i64,
        id: HarnessId,
        input: UpdateHarness,
    ) -> Result<Option<HarnessRow>> {
        let mut harnesses = self.harnesses.write();
        if let Some(harness) = harnesses.get_mut(&id).filter(|h| h.org_id == org_id) {
            if let Some(name) = input.name {
                harness.name = name;
            }
            if let Some(display_name) = input.display_name {
                harness.display_name = Some(display_name);
            }
            if let Some(description) = input.description {
                harness.description = Some(description);
            }
            if let Some(system_prompt) = input.system_prompt {
                harness.system_prompt = system_prompt;
            }
            if let Some(parent_harness_id) = input.parent_harness_id {
                harness.parent_harness_id = parent_harness_id;
            }
            if let Some(default_model_id) = input.default_model_id {
                harness.default_model_id = Some(default_model_id);
            }
            if let Some(tags) = input.tags {
                harness.tags = tags;
            }
            if let Some(initial_files) = input.initial_files {
                harness.initial_files = initial_files;
            }
            if let Some(network_access) = input.network_access {
                harness.network_access = network_access;
            }
            if let Some(status) = input.status {
                harness.status = status;
            }
            harness.updated_at = Self::now();
            return Ok(Some(harness.clone()));
        }
        Ok(None)
    }

    pub async fn delete_harness(&self, org_id: i64, id: HarnessId) -> Result<bool> {
        let mut harnesses = self.harnesses.write();
        if let Some(harness) = harnesses.get_mut(&id) {
            if harness.org_id != org_id || harness.status != "active" {
                return Ok(false);
            }
            harness.status = "archived".to_string();
            harness.archived_at = Some(Self::now());
            harness.updated_at = Self::now();
            return Ok(true);
        }
        Ok(false)
    }

    pub async fn list_child_harnesses(
        &self,
        org_id: i64,
        parent_id: HarnessId,
    ) -> Result<Vec<HarnessRow>> {
        let harnesses = self.harnesses.read();
        let mut result: Vec<_> = harnesses
            .values()
            .filter(|h| {
                h.org_id == org_id
                    && h.parent_harness_id == Some(parent_id)
                    && h.status != "deleted"
            })
            .cloned()
            .collect();
        result.sort_by_key(|invocation| std::cmp::Reverse(invocation.created_at));
        Ok(result)
    }

    pub async fn destroy_harness(&self, org_id: i64, id: HarnessId) -> Result<bool> {
        let mut harnesses = self.harnesses.write();
        if let Some(harness) = harnesses.get_mut(&id) {
            if harness.org_id != org_id || harness.status != "archived" {
                return Ok(false);
            }
            harness.status = "deleted".to_string();
            harness.deleted_at = Some(Self::now());
            harness.updated_at = Self::now();
            return Ok(true);
        }
        Ok(false)
    }

    // ============================================
    // Harness Capabilities
    // ============================================

    pub async fn get_harness_capabilities(
        &self,
        harness_id: Uuid,
    ) -> Result<Vec<HarnessCapabilityRow>> {
        let harness_id = HarnessId::from_uuid(harness_id);
        let caps = self.harness_capabilities.read();
        let mut result: Vec<_> = caps
            .iter()
            .filter(|((hid, _), _)| *hid == harness_id)
            .map(|(_, c)| c.clone())
            .collect();
        result.sort_by_key(|c| c.position);
        Ok(result)
    }

    pub async fn set_harness_capabilities(
        &self,
        harness_id: Uuid,
        capabilities: Vec<(String, i32, serde_json::Value)>,
    ) -> Result<Vec<HarnessCapabilityRow>> {
        let harness_id = HarnessId::from_uuid(harness_id);
        let now = Self::now();
        let mut caps = self.harness_capabilities.write();

        // Remove existing capabilities for this harness
        let to_remove: Vec<_> = caps
            .keys()
            .filter(|(hid, _)| *hid == harness_id)
            .cloned()
            .collect();
        for key in to_remove {
            caps.remove(&key);
        }

        // Add new capabilities
        let mut result = Vec::new();
        for (capability_id, position, config) in capabilities.into_iter() {
            let row = HarnessCapabilityRow {
                id: Uuid::now_v7(),
                harness_id,
                capability_id: capability_id.clone(),
                position,
                config,
                created_at: now,
            };
            caps.insert((harness_id, capability_id), row.clone());
            result.push(row);
        }

        Ok(result)
    }
}

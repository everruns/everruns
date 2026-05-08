// In-memory storage: Workspace Volume CRUD

use super::super::models::*;
use super::{InMemoryDatabase, matches_search_tokens};
use anyhow::{Result, bail};
use uuid::Uuid;

impl InMemoryDatabase {
    pub async fn create_volume(&self, org_id: i64, input: CreateVolumeRow) -> Result<VolumeRow> {
        let now = Self::now();
        let mut volumes = self.volumes.write();

        if volumes.values().any(|volume| {
            volume.org_id == org_id
                && volume.status != "deleted"
                && volume.name.eq_ignore_ascii_case(&input.name)
        }) {
            bail!("volume name already exists");
        }

        let id = Uuid::now_v7();
        let row = VolumeRow {
            id,
            org_id,
            public_id: input.public_id,
            name: input.name,
            description: input.description,
            source_type: input.source_type,
            source_config: input.source_config,
            is_readonly: input.is_readonly,
            sync_status: input.sync_status,
            last_synced_at: None,
            last_sync_error: None,
            owner_principal_id: input.owner_principal_id,
            resolved_owner_user_id: input.resolved_owner_user_id,
            status: "active".to_string(),
            created_at: now,
            updated_at: now,
            archived_at: None,
            deleted_at: None,
        };
        volumes.insert(id, row.clone());
        Ok(row)
    }

    pub async fn get_volume_by_public_id(
        &self,
        org_id: i64,
        public_id: &str,
    ) -> Result<Option<VolumeRow>> {
        Ok(self
            .volumes
            .read()
            .values()
            .find(|volume| {
                volume.org_id == org_id
                    && volume.public_id == public_id
                    && volume.status != "deleted"
            })
            .cloned())
    }

    pub async fn get_volume_by_id(&self, org_id: i64, id: Uuid) -> Result<Option<VolumeRow>> {
        Ok(self
            .volumes
            .read()
            .get(&id)
            .filter(|volume| volume.org_id == org_id && volume.status != "deleted")
            .cloned())
    }

    pub async fn get_volume_organization_id(&self, public_id: &str) -> Result<Option<i64>> {
        // The new postgres unique index on volumes(public_id) makes duplicate public_ids
        // impossible at the storage layer, but in-memory state can briefly contain
        // overlapping rows in tests that build fixtures across orgs. Pick the newest by
        // created_at to keep behavior deterministic across HashMap iteration orders.
        Ok(self
            .volumes
            .read()
            .values()
            .filter(|volume| volume.public_id == public_id && volume.status != "deleted")
            .max_by_key(|volume| volume.created_at)
            .map(|volume| volume.org_id))
    }

    pub async fn list_volumes(
        &self,
        org_id: i64,
        search: Option<&str>,
        include_archived: bool,
    ) -> Result<Vec<VolumeRow>> {
        let mut result: Vec<_> = self
            .volumes
            .read()
            .values()
            .filter(|volume| {
                volume.org_id == org_id
                    && if include_archived {
                        volume.status != "deleted"
                    } else {
                        volume.status == "active"
                    }
            })
            .filter(|volume| {
                matches_search_tokens(
                    search,
                    &[&volume.name, volume.description.as_deref().unwrap_or("")],
                )
            })
            .cloned()
            .collect();
        result.sort_by_key(|volume| std::cmp::Reverse(volume.created_at));
        Ok(result)
    }

    pub async fn update_volume(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdateVolume,
    ) -> Result<Option<VolumeRow>> {
        let mut volumes = self.volumes.write();
        if let Some(name) = input.name.as_ref()
            && volumes.values().any(|volume| {
                volume.org_id == org_id
                    && volume.id != id
                    && volume.status != "deleted"
                    && volume.name.eq_ignore_ascii_case(name)
            })
        {
            bail!("volume name already exists");
        }

        let Some(volume) = volumes.get_mut(&id) else {
            return Ok(None);
        };
        if volume.org_id != org_id || volume.status == "deleted" {
            return Ok(None);
        }

        if let Some(name) = input.name {
            volume.name = name;
        }
        if let Some(description) = input.description {
            volume.description = description;
        }
        if let Some(source_config) = input.source_config {
            volume.source_config = source_config;
        }
        if let Some(sync_status) = input.sync_status {
            volume.sync_status = sync_status;
        }
        if let Some(last_synced_at) = input.last_synced_at {
            volume.last_synced_at = last_synced_at;
        }
        if let Some(last_sync_error) = input.last_sync_error {
            volume.last_sync_error = last_sync_error;
        }
        if let Some(status) = input.status {
            volume.status = status.clone();
            match status.as_str() {
                "active" => volume.archived_at = None,
                "archived" => volume.archived_at = Some(Self::now()),
                "deleted" => volume.deleted_at = Some(Self::now()),
                _ => {}
            }
        }
        volume.updated_at = Self::now();
        Ok(Some(volume.clone()))
    }

    pub async fn archive_volume(&self, org_id: i64, id: Uuid) -> Result<bool> {
        let mut volumes = self.volumes.write();
        let Some(volume) = volumes.get_mut(&id) else {
            return Ok(false);
        };
        if volume.org_id != org_id || volume.status != "active" {
            return Ok(false);
        }
        volume.status = "archived".to_string();
        volume.archived_at = Some(Self::now());
        volume.updated_at = Self::now();
        Ok(true)
    }
}

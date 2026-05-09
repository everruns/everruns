// In-memory storage: Workspace Volume CRUD

use super::super::models::*;
use super::{InMemoryDatabase, matches_search_tokens};
use anyhow::{Result, bail};
use chrono::{DateTime, Duration, Utc};
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
        if let Some(source_type) = input.source_type {
            volume.source_type = source_type;
        }
        if let Some(source_config) = input.source_config {
            volume.source_config = source_config;
        }
        if let Some(is_readonly) = input.is_readonly {
            volume.is_readonly = is_readonly;
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

    pub async fn claim_next_volume_sync(&self) -> Result<Option<VolumeRow>> {
        let mut volumes = self.volumes.write();
        let Some(volume) = volumes
            .values_mut()
            .filter(|volume| {
                volume.status == "active"
                    && volume.source_type != "manual"
                    && (volume.sync_status == "pending"
                        || (volume.sync_status == "syncing"
                            && volume.updated_at < Self::now() - Duration::minutes(15))
                        || is_due_for_scheduled_sync(volume))
            })
            .min_by_key(|volume| volume.updated_at)
        else {
            return Ok(None);
        };

        volume.sync_status = "syncing".to_string();
        volume.last_sync_error = None;
        volume.updated_at = Self::now();
        Ok(Some(volume.clone()))
    }

    pub async fn complete_volume_sync(
        &self,
        volume_id: Uuid,
        claimed_at: DateTime<Utc>,
        files: Vec<CreateVolumeFileRow>,
    ) -> Result<Option<VolumeRow>> {
        let now = Self::now();
        let mut volumes = self.volumes.write();
        let Some(volume) = volumes.get_mut(&volume_id) else {
            return Ok(None);
        };
        if volume.status != "active"
            || volume.source_type == "manual"
            || volume.sync_status != "syncing"
            || volume.updated_at != claimed_at
        {
            return Ok(None);
        }

        let mut volume_files = self.volume_files.write();
        volume_files.retain(|_, file| file.volume_id != volume_id);
        for file in files {
            let id = Uuid::now_v7();
            let size_bytes = file.content.as_ref().map(|c| c.len() as i64).unwrap_or(0);
            volume_files.insert(
                id,
                VolumeFileRow {
                    id,
                    volume_id,
                    path: file.path,
                    content: file.content,
                    is_directory: file.is_directory,
                    size_bytes,
                    content_hash: file.content_hash,
                    created_at: now,
                    updated_at: now,
                },
            );
        }

        volume.sync_status = "synced".to_string();
        volume.last_synced_at = Some(now);
        volume.last_sync_error = None;
        volume.updated_at = now;
        Ok(Some(volume.clone()))
    }

    pub async fn fail_volume_sync(
        &self,
        volume_id: Uuid,
        claimed_at: DateTime<Utc>,
        error: &str,
    ) -> Result<Option<VolumeRow>> {
        let mut volumes = self.volumes.write();
        let Some(volume) = volumes.get_mut(&volume_id) else {
            return Ok(None);
        };
        if volume.status != "active"
            || volume.source_type == "manual"
            || volume.sync_status != "syncing"
            || volume.updated_at != claimed_at
        {
            return Ok(None);
        }

        volume.sync_status = "failed".to_string();
        volume.last_sync_error = Some(error.to_string());
        volume.updated_at = Self::now();
        Ok(Some(volume.clone()))
    }

    pub async fn list_all_volume_files(&self, volume_id: Uuid) -> Result<Vec<VolumeFileRow>> {
        let mut result: Vec<_> = self
            .volume_files
            .read()
            .values()
            .filter(|file| file.volume_id == volume_id)
            .cloned()
            .collect();
        result.sort_by_key(|file| file.path.clone());
        Ok(result)
    }
}

fn is_due_for_scheduled_sync(volume: &VolumeRow) -> bool {
    if volume.sync_status != "synced" && volume.sync_status != "failed" {
        return false;
    }
    let Some(interval_secs) = volume
        .source_config
        .get("sync_interval_secs")
        .and_then(|value| value.as_i64())
        .filter(|value| *value > 0)
    else {
        return false;
    };
    let anchor = volume.last_synced_at.unwrap_or(volume.updated_at);
    anchor < InMemoryDatabase::now() - Duration::seconds(interval_secs)
}

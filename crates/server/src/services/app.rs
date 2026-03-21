// App service for business logic
// Decision: channel_config secrets encrypted via EncryptionService (EVE-158)
// Decision: encrypt on write, decrypt in row_to_app; fallback to plaintext for migration

use crate::errors::ResourceNotFoundError;
use crate::storage::{
    AppRow, EncryptionService, StorageBackend,
    models::{CreateAppRow, UpdateApp},
};
use anyhow::Result;
use chrono::Utc;
use everruns_core::typed_id::{AgentId, AgentIdentityId, HarnessId};
use everruns_core::{App, AppId, AppStatus, Caller, ChannelType, Permission, Policy, Rule};
use everruns_durable::UpdateField;
use everruns_macros::policy;
use std::sync::Arc;
use uuid::Uuid;

use crate::api::apps::{CreateAppRequest, UpdateAppRequest};

/// Policy: View apps (read-only).
pub const APP_VIEW: Policy = Policy {
    id: "app.view",
    rules: &[Rule::UserHasPermission(Permission::OrgAgentsManage)],
};

/// Policy: Manage apps (create, update).
pub const APP_MANAGE: Policy = Policy {
    id: "app.manage",
    rules: &[Rule::UserHasPermission(Permission::OrgAgentsManage)],
};

/// Policy: Dangerous app operations (delete, publish, unpublish).
pub const APP_DANGEROUS: Policy = Policy {
    id: "app.dangerous",
    rules: &[
        Rule::UserHasPermission(Permission::OrgAgentsManage),
        Rule::UserHasPermission(Permission::OrgAppsDangerous),
    ],
};

pub struct AppService {
    db: Arc<StorageBackend>,
    encryption: Option<Arc<EncryptionService>>,
}

impl AppService {
    pub fn new(db: Arc<StorageBackend>, encryption: Option<Arc<EncryptionService>>) -> Self {
        Self { db, encryption }
    }

    /// Encrypt channel_config JSON to bytes. Returns None if encryption is not
    /// configured (dev/test mode) or the config is empty.
    fn encrypt_channel_config(&self, config: &serde_json::Value) -> Result<Option<Vec<u8>>> {
        if config.is_null() || (config.is_object() && config.as_object().unwrap().is_empty()) {
            return Ok(None);
        }
        let encryption = match self.encryption.as_ref() {
            Some(e) => e,
            None => return Ok(None), // No encryption in dev mode
        };
        let json = serde_json::to_string(config)?;
        Ok(Some(encryption.encrypt_string(&json)?))
    }

    /// Decrypt channel_config from encrypted bytes. Falls back to the plaintext
    /// column only for rows that haven't been migrated yet or when encryption
    /// is not configured (dev/test mode). If encrypted data is present but
    /// decryption/parsing fails, log and return null instead of silently
    /// reading from the plaintext column.
    fn decrypt_channel_config(
        &self,
        encrypted: Option<&[u8]>,
        plaintext_fallback: &serde_json::Value,
    ) -> serde_json::Value {
        match (encrypted, self.encryption.as_ref()) {
            // Pre-migration rows or dev/test mode: use plaintext column.
            (None, _) | (_, None) => plaintext_fallback.clone(),
            // Encrypted data with an available encryption service.
            (Some(data), Some(enc)) => {
                let json = match enc.decrypt_to_string(data) {
                    Ok(json) => json,
                    Err(err) => {
                        tracing::error!(
                            error = %err,
                            "Failed to decrypt channel_config; returning null instead of plaintext fallback"
                        );
                        return serde_json::Value::Null;
                    }
                };
                match serde_json::from_str(&json) {
                    Ok(val) => val,
                    Err(err) => {
                        tracing::error!(
                            error = %err,
                            "Failed to parse decrypted channel_config JSON; returning null"
                        );
                        serde_json::Value::Null
                    }
                }
            }
        }
    }

    #[policy(APP_MANAGE)]
    pub async fn create(&self, caller: &Caller, req: CreateAppRequest) -> Result<App> {
        let internal_uuid = Uuid::now_v7();
        let public_id = AppId::from_uuid(internal_uuid);

        // Validate harness and agent exist
        let harness_row = self
            .db
            .get_harness(caller.org_id, req.harness_id)
            .await?
            .ok_or_else(|| ResourceNotFoundError::new("Harness"))?;
        if harness_row.status != "active" {
            anyhow::bail!("Archived or deleted harnesses cannot be assigned");
        }
        let agent_row = self
            .db
            .get_agent_by_public_id(caller.org_id, &req.agent_id.to_string())
            .await?
            .ok_or_else(|| ResourceNotFoundError::new("Agent"))?;
        if agent_row.status != "active" {
            anyhow::bail!("Archived or deleted agents cannot be assigned");
        }
        let agent_identity_id = if let Some(identity_id) = req.agent_identity_id {
            let identity = self
                .db
                .get_agent_identity(caller.org_id, identity_id)
                .await?
                .ok_or_else(|| anyhow::anyhow!("Agent identity not found"))?;
            if identity.status != "active" {
                anyhow::bail!("Archived or deleted agent identities cannot be assigned");
            }
            Some(identity.id.uuid())
        } else {
            None
        };

        let channel_config = req.channel_config.unwrap_or_default();
        let channel_config_encrypted = self.encrypt_channel_config(&channel_config)?;

        // When encryption produced ciphertext, redact the plaintext column
        // so secrets are not stored in both columns.
        let stored_plaintext = if channel_config_encrypted.is_some() {
            serde_json::Value::Object(Default::default())
        } else {
            channel_config
        };

        let input = CreateAppRow {
            public_id: public_id.to_string(),
            name: req.name,
            description: req.description,
            harness_id: harness_row.id.uuid(),
            agent_id: agent_row.id.uuid(),
            agent_identity_id,
            channel_type: req.channel_type.to_string(),
            channel_config: stored_plaintext,
            channel_config_encrypted,
        };

        let row = self.db.create_app(caller.org_id, input).await?;
        Ok(self.row_to_app(row, caller.org_id).await)
    }

    #[policy(APP_VIEW)]
    pub async fn get_by_public_id(&self, caller: &Caller, public_id: &str) -> Result<Option<App>> {
        let row = self
            .db
            .get_app_by_public_id(caller.org_id, public_id)
            .await?;
        match row {
            Some(row) if row.status != "deleted" => {
                Ok(Some(self.row_to_app(row, caller.org_id).await))
            }
            None => Ok(None),
            Some(_) => Ok(None),
        }
    }

    /// Lookup app by public_id without org scoping (for unauthenticated webhooks).
    /// The org_id is derived from the row itself.
    pub async fn get_by_public_id_unscoped(&self, public_id: &str) -> Result<Option<App>> {
        let row = self.db.get_app_by_public_id_unscoped(public_id).await?;
        match row {
            Some(row) if row.status != "deleted" => {
                let org_id = row.org_id;
                Ok(Some(self.row_to_app(row, org_id).await))
            }
            None => Ok(None),
            Some(_) => Ok(None),
        }
    }

    #[policy(APP_VIEW)]
    pub async fn list(
        &self,
        caller: &Caller,
        search: Option<&str>,
        include_archived: bool,
    ) -> Result<Vec<App>> {
        let rows = self
            .db
            .list_apps(caller.org_id, search, include_archived)
            .await?;
        let mut apps = Vec::with_capacity(rows.len());
        for row in rows {
            apps.push(self.row_to_app(row, caller.org_id).await);
        }
        Ok(apps)
    }

    #[policy(APP_MANAGE)]
    pub async fn update(
        &self,
        caller: &Caller,
        public_id: &str,
        req: UpdateAppRequest,
    ) -> Result<Option<App>> {
        let existing = self
            .db
            .get_app_by_public_id(caller.org_id, public_id)
            .await?;
        let Some(existing) = existing else {
            return Ok(None);
        };
        if !matches!(existing.status.as_str(), "draft" | "published") {
            anyhow::bail!("Archived or deleted apps cannot be edited");
        }

        // Resolve harness/agent IDs if provided
        let harness_id = if let Some(hid) = req.harness_id {
            let h = self
                .db
                .get_harness(caller.org_id, hid)
                .await?
                .ok_or_else(|| ResourceNotFoundError::new("Harness"))?;
            if h.status != "active" {
                anyhow::bail!("Archived or deleted harnesses cannot be assigned");
            }
            Some(h.id.uuid())
        } else {
            None
        };

        let agent_id = if let Some(aid) = req.agent_id {
            let a = self
                .db
                .get_agent_by_public_id(caller.org_id, &aid.to_string())
                .await?
                .ok_or_else(|| ResourceNotFoundError::new("Agent"))?;
            if a.status != "active" {
                anyhow::bail!("Archived or deleted agents cannot be assigned");
            }
            Some(a.id.uuid())
        } else {
            None
        };

        // Encrypt channel_config. If the request provides a new config, encrypt it.
        // Otherwise, opportunistically encrypt the existing plaintext config for
        // pre-migration rows (when encryption is configured but encrypted column is NULL).
        let (channel_config, channel_config_encrypted) = if let Some(config) = req.channel_config {
            let encrypted = self.encrypt_channel_config(&config)?;
            // Redact plaintext when encryption produced ciphertext
            let stored_plaintext = if encrypted.is_some() {
                Some(serde_json::Value::Object(Default::default()))
            } else {
                Some(config)
            };
            (stored_plaintext, encrypted)
        } else if existing.channel_config_encrypted.is_none() && self.encryption.is_some() {
            // Opportunistic migration: encrypt existing plaintext on any update
            let encrypted = self.encrypt_channel_config(&existing.channel_config)?;
            let stored_plaintext = if encrypted.is_some() {
                Some(serde_json::Value::Object(Default::default()))
            } else {
                None
            };
            (stored_plaintext, encrypted)
        } else {
            (None, None)
        };

        let agent_identity_id = match req.agent_identity_id {
            UpdateField::Set(identity_id) => {
                let identity = self
                    .db
                    .get_agent_identity(caller.org_id, identity_id)
                    .await?
                    .ok_or_else(|| anyhow::anyhow!("Agent identity not found"))?;
                if identity.status != "active" {
                    anyhow::bail!("Archived or deleted agent identities cannot be assigned");
                }
                UpdateField::Set(identity.id.uuid())
            }
            UpdateField::Clear => UpdateField::Clear,
            UpdateField::Unchanged => UpdateField::Unchanged,
        };

        let input = UpdateApp {
            name: req.name,
            description: req.description,
            harness_id,
            agent_id,
            agent_identity_id,
            channel_type: req.channel_type.map(|ct| ct.to_string()),
            channel_config,
            channel_config_encrypted,
            status: req.status.map(|s| s.to_string()),
            published_at: UpdateField::Unchanged,
        };

        let row = self
            .db
            .update_app(caller.org_id, existing.id, input)
            .await?;
        match row {
            Some(row) => Ok(Some(self.row_to_app(row, caller.org_id).await)),
            None => Ok(None),
        }
    }

    #[policy(APP_DANGEROUS)]
    pub async fn delete(&self, caller: &Caller, public_id: &str) -> Result<bool> {
        let existing = self
            .db
            .get_app_by_public_id(caller.org_id, public_id)
            .await?;
        let Some(existing) = existing else {
            return Ok(false);
        };
        self.db.delete_app(caller.org_id, existing.id).await
    }

    #[policy(APP_DANGEROUS)]
    pub async fn destroy(&self, caller: &Caller, public_id: &str) -> Result<bool> {
        let existing = self
            .db
            .get_app_by_public_id(caller.org_id, public_id)
            .await?;
        let Some(existing) = existing else {
            return Ok(false);
        };
        if existing.status != "archived" {
            anyhow::bail!("App must be archived before deletion");
        }
        self.db.destroy_app(caller.org_id, existing.id).await
    }

    #[policy(APP_DANGEROUS)]
    pub async fn publish(&self, caller: &Caller, public_id: &str) -> Result<Option<App>> {
        let existing = self
            .db
            .get_app_by_public_id(caller.org_id, public_id)
            .await?;
        let Some(existing) = existing else {
            return Ok(None);
        };

        let input = UpdateApp {
            status: Some("published".to_string()),
            published_at: UpdateField::Set(Utc::now()),
            ..Default::default()
        };

        let row = self
            .db
            .update_app(caller.org_id, existing.id, input)
            .await?;
        match row {
            Some(row) => Ok(Some(self.row_to_app(row, caller.org_id).await)),
            None => Ok(None),
        }
    }

    #[policy(APP_DANGEROUS)]
    pub async fn unpublish(&self, caller: &Caller, public_id: &str) -> Result<Option<App>> {
        let existing = self
            .db
            .get_app_by_public_id(caller.org_id, public_id)
            .await?;
        let Some(existing) = existing else {
            return Ok(None);
        };

        let input = UpdateApp {
            status: Some("draft".to_string()),
            ..Default::default()
        };

        let row = self
            .db
            .update_app(caller.org_id, existing.id, input)
            .await?;
        match row {
            Some(row) => Ok(Some(self.row_to_app(row, caller.org_id).await)),
            None => Ok(None),
        }
    }

    async fn row_to_app(&self, row: AppRow, org_id: i64) -> App {
        // Harness uses HarnessId directly (no dual-ID pattern)
        let harness_id = HarnessId::from_uuid(row.harness_id);

        // Agent uses dual-ID pattern — resolve internal UUID to public_id
        let agent_id = self
            .db
            .get_agent_public_id(org_id, AgentId::from_uuid(row.agent_id))
            .await
            .ok()
            .flatten()
            .and_then(|pid| pid.parse::<AgentId>().ok())
            .unwrap_or_else(|| AgentId::from_uuid(row.agent_id));

        let public_id: AppId = row
            .public_id
            .parse()
            .unwrap_or_else(|_| AppId::from_uuid(row.id));

        App {
            public_id,
            internal_id: row.id,
            org_id,
            name: row.name,
            description: row.description,
            harness_id,
            agent_id,
            agent_identity_id: row.agent_identity_id.map(AgentIdentityId::from_uuid),
            channel_type: ChannelType::from_str_opt(&row.channel_type)
                .unwrap_or(ChannelType::Slack),
            channel_config: self.decrypt_channel_config(
                row.channel_config_encrypted.as_deref(),
                &row.channel_config,
            ),
            status: AppStatus::from(row.status.as_str()),
            published_at: row.published_at,
            created_at: row.created_at,
            updated_at: row.updated_at,
            archived_at: row.archived_at,
            deleted_at: row.deleted_at,
        }
    }
}

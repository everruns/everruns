// App query helpers — shared by commands and other domains.
//
// No policy checks, no input validation. Pure data access + mapping.

use crate::domains::common::CommandError;
use crate::services::row_to_principal;
use crate::storage::StorageBackend;
use crate::storage::encryption::EncryptionService;
use crate::storage::models::UpdateAppChannel;
use everruns_platform::{AgentVersionPolicy, App, AppChannel, AppStatus, ChannelType};
use everruns_provider::typed_id::AppId;
use everruns_provider::typed_id::{
    AgentId, AgentIdentityId, AgentVersionId, AppChannelId, HarnessId,
};
use std::sync::Arc;
use uuid::Uuid;

use super::types::{AppChannelRow, AppRow};

// ============================================================================
// Encryption helpers
// ============================================================================

/// Encrypt channel_config JSON to bytes. Returns None if encryption is not
/// configured (dev/test mode) or the config is empty.
pub fn encrypt_channel_config(
    encryption: Option<&Arc<EncryptionService>>,
    config: &serde_json::Value,
) -> anyhow::Result<Option<Vec<u8>>> {
    if config.is_null() || (config.is_object() && config.as_object().unwrap().is_empty()) {
        return Ok(None);
    }
    let encryption = match encryption {
        Some(e) => e,
        None => return Ok(None),
    };
    let json = serde_json::to_string(config)?;
    Ok(Some(encryption.encrypt_string(&json)?))
}

/// Decrypt channel_config from encrypted bytes. Falls back to the plaintext
/// column only for rows that haven't been migrated yet or when encryption
/// is not configured (dev/test mode).
pub fn decrypt_channel_config(
    encryption: Option<&Arc<EncryptionService>>,
    encrypted: Option<&[u8]>,
    plaintext_fallback: &serde_json::Value,
) -> serde_json::Value {
    match (encrypted, encryption) {
        (None, _) | (_, None) => plaintext_fallback.clone(),
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

/// Prepare channel_config for storage: encrypt if possible, redact plaintext when encrypted.
pub fn prepare_channel_config(
    encryption: Option<&Arc<EncryptionService>>,
    config: &serde_json::Value,
) -> anyhow::Result<(serde_json::Value, Option<Vec<u8>>)> {
    let encrypted = encrypt_channel_config(encryption, config)?;
    let stored_plaintext = if encrypted.is_some() {
        serde_json::Value::Object(Default::default())
    } else {
        config.clone()
    };
    Ok((stored_plaintext, encrypted))
}

// ============================================================================
// Row mapping
// ============================================================================

pub fn channel_row_to_channel(
    encryption: Option<&Arc<EncryptionService>>,
    row: AppChannelRow,
) -> AppChannel {
    let public_id: AppChannelId = row
        .public_id
        .parse()
        .unwrap_or_else(|_| AppChannelId::from_uuid(row.id));

    let channel_config = decrypt_channel_config(
        encryption,
        row.channel_config_encrypted.as_deref(),
        &row.channel_config,
    );

    AppChannel {
        public_id,
        internal_id: row.id,
        channel_type: ChannelType::from_str_opt(&row.channel_type).unwrap_or(ChannelType::Slack),
        channel_config,
        enabled: row.enabled,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

pub async fn row_to_app(
    db: &StorageBackend,
    encryption: Option<&Arc<EncryptionService>>,
    row: AppRow,
    org_id: i64,
) -> App {
    let harness_id = HarnessId::from_uuid(row.harness_id);

    let agent_id = match row.agent_id {
        Some(agent_uuid) => db
            .get_agent_public_id(org_id, AgentId::from_uuid(agent_uuid))
            .await
            .ok()
            .flatten()
            .and_then(|pid| pid.parse::<AgentId>().ok())
            .or_else(|| Some(AgentId::from_uuid(agent_uuid))),
        None => None,
    };

    let public_id: AppId = row
        .public_id
        .parse()
        .unwrap_or_else(|_| AppId::from_uuid(row.id));

    // Load channels from app_channels table; fall back to legacy columns
    let channel_rows = match db.list_app_channels(row.id).await {
        Ok(rows) => rows,
        Err(err) => {
            tracing::error!(
                app_id = %row.public_id,
                error = %err,
                "Failed to load app_channels; falling back to legacy columns"
            );
            Vec::new()
        }
    };
    let channels: Vec<AppChannel> = if channel_rows.is_empty() {
        // Fallback: synthesize a channel from legacy apps columns when no
        // app_channels rows exist (e.g. incomplete migration, DB restore).
        if let Some(ct) = row
            .channel_type
            .as_deref()
            .and_then(ChannelType::from_str_opt)
        {
            let config = decrypt_channel_config(
                encryption,
                row.channel_config_encrypted.as_deref(),
                &row.channel_config,
            );
            vec![AppChannel {
                public_id: AppChannelId::from_uuid(row.id),
                internal_id: row.id,
                channel_type: ct,
                channel_config: config,
                enabled: true,
                created_at: row.created_at,
                updated_at: row.updated_at,
            }]
        } else {
            Vec::new()
        }
    } else {
        channel_rows
            .into_iter()
            .map(|ch| channel_row_to_channel(encryption, ch))
            .collect()
    };
    let owner = match db.get_principal(org_id, row.owner_principal_id).await {
        Ok(row) => row
            .map(row_to_principal)
            .map(|principal| principal.summary()),
        Err(err) => {
            tracing::warn!(
                app_id = %row.public_id,
                owner_principal_id = %row.owner_principal_id,
                error = %err,
                "Failed to load app owner principal summary"
            );
            None
        }
    };
    let effective_owner = match row.resolved_owner_user_id {
        Some(user_id) => match db.get_principal_by_subject(org_id, "user", user_id).await {
            Ok(row) => row
                .map(row_to_principal)
                .map(|principal| principal.summary()),
            Err(err) => {
                tracing::warn!(
                    app_id = %row.public_id,
                    resolved_owner_user_id = %user_id,
                    error = %err,
                    "Failed to load app effective owner summary"
                );
                None
            }
        },
        None => None,
    };

    App {
        public_id,
        internal_id: row.id,
        org_id,
        name: row.name,
        description: row.description,
        harness_id,
        agent_id,
        agent_version_policy: AgentVersionPolicy::from(row.agent_version_policy.as_str()),
        agent_version_id: row.agent_version_id.map(AgentVersionId::from_uuid),
        agent_identity_id: row.agent_identity_id.map(AgentIdentityId::from_uuid),
        owner_principal_id: row.owner_principal_id,
        resolved_owner_user_id: row.resolved_owner_user_id,
        owner,
        effective_owner,
        channels,
        status: AppStatus::from(row.status.as_str()),
        published_at: row.published_at,
        created_at: row.created_at,
        updated_at: row.updated_at,
        archived_at: row.archived_at,
        deleted_at: row.deleted_at,
    }
}

// ============================================================================
// Data access helpers
// ============================================================================

/// Resolve app by public ID. Returns None if not found or deleted.
pub async fn get_by_public_id(
    db: &StorageBackend,
    encryption: Option<&Arc<EncryptionService>>,
    org_id: i64,
    public_id: &str,
) -> anyhow::Result<Option<App>> {
    let row = db.get_app_by_public_id(org_id, public_id).await?;
    match row {
        Some(row) if row.status != "deleted" => {
            Ok(Some(row_to_app(db, encryption, row, org_id).await))
        }
        _ => Ok(None),
    }
}

/// Resolve app by internal ID. Returns None if not found or deleted.
pub async fn get_by_internal_id(
    db: &StorageBackend,
    encryption: Option<&Arc<EncryptionService>>,
    org_id: i64,
    id: Uuid,
) -> anyhow::Result<Option<App>> {
    let row = db.get_app_by_id(org_id, id).await?;
    match row {
        Some(row) if row.status != "deleted" => {
            Ok(Some(row_to_app(db, encryption, row, org_id).await))
        }
        _ => Ok(None),
    }
}

/// Load a list of app rows into full App structs.
pub async fn load_apps_list(
    db: &StorageBackend,
    encryption: Option<&Arc<EncryptionService>>,
    rows: Vec<AppRow>,
    org_id: i64,
) -> anyhow::Result<Vec<App>> {
    let mut apps = Vec::with_capacity(rows.len());
    for row in rows {
        apps.push(row_to_app(db, encryption, row, org_id).await);
    }
    Ok(apps)
}

// ============================================================================
// Reference guards
// ============================================================================

fn referenced_app_names(apps: &[AppRow], predicate: impl Fn(&AppRow) -> bool) -> Option<String> {
    let names = apps
        .iter()
        .filter(|app| predicate(app))
        .map(|app| app.name.as_str())
        .collect::<Vec<_>>();
    if names.is_empty() {
        None
    } else {
        Some(names.join(", "))
    }
}

pub async fn ensure_no_app_references_to_agent(
    db: &StorageBackend,
    org_id: i64,
    agent_id: Uuid,
) -> Result<(), CommandError> {
    let apps = db.list_apps(org_id, None, false).await?;
    if let Some(names) = referenced_app_names(&apps, |app| app.agent_id == Some(agent_id)) {
        return Err(CommandError::conflict(format!(
            "Cannot archive or delete agent while apps still reference it: {names}"
        )));
    }
    Ok(())
}

pub async fn ensure_no_app_references_to_harness(
    db: &StorageBackend,
    org_id: i64,
    harness_id: Uuid,
) -> Result<(), CommandError> {
    let apps = db.list_apps(org_id, None, false).await?;
    if let Some(names) = referenced_app_names(&apps, |app| app.harness_id == harness_id) {
        return Err(CommandError::conflict(format!(
            "Cannot archive or delete harness while apps still reference it: {names}"
        )));
    }
    Ok(())
}

pub async fn ensure_no_app_references_to_agent_identity(
    db: &StorageBackend,
    org_id: i64,
    identity_id: Uuid,
) -> Result<(), CommandError> {
    let apps = db.list_apps(org_id, None, false).await?;
    if let Some(names) =
        referenced_app_names(&apps, |app| app.agent_identity_id == Some(identity_id))
    {
        return Err(CommandError::conflict(format!(
            "Cannot archive or delete agent identity while apps still reference it: {names}"
        )));
    }
    Ok(())
}

/// Lookup app by public_id without org scoping (for unauthenticated webhooks).
///
/// Used by Slack/AG-UI webhooks where the caller has no org context. The
/// returned `App` is populated with the owning org, so callers can still
/// enforce per-org rules downstream.
pub async fn get_by_public_id_unscoped(
    db: &StorageBackend,
    encryption: Option<&Arc<EncryptionService>>,
    public_id: &str,
) -> anyhow::Result<Option<App>> {
    let row = db.get_app_by_public_id_unscoped(public_id).await?;
    match row {
        Some(row) if row.status != "deleted" => {
            let org_id = row.org_id;
            Ok(Some(row_to_app(db, encryption, row, org_id).await))
        }
        _ => Ok(None),
    }
}

/// Update channel config with proper encryption handling.
/// Used by webhook handlers (unauthenticated, no caller context).
pub async fn update_channel_config_unscoped(
    db: &StorageBackend,
    encryption: Option<&Arc<EncryptionService>>,
    channel_internal_id: Uuid,
    config: &serde_json::Value,
) -> anyhow::Result<()> {
    let (stored_plaintext, encrypted) = prepare_channel_config(encryption, config)?;
    let input = UpdateAppChannel {
        channel_config: Some(stored_plaintext),
        channel_config_encrypted: encrypted,
        ..Default::default()
    };
    db.update_app_channel(channel_internal_id, input).await?;
    Ok(())
}

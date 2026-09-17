// App query helpers — shared by commands and other domains.
//
// No policy checks, no input validation. Pure data access + mapping.

use crate::domains::common::CommandError;
use crate::services::row_to_principal;
use crate::storage::StorageBackend;
use crate::storage::encryption::EncryptionService;
use crate::storage::models::UpdateAppChannel;
use everruns_durable::UpdateField;
use everruns_platform::{
    AgentVersionPolicy, App, AppChannel, AppEndpointAuthConfig, AppStatus, ChannelType,
    EndpointStatus,
};
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

pub struct PreparedChannelStorage {
    pub channel_config: serde_json::Value,
    pub channel_config_encrypted: Option<Vec<u8>>,
    pub auth: Option<serde_json::Value>,
    pub auth_encrypted: Option<Vec<u8>>,
}

/// Store transport configuration and endpoint auth independently. Auth may
/// contain secret-equivalent hashes and provider credentials, so it follows
/// the same encryption policy as channel configuration.
pub fn prepare_channel_storage(
    encryption: Option<&Arc<EncryptionService>>,
    config: &serde_json::Value,
) -> anyhow::Result<PreparedChannelStorage> {
    let mut transport = config.clone();
    let auth = transport
        .as_object_mut()
        .and_then(|object| object.remove("auth"))
        .filter(|value| !value.is_null());
    let (channel_config, mut channel_config_encrypted) =
        prepare_channel_config(encryption, &transport)?;
    if auth.is_some()
        && channel_config_encrypted.is_none()
        && let Some(encryption) = encryption
    {
        let json = serde_json::to_string(&transport)?;
        channel_config_encrypted = Some(encryption.encrypt_string(&json)?);
    }
    let (auth, auth_encrypted) = match auth {
        Some(auth) => {
            let encrypted = encrypt_channel_config(encryption, &auth)?;
            if encrypted.is_some() {
                (None, encrypted)
            } else {
                (Some(auth), None)
            }
        }
        None => (None, None),
    };
    Ok(PreparedChannelStorage {
        channel_config,
        channel_config_encrypted,
        auth,
        auth_encrypted,
    })
}

fn first_class_auth_value(
    encryption: Option<&Arc<EncryptionService>>,
    row: &AppChannelRow,
) -> Option<serde_json::Value> {
    if let Some(encrypted) = row.auth_encrypted.as_deref() {
        let Some(encryption) = encryption else {
            tracing::error!("Endpoint auth is encrypted but encryption is unavailable");
            return Some(serde_json::json!({"mode": "http_basic"}));
        };
        let json = match encryption.decrypt_to_string(encrypted) {
            Ok(json) => json,
            Err(err) => {
                tracing::error!(error = %err, "Failed to decrypt endpoint auth");
                return Some(serde_json::json!({"mode": "http_basic"}));
            }
        };
        return serde_json::from_str(&json)
            .map_err(|err| {
                tracing::error!(error = %err, "Failed to parse decrypted endpoint auth JSON");
            })
            .ok()
            .or_else(|| Some(serde_json::json!({"mode": "http_basic"})));
    }
    row.auth.clone()
}

pub fn decrypt_endpoint_auth(
    encryption: Option<&Arc<EncryptionService>>,
    row: &AppChannelRow,
) -> Option<AppEndpointAuthConfig> {
    first_class_auth_value(encryption, row).map(|value| {
        serde_json::from_value(value).unwrap_or_else(|err| {
            tracing::error!(error = %err, "Failed to parse endpoint auth");
            serde_json::from_value(serde_json::json!({"mode": "http_basic"}))
                .expect("fail-closed endpoint auth is valid")
        })
    })
}

/// Return a write-ready config that includes either first-class auth or the
/// encrypted legacy nested value. First-class storage is authoritative even
/// when its payload cannot be decrypted or parsed.
pub fn channel_config_with_auth(
    encryption: Option<&Arc<EncryptionService>>,
    row: &AppChannelRow,
) -> serde_json::Value {
    let mut config = decrypt_channel_config(
        encryption,
        row.channel_config_encrypted.as_deref(),
        &row.channel_config,
    );
    if (row.auth.is_some() || row.auth_encrypted.is_some())
        && let Some(object) = config.as_object_mut()
    {
        object.remove("auth");
        if let Some(auth) = first_class_auth_value(encryption, row) {
            object.insert("auth".to_string(), auth);
        }
    }
    config
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

    let mut channel_config = decrypt_channel_config(
        encryption,
        row.channel_config_encrypted.as_deref(),
        &row.channel_config,
    );
    let legacy_auth = channel_config
        .as_object_mut()
        .and_then(|object| object.remove("auth"));
    let auth = if row.auth.is_some() || row.auth_encrypted.is_some() {
        decrypt_endpoint_auth(encryption, &row).map(Box::new)
    } else {
        legacy_auth.and_then(|value| {
            serde_json::from_value(value)
                .map_err(|err| {
                    tracing::error!(error = %err, "Failed to parse legacy endpoint auth");
                })
                .ok()
        })
    };

    AppChannel {
        public_id,
        internal_id: row.id,
        channel_type: ChannelType::from_str_opt(&row.channel_type).unwrap_or(ChannelType::Slack),
        channel_config,
        auth,
        enabled: row.enabled,
        status: EndpointStatus::from(row.status.as_str()),
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
            let mut config = decrypt_channel_config(
                encryption,
                row.channel_config_encrypted.as_deref(),
                &row.channel_config,
            );
            let auth = config
                .as_object_mut()
                .and_then(|object| object.remove("auth"))
                .and_then(|value| serde_json::from_value(value).ok())
                .map(Box::new);
            vec![AppChannel {
                public_id: AppChannelId::from_uuid(row.id),
                internal_id: row.id,
                channel_type: ct,
                channel_config: config,
                auth,
                enabled: true,
                // Legacy fallback: this App predates `app_channels` rows, so the
                // only lifecycle it has is its own publish state.
                status: if row.status == "published" {
                    EndpointStatus::Live
                } else {
                    EndpointStatus::Draft
                },
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

/// Resolve an app from a globally unique channel public ID.
pub async fn get_by_channel_public_id_unscoped(
    db: &StorageBackend,
    encryption: Option<&Arc<EncryptionService>>,
    channel_public_id: &str,
) -> anyhow::Result<Option<App>> {
    let row = db
        .get_app_by_channel_public_id_unscoped(channel_public_id)
        .await?;
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
    let prepared = prepare_channel_storage(encryption, config)?;
    let input = UpdateAppChannel {
        channel_config: Some(prepared.channel_config),
        channel_config_encrypted: prepared.channel_config_encrypted,
        auth: UpdateField::from_option(prepared.auth),
        auth_encrypted: UpdateField::from_option(prepared.auth_encrypted),
        ..Default::default()
    };
    db.update_app_channel(channel_internal_id, input).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::generate_encryption_key;
    use chrono::Utc;

    fn row(
        channel_config: serde_json::Value,
        channel_config_encrypted: Option<Vec<u8>>,
        auth: Option<serde_json::Value>,
        auth_encrypted: Option<Vec<u8>>,
    ) -> AppChannelRow {
        AppChannelRow {
            id: Uuid::now_v7(),
            app_id: Uuid::now_v7(),
            public_id: AppChannelId::new().to_string(),
            channel_type: "ag_ui".to_string(),
            channel_config,
            channel_config_encrypted,
            auth,
            auth_encrypted,
            durable_schedule_id: None,
            enabled: true,
            status: "live".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn encryption() -> Arc<EncryptionService> {
        Arc::new(
            EncryptionService::new(&generate_encryption_key("endpoint-auth-test"), &[]).unwrap(),
        )
    }

    #[test]
    fn channel_storage_separates_and_encrypts_auth() {
        let encryption = encryption();
        let config = serde_json::json!({
            "anonymous": false,
            "token": "transport-secret",
            "auth": {
                "mode": "http_basic",
                "provider": {
                    "type": "http_basic",
                    "username": "operator",
                    "password_hash": "$argon2id$test"
                }
            }
        });

        let prepared = prepare_channel_storage(Some(&encryption), &config).unwrap();
        assert!(prepared.auth.is_none());
        assert!(prepared.auth_encrypted.is_some());
        let transport = decrypt_channel_config(
            Some(&encryption),
            prepared.channel_config_encrypted.as_deref(),
            &prepared.channel_config,
        );
        assert!(transport.get("auth").is_none());
        assert_eq!(transport["token"], "transport-secret");

        let auth: serde_json::Value = serde_json::from_str(
            &encryption
                .decrypt_to_string(prepared.auth_encrypted.as_deref().unwrap())
                .unwrap(),
        )
        .unwrap();
        assert_eq!(auth["provider"]["password_hash"], "$argon2id$test");
    }
    #[test]
    fn auth_only_storage_replaces_legacy_transport_with_encrypted_empty_object() {
        let encryption = encryption();
        let prepared = prepare_channel_storage(
            Some(&encryption),
            &serde_json::json!({"auth": {"mode": "anonymous"}}),
        )
        .unwrap();

        let transport = decrypt_channel_config(
            Some(&encryption),
            prepared.channel_config_encrypted.as_deref(),
            &prepared.channel_config,
        );
        assert_eq!(transport, serde_json::json!({}));
        assert!(prepared.auth_encrypted.is_some());
    }

    #[test]
    fn first_class_auth_overrides_and_removes_legacy_auth() {
        let row = row(
            serde_json::json!({
                "auth": {"mode": "anonymous"},
                "anonymous": true
            }),
            None,
            Some(serde_json::json!({"mode": "google_oidc", "provider": {
                "type": "google_oidc",
                "client_id": "client"
            }})),
            None,
        );

        let channel = channel_row_to_channel(None, row);
        assert_eq!(
            channel.auth.map(|auth| auth.mode),
            Some(everruns_platform::AppEndpointAuthMode::GoogleOidc)
        );
        assert!(channel.channel_config.get("auth").is_none());
    }

    #[test]
    fn corrupt_first_class_auth_fails_closed_without_legacy_fallback() {
        let row = row(
            serde_json::json!({"auth": {"mode": "anonymous"}}),
            None,
            None,
            Some(b"not-an-encrypted-payload".to_vec()),
        );

        let channel = channel_row_to_channel(Some(&encryption()), row);
        assert_eq!(
            channel.auth.map(|auth| auth.mode),
            Some(everruns_platform::AppEndpointAuthMode::HttpBasic)
        );
        assert!(channel.channel_config.get("auth").is_none());
    }

    #[test]
    fn encrypted_legacy_auth_is_available_for_lazy_split() {
        let encryption = encryption();
        let legacy = serde_json::json!({
            "anonymous": false,
            "auth": {"mode": "google_oidc", "provider": {
                "type": "google_oidc",
                "client_id": "legacy-client"
            }}
        });
        let encrypted = encrypt_channel_config(Some(&encryption), &legacy).unwrap();
        let row = row(serde_json::json!({}), encrypted, None, None);

        let config = channel_config_with_auth(Some(&encryption), &row);
        assert_eq!(config["auth"]["provider"]["client_id"], "legacy-client");
        let channel = channel_row_to_channel(Some(&encryption), row);
        assert_eq!(
            channel.auth.map(|auth| auth.mode),
            Some(everruns_platform::AppEndpointAuthMode::GoogleOidc)
        );
    }
}

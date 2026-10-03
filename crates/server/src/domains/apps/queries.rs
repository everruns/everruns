// Frozen App query helpers — read-only archival access and reference guards.
//
// No policy checks, no input validation. Pure data access + mapping. Endpoint
// config encryption and row mapping live in `domains::agent_channels::queries`.

use crate::domains::agent_channels::queries::{
    channel_row_to_channel, decrypt_channel_config, parse_legacy_channel_auth,
};
use crate::domains::common::CommandError;
use crate::services::row_to_principal;
use crate::storage::StorageBackend;
use crate::storage::encryption::EncryptionService;
use everruns_contracts::typed_id::AppId;
use everruns_contracts::typed_id::{AgentId, AgentVersionId, HarnessId, VirtualUserId};
use everruns_platform::{
    AgentChannel, AgentChannelId, AgentVersionPolicy, App, AppStatus, ChannelStatus, ChannelType,
};
use std::sync::Arc;
use uuid::Uuid;

use super::types::AppRow;

// ============================================================================
// Row mapping
// ============================================================================

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
    let channel_rows = match db.list_legacy_alias_channels(row.id).await {
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
    let channels: Vec<AgentChannel> = if channel_rows.is_empty() {
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
                .map(|value| Box::new(parse_legacy_channel_auth(value)));
            vec![AgentChannel {
                public_id: AgentChannelId::from_uuid(row.id),
                internal_id: row.id,
                channel_type: ct,
                channel_config: config,
                auth,
                enabled: true,
                // Legacy fallback: this App predates `app_channels` rows, so the
                // only lifecycle it has is its own publish state.
                status: if row.status == "published" {
                    ChannelStatus::Live
                } else {
                    ChannelStatus::Draft
                },
                agent_version_policy: AgentVersionPolicy::Default,
                agent_version_id: None,
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
    // The frozen App carried one version selection for all of its channels.
    let agent_version_policy = AgentVersionPolicy::from(row.agent_version_policy.as_str());
    let agent_version_id = row.agent_version_id.map(AgentVersionId::from_uuid);
    let channels: Vec<AgentChannel> = channels
        .into_iter()
        .map(|mut channel| {
            channel.agent_version_policy = agent_version_policy.clone();
            channel.agent_version_id = agent_version_id;
            channel
        })
        .collect();
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
        agent_version_policy,
        agent_version_id,
        virtual_user_id: row.virtual_user_id.map(VirtualUserId::from_uuid),
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

pub async fn ensure_no_app_references_to_virtual_user(
    db: &StorageBackend,
    org_id: i64,
    identity_id: Uuid,
) -> Result<(), CommandError> {
    let apps = db.list_apps(org_id, None, false).await?;
    if let Some(names) = referenced_app_names(&apps, |app| app.virtual_user_id == Some(identity_id))
    {
        return Err(CommandError::conflict(format!(
            "Cannot archive or delete virtual user while apps still reference it: {names}"
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

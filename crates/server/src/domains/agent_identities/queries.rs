// Agent identity query helpers — shared by commands and other domains.
//
// No policy checks, no input validation. Pure data access + mapping.

use crate::kernel_imports::{
    AgentIdentity, AgentIdentityStatus, everruns_provider::typed_id::AgentIdentityId,
};
use crate::services::row_to_principal;
use crate::storage::StorageBackend;

use super::types::AgentIdentityRow;

// ============================================================================
// Row mapping
// ============================================================================

pub async fn row_to_identity(
    db: &StorageBackend,
    org_id: i64,
    row: AgentIdentityRow,
) -> anyhow::Result<AgentIdentity> {
    let principal = db
        .get_principal_by_subject(org_id, "agent_identity", row.id.uuid())
        .await?
        .map(row_to_principal);
    let effective_owner = match principal.as_ref().and_then(|p| p.resolved_user_id) {
        Some(user_id) => db
            .get_principal_by_subject(org_id, "user", user_id)
            .await?
            .map(row_to_principal)
            .map(|principal| principal.summary()),
        None => None,
    };
    Ok(AgentIdentity {
        id: row.id,
        name: row.name,
        description: row.description,
        principal: principal.as_ref().map(|principal| principal.summary()),
        effective_owner,
        avatar_url: row.avatar_url,
        locale: row.locale,
        timezone: row.timezone,
        status: AgentIdentityStatus::from(row.status.as_str()),
        created_at: row.created_at,
        updated_at: row.updated_at,
        archived_at: row.archived_at,
        deleted_at: row.deleted_at,
    })
}

// ============================================================================
// Data access helpers
// ============================================================================

/// Load agent identity by ID, excluding deleted.
pub async fn get_by_id(
    db: &StorageBackend,
    org_id: i64,
    id: AgentIdentityId,
) -> anyhow::Result<Option<AgentIdentity>> {
    match db
        .get_agent_identity(org_id, id)
        .await?
        .filter(|row| row.status != "deleted")
    {
        Some(row) => Ok(Some(row_to_identity(db, org_id, row).await?)),
        None => Ok(None),
    }
}

// ============================================================================
// Validation helpers
// ============================================================================

pub fn validate_locale(locale: &str) -> anyhow::Result<()> {
    if !everruns_core::localization::is_allowed_locale(locale) {
        anyhow::bail!("Unsupported locale: {locale}");
    }
    Ok(())
}

pub fn validate_timezone(tz: &str) -> anyhow::Result<()> {
    if !everruns_core::localization::is_allowed_timezone(tz) {
        anyhow::bail!("Unsupported timezone: {tz}");
    }
    Ok(())
}

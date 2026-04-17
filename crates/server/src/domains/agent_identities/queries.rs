// Agent identity query helpers — shared by commands and other domains.
//
// No policy checks, no input validation. Pure data access + mapping.

use crate::storage::StorageBackend;
use everruns_core::{AgentIdentity, AgentIdentityId, AgentIdentityStatus};

use super::types::AgentIdentityRow;

// ============================================================================
// Row mapping
// ============================================================================

pub fn row_to_identity(row: AgentIdentityRow) -> AgentIdentity {
    AgentIdentity {
        id: row.id,
        name: row.name,
        description: row.description,
        avatar_url: row.avatar_url,
        locale: row.locale,
        timezone: row.timezone,
        status: AgentIdentityStatus::from(row.status.as_str()),
        created_at: row.created_at,
        updated_at: row.updated_at,
        archived_at: row.archived_at,
        deleted_at: row.deleted_at,
    }
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
    Ok(db
        .get_agent_identity(org_id, id)
        .await?
        .filter(|row| row.status != "deleted")
        .map(row_to_identity))
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
